//! Building MIR from HIR (spec 2.5). Phase A: the current HIR, translated
//! faithfully, legacy ownership nodes included, no scope frames.
//!
//! The HIR is read through two functions only, `place` and `expr_span`, so
//! that the HIR rewrite of WP0a touches nothing else here except `match` arms.

use std::collections::HashMap;

use super::*;
use crate::hir::{Expr, ExprKind, Jump, Label, Stmt, StrFn};

/// A function's name for dumps: free functions lose their `drachm_f_` prefix.
pub fn display_name(symbol: &str) -> &str {
    symbol.strip_prefix("drachm_f_").unwrap_or(symbol)
}

/// A HIR place as a MIR place.
fn place(p: &hir::Place) -> Place {
    Place { local: Local::new(p.local + 1), proj: p.fields.iter().map(|&f| Proj::Field(f)).collect() }
}

/// The source span of an expression (phase A: the HIR has none).
fn expr_span(_: &Expr) -> Span {
    Span::new(0, 0)
}

/// Does a method call borrow its receiver `&mut`? Until WP3 infers it: always.
fn receiver_mutable(_f: FuncId) -> bool {
    true
}

pub fn program(hir: &hir::Program, _diags: &mut Vec<Diagnostic>) -> Program {
    let mut strings = IndexVec::new();
    let mut str_ids = HashMap::new();
    let bodies = hir
        .funcs
        .iter()
        .enumerate()
        .map(|(id, f)| Some(Builder::new(hir, id, f, &mut strings, &mut str_ids).build()))
        .collect();
    Program {
        bodies,
        main: hir.main,
        strings,
        self_modes: hir.funcs.iter().map(|f| f.has_self.then_some(SelfMode::Mut)).collect(),
        lives: hir.funcs.iter().map(|_| LivesSig::Unknown).collect(),
    }
}

/// A name no program can write: typeck's hidden locals (`$3`, `range start`).
fn is_hidden(name: &str) -> bool {
    !name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

struct BlockBuild {
    statements: Vec<Statement>,
    terminator: Option<Terminator>,
}

enum Target {
    Loop { exit: BlockId, step: BlockId },
    /// `tests[n]` is where `Next(l, n)` goes; the last entry is `exit`.
    If { exit: BlockId, tests: Vec<BlockId> },
}

struct Builder<'a> {
    hir: &'a hir::Program,
    id: FuncId,
    func: &'a hir::Func,
    strings: &'a mut IndexVec<StrId, String>,
    str_ids: &'a mut HashMap<String, StrId>,
    locals: IndexVec<Local, LocalDecl>,
    blocks: Vec<BlockBuild>,
    cur: BlockId,
    targets: HashMap<Label, Target>,
    span: Span,
}

impl<'a> Builder<'a> {
    fn new(
        hir: &'a hir::Program,
        id: FuncId,
        func: &'a hir::Func,
        strings: &'a mut IndexVec<StrId, String>,
        str_ids: &'a mut HashMap<String, StrId>,
    ) -> Self {
        let mut b = Builder {
            hir,
            id,
            func,
            strings,
            str_ids,
            locals: IndexVec::new(),
            blocks: Vec::new(),
            cur: BlockId(0),
            targets: HashMap::new(),
            span: Span::new(0, 0),
        };
        b.cur = b.new_block();
        b
    }

    fn build(mut self) -> Body {
        let func = self.func;
        self.decl(func.ret, LocalKind::Return);
        for (i, l) in func.locals.iter().enumerate() {
            let kind = if i < func.params {
                LocalKind::Param { name: l.name.clone(), mutability: Mutability::Fixed }
            } else if is_hidden(&l.name) {
                LocalKind::Temp
            } else {
                LocalKind::User { name: l.name.clone(), mutability: Mutability::Fixed }
            };
            let local = self.decl(l.ty, kind);
            if i >= func.params {
                self.push(StatementKind::StorageLive(local));
            }
        }

        self.block(&func.body);

        let returns_value = func.ret != Type::None;
        let end = self.cur;
        self.end(if returns_value { TerminatorKind::Unreachable } else { TerminatorKind::Return });
        let (blocks, remap) = self.prune();
        let fallthrough_end = if returns_value { remap[end.index()] } else { None };

        let mut scopes = IndexVec::new();
        scopes.push(Scope { parent: None, span: Span::new(0, 0), kind: ScopeKind::Function });
        Body {
            func: self.id,
            name: display_name(&func.symbol).to_string(),
            span: Span::new(0, 0),
            arg_count: func.params,
            locals: self.locals,
            blocks,
            scopes,
            unsafe_flag: false,
            unsafe_fn: false,
            fallthrough_end,
        }
    }

    /// Drop the blocks `bb0` cannot reach (code after `return`, the exit of a
    /// loop with no way out). Returns the blocks and, for each old block, its
    /// new id.
    fn prune(&mut self) -> (IndexVec<BlockId, BasicBlock>, Vec<Option<BlockId>>) {
        let n = self.blocks.len();
        let mut reachable = vec![false; n];
        let mut stack = vec![0usize];
        reachable[0] = true;
        while let Some(b) = stack.pop() {
            let t = self.blocks[b].terminator.as_ref().expect("a reachable block was never terminated");
            for s in traversal::successors(t) {
                if !reachable[s.index()] {
                    reachable[s.index()] = true;
                    stack.push(s.index());
                }
            }
        }
        let mut remap = vec![None; n];
        let mut next = 0;
        for i in 0..n {
            if reachable[i] {
                remap[i] = Some(BlockId::new(next));
                next += 1;
            }
        }
        let mut out = IndexVec::new();
        for (i, b) in std::mem::take(&mut self.blocks).into_iter().enumerate() {
            if !reachable[i] {
                continue;
            }
            let mut t = b.terminator.unwrap();
            map_targets(&mut t.kind, &mut |s| remap[s.index()].expect("target of a reachable block"));
            out.push(BasicBlock { statements: b.statements, terminator: t });
        }
        (out, remap)
    }

    // ---- low level -------------------------------------------------------

    fn decl(&mut self, ty: Type, kind: LocalKind) -> Local {
        self.locals.push(LocalDecl { ty, borrow: None, kind, span: self.span, scope: ScopeId(0), in_memory: false })
    }

    fn new_temp(&mut self, ty: Type) -> Local {
        self.decl(ty, LocalKind::Temp)
    }

    /// A temp holding a `&ty` (`&mut ty`).
    fn new_ref_temp(&mut self, ty: Type, mutable: bool) -> Local {
        let l = self.decl(ty, LocalKind::Temp);
        self.locals[l].borrow = Some(mutable);
        l
    }

    fn local(&self, id: hir::LocalId) -> Local {
        Local::new(id + 1)
    }

    fn new_block(&mut self) -> BlockId {
        self.blocks.push(BlockBuild { statements: Vec::new(), terminator: None });
        BlockId::new(self.blocks.len() - 1)
    }

    fn push(&mut self, kind: StatementKind) {
        let s = Statement { kind, span: self.span, unsafe_ctx: false };
        self.blocks[self.cur.index()].statements.push(s);
    }

    fn assign_stmt(&mut self, dest: Place, rv: Rvalue) {
        self.push(StatementKind::Assign(dest, rv));
    }

    /// Terminate the current block. `cur` must be replaced before more code
    /// is emitted.
    fn end(&mut self, kind: TerminatorKind) {
        let b = &mut self.blocks[self.cur.index()];
        assert!(b.terminator.is_none(), "block {} terminated twice", self.cur);
        b.terminator = Some(Terminator { kind, span: self.span, unsafe_ctx: false });
    }

    /// Terminate with a jump out of straight-line code (`return`, `break`):
    /// anything after it, up to the end of the HIR block, is dead.
    fn diverge(&mut self, kind: TerminatorKind) {
        self.end(kind);
        self.cur = self.new_block();
    }

    fn goto(&mut self, to: BlockId) {
        self.end(TerminatorKind::Goto(to));
    }

    fn intern(&mut self, s: &str) -> StrId {
        if let Some(&id) = self.str_ids.get(s) {
            return id;
        }
        let id = self.strings.push(s.to_string());
        self.str_ids.insert(s.to_string(), id);
        id
    }

    fn read(&self, p: Place, ty: Type) -> Operand {
        if is_copy(self.hir, ty) { Operand::Copy(p) } else { Operand::Move(p) }
    }

    /// Put an operand that reads a place into a fresh temp now, so that later
    /// operands cannot change what it reads (B1).
    fn freeze(&mut self, op: Operand, ty: Type) -> Operand {
        if matches!(op, Operand::Const(_)) {
            return op;
        }
        let t = self.new_temp(ty);
        self.assign_stmt(Place::local(t), Rvalue::Use(op));
        self.read(Place::local(t), ty)
    }

    /// What reading a value of type `none` gives. Nothing ever looks at it.
    fn unit() -> Operand {
        Operand::Const(Constant::Int(0, Type::None))
    }

    // ---- statements ------------------------------------------------------

    fn block(&mut self, b: &[Stmt]) {
        for s in b {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Assign(p, e) => self.assign(place(p), e),
            Stmt::If { label, branches } => self.if_stmt(*label, branches),
            Stmt::Loop { label, cond, body, step } => self.loop_stmt(*label, cond.as_ref(), body, step),
            Stmt::Jump(j) => {
                let to = match *j {
                    Jump::Break(l) => match self.targets[&l] {
                        Target::Loop { exit, .. } => exit,
                        Target::If { .. } => unreachable!("Break targets an If"),
                    },
                    Jump::Continue(l) => match self.targets[&l] {
                        Target::Loop { step, .. } => step,
                        Target::If { .. } => unreachable!("Continue targets an If"),
                    },
                    Jump::Leave(l) => match self.targets[&l] {
                        Target::If { exit, .. } => exit,
                        Target::Loop { .. } => unreachable!("Leave targets a Loop"),
                    },
                    Jump::Next(l, n) => match &self.targets[&l] {
                        Target::If { tests, .. } => tests[n],
                        Target::Loop { .. } => unreachable!("Next targets a Loop"),
                    },
                };
                self.diverge(TerminatorKind::Goto(to));
            }
            Stmt::Return(e) => {
                if let Some(e) = e {
                    if e.ty == Type::None {
                        self.operand(e);
                    } else {
                        self.assign(Place::local(Local(0)), e);
                    }
                }
                self.diverge(TerminatorKind::Return);
            }
            Stmt::Expr(e) => {
                self.operand(e);
            }
            Stmt::Drop(p) => {
                self.push(StatementKind::Drop { place: place(p), flag: None, reason: DropReason::ScopeEnd });
            }
        }
    }

    fn if_stmt(&mut self, label: Label, branches: &[hir::Branch]) {
        let exit = self.new_block();
        let bodies: Vec<BlockId> = branches.iter().map(|_| self.new_block()).collect();
        // A branch with no test goes straight to its body.
        let mut tests: Vec<BlockId> =
            branches.iter().zip(&bodies).map(|(b, &body)| if b.test.is_some() { self.new_block() } else { body }).collect();
        tests.push(exit);
        self.targets.insert(label, Target::If { exit, tests: tests.clone() });

        self.goto(tests[0]);
        for (k, br) in branches.iter().enumerate() {
            if let Some(test) = &br.test {
                self.cur = tests[k];
                let c = self.operand(test);
                self.end(TerminatorKind::If { cond: c, then: bodies[k], els: tests[k + 1] });
            }
            self.cur = bodies[k];
            self.block(&br.body);
            self.goto(exit);
        }
        self.cur = exit;
    }

    fn loop_stmt(&mut self, label: Label, cond: Option<&Expr>, body: &[Stmt], step: &[Stmt]) {
        let head = self.new_block();
        let body_bb = self.new_block();
        let step_bb = self.new_block();
        let exit = self.new_block();
        self.targets.insert(label, Target::Loop { exit, step: step_bb });

        self.goto(head);
        self.cur = head;
        match cond {
            // R7: a loop that cannot end has no edge to its exit.
            Some(c) if !matches!(c.kind, ExprKind::Bool(true)) => {
                let c = self.operand(c);
                self.end(TerminatorKind::If { cond: c, then: body_bb, els: exit });
            }
            _ => self.goto(body_bb),
        }
        self.cur = body_bb;
        self.block(body);
        self.goto(step_bb);
        self.cur = step_bb;
        self.block(step);
        self.goto(head);
        self.cur = exit;
    }

    /// `dest = e`. A call goes through a temp (B7).
    fn assign(&mut self, dest: Place, e: &Expr) {
        self.span = expr_span(e);
        match &e.kind {
            ExprKind::Neg(_)
            | ExprKind::Not(_)
            | ExprKind::BitNot(_)
            | ExprKind::Binary(..)
            | ExprKind::Shift { .. }
            | ExprKind::IntPow(..)
            | ExprKind::Convert(_)
            | ExprKind::Struct(_)
            | ExprKind::Copy(_) => {
                let rv = self.rvalue(e);
                self.assign_stmt(dest, rv);
            }
            _ => {
                let op = self.operand(e);
                if e.ty != Type::None {
                    self.assign_stmt(dest, Rvalue::Use(op));
                }
            }
        }
    }

    // ---- expressions -----------------------------------------------------

    /// A place expression as a place, without reading it. Anything else is
    /// evaluated into a temp.
    fn as_place(&mut self, e: &Expr) -> Place {
        match &e.kind {
            ExprKind::Local(l) => Place::local(self.local(*l)),
            ExprKind::Field(inner, f) => {
                let mut p = self.as_place(inner);
                p.proj.push(Proj::Field(*f));
                p
            }
            ExprKind::Temp(l, inner) => {
                let p = Place::local(self.local(*l));
                self.assign(p.clone(), inner);
                p
            }
            _ => match self.operand(e) {
                Operand::Copy(p) | Operand::Move(p) => p,
                Operand::Const(c) => {
                    let t = self.new_temp(e.ty);
                    self.assign_stmt(Place::local(t), Rvalue::Use(Operand::Const(c)));
                    Place::local(t)
                }
            },
        }
    }

    /// Evaluate `e` and give its value as an operand.
    fn operand(&mut self, e: &Expr) -> Operand {
        self.span = expr_span(e);
        match &e.kind {
            ExprKind::Int(v) => Operand::Const(Constant::Int(*v, e.ty)),
            ExprKind::Float(v) => Operand::Const(Constant::Float(*v, e.ty)),
            ExprKind::Bool(b) => Operand::Const(Constant::Bool(*b)),
            ExprKind::Str(s) => Operand::Const(Constant::Str(self.intern(s))),
            ExprKind::Local(_) | ExprKind::Field(..) | ExprKind::Temp(..) => {
                let p = self.as_place(e);
                self.read(p, e.ty)
            }
            ExprKind::Move(l) => Operand::Move(Place::local(self.local(*l))),
            ExprKind::Neg(_)
            | ExprKind::Not(_)
            | ExprKind::BitNot(_)
            | ExprKind::Binary(..)
            | ExprKind::Shift { .. }
            | ExprKind::IntPow(..)
            | ExprKind::Convert(_)
            | ExprKind::Struct(_)
            | ExprKind::Copy(_) => {
                let rv = self.rvalue(e);
                let t = self.new_temp(e.ty);
                self.assign_stmt(Place::local(t), rv);
                self.read(Place::local(t), e.ty)
            }
            ExprKind::Scoped(inner, locals) => {
                // The value goes into a temp before the drops, which might
                // free what it reads.
                let mut op = self.operand(inner);
                if inner.ty != Type::None {
                    op = self.freeze(op, inner.ty);
                }
                for &l in locals {
                    let place = Place::local(self.local(l));
                    self.push(StatementKind::Drop { place, flag: None, reason: DropReason::ScopeEnd });
                }
                op
            }
            ExprKind::Logic { and, lhs, rhs } => {
                let t = self.new_temp(Type::Bool);
                let l = self.operand(lhs);
                self.assign_stmt(Place::local(t), Rvalue::Use(l));
                let eval_rhs = self.new_block();
                let join = self.new_block();
                let c = Operand::Copy(Place::local(t));
                let (then, els) = if *and { (eval_rhs, join) } else { (join, eval_rhs) };
                self.end(TerminatorKind::If { cond: c, then, els });
                self.cur = eval_rhs;
                let r = self.operand(rhs);
                self.assign_stmt(Place::local(t), Rvalue::Use(r));
                self.goto(join);
                self.cur = join;
                Operand::Copy(Place::local(t))
            }
            ExprKind::IncDec { place: p, inc, prefix } => {
                let p = place(p);
                let one = match e.ty {
                    Type::Float(_) => Constant::Float(1.0, e.ty),
                    _ => Constant::Int(1, e.ty),
                };
                let old = self.new_temp(e.ty);
                self.assign_stmt(Place::local(old), Rvalue::Use(Operand::Copy(p.clone())));
                let op = if *inc { hir::BinOp::Add } else { hir::BinOp::Sub };
                self.assign_stmt(p.clone(), Rvalue::Binary(op, Operand::Copy(p.clone()), Operand::Const(one)));
                if *prefix {
                    let new = self.new_temp(e.ty);
                    self.assign_stmt(Place::local(new), Rvalue::Use(Operand::Copy(p)));
                    Operand::Copy(Place::local(new))
                } else {
                    Operand::Copy(Place::local(old))
                }
            }
            ExprKind::Call(f, args) => {
                let has_self = self.hir.funcs[*f].has_self;
                let mutable = receiver_mutable(*f);
                let es: Vec<&Expr> = args.iter().collect();
                let ops = self.operands(&es, &|i| (i == 0 && has_self).then_some(mutable));
                self.call(Callee::Func(*f), ops, e.ty)
            }
            ExprKind::StrCall(f, args) => {
                let params = f.params();
                let es: Vec<&Expr> = args.iter().collect();
                let ops = self.operands(&es, &|i| (params.get(i) == Some(&Type::Str)).then_some(*f == StrFn::Append && i == 0));
                self.call(Callee::Str(*f), ops, e.ty)
            }
            ExprKind::Out { newline, arg } => {
                let ops = self.operands(&[&**arg], &|_| Some(false));
                self.call(Callee::Out { newline: *newline }, ops, Type::None)
            }
            ExprKind::Text(v) => {
                let ops = self.operands(&[&**v], &|_| None);
                self.call(Callee::Text, ops, e.ty)
            }
            ExprKind::Radix { value, base, upper } => {
                let ops = self.operands(&[&**value], &|_| None);
                self.call(Callee::Radix { base: *base, upper: *upper }, ops, e.ty)
            }
            ExprKind::Join(parts) => {
                let es: Vec<&Expr> = parts.iter().collect();
                let ops = self.operands(&es, &|_| Some(false));
                self.call(Callee::Join, ops, e.ty)
            }
        }
    }

    /// The rvalue computing `e` (one of the kinds `rvalue` handles).
    fn rvalue(&mut self, e: &Expr) -> Rvalue {
        let none = |_: usize| None;
        match &e.kind {
            ExprKind::Neg(a) => Rvalue::Neg(self.operand(a)),
            ExprKind::Not(a) => Rvalue::Not(self.operand(a)),
            ExprKind::BitNot(a) => Rvalue::BitNot(self.operand(a)),
            ExprKind::Binary(op, a, b) => {
                let mut o = self.operands(&[&**a, &**b], &none).into_iter();
                Rvalue::Binary(*op, o.next().unwrap(), o.next().unwrap())
            }
            ExprKind::Shift { left, value, amount } => {
                let mut o = self.operands(&[&**value, &**amount], &none).into_iter();
                Rvalue::Shift { left: *left, value: o.next().unwrap(), amount: o.next().unwrap() }
            }
            ExprKind::IntPow(a, b) => {
                let mut o = self.operands(&[&**a, &**b], &none).into_iter();
                Rvalue::IntPow(o.next().unwrap(), o.next().unwrap())
            }
            ExprKind::Convert(a) => Rvalue::Convert(self.operand(a), e.ty),
            ExprKind::Struct(fields) => {
                let sid = e.ty.struct_id().expect("a struct value has a struct type");
                let es: Vec<&Expr> = fields.iter().map(|(_, v)| v).collect();
                let ops = self.operands(&es, &none);
                let mut by_field: Vec<Option<Operand>> = vec![None; self.hir.structs[sid].fields.len()];
                for ((f, _), op) in fields.iter().zip(ops) {
                    by_field[*f] = Some(op);
                }
                Rvalue::Aggregate(sid, by_field.into_iter().map(|o| o.expect("every field is given")).collect())
            }
            ExprKind::Copy(inner) => Rvalue::DeepCopy(self.as_place(inner)),
            _ => unreachable!("rvalue of {}", kind_name(&e.kind)),
        }
    }

    /// Evaluate operands left to right. `borrow(i)` says operand `i` is passed
    /// as `&place` (`Some(false)`) or `&mut place` (`Some(true)`). An operand
    /// that reads a place is copied into a temp when a later one might change
    /// that place (`f(i, i++)`).
    fn operands(&mut self, es: &[&Expr], borrow: &dyn Fn(usize) -> Option<bool>) -> Vec<Operand> {
        let mut out = Vec::new();
        for (i, e) in es.iter().enumerate() {
            if let Some(mutable) = borrow(i) {
                let p = self.as_place(e);
                let r = self.new_ref_temp(e.ty, mutable);
                let kind = if mutable { BorrowKind::Mut { two_phase: false } } else { BorrowKind::Shared };
                self.assign_stmt(Place::local(r), Rvalue::Ref(kind, p));
                let rp = Place::local(r);
                out.push(if mutable { Operand::Move(rp) } else { Operand::Copy(rp) });
            } else {
                let mut op = self.operand(e);
                let stable = matches!(&op, Operand::Copy(p) | Operand::Move(p)
                    if p.proj.is_empty() && p.local.index() > self.func.locals.len());
                if !stable && es[i + 1..].iter().any(|l| !pure(l)) {
                    op = self.freeze(op, e.ty);
                }
                out.push(op);
            }
        }
        out
    }

    /// Emit a call to a fresh target block; its result goes to a fresh temp.
    fn call(&mut self, callee: Callee, ops: Vec<Operand>, ret: Type) -> Operand {
        let dest = (ret != Type::None).then(|| Place::local(self.new_temp(ret)));
        let next = self.new_block();
        let args = ops.into_iter().map(|op| Arg { op, span: self.span }).collect();
        self.end(TerminatorKind::Call { callee, args, dest: dest.clone(), target: next });
        self.cur = next;
        match dest {
            Some(p) => self.read(p, ret),
            None => Self::unit(),
        }
    }
}

/// Can evaluating `e` change anything another operand reads? Conservative.
fn pure(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Bool(_) | ExprKind::Str(_) | ExprKind::Local(_) | ExprKind::Move(_) => true,
        ExprKind::Field(a, _) | ExprKind::Neg(a) | ExprKind::Not(a) | ExprKind::BitNot(a) | ExprKind::Convert(a) | ExprKind::Copy(a) => pure(a),
        ExprKind::Binary(_, a, b) | ExprKind::IntPow(a, b) => pure(a) && pure(b),
        ExprKind::Shift { value, amount, .. } => pure(value) && pure(amount),
        ExprKind::Logic { lhs, rhs, .. } => pure(lhs) && pure(rhs),
        ExprKind::Struct(fields) => fields.iter().all(|(_, v)| pure(v)),
        _ => false,
    }
}

fn kind_name(k: &ExprKind) -> &'static str {
    match k {
        ExprKind::Int(_) => "Int",
        ExprKind::Float(_) => "Float",
        ExprKind::Bool(_) => "Bool",
        ExprKind::Str(_) => "Str",
        ExprKind::Local(_) => "Local",
        ExprKind::Field(..) => "Field",
        ExprKind::Struct(_) => "Struct",
        ExprKind::Neg(_) => "Neg",
        ExprKind::Not(_) => "Not",
        ExprKind::BitNot(_) => "BitNot",
        ExprKind::Binary(..) => "Binary",
        ExprKind::Shift { .. } => "Shift",
        ExprKind::IntPow(..) => "IntPow",
        ExprKind::Logic { .. } => "Logic",
        ExprKind::Convert(_) => "Convert",
        ExprKind::Call(..) => "Call",
        ExprKind::IncDec { .. } => "IncDec",
        ExprKind::Text(_) => "Text",
        ExprKind::Radix { .. } => "Radix",
        ExprKind::Copy(_) => "Copy",
        ExprKind::Move(_) => "Move",
        ExprKind::Temp(..) => "Temp",
        ExprKind::Scoped(..) => "Scoped",
        ExprKind::StrCall(..) => "StrCall",
        ExprKind::Join(_) => "Join",
        ExprKind::Out { .. } => "Out",
    }
}

fn map_targets(k: &mut TerminatorKind, f: &mut dyn FnMut(BlockId) -> BlockId) {
    match k {
        TerminatorKind::Goto(b) => *b = f(*b),
        TerminatorKind::If { then, els, .. } => {
            *then = f(*then);
            *els = f(*els);
        }
        TerminatorKind::Switch { cases, otherwise, .. } => {
            for (_, b) in cases {
                *b = f(*b);
            }
            *otherwise = f(*otherwise);
        }
        TerminatorKind::Call { target, .. } => *target = f(*target),
        TerminatorKind::Return | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => {}
    }
}
