//! Name resolution, type checking and definite-assignment analysis.
//! Turns the syntax tree into `hir`, reporting every problem it finds.
//! Loops, jumps and `match` are in `control`.

mod control;

use std::collections::HashMap;

use control::{Target, Test};

use crate::ast::{self, BinOp, ExprKind, OpClass, Stmt, UnOp};
use crate::consts::{Const, FoldError, Folded};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir;
use crate::types::{Type, TypeError};

struct Sig {
    id: hir::FuncId,
    /// `None` where the written type did not resolve (already reported).
    params: Vec<Option<Type>>,
    ret: Option<Type>,
    span: Span,
}

/// Checks the whole program. The result is only fit for codegen when no
/// errors were reported.
pub fn check(prog: &ast::Program, src: &str, diags: &mut Vec<Diagnostic>) -> hir::Program {
    let mut io_imported = false;
    for import in &prog.imports {
        if import.module == "io" {
            io_imported = true;
        } else {
            diags.push(
                Diagnostic::at(&codes::E0201, import.span, format!("unknown module `{}`", import.module))
                    .help("`io` is the only module so far"),
            );
        }
    }

    let mut sigs: HashMap<String, Sig> = HashMap::new();
    let mut ids = Vec::new();
    for f in &prog.funcs {
        let sig = Sig {
            id: sigs.len(),
            params: f.params.iter().map(|p| resolve_binding_type(&p.ty, diags)).collect(),
            ret: resolve(&f.ret, diags),
            span: f.span,
        };
        if let Some(first) = sigs.get(&f.name) {
            diags.push(
                Diagnostic::at(&codes::E0202, f.name_span, format!("function `{}` is defined twice", f.name))
                    .note("defined again here")
                    .label(first.span, "first defined here"),
            );
            ids.push(None);
            continue;
        }
        ids.push(Some(sig.id));
        sigs.insert(f.name.clone(), sig);
    }

    match sigs.get("main") {
        None => diags.push(
            Diagnostic::new(&codes::E0204, "this program has no `main` function")
                .help("add `none main() { }`; it is where the program starts"),
        ),
        Some(s) if !s.params.is_empty() || !matches!(s.ret, Some(Type::None | Type::S32) | None) => diags.push(
            Diagnostic::at(&codes::E0204, s.span, "`main` has the wrong shape")
                .note("must be `none main()` or `sint32 main()`")
                .help("`sint32 main()` returns the program's exit code"),
        ),
        _ => {}
    }

    let mut funcs = Vec::new();
    for (f, id) in prog.funcs.iter().zip(ids) {
        let ret = resolve_quiet(&f.ret);
        let mut cx = Checker {
            src,
            sigs: &sigs,
            io_imported,
            diags,
            ret,
            scopes: vec![HashMap::new()],
            locals: Vec::new(),
            flow: Flow { definite: Vec::new(), maybe: Vec::new(), reachable: true },
            loop_depth: 0,
            targets: Vec::new(),
            labels: 0,
        };
        for p in &f.params {
            let ty = resolve_quiet(&p.ty).filter(|t| *t != Type::None);
            let id = cx.declare(&p.name, ty, p.mutable, p.span, true);
            cx.flow.definite[id] = true;
            cx.flow.maybe[id] = true;
        }
        let body = cx.block(&f.body);
        if cx.flow.reachable && ret.is_some_and(|t| t != Type::None) {
            cx.diags.push(
                Diagnostic::at(
                    &codes::E0105,
                    f.span,
                    format!("function `{}` can finish without returning a value", f.name),
                )
                .note(format!("promises to return `{}`", ret.unwrap()))
                .help("add a `return` at the end, or to every branch"),
            );
        }
        let locals = cx.locals.into_iter().map(|l| hir::Local { name: l.name, ty: l.ty.unwrap_or(Type::None) }).collect();
        // A duplicate keeps being checked, but only the first definition is emitted.
        if id.is_some() {
            funcs.push(hir::Func {
                name: f.name.clone(),
                params: f.params.len(),
                locals,
                ret: ret.unwrap_or(Type::None),
                body,
            });
        }
    }
    hir::Program { funcs, main: sigs.get("main").map_or(0, |s| s.id) }
}

fn resolve_quiet(t: &ast::TypeName) -> Option<Type> {
    if t.name == "none" { Some(Type::None) } else { Type::from_name(&t.name).ok() }
}

fn resolve(t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
    if t.name == "none" {
        return Some(Type::None);
    }
    match Type::from_name(&t.name) {
        Ok(ty) => Some(ty),
        Err(TypeError::NotYet) => {
            diags.push(Diagnostic::at(&codes::E0901, t.span, format!("the type `{}` is not supported yet", t.name)));
            None
        }
        Err(TypeError::Unknown) => {
            let mut d = Diagnostic::at(&codes::E0201, t.span, format!("unknown type `{}`", t.name));
            if let Some(hint) = type_hint(&t.name) {
                d = d.help(hint);
            }
            diags.push(d);
            None
        }
    }
}

/// The type of a binding or parameter, which cannot be `none`.
fn resolve_binding_type(t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
    match resolve(t, diags) {
        Some(Type::None) => {
            diags.push(
                Diagnostic::at(&codes::E0104, t.span, "a binding cannot have type `none`")
                    .note("`none` holds no value"),
            );
            None
        }
        ty => ty,
    }
}

/// People arriving from other languages reach for these names.
fn type_hint(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let suggestion = match lower.as_str() {
        "int" | "i32" | "integer" | "long" | "short" => "sint32",
        "i8" => "sint8",
        "i16" => "sint16",
        "i64" | "isize" => "sint64",
        "i128" => "sint128",
        "uint" | "u32" | "unsigned" => "uint32",
        "u8" | "byte" | "char" => "uint8",
        "u16" => "uint16",
        "u64" | "usize" | "size_t" => "uint64",
        "u128" => "uint128",
        "float" | "double" | "f64" | "number" | "real" => "float64",
        "f32" => "float32",
        "boolean" => "bool",
        "void" | "nothing" | "unit" => "none",
        "fn" | "func" | "function" | "def" | "fun" => {
            return Some("there is no function keyword: a function starts with its return type, like `none main() { }`".into());
        }
        _ if lower.starts_with("uint") || lower.starts_with("sint") => {
            return Some("integer types come in 8, 16, 32, 64 and 128 bits".into());
        }
        _ if lower.starts_with("float") => return Some("the float types are `float32` and `float64`".into()),
        _ => return None,
    };
    Some(format!("Drachm calls this `{suggestion}`"))
}

/// An expression is either still an untyped constant or already has a type.
enum Val {
    Const(Const),
    Expr(hir::Expr),
}

/// Where a value came from in the source, for messages about it.
#[derive(Clone, Copy)]
struct Origin {
    span: Span,
    /// Needs parentheses before `::to<T>()` can be appended.
    compound: bool,
}

fn origin(e: &ast::Expr) -> Origin {
    let compound = !e.paren && matches!(e.kind, ExprKind::Unary(..) | ExprKind::Binary(..) | ExprKind::IncDec { .. });
    Origin { span: e.span, compound }
}

struct LocalInfo {
    name: String,
    ty: Option<Type>,
    mutable: bool,
    is_param: bool,
    span: Span,
    loop_depth: u32,
}

#[derive(Clone)]
struct Flow {
    /// Certainly assigned on every path to here.
    definite: Vec<bool>,
    /// Assigned on at least one path to here.
    maybe: Vec<bool>,
    reachable: bool,
}

impl Flow {
    /// The state after two branches rejoin.
    fn join(mut self, other: Flow) -> Flow {
        if !other.reachable {
            return self;
        }
        if !self.reachable {
            return other;
        }
        for (a, b) in self.definite.iter_mut().zip(&other.definite) {
            *a &= *b;
        }
        for (a, b) in self.maybe.iter_mut().zip(&other.maybe) {
            *a |= *b;
        }
        self
    }

    /// Forget locals declared inside a block that has ended.
    fn truncate(&mut self, len: usize) {
        self.definite.resize(len, false);
        self.maybe.resize(len, false);
    }
}

struct Checker<'a> {
    src: &'a str,
    sigs: &'a HashMap<String, Sig>,
    io_imported: bool,
    diags: &'a mut Vec<Diagnostic>,
    /// `None` if the return type did not resolve.
    ret: Option<Type>,
    scopes: Vec<HashMap<String, hir::LocalId>>,
    locals: Vec<LocalInfo>,
    flow: Flow,
    loop_depth: u32,
    /// The loops and `if`/`match` levels around the current statement, innermost last.
    targets: Vec<Target>,
    /// Labels handed out so far in this function.
    labels: hir::Label,
}

fn int_lit(bits: u128, ty: Type) -> hir::Expr {
    hir::Expr { kind: hir::ExprKind::Int(bits), ty }
}

impl Checker<'_> {
    fn lookup(&self, name: &str) -> Option<hir::LocalId> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn declare(&mut self, name: &str, ty: Option<Type>, mutable: bool, span: Span, is_param: bool) -> hir::LocalId {
        let id = self.locals.len();
        self.locals.push(LocalInfo { name: name.to_string(), ty, mutable, is_param, span, loop_depth: self.loop_depth });
        self.scopes.last_mut().unwrap().insert(name.to_string(), id);
        self.flow.definite.resize(id + 1, false);
        self.flow.maybe.resize(id + 1, false);
        id
    }

    fn text(&self, span: Span) -> &str {
        &self.src[span.start.min(self.src.len())..span.end.min(self.src.len())]
    }

    /// Source text of a value for use inside a message, if it is short.
    fn quote(&self, span: Span, fallback: &str) -> String {
        let t = self.text(span);
        if t.len() <= 32 && !t.contains('\n') { format!("`{t}`") } else { fallback.to_string() }
    }

    fn block(&mut self, b: &ast::Block) -> hir::Block {
        self.scopes.push(HashMap::new());
        let mut out = Vec::new();
        for s in b {
            self.stmt(s, &mut out);
        }
        self.scopes.pop();
        out
    }

    /// Check a branch starting from flow state `from`; returns the state at its end.
    fn branch(&mut self, b: &ast::Block, from: Flow) -> (hir::Block, Flow) {
        let len = from.definite.len();
        self.flow = from;
        let block = self.block(b);
        self.flow.truncate(len);
        (block, self.flow.clone())
    }

    fn stmt(&mut self, s: &Stmt, out: &mut hir::Block) {
        match s {
            Stmt::Let { mutable, ty, name, name_span, value, span: _ } => {
                let ty = resolve_binding_type(ty, self.diags);
                // The value is checked before the name exists, so `uint32 x = x;`
                // refers to an outer `x`.
                let init = value.as_ref().map(|v| match ty {
                    Some(t) => self.coerce(v, t),
                    None => {
                        self.expr(v);
                        None
                    }
                });
                let id = self.declare(name, ty, *mutable, *name_span, false);
                if let Some(init) = init {
                    self.flow.definite[id] = true;
                    self.flow.maybe[id] = true;
                    if let Some(e) = init {
                        out.push(hir::Stmt::Assign(id, e));
                    }
                }
            }
            Stmt::Assign { name, name_span, op, value, span } => {
                let Some(id) = self.lookup(name) else {
                    self.diags.push(Diagnostic::at(&codes::E0201, *name_span, format!("unknown binding `{name}`")));
                    self.expr(value);
                    return;
                };
                let ty = self.locals[id].ty;
                let rhs = match op {
                    None => {
                        self.check_assignable(id, *name_span, false);
                        match ty {
                            Some(t) => self.coerce(value, t),
                            None => {
                                self.expr(value);
                                None
                            }
                        }
                    }
                    Some(op) => {
                        // `x += v` is `x = x + v`: it reads `x` first.
                        self.check_assignable(id, *name_span, true);
                        let current = self.read_local(id, *name_span);
                        let rv = self.expr(value);
                        match (current, rv) {
                            (Some(l), Some(r)) => {
                                let lo = Origin { span: *name_span, compound: false };
                                self.binary_vals(*op, Val::Expr(l), lo, r, origin(value), *span).and_then(|v| match v {
                                    Val::Expr(e) => Some(e),
                                    Val::Const(_) => None,
                                })
                            }
                            _ => None,
                        }
                    }
                };
                self.flow.definite[id] = true;
                self.flow.maybe[id] = true;
                if let Some(e) = rhs {
                    out.push(hir::Stmt::Assign(id, e));
                }
            }
            Stmt::If { branches, els, span } => {
                let label = self.new_label();
                let mut parts: Vec<(Test, &ast::Block)> = branches.iter().map(|(c, b)| (Test::Cond(c), b)).collect();
                if let Some(els) = els {
                    parts.push((Test::Always, els));
                }
                if let Some(branches) = self.chain(label, *span, false, parts, false) {
                    out.push(hir::Stmt::If { label, branches });
                }
            }
            Stmt::While { cond, body, span } => {
                let c = self.coerce(cond, Type::Bool);
                let label = self.new_label();
                let forever = matches!(cond.kind, ExprKind::Bool(true));
                let (body, step) = self.looped(label, *span, forever, body, None);
                if let Some(c) = c {
                    out.push(hir::Stmt::Loop { label, cond: Some(c), body, step });
                }
            }
            Stmt::For { init, cond, step, body, span } => {
                // The init binding belongs to the loop.
                let len = self.flow.definite.len();
                self.scopes.push(HashMap::new());
                if let Some(init) = init {
                    self.stmt(init, out);
                }
                let c = cond.as_ref().map(|c| self.coerce(c, Type::Bool));
                let forever = cond.as_ref().is_none_or(|c| matches!(c.kind, ExprKind::Bool(true)));
                let label = self.new_label();
                let (body, step) = self.looped(label, *span, forever, body, step.as_deref());
                self.scopes.pop();
                self.flow.truncate(len);
                match c {
                    Some(None) => {}
                    c => out.push(hir::Stmt::Loop { label, cond: c.flatten(), body, step }),
                }
            }
            Stmt::ForEach { .. } => self.for_each(s, out),
            Stmt::Match { value, cases, span } => self.match_stmt(value, cases, *span, out),
            Stmt::Break { cont, count, span } => {
                if let Some(j) = self.break_target(*cont, *count, *span) {
                    out.push(hir::Stmt::Jump(j));
                }
                self.flow.reachable = false;
            }
            Stmt::Skip { target, span } => {
                if let Some(j) = self.skip_target(*target, *span) {
                    out.push(hir::Stmt::Jump(j));
                }
                self.flow.reachable = false;
            }
            Stmt::Return { value, span } => {
                let v = match (value, self.ret) {
                    (Some(v), Some(Type::None)) => {
                        self.expr(v);
                        self.diags.push(
                            Diagnostic::at(&codes::E0101, v.span, "this function returns nothing, but a value is returned")
                                .note("remove this value")
                                .help("or change the function's return type from `none`"),
                        );
                        None
                    }
                    (Some(v), Some(t)) => self.coerce(v, t).map(Some),
                    (Some(v), None) => {
                        self.expr(v);
                        None
                    }
                    (None, Some(t)) if t != Type::None => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0105, *span, format!("this function must return a `{t}`"))
                                .note("returns nothing"),
                        );
                        None
                    }
                    (None, _) => Some(None),
                };
                self.flow.reachable = false;
                if let Some(v) = v {
                    out.push(hir::Stmt::Return(v));
                }
            }
            Stmt::Expr(e) => {
                if let Some(Val::Expr(e)) = self.expr(e) {
                    out.push(hir::Stmt::Expr(e));
                }
            }
        }
    }

    /// Report if local `id` may not be assigned (or changed, for `++` and `+=`) here.
    fn check_assignable(&mut self, id: hir::LocalId, at: Span, changes: bool) {
        let l = &self.locals[id];
        if l.mutable {
            return;
        }
        let name = &l.name;
        let ty = l.ty.map_or(String::new(), |t| format!("{t} "));
        let d = if l.is_param {
            Diagnostic::at(&codes::E0205, at, format!("cannot change parameter `{name}`: it is not `mut`"))
                .note("changed here")
                .label(l.span, "declared here without `mut`")
                .help(format!("declare it as `mut {ty}{name}` to allow changes"))
        } else if changes || self.flow.maybe[id] {
            Diagnostic::at(&codes::E0205, at, format!("cannot change `{name}`: it is not `mut`"))
                .note(if changes { "changed here" } else { "assigned a second time here" })
                .label(l.span, "declared here without `mut`")
                .help(format!("declare it as `mut {ty}{name}` to allow changes"))
        } else if l.loop_depth != self.loop_depth {
            // Its first assignment, but a loop would repeat it.
            Diagnostic::at(&codes::E0205, at, format!("cannot assign `{name}` inside a loop: it is not `mut`"))
                .note("the loop would assign it again each time round")
                .label(l.span, "declared here, outside the loop, without `mut`")
                .help(format!("declare it as `mut {ty}{name}`, or move the declaration into the loop"))
        } else {
            return;
        };
        self.diags.push(d);
    }

    /// A read of local `id`; reports if it may not have a value yet.
    fn read_local(&mut self, id: hir::LocalId, at: Span) -> Option<hir::Expr> {
        let l = &self.locals[id];
        if !self.flow.definite[id] {
            let name = &l.name;
            let label = if self.flow.maybe[id] { "only some paths to here give it one" } else { "nothing has been assigned to it yet" };
            let d = Diagnostic::at(&codes::E0207, at, format!("`{name}` is read before it certainly has a value"))
                .note(label)
                .label(l.span, "declared here without a value")
                .help(format!("give `{name}` a value on every path before this point"));
            self.diags.push(d);
            // One report per binding is enough.
            self.flow.definite[id] = true;
        }
        let ty = self.locals[id].ty?;
        Some(hir::Expr { kind: hir::ExprKind::Local(id), ty })
    }

    /// Check `e` and convert it to `to`, reporting a mismatch or a lossy conversion.
    fn coerce(&mut self, e: &ast::Expr, to: Type) -> Option<hir::Expr> {
        let v = self.expr(e)?;
        self.into(v, to, origin(e))
    }

    fn materialise(&mut self, v: Val, span: Span) -> Option<hir::Expr> {
        match v {
            Val::Expr(e) => Some(e),
            Val::Const(c) => match c.default_type() {
                Some(ty) => Some(self.fit_const(&c, ty, span, true)),
                None => {
                    self.diags.push(
                        Diagnostic::at(&codes::E0002, span, "this constant is too large for any number type")
                            .note("does not fit in 128 bits"),
                    );
                    None
                }
            },
        }
    }

    fn into(&mut self, v: Val, to: Type, from: Origin) -> Option<hir::Expr> {
        match v {
            Val::Const(c) if to.is_numeric() => Some(self.fit_const(&c, to, from.span, false)),
            Val::Const(_) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0101, from.span, format!("expected `{to}`, found a number"))
                        .note("this is a number"),
                );
                None
            }
            Val::Expr(e) if e.ty == to => Some(e),
            Val::Expr(e) if e.ty == Type::None => {
                self.diags.push(
                    Diagnostic::at(&codes::E0104, from.span, format!("expected `{to}`, but this produces no value"))
                        .note("returns `none`"),
                );
                None
            }
            Val::Expr(e) if e.ty.is_numeric() && to.is_numeric() => {
                if !e.ty.converts_losslessly_to(to) {
                    self.warn_lossy(e.ty, to, from);
                }
                Some(hir::Expr { kind: hir::ExprKind::Convert(Box::new(e)), ty: to })
            }
            Val::Expr(e) => {
                let mut d = Diagnostic::at(&codes::E0101, from.span, format!("expected `{to}`, found `{}`", e.ty))
                    .note(format!("this is `{}`", e.ty));
                if e.ty == Type::Bool && to.is_numeric() || e.ty.is_numeric() && to == Type::Bool {
                    d = d.help("`bool` is not a number and never converts to or from one");
                }
                self.diags.push(d);
                None
            }
        }
    }

    fn warn_lossy(&mut self, from: Type, to: Type, at: Origin) {
        let note = match (from, to) {
            (Type::Int { .. }, Type::Int { .. }) => format!(
                "`{from}` holds {}; `{to}` holds {}; values outside that wrap around",
                from.describe_range(),
                to.describe_range()
            ),
            (Type::Float(_), Type::Int { .. }) => format!(
                "`{to}` holds only whole numbers from {}; the fraction is dropped and values outside that wrap around",
                to.describe_range()
            ),
            (Type::Int { .. }, Type::Float(_)) => {
                format!("`{to}` holds {}; `{from}` goes up to {}; larger values are rounded", to.describe_range(), from.int_range().unwrap().1)
            }
            _ => format!("`{to}` keeps about 7 significant digits, `{from}` about 16; the value is rounded"),
        };
        let text = self.text(at.span);
        let fix = if text.len() > 40 || text.contains('\n') {
            format!("add `::to<{to}>()` to say the conversion is intended")
        } else if at.compound {
            format!("write `({text})::to<{to}>()` to say the conversion is intended")
        } else {
            format!("write `{text}::to<{to}>()` to say the conversion is intended")
        };
        self.diags.push(
            Diagnostic::at(&codes::W0102, at.span, format!("converting `{from}` to `{to}` can lose information"))
                .note(format!("this is `{from}`"))
                .note_line(note)
                .help(fix),
        );
    }

    /// Give constant `c` the type `to`. Unless `explicit`, a value that does
    /// not fit is reported (W0102); either way it wraps.
    fn fit_const(&mut self, c: &Const, to: Type, span: Span, explicit: bool) -> hir::Expr {
        let e = match to {
            Type::Int { .. } => int_lit(c.wrap_to_int(to), to),
            _ => hir::Expr { kind: hir::ExprKind::Float(c.to_float(to)), ty: to },
        };
        if !explicit && !c.fits(to) {
            let what = self.quote(span, "this constant");
            let (becomes, note) = match (&e.kind, c) {
                (hir::ExprKind::Int(bits), Const::Dec(r)) if !r.is_integer() => (
                    match Const::from_int_bits(*bits, to) {
                        Const::Int(v) => v.to_string(),
                        Const::Dec(_) => unreachable!(),
                    },
                    format!("`{to}` holds only whole numbers, from {}; the fraction is dropped", to.describe_range()),
                ),
                (hir::ExprKind::Int(bits), _) => (
                    match Const::from_int_bits(*bits, to) {
                        Const::Int(v) => v.to_string(),
                        Const::Dec(_) => unreachable!(),
                    },
                    format!("`{to}` holds {}; a constant outside that wraps around", to.describe_range()),
                ),
                (hir::ExprKind::Float(f), _) if f.is_infinite() => {
                    ("infinity".to_string(), format!("it is beyond the largest `{to}`"))
                }
                (hir::ExprKind::Float(f), _) => {
                    (format!("{f:?}"), format!("`{to}` holds {}; this one is rounded", to.describe_range()))
                }
                _ => unreachable!(),
            };
            self.diags.push(
                Diagnostic::at(&codes::W0102, span, format!("{what} does not fit `{to}`"))
                    .note(format!("becomes {becomes}"))
                    .note_line(note)
                    .help(format!("use a value that fits or a bigger type; `::to<{to}>()` converts without this warning")),
            );
        }
        e
    }

    /// The value of `e`, or `None` if an error was already reported inside it.
    fn expr(&mut self, e: &ast::Expr) -> Option<Val> {
        let typed = |kind, ty| Some(Val::Expr(hir::Expr { kind, ty }));
        match &e.kind {
            ExprKind::Num(n) => match n.suffix {
                Some(ty) => Some(Val::Expr(self.fit_const(&n.value, ty, e.span, false))),
                None => Some(Val::Const(n.value.clone())),
            },
            ExprKind::Bool(b) => typed(hir::ExprKind::Bool(*b), Type::Bool),
            ExprKind::Str(s) => typed(hir::ExprKind::Str(s.clone()), Type::Str),
            ExprKind::Var(name) => match self.lookup(name) {
                Some(id) => self.read_local(id, e.span).map(Val::Expr),
                None => {
                    let mut d = Diagnostic::at(&codes::E0201, e.span, format!("unknown name `{name}`"));
                    if name == "io" {
                        d = d.help("`io` is a module; call its functions like `io::outln(...)`");
                    }
                    self.diags.push(d);
                    None
                }
            },
            ExprKind::Unary(op, inner) => self.unary(*op, inner, e.span),
            ExprKind::Binary(op @ (BinOp::And | BinOp::Or), l, r) => {
                let lhs = self.coerce(l, Type::Bool);
                let rhs = self.coerce(r, Type::Bool);
                typed(hir::ExprKind::Logic { and: *op == BinOp::And, lhs: Box::new(lhs?), rhs: Box::new(rhs?) }, Type::Bool)
            }
            ExprKind::Binary(op, l, r) => {
                let lv = self.expr(l);
                let rv = self.expr(r);
                self.binary_vals(*op, lv?, origin(l), rv?, origin(r), e.span)
            }
            ExprKind::IncDec { inc, prefix, target } => {
                let sym = if *inc { "++" } else { "--" };
                let ExprKind::Var(name) = &target.kind else {
                    self.diags.push(
                        Diagnostic::at(&codes::E0005, target.span, format!("`{sym}` only works on a binding"))
                            .note("this is not a binding"),
                    );
                    return None;
                };
                let Some(id) = self.lookup(name) else {
                    self.diags.push(Diagnostic::at(&codes::E0201, target.span, format!("unknown binding `{name}`")));
                    return None;
                };
                self.check_assignable(id, target.span, true);
                let cur = self.read_local(id, target.span)?;
                if !cur.ty.is_numeric() {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, target.span, format!("`{sym}` needs a number, found `{}`", cur.ty))
                            .note(format!("this is `{}`", cur.ty)),
                    );
                    return None;
                }
                typed(hir::ExprKind::IncDec { local: id, inc: *inc, prefix: *prefix }, cur.ty)
            }
            ExprKind::Call { name, args } => {
                let Some(sig) = self.sigs.get(name) else {
                    self.diags.push(Diagnostic::at(&codes::E0201, e.span, format!("unknown function `{name}`")));
                    for a in args {
                        self.expr(a);
                    }
                    return None;
                };
                if sig.params.len() != args.len() {
                    let n = sig.params.len();
                    self.diags.push(
                        Diagnostic::at(
                            &codes::E0103,
                            e.span,
                            format!("`{name}` takes {n} argument{}, but {} given", if n == 1 { "" } else { "s" }, args.len()),
                        )
                        .label(sig.span, format!("`{name}` is defined here")),
                    );
                    for a in args {
                        self.expr(a);
                    }
                    return sig.ret.map(|ty| Val::Expr(hir::Expr { kind: hir::ExprKind::Call(sig.id, Vec::new()), ty }));
                }
                let mut out = Vec::new();
                let mut ok = true;
                for (a, p) in args.iter().zip(&sig.params) {
                    let v = match p {
                        Some(p) => self.coerce(a, *p),
                        None => {
                            self.expr(a);
                            None
                        }
                    };
                    match v {
                        Some(v) => out.push(v),
                        None => ok = false,
                    }
                }
                let ty = sig.ret?;
                if !ok {
                    out.clear();
                }
                typed(hir::ExprKind::Call(sig.id, out), ty)
            }
            ExprKind::Method { recv, name, name_span, targs, args } => {
                if let ExprKind::Var(module) = &recv.kind
                    && self.lookup(module).is_none()
                    && module == "io"
                {
                    return self.io_call(name, *name_span, recv.span, targs, args, e.span);
                }
                self.method(recv, name, *name_span, targs, args)
            }
            ExprKind::Range(..) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0009, e.span, "a range can only be used after `of` in a `for`, or as a `case` pattern")
                        .note("this is a range")
                        .help("to go through the numbers from a to b, write `for (uint32 i of a..b) { }`"),
                );
                None
            }
        }
    }

    fn io_call(&mut self, name: &str, name_span: Span, module_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        if !self.io_imported {
            self.diags.push(
                Diagnostic::at(&codes::E0206, module_span, "module `io` is used but not imported")
                    .note("not imported")
                    .help("add `in io;` at the top of the file"),
            );
            // Say it once per file.
            self.io_imported = true;
        }
        let newline = match name {
            "out" => false,
            "outln" => true,
            _ => {
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("module `io` has no function `{name}`"))
                        .help("it has `io::out` and `io::outln`"),
                );
                for a in args {
                    self.expr(a);
                }
                return None;
            }
        };
        if let Some(t) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, t.span, format!("`io::{name}` takes no type arguments")));
        }
        if args.len() != 1 {
            self.diags.push(Diagnostic::at(
                &codes::E0103,
                span,
                format!("`io::{name}` takes 1 argument, but {} given", args.len()),
            ));
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let arg = match self.expr(&args[0])? {
            Val::Expr(a) if a.ty == Type::Str => a,
            other => {
                let found = match &other {
                    Val::Const(_) => "a number".to_string(),
                    Val::Expr(a) => format!("`{}`", a.ty),
                };
                let o = origin(&args[0]);
                let text = self.text(o.span);
                let fix = if o.compound { format!("({text})::text()") } else { format!("{text}::text()") };
                let mut d = Diagnostic::at(&codes::E0101, o.span, format!("`io::{name}` prints text, found {found}"))
                    .note(format!("this is {found}"));
                if !matches!(&other, Val::Expr(a) if a.ty == Type::None) && !text.contains('\n') {
                    d = d.help(format!("turn it into text: `io::{name}({fix})`"));
                }
                self.diags.push(d);
                return None;
            }
        };
        Some(Val::Expr(hir::Expr { kind: hir::ExprKind::Out { newline, arg: Box::new(arg) }, ty: Type::None }))
    }

    fn method(&mut self, recv: &ast::Expr, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr]) -> Option<Val> {
        let value = self.expr(recv);
        for a in args {
            if !matches!(name, "text" | "to") {
                self.expr(a);
            }
        }
        let value = value?;
        let no_args = |cx: &mut Self| {
            if let Some(a) = args.first() {
                cx.diags.push(Diagnostic::at(&codes::E0103, a.span, format!("`{name}` takes no arguments")));
            }
        };
        match name {
            "text" => {
                no_args(self);
                if let Some(t) = targs.first() {
                    self.diags.push(Diagnostic::at(&codes::E0103, t.span, "`text` takes no type arguments"));
                }
                let v = self.materialise(value, recv.span)?;
                if !v.ty.is_numeric() && v.ty != Type::Bool {
                    self.diags.push(
                        Diagnostic::at(&codes::E0201, name_span, format!("`{}` has no method `text`", v.ty))
                            .label(recv.span, format!("this is `{}`", v.ty)),
                    );
                    return None;
                }
                Some(Val::Expr(hir::Expr { kind: hir::ExprKind::Text(Box::new(v)), ty: Type::Str }))
            }
            "to" => {
                no_args(self);
                if targs.len() != 1 {
                    self.diags.push(
                        Diagnostic::at(&codes::E0103, name_span, "`to` needs the type to convert to")
                            .help("for example `value::to<uint8>()`"),
                    );
                    return None;
                }
                let to = resolve(&targs[0], self.diags)?;
                if !to.is_numeric() {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, targs[0].span, format!("`to` converts between number types, not to `{to}`")),
                    );
                    return None;
                }
                match value {
                    Val::Const(c) => Some(Val::Expr(self.fit_const(&c, to, recv.span, true))),
                    Val::Expr(v) if v.ty == to => Some(Val::Expr(v)),
                    Val::Expr(v) if v.ty.is_numeric() => {
                        Some(Val::Expr(hir::Expr { kind: hir::ExprKind::Convert(Box::new(v)), ty: to }))
                    }
                    Val::Expr(v) => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0101, recv.span, format!("`to` converts numbers, found `{}`", v.ty))
                                .note(format!("this is `{}`", v.ty)),
                        );
                        None
                    }
                }
            }
            _ => {
                let on = match &value {
                    Val::Const(_) => "numbers have".to_string(),
                    Val::Expr(v) => format!("`{}` has", v.ty),
                };
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("{on} no method `{name}`"))
                        .help("the methods so far are `text()` and `to<T>()`"),
                );
                None
            }
        }
    }

    fn unary(&mut self, op: UnOp, inner: &ast::Expr, span: Span) -> Option<Val> {
        if op == UnOp::Not {
            let v = self.coerce(inner, Type::Bool)?;
            return Some(Val::Expr(hir::Expr { kind: hir::ExprKind::Not(Box::new(v)), ty: Type::Bool }));
        }
        let (sym, wants) = if op == UnOp::Neg { ("-", "a number") } else { ("~", "a whole number") };
        match self.expr(inner)? {
            Val::Const(c) if op == UnOp::Neg => Some(Val::Const(c.neg())),
            Val::Const(c) => match c.bit_not() {
                Ok(c) => Some(Val::Const(c)),
                Err(_) => {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, span, "`~` needs a whole number, found a decimal")
                            .label(inner.span, "this is a decimal"),
                    );
                    None
                }
            },
            Val::Expr(v) if (op == UnOp::Neg && v.ty.is_numeric()) || (op == UnOp::BitNot && v.ty.is_int()) => {
                let ty = v.ty;
                let kind = if op == UnOp::Neg { hir::ExprKind::Neg(Box::new(v)) } else { hir::ExprKind::BitNot(Box::new(v)) };
                Some(Val::Expr(hir::Expr { kind, ty }))
            }
            Val::Expr(v) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0101, inner.span, format!("`{sym}` needs {wants}, found `{}`", v.ty))
                        .note(format!("this is `{}`", v.ty)),
                );
                None
            }
        }
    }

    fn fold_error(&mut self, err: FoldError, op: BinOp, span: Span, r: Origin) {
        let sym = op.symbol();
        let d = match err {
            FoldError::DivisionByZero => Diagnostic::at(&codes::E0107, span, "this constant expression divides by zero")
                .label(r.span, "this is zero"),
            FoldError::TooLarge => Diagnostic::at(&codes::E0002, span, "this constant expression is too large to evaluate")
                .note("the result would not fit any number type"),
            FoldError::NeedsWhole => Diagnostic::at(&codes::E0101, span, format!("`{sym}` needs whole numbers, found a decimal")),
            FoldError::NegativeShift => Diagnostic::at(&codes::E0101, r.span, format!("cannot `{sym}` an untyped constant by a negative amount"))
                .note("negative")
                .help("give the left side a type (for example a `u32` suffix) for the amount to wrap around its width"),
            FoldError::NotANumber => Diagnostic::at(&codes::E0107, span, "this constant expression has no numeric result"),
        };
        self.diags.push(d);
    }

    /// A two-sided operator applied to already-checked operands.
    fn binary_vals(&mut self, op: BinOp, lv: Val, lo: Origin, rv: Val, ro: Origin, span: Span) -> Option<Val> {
        let sym = op.symbol();
        let class = op.class();

        if let (Val::Const(a), Val::Const(b)) = (&lv, &rv) {
            return match Const::binary(op, a, b) {
                Ok(Folded::Value(c)) => Some(Val::Const(c)),
                Ok(Folded::Bool(b)) => Some(Val::Expr(hir::Expr { kind: hir::ExprKind::Bool(b), ty: Type::Bool })),
                Err(err) => {
                    self.fold_error(err, op, span, ro);
                    None
                }
            };
        }

        // An untyped constant on the left takes the right side's type if it
        // fits there exactly; otherwise it gets its own default type and leads.
        let lv = match (lv, &rv) {
            (Val::Const(c), Val::Expr(r)) => {
                let counts = class == OpClass::Shift || (op == BinOp::Pow && !c.is_decimal() && r.ty.is_int());
                if !counts && r.ty.is_numeric() && c.fits(r.ty) {
                    Val::Expr(self.fit_const(&c, r.ty, lo.span, true))
                } else {
                    Val::Expr(self.materialise(Val::Const(c), lo.span)?)
                }
            }
            (lv, _) => lv,
        };
        let Val::Expr(lhs) = lv else { unreachable!() };
        let lead = lhs.ty;

        let wants = match class {
            OpClass::Arithmetic => "numbers",
            OpClass::Shift | OpClass::Bitwise => "whole numbers",
            OpClass::Comparison | OpClass::Logic => "numbers",
        };
        let lead_ok = match class {
            OpClass::Arithmetic => lead.is_numeric(),
            OpClass::Shift | OpClass::Bitwise => lead.is_int(),
            OpClass::Comparison => lead.is_numeric() || (lead == Type::Bool && matches!(op, BinOp::Eq | BinOp::Ne)),
            OpClass::Logic => unreachable!(),
        };
        if !lead_ok {
            let mut d = Diagnostic::at(&codes::E0101, lo.span, format!("`{sym}` needs {wants}, found `{lead}`"))
                .note(format!("this is `{lead}`"));
            if lead == Type::Bool && class == OpClass::Comparison {
                d = d.help("`bool` values can only be compared with `==` and `!=`");
            }
            self.diags.push(d);
            return None;
        }

        // Counts: the right side of a shift or an integer power keeps its own type.
        if class == OpClass::Shift || (op == BinOp::Pow && lead.is_int()) {
            let count = match rv {
                Val::Const(c) if !c.is_decimal() => {
                    if class == OpClass::Shift {
                        self.check_shift_amount(&c, lead, sym, ro);
                    }
                    // Only the low bits of a shift amount matter, so any width will do.
                    match c.default_type() {
                        Some(ty) => self.fit_const(&c, ty, ro.span, true),
                        None if class == OpClass::Shift => int_lit(c.wrap_to_int(Type::U32), Type::U32),
                        None => self.materialise(Val::Const(c), ro.span)?,
                    }
                }
                Val::Expr(r) if r.ty.is_int() => r,
                // A fractional exponent on a whole-number base: the left side
                // leads, so the exponent becomes a whole number (with a warning).
                other if op == BinOp::Pow && !matches!(&other, Val::Expr(r) if !r.ty.is_numeric()) => self.into(other, lead, ro)?,
                other => {
                    let found = match &other {
                        Val::Const(_) => "a decimal".to_string(),
                        Val::Expr(r) => format!("`{}`", r.ty),
                    };
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, ro.span, format!("`{sym}` needs a whole number on the right, found {found}"))
                            .note(format!("this is {found}")),
                    );
                    return None;
                }
            };
            let kind = if class == OpClass::Shift {
                hir::ExprKind::Shift { left: op == BinOp::Shl, value: Box::new(lhs), amount: Box::new(count) }
            } else {
                hir::ExprKind::IntPow(Box::new(lhs), Box::new(count))
            };
            return Some(Val::Expr(hir::Expr { kind, ty: lead }));
        }

        // Everything else: the left side leads and the right side converts to it.
        if let Val::Expr(r) = &rv
            && r.ty != lead
            && !(r.ty.is_numeric() && lead.is_numeric())
        {
            let mut d = Diagnostic::at(&codes::E0101, ro.span, format!("`{sym}` has `{lead}` on the left but `{}` on the right", r.ty))
                .note(format!("this is `{}`", r.ty))
                .label(lo.span, format!("this is `{lead}`"));
            if r.ty == Type::Bool || lead == Type::Bool {
                d = d.help("`bool` is not a number and never converts to or from one");
            }
            self.diags.push(d);
            return None;
        }
        if let (Val::Const(c), BinOp::Div | BinOp::Rem, true) = (&rv, op, lead.is_int())
            && c.wrap_to_int(lead) == 0
            && c.fits(lead)
        {
            self.diags.push(
                Diagnostic::at(&codes::E0107, span, "this divides by a constant zero")
                    .label(ro.span, "this is zero")
                    .help("whole-number division by zero always panics"),
            );
            return None;
        }
        let rhs = self.into(rv, lead, ro)?;
        let hop = match op {
            BinOp::Add => hir::BinOp::Add,
            BinOp::Sub => hir::BinOp::Sub,
            BinOp::Mul => hir::BinOp::Mul,
            BinOp::Div => hir::BinOp::Div,
            BinOp::Rem => hir::BinOp::Rem,
            BinOp::Pow => hir::BinOp::Pow,
            BinOp::BitAnd => hir::BinOp::BitAnd,
            BinOp::BitOr => hir::BinOp::BitOr,
            BinOp::BitXor => hir::BinOp::BitXor,
            BinOp::Eq => hir::BinOp::Eq,
            BinOp::Ne => hir::BinOp::Ne,
            BinOp::Lt => hir::BinOp::Lt,
            BinOp::Le => hir::BinOp::Le,
            BinOp::Gt => hir::BinOp::Gt,
            BinOp::Ge => hir::BinOp::Ge,
            BinOp::Shl | BinOp::Shr | BinOp::And | BinOp::Or => unreachable!(),
        };
        let ty = if hop.is_comparison() { Type::Bool } else { lead };
        Some(Val::Expr(hir::Expr { kind: hir::ExprKind::Binary(hop, Box::new(lhs), Box::new(rhs)), ty }))
    }

    /// A constant shift amount outside `0..width` wraps around the width (W0106).
    fn check_shift_amount(&mut self, amount: &Const, value_ty: Type, sym: &str, at: Origin) {
        let Type::Int { bits, .. } = value_ty else { return };
        let width_ty = Type::Int { bits: 128, signed: false };
        let wrapped = amount.wrap_to_int(width_ty) % bits as u128;
        if matches!(amount, Const::Int(v) if *v == wrapped.into()) {
            return;
        }
        let what = self.quote(at.span, "this amount");
        self.diags.push(
            Diagnostic::at(&codes::W0106, at.span, format!("shifting a `{value_ty}` by {what} wraps around to a shift by {wrapped}"))
                .note(format!("`{value_ty}` is {bits} bits wide"))
                .note_line(format!("a shift amount outside 0 to {} wraps around the width", bits - 1))
                .help(format!("write `{sym} {wrapped}` if that is what you mean, or use a wider type")),
        );
    }
}
