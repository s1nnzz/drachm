//! Loops, `break` / `continue`, `if` chains, `skip` and `match`: checking
//! them, following definite assignment along every jump, and lowering them
//! to `hir::Stmt::If`, `Loop` and `Jump`.
//!
//! How `skip` counts (SPEC section 3):
//! - A *level* is a whole `if` / `else if` / `else` chain, or a whole `match`
//!   (its cases are the branches). A statement is inside a level when it is
//!   in one of the level's branch bodies.
//! - Loops are walls: only the levels inside the body of the innermost loop
//!   around the `skip` count, or, outside any loop, the levels inside the
//!   function body. Number those levels 1 (innermost) to L (outermost).
//! - `skip N;` leaves levels 1 to N and carries on after level N, skipping
//!   its remaining branches. N = 0 and N > L are errors.
//! - `skip;` is `skip L;`: it leaves the outermost level.
//! - `skip to next;` leaves the current branch of level 1 and tests the next
//!   branch: the next `else if` condition or `case` patterns, or runs the
//!   `else`. In the last branch it is an error.
//! - With no level at all (L = 0), every form of `skip` is an error.

use std::cmp::Ordering;
use std::collections::HashMap;

use num_rational::BigRational;

use super::{int_lit, origin, str_lit, Checker, Val};
use crate::ast::{self, ExprKind, Pattern, SkipTarget, StrPart};
use crate::consts::Const;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, StrFn};
use crate::types::Type;

/// Something a jump can go to.
pub(super) enum Target {
    Loop { label: hir::Label, span: Span },
    /// One `skip` level: an `if` chain or a `match`, now in branch number `branch`.
    Level {
        label: hir::Label,
        span: Span,
        is_match: bool,
        /// An `if` chain that ends in `else`.
        has_else: bool,
        branch: usize,
        branches: usize,
        /// Some `skip to next` goes to the last branch's test.
        next_to_last: bool,
    },
}

/// How a branch of a chain is chosen.
pub(super) enum Test<'e> {
    /// An `if` / `else if` condition, checked when the chain gets to it.
    Cond(&'e ast::Expr),
    /// `else` or `case _`.
    Always,
    /// A `case`'s patterns, already checked; `None` if they had an error.
    Value(Option<hir::Expr>),
    /// A `case` of an enum `match`: tests, bindings and a guard.
    Pattern(Box<super::enums::PatCase<'e>>),
}

/// The test of a `case` on a number or text: its patterns (`Some(None)` when
/// they always match, `None` after an error), then its guard if it has one.
fn guarded_test(pattern: Option<Option<hir::Expr>>, guard: Option<hir::Expr>, guarded: bool) -> Test<'static> {
    let guard = if guarded { guard } else { None };
    match (pattern, guard) {
        (None, _) => Test::Value(None),
        (Some(None), None) if !guarded => Test::Always,
        (Some(None), None) => Test::Value(None),
        (Some(Some(p)), None) => Test::Value(Some(p)),
        (Some(p), Some(g)) => Test::Value(Some(match p {
            Some(p) => logic(true, p, g),
            None => g,
        })),
    }
}

/// The constant value of a pattern, in the matched type.
#[derive(Clone, Copy)]
pub(super) enum Lit {
    Int(u128),
    Float(f64),
    Bool(bool),
}

fn local(id: hir::LocalId, ty: Type) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Local(id), ty)
}

fn binary(op: hir::BinOp, l: hir::Expr, r: hir::Expr) -> hir::Expr {
    let ty = if op.is_comparison() { Type::Bool } else { l.ty };
    hir::Expr::new(hir::ExprKind::Binary(op, Box::new(l), Box::new(r)), ty)
}

/// The direction a range loop counts in.
#[derive(Clone, Copy)]
enum Dir {
    Up,
    Down,
    /// A step known only at run time: its sign picks the way, at each round.
    Either,
}

/// The sign of a step written as a literal (`2`, `-0.5`, `0`); `None` for any
/// other step, whose sign is only known at run time.
fn literal_sign(step: &ast::Expr) -> Option<Ordering> {
    let value = |c: &crate::consts::Const| match c {
        crate::consts::Const::Int(v) => v.cmp(&num_bigint::BigInt::from(0)),
        crate::consts::Const::Dec(r) => r.cmp(&num_rational::BigRational::from_integer(num_bigint::BigInt::from(0))),
        crate::consts::Const::NegZero => Ordering::Equal,
    };
    match &step.kind {
        ExprKind::Num(n) => Some(value(&n.value)),
        ExprKind::Unary(ast::UnOp::Neg, inner) => match &inner.kind {
            ExprKind::Num(n) => Some(value(&n.value).reverse()),
            _ => None,
        },
        _ => None,
    }
}

fn not(e: hir::Expr) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Not(Box::new(e)), Type::Bool)
}

fn logic(and: bool, lhs: hir::Expr, rhs: hir::Expr) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Logic { and, lhs: Box::new(lhs), rhs: Box::new(rhs) }, Type::Bool)
}

/// A call of a string runtime function; its result has the function's type.
fn str_fn(f: StrFn, args: Vec<hir::Expr>) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::StrCall(f, args), f.ret())
}

const U64: Type = Type::Int { bits: 64, signed: false };

/// `*text` for a hidden `&string` local.
fn text_at(text: hir::LocalId, text_ty: Type) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Deref(Box::new(local(text, text_ty))), Type::Str)
}

/// The shared borrow of `place` that a hidden local holds.
fn shared_borrow(place: hir::Expr) -> hir::Expr {
    let ty = Type::borrow(false, Type::Str);
    hir::Expr::new(hir::ExprKind::Borrow { mutable: false, place: Box::new(place) }, ty)
}

fn mask(ty: Type) -> u128 {
    match ty {
        Type::Int { bits: 128, .. } => u128::MAX,
        Type::Int { bits, .. } => (1u128 << bits) - 1,
        _ => unreachable!("not an integer type"),
    }
}

impl Lit {
    pub(super) fn expr(self, ty: Type) -> hir::Expr {
        let kind = match self {
            Lit::Int(b) => hir::ExprKind::Int(b),
            Lit::Float(f) => hir::ExprKind::Float(f),
            Lit::Bool(b) => hir::ExprKind::Bool(b),
        };
        hir::Expr::new(kind, ty)
    }

    /// The value's place in an unsigned order that agrees with the type's
    /// own order, for working out which values the cases cover. Float zero
    /// and negative zero are equal, so they share a key.
    pub(super) fn key(self, ty: Type) -> u128 {
        match self {
            Lit::Int(b) => match ty {
                Type::Int { bits, signed: true } => b ^ (1u128 << (bits - 1)),
                _ => b,
            },
            Lit::Float(f) => {
                let bits = if f == 0.0 { 0 } else { f.to_bits() };
                // Positive floats order like their bits; negative ones in
                // reverse. The `+ 1` puts the smallest negative right below zero.
                (if bits >> 63 == 0 { bits | 1 << 63 } else { (!bits).wrapping_add(1) }) as u128
            }
            Lit::Bool(b) => b as u128,
        }
    }
}

/// Every key a value of `ty` can have (floats: besides NaN, which has none).
fn domain(ty: Type) -> (u128, u128) {
    match ty {
        Type::Int { .. } => (0, mask(ty)),
        Type::Bool => (0, 1),
        _ => (0, u64::MAX as u128),
    }
}

/// The value with key `k` in an integer type or `bool`, as source text.
fn describe(k: u128, ty: Type) -> String {
    match ty {
        Type::Bool => (k == 1).to_string(),
        Type::Int { bits, signed } => {
            let b = if signed { k ^ (1u128 << (bits - 1)) } else { k };
            match Const::from_int_bits(b, ty) {
                Const::Int(v) => v.to_string(),
                Const::Dec(_) | Const::NegZero => unreachable!(),
            }
        }
        _ => unreachable!("floats are never listed"),
    }
}

/// Evaluate a typed constant expression: literals, `-`, `~`, `!` and
/// conversions, with the same results the program would get at run time.
fn fold(e: hir::Expr) -> Option<Lit> {
    let ty = e.ty;
    Some(match e.kind {
        hir::ExprKind::Int(b) => Lit::Int(b),
        hir::ExprKind::Float(f) => Lit::Float(f),
        hir::ExprKind::Bool(b) => Lit::Bool(b),
        hir::ExprKind::Neg(inner) => match fold(*inner)? {
            Lit::Int(b) => Lit::Int(b.wrapping_neg() & mask(ty)),
            Lit::Float(f) => Lit::Float(-f),
            Lit::Bool(_) => return None,
        },
        hir::ExprKind::BitNot(inner) => match fold(*inner)? {
            Lit::Int(b) => Lit::Int(!b & mask(ty)),
            _ => return None,
        },
        hir::ExprKind::Not(inner) => match fold(*inner)? {
            Lit::Bool(b) => Lit::Bool(!b),
            _ => return None,
        },
        hir::ExprKind::Convert(inner) => {
            let from = inner.ty;
            let v = fold(*inner)?;
            let exact = match v {
                Lit::Int(b) => Const::from_int_bits(b, from),
                Lit::Float(f) => match BigRational::from_float(f) {
                    Some(r) => Const::Dec(r),
                    // NaN and the infinities become 0.
                    None => Const::Int(0.into()),
                },
                Lit::Bool(_) => return None,
            };
            match (ty, v) {
                (Type::Int { .. }, _) => Lit::Int(exact.wrap_to_int(ty)),
                (Type::Float(32), Lit::Float(f)) => Lit::Float(f as f32 as f64),
                (_, Lit::Float(f)) => Lit::Float(f),
                _ => Lit::Float(exact.to_float(ty)),
            }
        }
        _ => return None,
    })
}

/// The keys the cases so far handle: sorted, disjoint, non-touching ranges.
#[derive(Default)]
struct Coverage(Vec<(u128, u128)>);

impl Coverage {
    fn covers(&self, (lo, hi): (u128, u128)) -> bool {
        self.0.iter().any(|&(a, b)| a <= lo && hi <= b)
    }

    fn add(&mut self, lo: u128, hi: u128) {
        self.0.push((lo, hi));
        self.0.sort_unstable();
        let mut merged: Vec<(u128, u128)> = Vec::new();
        for (a, b) in self.0.drain(..) {
            match merged.last_mut() {
                Some(last) if a <= last.1.saturating_add(1) => last.1 = last.1.max(b),
                _ => merged.push((a, b)),
            }
        }
        self.0 = merged;
    }

    /// The ranges of keys in `lo..=hi` that are not covered.
    fn gaps(&self, (lo, hi): (u128, u128)) -> Vec<(u128, u128)> {
        let mut out = Vec::new();
        let mut next = Some(lo);
        for &(a, b) in &self.0 {
            if let Some(n) = next
                && a > n
            {
                out.push((n, a - 1));
            }
            next = b.checked_add(1);
        }
        if let Some(n) = next
            && n <= hi
        {
            out.push((n, hi));
        }
        out
    }
}

/// A `case` pattern of a `string` match: exact text, or a `globbed` pattern.
#[derive(Clone)]
pub(super) enum TextPat {
    Exact(String),
    Glob(String),
}

impl TextPat {
    /// `globbed "*"` matches every text, so its case is a catch-all.
    fn matches_all(&self) -> bool {
        matches!(self, TextPat::Glob(g) if g == "*")
    }

    /// Is every text that `p` matches also matched by `self`? Only the cases
    /// the coverage rule covers (see `match_text`).
    fn covers(&self, p: &TextPat) -> bool {
        match (self, p) {
            (TextPat::Exact(a), TextPat::Exact(b)) => a == b,
            (TextPat::Glob(g), TextPat::Exact(t)) => glob_matches(g, t),
            (TextPat::Glob(a), TextPat::Glob(b)) => a == b || a == "*",
            (TextPat::Exact(_), TextPat::Glob(_)) => false,
        }
    }
}

/// Does `text` match `pattern`, where each `*` matches any run of text (none
/// included) and the whole text must match? The runtime's `drachm_str_glob`
/// does the same on bytes; for valid UTF-8 the answers agree.
fn glob_matches(pattern: &str, text: &str) -> bool {
    let mut parts = pattern.split('*').peekable();
    let first = parts.next().unwrap_or("");
    let Some(mut rest) = text.strip_prefix(first) else { return false };
    while let Some(seg) = parts.next() {
        if parts.peek().is_none() {
            return rest.ends_with(seg);
        }
        match rest.find(seg) {
            Some(at) => rest = &rest[at + seg.len()..],
            None => return false,
        }
    }
    rest.is_empty()
}

impl Checker<'_> {
    pub(super) fn new_label(&mut self) -> hir::Label {
        self.labels += 1;
        self.labels - 1
    }

    fn target_span(&self, idx: usize) -> Span {
        match &self.targets[idx] {
            Target::Loop { span, .. } | Target::Level { span, .. } => *span,
        }
    }

    /// The label a jump to `targets[idx]` names.
    fn record(&mut self, idx: usize) -> hir::Label {
        match &self.targets[idx] {
            Target::Loop { label, .. } | Target::Level { label, .. } => *label,
        }
    }

    /// Check a chain of branches (an `if` chain or a `match`) as one `skip`
    /// level. `exhaustive` says that some branch always runs even though none
    /// is `Test::Always`: the last test is then left out, so the control-flow
    /// graph (which decides whether a function can fall off its end) has no
    /// path past the chain. Returns `None` if a test had an error.
    pub(super) fn chain(
        &mut self,
        label: hir::Label,
        span: Span,
        is_match: bool,
        parts: Vec<(Test, &ast::Block)>,
        exhaustive: bool,
    ) -> Option<Vec<hir::Branch>> {
        let branches = parts.len();
        let has_else = !is_match && matches!(parts.last(), Some((Test::Always, _)));
        self.targets.push(Target::Level { label, span, is_match, has_else, branch: 0, branches, next_to_last: false });
        let mut out = Vec::new();
        let mut ok = true;
        for (i, (test, body)) in parts.into_iter().enumerate() {
            let mut bound = None;
            let (test, always) = match test {
                Test::Cond(c) => (self.cond(c), false),
                Test::Always => (None, true),
                Test::Value(v) => (v, false),
                Test::Pattern(pc) => {
                    let pc = *pc;
                    let always = pc.test.is_none();
                    bound = Some((pc.binds, pc.pre, pc.post, pc.guard));
                    (pc.test, always)
                }
            };
            ok &= always || test.is_some();
            if let Some(Target::Level { branch, .. }) = self.targets.last_mut() {
                *branch = i;
            }
            let block_end = body.end;
            let body = match bound {
                None => self.block(body),
                Some((binds, pre, post, guard)) => {
                    self.scopes.push(HashMap::new());
                    // A guard sees what is copied out, not what is taken out afterwards.
                    for (name, id, taken) in &binds {
                        if !*taken || guard.is_none() {
                            self.scopes.last_mut().unwrap().insert(name.clone(), *id);
                        }
                    }
                    let guard = guard.map(|g| self.cond(g));
                    for (name, id, taken) in &binds {
                        if *taken {
                            self.scopes.last_mut().unwrap().insert(name.clone(), *id);
                        }
                    }
                    let user = self.block(body);
                    self.scopes.pop();
                    let mut all = pre;
                    match guard {
                        None => {
                            all.extend(post);
                            all.extend(user);
                        }
                        Some(None) => {
                            ok = false;
                            all.extend(user);
                        }
                        Some(Some(g)) => {
                            // The guard decides after the bindings exist; if it fails the next case is tried.
                            let inner = self.new_label();
                            let mut taken = post;
                            taken.extend(user);
                            taken.push(hir::Stmt::Jump(hir::Jump::Leave(label)));
                            all.push(hir::Stmt::If { label: inner, branches: vec![hir::Branch { test: Some(g), body: taken, end: block_end }] });
                            all.push(hir::Stmt::Jump(hir::Jump::Next(label, i + 1)));
                        }
                    }
                    all
                }
            };
            out.push(hir::Branch { test, body, end: block_end });
        }
        let Some(Target::Level { next_to_last, .. }) = self.targets.pop() else { unreachable!() };
        // A `skip to next` into the last test still needs it.
        if exhaustive && ok && !next_to_last {
            if let Some(last) = out.last_mut() {
                last.test = None;
            }
        }
        ok.then_some(out)
    }

    /// Check a loop body and step. Returns the body and the step.
    pub(super) fn looped(&mut self, label: hir::Label, span: Span, body: &ast::Block, step: Option<&ast::Stmt>) -> (hir::Block, hir::Block) {
        self.targets.push(Target::Loop { label, span });
        let body = self.block(body);
        let mut step_b = Vec::new();
        if let Some(s) = step {
            self.scopes.push(HashMap::new());
            self.stmt(s, &mut step_b);
            self.scopes.pop();
        }
        self.targets.pop();
        (body, step_b)
    }

    /// `for (T name of a..b) { }`. Lowered to
    ///
    /// ```text
    /// start = a; end = b; step = by;            // `by` only when written
    /// if (step is zero at run time) stop;       // only when the step is not a literal
    /// if (start <= end) {                       // `start >= end` when counting down
    ///     counter = start;
    ///     loop { name = counter; body; step: if (counter + by > end || ...) break; counter += by; }
    /// }
    /// ```
    ///
    /// `by` is 1 unless the loop says `by n`. Its sign gives the direction: `10..0 by -2`
    /// counts down, and a step that points away from the end gives an empty loop. The
    /// counter never steps past `end`: the sum is tested for going over `end`, and for
    /// wrapping round at the type's largest or smallest value (overflow wraps).
    ///
    /// A float range (`float64 x of 0.0..1.0 by 0.25`) has no counter of its own: value
    /// `k` is `start + k * step`, with a whole-number `k`, so nothing drifts. The loop
    /// stops at the last value not past `end`. Its `by` is required.
    pub(super) fn for_each(&mut self, s: &ast::Stmt, out: &mut hir::Block) {
        let ast::Stmt::ForEach { mutable, ty: ty_name, name, name_span, iter, step, body, span } = s else { unreachable!() };
        let ty = self.g.resolve_binding_type(ty_name, self.diags);
        let ExprKind::Range(lo, hi) = &iter.kind else {
            return self.for_text(s, ty, out);
        };
        let bounds = match ty {
            Some(t) if t.is_numeric() => {
                let a = self.coerce(lo, t);
                let b = self.coerce(hi, t);
                a.zip(b)
            }
            _ => {
                self.expr(lo);
                self.expr(hi);
                if let Some(t) = ty {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, ty_name.span, format!("a range loop counts numbers, so its variable cannot be `{t}`"))
                            .note(format!("this is `{t}`"))
                            .help("use a number type such as `uint32`, `sint64` or `float64`"),
                    );
                }
                None
            }
        };

        // The sign of a literal step, when the step is written as one.
        let sign = step.as_ref().and_then(|st| literal_sign(st));
        // `by n`: the step, of the loop's type. A literal zero is an error; any other
        // step is checked like a value.
        let unsigned_down = sign == Some(Ordering::Less)
            && bounds.as_ref().is_some_and(|(a, _)| matches!(a.ty.bare(), Type::Int { signed: false, .. }));
        let by = match (step, &bounds) {
            (None, Some((a, _))) if a.ty.is_float() => {
                self.diags.push(
                    Diagnostic::at(&codes::E0101, iter.span, "a range over floats needs a `by` step")
                        .note("a float range has no step of its own")
                        .help("say how far apart the values are: `for (float64 x of 0.0..1.0 by 0.25)`"),
                );
                None
            }
            (None, _) => None,
            (Some(st), _) if sign == Some(Ordering::Equal) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0115, st.span, "a range step of zero never moves on")
                        .note("the loop would never end")
                        .help("write a step that is not zero, or leave out `by` for a step of 1"),
                );
                self.expr(st);
                None
            }
            (Some(st), Some((a, _))) => {
                let t = a.ty;
                if unsigned_down && let ExprKind::Unary(ast::UnOp::Neg, magnitude) = &st.kind {
                    self.diags.push(Diagnostic::at(
                        &codes::W0102,
                        st.span,
                        format!("a negative step counts down over `{t}`"),
                    ));
                    self.coerce(magnitude, t)
                } else {
                    self.coerce(st, t)
                }
            }
            (Some(st), None) => {
                self.expr(st);
                None
            }
        };
        // Which way the counter goes. A literal step says; a step known only at run time
        // may point either way unless the type is unsigned (which has no negative step).
        let dir = match (step, sign) {
            (None, _) => Dir::Up,
            (Some(_), Some(Ordering::Less)) => Dir::Down,
            (Some(_), Some(_)) => Dir::Up,
            (Some(_), None) => match &bounds {
                Some((a, _)) if a.ty.is_float() || matches!(a.ty.bare(), Type::Int { signed: true, .. }) => Dir::Either,
                _ => Dir::Up,
            },
        };
        // A step only known at run time is checked before the loop starts.
        let check_zero = by.is_some() && (sign.is_none() || unsigned_down);

        self.scopes.push(HashMap::new());
        // The variable belongs to the loop: it gets a new value each time round.
        let var = self.declare(name, ty, *mutable, *name_span, false);
        let label = self.new_label();
        let body_end = body.end;
        let (body, _) = self.looped(label, *span, body, None);
        self.scopes.pop();

        let Some((a, b)) = bounds else { return };
        let t = a.ty;
        let by = by.map(|e| (self.hidden_named("range step", t), e));
        let mut out_stmts = Vec::new();
        let start = self.hidden_named("range start", t);
        let end = self.hidden_named("range end", t);
        out_stmts.push(hir::Stmt::Assign(hir::Place::local(start), a));
        out_stmts.push(hir::Stmt::Assign(hir::Place::local(end), b));
        if let Some((id, e)) = &by {
            out_stmts.push(hir::Stmt::Assign(hir::Place::local(*id), e.clone()));
        }
        if check_zero && let Some((id, _)) = &by {
            let is_zero = if t.is_float() {
                // Zero, or not a number at all: neither is a step.
                let zero = hir::Expr::new(hir::ExprKind::Float(0.0), t);
                logic(true, not(binary(hir::BinOp::Gt, local(*id, t), zero.clone())), not(binary(hir::BinOp::Lt, local(*id, t), zero)))
            } else {
                binary(hir::BinOp::Eq, local(*id, t), int_lit(0, t))
            };
            out_stmts.push(hir::Stmt::If {
                label: self.new_label(),
                branches: vec![hir::Branch { test: Some(is_zero), body: vec![hir::Stmt::Stop(hir::StopKind::ZeroStep)], end: Span::new(0, 0) }],
            });
        }

        if t.is_float() {
            // Value k is start + k * step, from a whole-number counter.
            let index = self.hidden_named("range index", Type::U64);
            let value = self.hidden_named("range value", t);
            let step_now = match &by {
                Some((id, _)) => local(*id, t),
                None => hir::Expr::new(hir::ExprKind::Float(1.0), t),
            };
            let counted = hir::Expr::new(hir::ExprKind::Convert(Box::new(local(index, Type::U64))), t);
            let v = binary(hir::BinOp::Add, local(start, t), binary(hir::BinOp::Mul, counted, step_now.clone()));
            out_stmts.push(hir::Stmt::Assign(hir::Place::local(index), int_lit(0, Type::U64)));
            let up = binary(hir::BinOp::Le, local(value, t), local(end, t));
            let down = binary(hir::BinOp::Ge, local(value, t), local(end, t));
            // Still in range: not past the end in the direction of the step.
            let in_range = match dir {
                Dir::Up => up,
                Dir::Down => down,
                Dir::Either => {
                    let zero = hir::Expr::new(hir::ExprKind::Float(0.0), t);
                    logic(
                        false,
                        logic(true, binary(hir::BinOp::Gt, step_now.clone(), zero.clone()), up),
                        logic(true, binary(hir::BinOp::Lt, step_now, zero), down),
                    )
                }
            };
            let mut inner = vec![
                hir::Stmt::Assign(hir::Place::local(value), v),
                hir::Stmt::If {
                    label: self.new_label(),
                    branches: vec![hir::Branch { test: Some(not(in_range)), body: vec![hir::Stmt::Jump(hir::Jump::Break(label))], end: Span::new(0, 0) }],
                },
                hir::Stmt::Decl(var),
                hir::Stmt::Assign(hir::Place::local(var), local(value, t)),
            ];
            inner.extend(body);
            let advance = vec![hir::Stmt::Assign(hir::Place::local(index), binary(hir::BinOp::Add, local(index, Type::U64), int_lit(1, Type::U64)))];
            out_stmts.push(hir::Stmt::Loop { label, cond: None, body: inner, step: advance, end: body_end });
            out.extend(out_stmts);
            return;
        }

        let counter = self.hidden_named("range counter", t);
        // The next counter value; it is past `end` when it is above `end`, or when the
        // addition wrapped round (then it is below the counter).
        let by_now = match &by {
            Some((id, _)) => local(*id, t),
            None => int_lit(1, t),
        };
        let advance_op = if unsigned_down { hir::BinOp::Sub } else { hir::BinOp::Add };
        let next = binary(advance_op, local(counter, t), by_now.clone());
        let past_up = logic(false, binary(hir::BinOp::Gt, next.clone(), local(end, t)), binary(hir::BinOp::Lt, next.clone(), local(counter, t)));
        let past_down = if unsigned_down {
            binary(hir::BinOp::Gt, by_now.clone(), binary(hir::BinOp::Sub, local(counter, t), local(end, t)))
        } else {
            logic(false, binary(hir::BinOp::Lt, next.clone(), local(end, t)), binary(hir::BinOp::Gt, next.clone(), local(counter, t)))
        };
        let past_end = match dir {
            Dir::Up => past_up,
            Dir::Down => past_down,
            Dir::Either => {
                let zero = int_lit(0, t);
                logic(
                    false,
                    logic(true, binary(hir::BinOp::Gt, by_now.clone(), zero.clone()), past_up),
                    logic(true, binary(hir::BinOp::Lt, by_now, zero), past_down),
                )
            }
        };
        // Starts only when there is something to count: `start <= end`, or `start >= end`
        // when counting down.
        let up_ok = binary(hir::BinOp::Le, local(start, t), local(end, t));
        let down_ok = binary(hir::BinOp::Ge, local(start, t), local(end, t));
        let enter = match dir {
            Dir::Up => up_ok,
            Dir::Down => down_ok,
            Dir::Either => {
                let by_id = by.as_ref().map(|(id, _)| *id).expect("a run-time step has a local");
                let zero = int_lit(0, t);
                logic(
                    false,
                    logic(true, binary(hir::BinOp::Gt, local(by_id, t), zero.clone()), up_ok),
                    logic(true, binary(hir::BinOp::Lt, local(by_id, t), zero), down_ok),
                )
            }
        };
        let mut inner = vec![hir::Stmt::Decl(var), hir::Stmt::Assign(hir::Place::local(var), local(counter, t))];
        inner.extend(body);
        let advance = vec![
            hir::Stmt::If {
                label: self.new_label(),
                branches: vec![hir::Branch {
                    test: Some(past_end),
                    body: vec![hir::Stmt::Jump(hir::Jump::Break(label))],
                    end: Span::new(0, 0),
                }],
            },
            hir::Stmt::Assign(hir::Place::local(counter), next),
        ];
        out_stmts.push(hir::Stmt::Assign(hir::Place::local(counter), local(start, t)));
        out_stmts.push(hir::Stmt::If {
            label: self.new_label(),
            branches: vec![hir::Branch {
                test: Some(enter),
                body: vec![hir::Stmt::Loop { label, cond: None, body: inner, step: advance, end: body_end }],
                end: Span::new(0, 0),
            }],
        });
        out.extend(out_stmts);
    }

    /// `for (T name of text) { }` where `text` is not a range. `string`
    /// gives an owned copy of each character, `&string` a borrowed view of
    /// it; anything else is an error. Lowered to
    ///
    /// ```text
    /// owned = text;              // only when `text` is not a place
    /// hidden = &owned;           // a real shared borrow: the body cannot change it (E0302)
    /// pos = 0;
    /// loop while (pos < byte_len(*hidden)) {
    ///     name = next_char(*hidden, pos);      // `&string`: view = char_view(*hidden, pos); name = &view
    ///     pos = pos + byte_len(name);          // before the body, so `continue` is safe
    ///     body
    /// }
    /// ```
    fn for_text(&mut self, s: &ast::Stmt, ty: Option<Type>, out: &mut hir::Block) {
        let ast::Stmt::ForEach { mutable, ty: ty_name, name, name_span, iter, body, span, .. } = s else { unreachable!() };
        let source = match self.expr(iter) {
            Some(Val::Expr(e)) if e.ty == Type::Str => Some(e),
            Some(Val::Expr(e)) if e.ty.is_list() || e.ty.is_array() => return self.for_list(s, ty, e, iter.span, out),
            Some(Val::Expr(e)) if self.g.ifaces.iterable.contains_key(&e.ty) => return self.for_iterable(s, ty, e, iter.span, out),
            Some(v) => {
                self.not_iterable(v, iter);
                None
            }
            None => None,
        };
        let text_ref = Type::borrow(false, Type::Str);
        // `string c`: an owned copy of each character. `&string c`: a view into the text.
        let kind = ty.filter(|t| *t == Type::Str || *t == text_ref);
        if let (Some(t), Some(_)) = (ty, &source)
            && kind.is_none()
        {
            self.diags.push(
                Diagnostic::at(&codes::E0101, ty_name.span, format!("a loop over text needs a `string` or `&string` variable, not `{t}`"))
                    .note(format!("this is `{t}`"))
                    .help("write `for (string c of text) { }` for a copy of each character, or `for (&string c of text) { }` for a view"),
            );
        }
        self.scopes.push(HashMap::new());
        let var = self.declare(name, ty, *mutable, *name_span, false);
        let label = self.new_label();
        let body_end = body.end;
        let (body, _) = self.looped(label, *span, body, None);
        self.scopes.pop();

        let Some(mut text) = source.filter(|_| kind.is_some()) else { return };
        let is_view = kind == Some(text_ref.clone());
        if !text.is_place() {
            let owned = self.hidden_named("for text", Type::Str);
            out.push(hir::Stmt::Decl(owned));
            out.push(hir::Stmt::Assign(hir::Place::local(owned), text));
            text = local(owned, Type::Str);
        }
        let hidden = self.hidden_named("for text borrow", text_ref.clone());
        let pos = self.hidden_named("for position", U64);
        out.push(hir::Stmt::Assign(hir::Place::local(hidden), shared_borrow(text)));
        out.push(hir::Stmt::Assign(hir::Place::local(pos), int_lit(0, U64)));

        let cond = binary(hir::BinOp::Lt, local(pos, U64), str_fn(StrFn::ByteLen, vec![text_at(hidden, text_ref.clone())]));
        let mut inner = Vec::new();
        if is_view {
            // The view is a hidden local made fresh each time round; `c` borrows it.
            let view = self.hidden_named("for character view", Type::Str);
            inner.push(hir::Stmt::Decl(view));
            inner.push(hir::Stmt::Assign(hir::Place::local(view), str_fn(StrFn::CharView, vec![text_at(hidden, text_ref.clone()), local(pos, U64)])));
            inner.push(hir::Stmt::Assign(hir::Place::local(pos), binary(hir::BinOp::Add, local(pos, U64), str_fn(StrFn::ByteLen, vec![local(view, Type::Str)]))));
            inner.push(hir::Stmt::Decl(var));
            inner.push(hir::Stmt::Assign(hir::Place::local(var), shared_borrow(local(view, Type::Str))));
        } else {
            inner.push(hir::Stmt::Decl(var));
            inner.push(hir::Stmt::Assign(hir::Place::local(var), str_fn(StrFn::NextChar, vec![text_at(hidden, text_ref.clone()), local(pos, U64)])));
            inner.push(hir::Stmt::Assign(
                hir::Place::local(pos),
                binary(hir::BinOp::Add, local(pos, U64), str_fn(StrFn::ByteLen, vec![local(var, Type::Str)])),
            ));
        }
        inner.extend(body);
        out.push(hir::Stmt::Loop { label, cond: Some(cond), body: inner, step: Vec::new(), end: body_end });
    }

    /// Report the thing after `of` that is not a range or a string.
    fn not_iterable(&mut self, v: Val, iter: &ast::Expr) {
        let found = match &v {
            Val::Const(_) | Val::UConst(..) => "a number".to_string(),
            Val::Expr(e) => format!("`{}`", e.ty),
            Val::NoneLit(_) => "`none`".to_string(),
        };
        let help = match &v {
            Val::Expr(e) if e.ty.is_map() => "a map is walked with two variables: `for (string key, uint32 value of map) { }`",
            _ => "to count, loop over a range: `for (uint32 i of 0..9) { }`; to walk text, loop over a `string`; to walk your own type, give it `with Iterable<T>` and a method `T? at(uint64 i)`",
        };
        let d = Diagnostic::at(&codes::E0101, iter.span, format!("cannot loop over {found}")).note(format!("this is {found}")).help(help);
        self.diags.push(d);
    }

    /// `break`, `continue`, with an optional count of loops.
    pub(super) fn break_target(&mut self, cont: bool, count: Option<(u64, Span)>, span: Span) -> Option<hir::Jump> {
        let word = if cont { "continue" } else { "break" };
        let loops: Vec<usize> = (0..self.targets.len()).rev().filter(|&i| matches!(self.targets[i], Target::Loop { .. })).collect();
        let n = count.map_or(1, |c| c.0);
        let at = count.map_or(span, |c| c.1);
        let k = loops.len();
        let d = if loops.is_empty() {
            Diagnostic::at(&codes::E0011, span, format!("`{word}` is not inside a loop"))
                .note("no loop here")
                .help(format!("`{word}` works on loops; use `skip` to leave an `if`, or `return` to leave the function"))
        } else if n == 0 {
            Diagnostic::at(&codes::E0011, at, format!("`{word} 0` counts no loops"))
                .note("the count is at least 1")
                .help(format!("`{word};` is the same as `{word} 1;`, for the innermost loop"))
        } else if n > k as u64 {
            Diagnostic::at(
                &codes::E0011,
                at,
                format!("`{word} {n}` counts {n} loops, but there {} only {k} around it", if k == 1 { "is" } else { "are" }),
            )
            .note("too many")
            .label(self.target_span(loops[k - 1]), "the outermost loop")
            .help(format!("`{word} {k};` {} the outermost loop", if cont { "continues" } else { "leaves" }))
        } else {
            let idx = loops[n as usize - 1];
            let label = self.record(idx);
            return Some(if cont { hir::Jump::Continue(label) } else { hir::Jump::Break(label) });
        };
        self.diags.push(d);
        None
    }

    /// `skip`, `skip N` and `skip to next`, counted as described at the top of this file.
    pub(super) fn skip_target(&mut self, target: SkipTarget, span: Span) -> Option<hir::Jump> {
        let wall = self.targets.iter().rposition(|t| matches!(t, Target::Loop { .. }));
        // Innermost first.
        let levels: Vec<usize> = (wall.map_or(0, |w| w + 1)..self.targets.len()).rev().collect();
        let what = match target {
            SkipTarget::Outermost => "skip".to_string(),
            SkipTarget::Levels(n, _) => format!("skip {n}"),
            SkipTarget::Next => "skip to next".to_string(),
        };
        let wall_label = "loops are walls: `skip` cannot leave this loop";
        let k = levels.len();
        let d = if levels.is_empty() {
            let d = Diagnostic::at(&codes::E0012, span, format!("`{what}` is not inside an `if` or a `match`")).note("nothing to skip here");
            match wall {
                Some(w) => d
                    .label(self.target_span(w), wall_label)
                    .help("use `break` to leave the loop, or `continue` to go round again"),
                None => d.help("`skip` leaves `if` blocks; use `return` to leave the function"),
            }
        } else if let SkipTarget::Next = target {
            let idx = levels[0];
            let Target::Level { branch, branches, is_match, has_else, span: level_span, .. } = self.targets[idx] else { unreachable!() };
            if branch + 1 < branches {
                if branch + 2 == branches
                    && let Target::Level { next_to_last, .. } = &mut self.targets[idx]
                {
                    *next_to_last = true;
                }
                return Some(hir::Jump::Next(self.record(idx), branch + 1));
            }
            let (label, help) = if is_match {
                ("this `match` has no `case` after this one", "`skip 1;` leaves the `match`")
            } else if has_else {
                ("this chain has no branch after its `else`", "`skip 1;` leaves the `if`")
            } else {
                ("this chain has no branch after this one", "add an `else` branch, or write `skip 1;` to leave the `if`")
            };
            Diagnostic::at(&codes::E0012, span, "`skip to next` is in the last branch, so there is no next one")
                .note("in the last branch")
                .label(level_span, label)
                .help(help)
        } else {
            let (n, at) = match target {
                SkipTarget::Levels(n, s) => (n, s),
                _ => (k as u64, span),
            };
            if n == 0 {
                Diagnostic::at(&codes::E0012, at, "`skip 0` leaves nothing")
                    .note("the count is at least 1")
                    .help("`skip 1;` leaves the innermost `if`, and `skip;` the outermost")
            } else if n > k as u64 {
                let mut d = Diagnostic::at(
                    &codes::E0012,
                    at,
                    format!("`skip {n}` would leave {n} levels, but it is inside only {k}"),
                )
                .note("too many")
                .label(self.target_span(levels[k - 1]), "the outermost level it can leave")
                .note_line("a whole `if` / `else if` / `else` chain is one level, and so is a `match`");
                if let Some(w) = wall {
                    d = d.label(self.target_span(w), wall_label);
                }
                d.help(format!("`skip {k};` (or `skip;`) leaves the outermost level"))
            } else {
                return Some(hir::Jump::Leave(self.record(levels[n as usize - 1])));
            }
        };
        self.diags.push(d);
        None
    }

    pub(super) fn match_stmt(&mut self, value: &ast::Expr, cases: &[ast::Case], span: Span, out: &mut hir::Block) {
        let v = self.expr(value).and_then(|v| self.materialise(v, value.span));
        if matches!(&v, Some(x) if x.ty.enum_id().is_some()) {
            return self.enum_match(v.unwrap(), value, cases, span, out);
        }
        let v = match v {
            Some(v) if v.ty == Type::Str => return self.match_text(v, cases, span, out),
            Some(v) if v.ty.is_numeric() || v.ty == Type::Bool => Some(v),
            Some(v) => {
                let d = if v.ty == Type::None {
                    Diagnostic::at(&codes::E0104, value.span, "`match` needs a value, but this produces none").note("returns `none`")
                } else {
                    // OPEN: no struct patterns exist, so matching a struct is an error.
                    Diagnostic::at(&codes::E0101, value.span, format!("`match` works on numbers and bools, not `{}`", v.ty))
                        .note(format!("this is `{}`", v.ty))
                        .help("compare its fields in the cases, or use `if` with `==`")
                };
                self.diags.push(d);
                None
            }
            None => None,
        };
        let ty = v.as_ref().map(|v| v.ty);
        // Evaluated once, before any case is tested.
        let tmp = v.map(|v| {
            let id = self.hidden_named("match value", v.ty);
            out.push(hir::Stmt::Assign(hir::Place::local(id), v));
            id
        });

        let mut cover = Coverage::default();
        // The first `case _`.
        let mut any: Option<Span> = None;
        let mut clean = true;
        let mut tests = Vec::new();
        for case in cases {
            let mut keys = Vec::new();
            let mut values = Vec::new();
            let mut wildcard = false;
            let mut ok = true;
            for p in &case.patterns {
                match p {
                    Pattern::Any(s) => {
                        wildcard = true;
                        if case.patterns.len() > 1 {
                            // Open: whether `_` may share a case with other patterns (it would make them pointless).
                            self.diags.push(
                                Diagnostic::at(&codes::E0109, *s, "`_` must be the only pattern in its `case`")
                                    .note("this matches everything")
                                    .help("`case _` already matches every value; remove the other patterns"),
                            );
                            ok = false;
                        }
                    }
                    Pattern::Glob(g) => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0109, g.span, "`globbed` is only for a `match` on `string`")
                                .note("this is a glob")
                                .help("write the number as a plain value, such as `case 5`"),
                        );
                        ok = false;
                    }
                    Pattern::Variant { span, .. } => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0525, *span, "this pattern is a variant, but the value is not an enum")
                                .note("only an enum is matched with variants"),
                        );
                        ok = false;
                    }
                    Pattern::Bind { name_span, .. } => {
                        self.diags.push(Diagnostic::at(&codes::E0901, *name_span, "this pattern is not supported yet"));
                        ok = false;
                    }
                    Pattern::Expr(e) => match self.pattern(e, ty) {
                        Some((lo, hi)) => {
                            let t = ty.unwrap();
                            keys.push((lo.key(t), hi.key(t)));
                            values.push((lo, hi));
                        }
                        None => ok = false,
                    },
                }
            }
            if let (Some(t), true) = (ty, ok) {
                let why = if any.is_some() {
                    Some("it comes after `case _`, which matches everything")
                } else if wildcard {
                    (!t.is_float() && cover.covers(domain(t))).then_some("the cases before it already handle every value")
                } else {
                    keys.iter().all(|k| cover.covers(*k)).then_some("every value it matches is handled by an earlier case")
                };
                if let Some(why) = why {
                    let mut d = Diagnostic::at(&codes::W0111, case.span, "this case can never run").note(why);
                    if let Some(a) = any {
                        d = d.label(a, "this matches everything");
                    }
                    self.diags.push(d.help("remove it, or move it above the case that handles its values"));
                }
            }
            // A guard (`case 3 if x > 1`) is checked after the pattern; a guarded
            // case never counts as covering its values.
            let guard = match &case.guard {
                Some(g) => match self.cond(g) {
                    Some(c) => Some(c),
                    None => {
                        ok = false;
                        None
                    }
                },
                None => None,
            };
            clean &= ok;
            let guarded = case.guard.is_some();
            if !guarded {
                for (lo, hi) in keys {
                    cover.add(lo, hi);
                }
            }
            if wildcard && ok && !guarded && any.is_none() {
                any = Some(case.span);
            }
            let pattern = match (tmp, ty) {
                _ if wildcard => Some(None),
                (Some(tmp), Some(t)) if ok => {
                    let test = values.into_iter().map(|(lo, hi)| {
                        if lo.key(t) == hi.key(t) {
                            binary(hir::BinOp::Eq, local(tmp, t), lo.expr(t))
                        } else {
                            logic(true, binary(hir::BinOp::Ge, local(tmp, t), lo.expr(t)), binary(hir::BinOp::Le, local(tmp, t), hi.expr(t)))
                        }
                    });
                    Some(test.reduce(|a, b| logic(false, a, b)))
                }
                _ => None,
            };
            tests.push(guarded_test(pattern, guard, guarded));
        }

        let exhaustive = any.is_some() || ty.is_some_and(|t| !t.is_float() && cover.covers(domain(t)));
        if let Some(t) = ty
            && clean
            && !exhaustive
        {
            self.warn_missing(t, &cover, value, span);
        }
        let label = self.new_label();
        let parts = tests.into_iter().zip(cases).map(|(t, c)| (t, &c.body)).collect();
        if let Some(branches) = self.chain(label, span, true, parts, exhaustive)
            && !branches.is_empty()
        {
            out.push(hir::Stmt::If { label, branches });
        }
    }

    /// `match (text) { case ... }`. The text is borrowed for the whole match,
    /// never moved, so a place stays usable afterwards. A plain string pattern
    /// matches the text exactly; `globbed "..."` treats each `*` as a wildcard
    /// (`drachm_str_glob`). Only `case _`, or a `case globbed "*"`, makes the
    /// match exhaustive.
    ///
    /// The coverage check is deliberately partial (so it never gives a false
    /// W0111): an exact pattern is covered by an earlier exact pattern with the
    /// same text, or an earlier glob that matches it; a glob only by an earlier
    /// glob with the same text, or an earlier `globbed "*"`. Cleverer cases
    /// (does `globbed "a*"` cover `globbed "ab*"`?) are not checked.
    fn match_text(&mut self, v: hir::Expr, cases: &[ast::Case], span: Span, out: &mut hir::Block) {
        let text_ty = Type::borrow(false, Type::Str);
        let text = if v.is_place() {
            v
        } else {
            let owned = self.hidden_named("match value", Type::Str);
            out.push(hir::Stmt::Decl(owned));
            out.push(hir::Stmt::Assign(hir::Place::local(owned), v));
            local(owned, Type::Str)
        };
        let hidden = self.hidden_named("match text", text_ty.clone());
        out.push(hir::Stmt::Assign(hir::Place::local(hidden), shared_borrow(text)));

        let mut cover: Vec<TextPat> = Vec::new();
        let mut any: Option<Span> = None;
        let mut clean = true;
        let mut tests = Vec::new();
        for case in cases {
            let mut pats = Vec::new();
            let mut wildcard = false;
            let mut ok = true;
            for p in &case.patterns {
                match p {
                    Pattern::Any(s) => {
                        wildcard = true;
                        if case.patterns.len() > 1 {
                            self.diags.push(
                                Diagnostic::at(&codes::E0109, *s, "`_` must be the only pattern in its `case`")
                                    .note("this matches everything")
                                    .help("`case _` already matches every value; remove the other patterns"),
                            );
                            ok = false;
                        }
                    }
                    Pattern::Expr(e) => match self.text_pattern(e, false) {
                        Some(pat) => pats.push(pat),
                        None => ok = false,
                    },
                    Pattern::Glob(e) => match self.text_pattern(e, true) {
                        Some(pat) => pats.push(pat),
                        None => ok = false,
                    },
                    Pattern::Variant { span, .. } => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0525, *span, "this pattern is a variant, but the value is not an enum")
                                .note("only an enum is matched with variants"),
                        );
                        ok = false;
                    }
                    Pattern::Bind { name_span, .. } => {
                        self.diags.push(Diagnostic::at(&codes::E0901, *name_span, "this pattern is not supported yet"));
                        ok = false;
                    }
                }
            }
            wildcard |= pats.iter().any(TextPat::matches_all);
            let guard = match &case.guard {
                Some(g) => match self.cond(g) {
                    Some(c) => Some(c),
                    None => {
                        ok = false;
                        None
                    }
                },
                None => None,
            };
            let guarded = case.guard.is_some();
            if ok {
                let why = if any.is_some() {
                    Some("it comes after a case that matches every text")
                } else if !wildcard {
                    pats.iter().all(|p| cover.iter().any(|c| c.covers(p))).then_some("every value it matches is handled by an earlier case")
                } else {
                    None
                };
                if let Some(why) = why {
                    let mut d = Diagnostic::at(&codes::W0111, case.span, "this case can never run").note(why);
                    if let Some(a) = any {
                        d = d.label(a, "this matches everything");
                    }
                    self.diags.push(d.help("remove it, or move it above the case that handles its values"));
                }
                if wildcard && !guarded && any.is_none() {
                    any = Some(case.span);
                }
                if !guarded {
                    cover.extend(pats.iter().cloned());
                }
            }
            clean &= ok;
            let pattern = if wildcard {
                Some(None)
            } else if ok {
                let ands = pats.into_iter().map(|p| match p {
                    TextPat::Exact(t) => str_fn(StrFn::Eq, vec![text_at(hidden, text_ty.clone()), str_lit(t)]),
                    TextPat::Glob(g) => str_fn(StrFn::Glob, vec![text_at(hidden, text_ty.clone()), str_lit(g)]),
                });
                Some(ands.reduce(|a, b| logic(false, a, b)))
            } else {
                None
            };
            tests.push(guarded_test(pattern, guard, guarded));
        }

        let exhaustive = any.is_some();
        if clean && !exhaustive {
            let d = Diagnostic::at(&codes::W0110, span, "this `match` on `string` does not handle every value")
                .note("there is no `case _`")
                .note_line("text has too many possible values to list")
                .help("add `case _ { }` to handle every other value");
            self.diags.push(d);
        }
        let label = self.new_label();
        let parts = tests.into_iter().zip(cases).map(|(t, c)| (t, &c.body)).collect();
        if let Some(branches) = self.chain(label, span, true, parts, exhaustive)
            && !branches.is_empty()
        {
            out.push(hir::Stmt::If { label, branches });
        }
    }

    /// One `case` pattern of a `string` match: a string literal with no
    /// `${}` (`globbed` ones are globs). E0109 for interpolation and ranges,
    /// E0101 for another type.
    pub(super) fn text_pattern(&mut self, e: &ast::Expr, glob: bool) -> Option<TextPat> {
        if let ExprKind::Range(..) = e.kind {
            self.diags.push(
                Diagnostic::at(&codes::E0109, e.span, "a range pattern needs numbers, but this `match` is on `string`")
                    .note("this is a range")
                    .help("list the texts instead: `case \"a\", \"b\" { }`"),
            );
            return None;
        }
        if let ExprKind::Str(parts) = &e.kind {
            if parts.iter().any(|p| matches!(p, StrPart::Interp(_))) {
                self.diags.push(
                    Diagnostic::at(&codes::E0109, e.span, "a pattern must be a plain string")
                        .note("this has `${}` in it")
                        .help("write the text out in full, without `${}`"),
                );
                return None;
            }
            let mut text = String::new();
            for p in parts {
                match p {
                    StrPart::Raw(t, _) => text.push_str(t),
                    StrPart::Escape(c, _) => text.push(*c),
                    StrPart::Interp(_) => unreachable!("checked above"),
                }
            }
            return Some(if glob { TextPat::Glob(text) } else { TextPat::Exact(text) });
        }
        match self.expr(e)? {
            Val::Expr(h) if h.ty == Type::Str => {
                self.diags.push(
                    Diagnostic::at(&codes::E0109, e.span, "a pattern must be a plain string")
                        .note("this is worked out when the program runs")
                        .help("compare it in an `if` instead: `if (text == other) { }`"),
                );
            }
            v => {
                let found = match &v {
                    Val::Const(_) | Val::UConst(..) => "a number".to_string(),
                    Val::Expr(h) => format!("`{}`", h.ty),
                    Val::NoneLit(_) => "`none`".to_string(),
                };
                self.diags.push(
                    Diagnostic::at(&codes::E0101, e.span, "a `case` of a `string` match must be a string")
                        .note(format!("this is {found}"))
                        .help("write the text in quotes, such as `case \"/\"`"),
                );
            }
        }
        None
    }

    /// W0110 for a `match` whose cases leave values unhandled. Small types
    /// get the missing values listed.
    fn warn_missing(&mut self, ty: Type, cover: &Coverage, value: &ast::Expr, span: Span) {
        let what = self.quote(value.span, "the value");
        // C can send a value its header does not list, so a C enum always needs a catch-all.
        if ty.is_c_enum() {
            self.diags.push(
                Diagnostic::at(&codes::E0705, span, format!("this `match` on `{ty}` needs a `case _`"))
                    .note(format!("{what} may hold a value the C header does not list"))
                    .help("add `case _ { }` for the values no case names"),
            );
            return;
        }
        let small = match ty {
            Type::Bool => true,
            Type::Int { bits, .. } => bits <= 16,
            _ => false,
        };
        let d = if small {
            let gaps = cover.gaps(domain(ty));
            let items: Vec<String> = gaps
                .iter()
                .take(4)
                .map(|&(a, b)| if a == b { format!("`{}`", describe(a, ty)) } else { format!("`{}..{}`", describe(a, ty), describe(b, ty)) })
                .collect();
            let list = match items.len() {
                _ if gaps.len() > 4 => format!("{} and {} more", items.join(", "), gaps.len() - 4),
                1 => items[0].clone(),
                n => format!("{} or {}", items[..n - 1].join(", "), items[n - 1]),
            };
            let them = if gaps.len() == 1 && gaps[0].0 == gaps[0].1 { "it" } else { "them" };
            Diagnostic::at(&codes::W0110, span, format!("this `match` does not handle {list}"))
                .note(format!("{what} is `{ty}`"))
                .help(format!("add a `case` for {them}, or `case _ {{ }}` for everything else"))
        } else {
            let mut d = Diagnostic::at(&codes::W0110, span, format!("this `match` on `{ty}` does not handle every value"))
                .note("there is no `case _`")
                .help("add `case _ { }` for the values no case handles");
            if ty.is_float() {
                d = d.note_line("on floats only `case _` handles every value: NaN is equal to nothing");
            }
            d
        };
        self.diags.push(d);
    }

    /// A `case` pattern: a constant, or a range of two. Returns its lowest
    /// and highest value (the same for a single value).
    fn pattern(&mut self, e: &ast::Expr, ty: Option<Type>) -> Option<(Lit, Lit)> {
        let ExprKind::Range(lo, hi) = &e.kind else {
            let v = self.pattern_value(e, ty)?;
            return Some((v, v));
        };
        if ty == Some(Type::Bool) {
            self.diags.push(
                Diagnostic::at(&codes::E0109, e.span, "a range pattern needs numbers, but this `match` is on `bool`")
                    .note("this is a range")
                    .help("write `case true` or `case false`"),
            );
            return None;
        }
        let a = self.pattern_value(lo, ty);
        let b = self.pattern_value(hi, ty);
        let (a, b, t) = (a?, b?, ty?);
        if a.key(t) > b.key(t) {
            // Open: an empty range could instead be a warning (it matches nothing).
            let (l, h) = (self.text(lo.span).to_string(), self.text(hi.span).to_string());
            self.diags.push(
                Diagnostic::at(&codes::E0109, e.span, "this range is empty: its start is above its end")
                    .note("matches nothing")
                    .help(format!("ranges go from low to high: `{h}..{l}`")),
            );
            return None;
        }
        Some((a, b))
    }

    /// One constant in a pattern, converted to the matched type.
    pub(super) fn pattern_value(&mut self, e: &ast::Expr, ty: Option<Type>) -> Option<Lit> {
        let v = self.expr(e)?;
        let ty = ty?;
        match v {
            Val::Const(c) => fold(self.into(Val::Const(c), ty, origin(e))?),
            v @ Val::UConst(..) => fold(self.into(v, ty, origin(e))?),
            Val::NoneLit(s) => {
                self.none_without_type(s);
                None
            }
            Val::Expr(h) => {
                let from = h.ty;
                let Some(lit) = fold(h) else {
                    self.diags.push(
                        Diagnostic::at(&codes::E0109, e.span, "a `case` pattern must be a constant")
                            .note("this is worked out when the program runs")
                            .help("compare it in an `if` instead: `if (value == other) { }`"),
                    );
                    return None;
                };
                // A typed constant of another type converts by the usual rules (W0102 if lossy).
                fold(self.into(Val::Expr(lit.expr(from)), ty, origin(e))?)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_merges_and_finds_gaps() {
        let mut c = Coverage::default();
        c.add(10, 20);
        c.add(0, 4);
        c.add(5, 9);
        assert_eq!(c.0, [(0, 20)]);
        assert!(c.covers((3, 17)));
        assert!(!c.covers((15, 21)));
        c.add(u128::MAX - 1, u128::MAX);
        assert_eq!(c.gaps((0, u128::MAX)), [(21, u128::MAX - 2)]);
        assert_eq!(Coverage::default().gaps((0, 1)), [(0, 1)]);
    }

    #[test]
    fn text_globs_match_the_runtime_rule() {
        assert!(glob_matches("/api/*", "/api/"));
        assert!(!glob_matches("/api/*", "/api"));
        assert!(glob_matches("*.png", "a.png"));
        assert!(glob_matches("a**b", "aXXb"));
        assert!(!glob_matches("*ab*ab", "aab"));
        assert!(glob_matches("*ab*ab", "abab"));
        assert!(glob_matches("*", ""));
        assert!(glob_matches("abc", "abc") && !glob_matches("abc", "abcd"));
        // Coverage: an earlier glob covers an exact text it matches; a glob
        // is covered only by the same glob or by "*".
        let g = TextPat::Glob("a*".into());
        assert!(g.covers(&TextPat::Exact("ab".into())));
        assert!(!TextPat::Exact("ab".into()).covers(&TextPat::Glob("ab*".into())));
        assert!(TextPat::Glob("*".into()).covers(&TextPat::Glob("x*".into())));
        assert!(!g.covers(&TextPat::Glob("ab*".into())));
    }

    #[test]
    fn keys_keep_the_order() {
        let floats = [f64::NEG_INFINITY, -1e300, -1.0, -5e-324, 0.0, 5e-324, 1.0, f64::INFINITY];
        let keys: Vec<u128> = floats.iter().map(|f| Lit::Float(*f).key(Type::F64)).collect();
        assert!(keys.windows(2).all(|w| w[0] < w[1]), "{keys:?}");
        // Zero and negative zero are equal; the smallest negative sits right below them.
        assert_eq!(Lit::Float(-0.0).key(Type::F64), keys[4]);
        assert_eq!(keys[3] + 1, keys[4]);

        let s8 = Type::Int { bits: 8, signed: true };
        let ints: Vec<u128> = [0x80u128, 0xFF, 0, 1, 0x7F].iter().map(|b| Lit::Int(*b).key(s8)).collect();
        assert_eq!(ints, [0, 0x7F, 0x80, 0x81, 0xFF]);
        assert_eq!(describe(0, s8), "-128");
        assert_eq!(describe(0xFF, s8), "127");
    }
}
