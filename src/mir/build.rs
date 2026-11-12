//! Building MIR from HIR (spec 2.5, phase B).
//!
//! The MIR makes the moves, copies, temporaries and drops: the HIR only says
//! what the program wrote. Reading a place of a type that is not Copy moves
//! it; `copy x` is `Rvalue::DeepCopy`; an owned value used as a place goes
//! into a temporary that dies with its statement. User locals come from
//! `Stmt::Decl` and the block structure. The HIR is read through `place` and
//! `expr_span`.

use std::collections::{HashMap, HashSet};

use super::*;
use crate::diag::codes;
use crate::hir::{Expr, ExprKind, Jump, Label, Stmt, StrFn};

/// A function's name for dumps: free functions lose their `drachm_f_` prefix.
pub fn display_name(symbol: &str) -> std::borrow::Cow<'_, str> {
    if let Some(rest) = symbol.strip_prefix("drachm_g_") {
        return std::borrow::Cow::Owned(crate::types::demangle(rest));
    }
    std::borrow::Cow::Borrowed(symbol.strip_prefix("drachm_f_").unwrap_or(symbol))
}

const NO_SPAN: Span = Span { start: 0, end: 0 };

/// The source span of an expression.
fn expr_span(e: &Expr) -> Span {
    e.span
}

/// Does a method call borrow its receiver `&mut`? Only if the method is `Mut`.
fn receiver_mutable(modes: &[Option<SelfMode>], f: FuncId) -> bool {
    matches!(modes[f], Some(SelfMode::Mut(_)))
}

pub fn program(hir: &hir::Program, diags: &mut Vec<Diagnostic>) -> Program {
    let self_modes = crate::ownership::infer(hir);
    let mut strings = IndexVec::new();
    let mut str_ids = HashMap::new();
    let bodies = hir
        .funcs
        .iter()
        .enumerate()
        .map(|(id, f)| Some(Builder::new(hir, id, f, &self_modes, &mut strings, &mut str_ids, diags).build()))
        .collect();
    Program {
        bodies,
        main: hir.main,
        strings,
        self_modes,
        lives: (0..hir.funcs.len()).map(|f| super::lives::initial(hir, f)).collect(),
    }
}

/// A name no program can write: typeck's hidden locals (`$3`, `range start`).
fn is_hidden(name: &str) -> bool {
    !name.chars().all(|c| c.is_alphanumeric() || c == '_')
}

fn mutability(m: hir::Mutability) -> Mutability {
    match m {
        hir::Mutability::Fixed => Mutability::Fixed,
        hir::Mutability::Mut => Mutability::Mut,
        hir::Mutability::Open => Mutability::Open,
    }
}

/// The locals declared by `Stmt::Decl` anywhere in a block (failure handlers included).
fn decls(b: &[Stmt], out: &mut HashSet<usize>) {
    for s in b {
        match s {
            Stmt::Decl(l) => {
                out.insert(*l);
            }
            Stmt::If { branches, .. } => {
                for br in branches {
                    if let Some(t) = &br.test {
                        decls_expr(t, out);
                    }
                    decls(&br.body, out);
                }
            }
            Stmt::Loop { cond, body, step, .. } => {
                if let Some(c) = cond {
                    decls_expr(c, out);
                }
                decls(body, out);
                decls(step, out);
            }
            Stmt::Unsafe(b) => decls(b, out),
            Stmt::Assign(_, e) | Stmt::Expr(e) | Stmt::InitField(_, e) | Stmt::Return(Some(e)) | Stmt::Resume(_, Some(e)) => decls_expr(e, out),
            _ => {}
        }
    }
}

fn decls_expr(e: &Expr, out: &mut HashSet<usize>) {
    if let ExprKind::Handled { arms, .. } = &e.kind {
        for a in arms {
            decls(&a.body, out);
        }
    }
    for c in e.children() {
        decls_expr(c, out);
    }
}

struct BlockBuild {
    statements: Vec<Statement>,
    terminator: Option<Terminator>,
}

enum Target {
    /// `depth` is the number of scope frames outside the statement.
    Loop { exit: BlockId, step: BlockId, depth: usize },
    /// `tests[n]` is where `Next(l, n)` goes; the last entry is `exit`.
    If { exit: BlockId, tests: Vec<BlockId>, depth: usize },
}

#[derive(Clone, Copy)]
struct Owned {
    local: Local,
    /// A `StorageDead` follows the drop.
    dead: bool,
}

struct Frame {
    scope: ScopeId,
    owned: Vec<Owned>,
}

struct Builder<'a> {
    hir: &'a hir::Program,
    id: FuncId,
    /// By `FuncId`: how each method borrows `self`.
    modes: &'a [Option<SelfMode>],
    func: &'a hir::Func,
    strings: &'a mut IndexVec<StrId, String>,
    str_ids: &'a mut HashMap<String, StrId>,
    diags: &'a mut Vec<Diagnostic>,
    locals: IndexVec<Local, LocalDecl>,
    scopes: IndexVec<ScopeId, Scope>,
    blocks: Vec<BlockBuild>,
    cur: BlockId,
    frames: Vec<Frame>,
    targets: HashMap<Label, Target>,
    /// For each failure handler being lowered: where `maybe v;` puts its value
    /// (none when the call returns nothing), the block after the call, and
    /// the number of frames outside the call.
    resumes: HashMap<Label, (Option<Local>, BlockId, usize)>,
    span: Span,
    unsafe_depth: u32,
    /// The local a `Decl` just declared, until the next statement.
    just_declared: Option<Local>,
    /// The statement being lowered initialises the local `Decl` just declared.
    decl_init: bool,
    /// The local whose fields a `fill` or constructor is giving values now.
    init_local: Option<Local>,
}

impl<'a> Builder<'a> {
    fn new(
        hir: &'a hir::Program,
        id: FuncId,
        func: &'a hir::Func,
        modes: &'a [Option<SelfMode>],
        strings: &'a mut IndexVec<StrId, String>,
        str_ids: &'a mut HashMap<String, StrId>,
        diags: &'a mut Vec<Diagnostic>,
    ) -> Self {
        let mut b = Builder {
            hir,
            id,
            func,
            modes,
            strings,
            str_ids,
            diags,
            locals: IndexVec::new(),
            scopes: IndexVec::new(),
            blocks: Vec::new(),
            cur: BlockId(0),
            frames: Vec::new(),
            targets: HashMap::new(),
            resumes: HashMap::new(),
            span: NO_SPAN,
            unsafe_depth: 0,
            just_declared: None,
            decl_init: false,
            init_local: None,
        };
        b.cur = b.new_block();
        b
    }

    fn build(mut self) -> Body {
        let func = self.func;
        self.span = func.span;
        self.push_frame(ScopeKind::Function, func.span);
        self.decl(func.ret, LocalKind::Return);
        let mut declared = HashSet::new();
        decls(&func.body, &mut declared);
        for (i, l) in func.locals.iter().enumerate() {
            let mutability = mutability(l.mutability);
            let kind = if i < func.params {
                LocalKind::Param { name: l.name.clone(), mutability, written: l.written.clone() }
            } else if declared.contains(&i) || !is_hidden(&l.name) {
                LocalKind::User { name: l.name.clone(), mutability, written: l.written.clone() }
            } else {
                LocalKind::Temp
            };
            // A method's `self` is the caller's value: a borrow of it.
            let ty = if i == 0 && func.has_self { Type::borrow(receiver_mutable(self.modes, self.id), l.ty) } else { l.ty };
            self.span = l.span;
            let local = self.decl(ty, kind);
            if i < func.params {
                if needs_drop(self.hir, ty) {
                    self.register(local, false);
                }
            } else if !declared.contains(&i) {
                self.push(StatementKind::StorageLive(local));
            }
        }
        self.span = func.body_end;

        self.block(&func.body);

        let returns_value = func.ret != Type::None;
        let end = self.cur;
        self.span = func.body_end;
        self.pop_frame(func.body_end);
        self.end(if returns_value { TerminatorKind::Unreachable } else { TerminatorKind::Return });
        let (blocks, remap) = self.prune();
        let fallthrough_end = if returns_value { remap[end.index()] } else { None };

        Body {
            func: self.id,
            name: display_name(&func.symbol).to_string(),
            span: func.span,
            arg_count: func.params,
            locals: self.locals,
            blocks,
            scopes: self.scopes,
            unsafe_flag: false,
            unsafe_fn: func.is_unsafe,
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

    fn cur_scope(&self) -> ScopeId {
        self.frames.last().map_or(ScopeId(0), |f| f.scope)
    }

    fn decl(&mut self, ty: Type, kind: LocalKind) -> Local {
        let scope = self.cur_scope();
        self.locals.push(LocalDecl { ty, kind, span: self.span, scope, in_memory: false })
    }

    /// A fresh temp. If its type has drop glue it dies at the end of the
    /// innermost scope frame (B4), unless it is moved first.
    fn new_temp(&mut self, ty: Type) -> Local {
        let t = self.decl(ty, LocalKind::Temp);
        if needs_drop(self.hir, ty) {
            self.register(t, false);
        }
        t
    }

    /// A temp holding a `&ty` (`&mut ty`).
    fn new_ref_temp(&mut self, ty: Type, mutable: bool) -> Local {
        self.decl(Type::borrow(mutable, ty), LocalKind::Temp)
    }

    fn local(&self, id: hir::LocalId) -> Local {
        Local::new(id + 1)
    }

    /// The place a HIR local stands for: a method's `self` is read through its borrow.
    fn local_place(&self, id: hir::LocalId) -> Place {
        let mut p = Place::local(self.local(id));
        if id == 0 && self.func.has_self {
            p.proj.push(Proj::Deref);
        }
        p
    }

    /// A HIR place as a MIR place (`self` of a method is read through its
    /// borrow). An index in it is evaluated and checked here, once.
    fn hir_place(&mut self, p: &hir::Place) -> Place {
        let mut out = Place::local(Local::new(p.local + 1));
        if p.local == 0 && self.func.has_self {
            out.proj.push(Proj::Deref);
        }
        for pr in &p.proj {
            match pr {
                hir::HirProj::Field(f) => out.proj.push(Proj::Field(*f)),
                hir::HirProj::Deref => out.proj.push(Proj::Deref),
                hir::HirProj::Downcast(v) => out.proj.push(Proj::Downcast(*v)),
                hir::HirProj::Index(idx) => {
                    let ty = crate::mir::ty::place_ty_in(&self.locals, self.hir, &out);
                    let l = self.index_local(&out, ty, idx);
                    out.proj.push(Proj::Index(l));
                }
            }
        }
        out
    }

    /// Evaluate the index `idx` of the list or array place `base` (of type
    /// `ty`) and check it against the length: a `uint64` local that is a valid
    /// position. A negative signed index counts from the end (Q10). Out of
    /// range ends the program through `IndexPanic`, in a block of its own.
    fn index_local(&mut self, base: &Place, ty: Type, idx: &Expr) -> Local {
        self.at(expr_span(idx));
        let span = self.span;
        let iop = self.operand(idx);
        // A constant index into an array was checked by the type checker (E0620).
        if let (ExprKind::Int(_), Type::Array { .. }) = (&idx.kind, ty) {
            let a = self.decl(Type::U64, LocalKind::Temp);
            self.assign_stmt(Place::local(a), Rvalue::Use(iop));
            return a;
        }
        let signed = matches!(idx.ty, Type::Int { signed: true, .. });
        let len_op = match ty {
            Type::Array { len, .. } => Operand::Const(Constant::Int(len as u128, Type::U64)),
            _ => {
                let l = self.decl(Type::U64, LocalKind::Temp);
                self.assign_stmt(Place::local(l), Rvalue::Len(base.clone()));
                Operand::Copy(Place::local(l))
            }
        };
        let conv = |b: &mut Self, op: Operand, from: Type, to: Type| -> Operand {
            if from == to {
                return op;
            }
            let t = b.decl(to, LocalKind::Temp);
            b.assign_stmt(Place::local(t), Rvalue::Convert(op, to));
            Operand::Copy(Place::local(t))
        };
        let a = self.decl(Type::U64, LocalKind::Temp);
        let shown;
        if signed {
            let i = conv(self, iop, idx.ty, Type::S64);
            let i = self.freeze(i, Type::S64);
            shown = i.clone();
            let neg = self.decl(Type::Bool, LocalKind::Temp);
            self.assign_stmt(Place::local(neg), Rvalue::Binary(hir::BinOp::Lt, i.clone(), Operand::Const(Constant::Int(0, Type::S64))));
            let u = conv(self, i, Type::S64, Type::U64);
            let (from_end, from_start, join) = (self.new_block(), self.new_block(), self.new_block());
            self.end(TerminatorKind::If { cond: Operand::Copy(Place::local(neg)), then: from_end, els: from_start });
            self.cur = from_end;
            self.assign_stmt(Place::local(a), Rvalue::Binary(hir::BinOp::Add, u.clone(), len_op.clone()));
            self.goto(join);
            self.cur = from_start;
            self.assign_stmt(Place::local(a), Rvalue::Use(u));
            self.goto(join);
            self.cur = join;
        } else {
            let u = conv(self, iop, idx.ty, Type::U64);
            shown = u.clone();
            self.assign_stmt(Place::local(a), Rvalue::Use(u));
        }
        let ok_flag = self.decl(Type::Bool, LocalKind::Temp);
        self.assign_stmt(Place::local(ok_flag), Rvalue::Binary(hir::BinOp::Lt, Operand::Copy(Place::local(a)), len_op.clone()));
        let (ok, bad, dead) = (self.new_block(), self.new_block(), self.new_block());
        self.end(TerminatorKind::If { cond: Operand::Copy(Place::local(ok_flag)), then: ok, els: bad });
        self.cur = bad;
        let args = vec![Arg { op: shown, span }, Arg { op: len_op, span }];
        self.end(TerminatorKind::Call { callee: Callee::IndexPanic { unsigned: !signed }, args, dest: None, target: dead });
        self.cur = dead;
        self.end(TerminatorKind::Unreachable);
        self.cur = ok;
        a
    }

    fn at(&mut self, s: Span) {
        if s != NO_SPAN {
            self.span = s;
        }
    }

    fn new_block(&mut self) -> BlockId {
        self.blocks.push(BlockBuild { statements: Vec::new(), terminator: None });
        BlockId::new(self.blocks.len() - 1)
    }

    fn push(&mut self, kind: StatementKind) {
        let mark = match (&kind, self.init_local) {
            (StatementKind::Assign(dest, _), Some(l)) if dest.local == l => Mark::InitWrite,
            _ => Mark::None,
        };
        let s = Statement { kind, span: self.span, unsafe_ctx: self.unsafe_depth > 0, mark };
        self.blocks[self.cur.index()].statements.push(s);
    }

    /// A `FakeRead` of `local` that the checks of `check.rs` act on.
    fn check_stmt(&mut self, local: Local, kind: hir::CheckKind) {
        let s = Statement {
            kind: StatementKind::FakeRead(Place::local(local)),
            span: self.span,
            unsafe_ctx: self.unsafe_depth > 0,
            mark: Mark::Check(kind),
        };
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
        b.terminator = Some(Terminator { kind, span: self.span, unsafe_ctx: self.unsafe_depth > 0 });
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

    // ---- scope frames (B4, B5) -------------------------------------------

    fn push_frame(&mut self, kind: ScopeKind, span: Span) {
        let parent = self.frames.last().map(|f| f.scope);
        let scope = self.scopes.push(Scope { parent, span, kind });
        self.frames.push(Frame { scope, owned: Vec::new() });
    }

    /// Leave the innermost frame normally: drops and `StorageDead`s, last
    /// declared first.
    fn pop_frame(&mut self, end: Span) {
        let f = self.frames.pop().expect("a frame to leave");
        self.at(end);
        self.emit_drops(&f.owned);
    }

    fn emit_drops(&mut self, owned: &[Owned]) {
        for o in owned.iter().rev() {
            if needs_drop(self.hir, self.locals[o.local].ty) {
                self.push(StatementKind::Drop { place: Place::local(o.local), flag: None, reason: DropReason::ScopeEnd });
            }
            if o.dead {
                self.push(StatementKind::StorageDead(o.local));
            }
        }
    }

    /// Leave every frame from `depth` outwards, innermost first, for a jump
    /// (the frames themselves stay: the code after the jump is dead).
    fn unwind(&mut self, depth: usize) {
        let owned: Vec<Owned> = self.frames[depth..].iter().rev().flat_map(|f| f.owned.iter().rev().copied()).collect();
        // `emit_drops` walks backwards, so hand it the list reversed.
        let owned: Vec<Owned> = owned.into_iter().rev().collect();
        self.emit_drops(&owned);
    }

    fn register(&mut self, local: Local, dead: bool) {
        let f = self.frames.last_mut().expect("a frame to own the local");
        if !f.owned.iter().any(|o| o.local == local) {
            f.owned.push(Owned { local, dead });
        }
    }

    /// Run `f` inside a statement frame: the temps it makes die at the end.
    fn in_stmt<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        self.push_frame(ScopeKind::Block, self.span);
        let r = f(self);
        self.pop_frame(self.span);
        r
    }

    /// B9: the temp `t` made for `&temp` in a declaration dies after the
    /// declared local, with the block, not with the statement.
    fn hoist(&mut self, t: Local) {
        let n = self.frames.len();
        if n < 2 {
            return;
        }
        let Some(i) = self.frames[n - 1].owned.iter().position(|o| o.local == t) else { return };
        let o = self.frames[n - 1].owned.remove(i);
        let at = self.frames[n - 2].owned.len().saturating_sub(1);
        self.frames[n - 2].owned.insert(at, o);
    }

    // ---- statements ------------------------------------------------------

    fn block(&mut self, b: &[Stmt]) {
        for s in b {
            self.stmt(s);
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        let declared = self.just_declared.take();
        match s {
            Stmt::Assign(p, e) => {
                self.at(p.span);
                let fresh = declared == Some(self.local(p.local)) && p.proj.is_empty();
                self.decl_init = fresh;
                // A just-declared local holds nothing: nothing to drop first.
                self.in_stmt(|b| {
                    let dest = b.hir_place(p);
                    b.assign(dest, e, !fresh)
                });
                self.decl_init = false;
            }
            Stmt::If { label, branches } => self.if_stmt(*label, branches),
            Stmt::Loop { label, cond, body, step, end } => self.loop_stmt(*label, cond.as_ref(), body, step, *end),
            Stmt::Jump(j) => {
                let (to, depth) = match *j {
                    Jump::Break(l) => match self.targets[&l] {
                        Target::Loop { exit, depth, .. } => (exit, depth),
                        Target::If { .. } => unreachable!("Break targets an If"),
                    },
                    Jump::Continue(l) => match self.targets[&l] {
                        Target::Loop { step, depth, .. } => (step, depth),
                        Target::If { .. } => unreachable!("Continue targets an If"),
                    },
                    Jump::Leave(l) => match self.targets[&l] {
                        Target::If { exit, depth, .. } => (exit, depth),
                        Target::Loop { .. } => unreachable!("Leave targets a Loop"),
                    },
                    Jump::Next(l, n) => match &self.targets[&l] {
                        Target::If { tests, depth, .. } => (tests[n], *depth),
                        Target::Loop { .. } => unreachable!("Next targets a Loop"),
                    },
                };
                self.unwind(depth);
                self.diverge(TerminatorKind::Goto(to));
            }
            Stmt::Return(e) => {
                self.push_frame(ScopeKind::Block, self.span);
                if let Some(e) = e {
                    if e.ty == Type::None {
                        self.operand(e);
                    } else {
                        self.assign(Place::local(Local(0)), e, false);
                    }
                }
                self.unwind(0);
                self.frames.pop();
                self.diverge(TerminatorKind::Return);
            }
            Stmt::Expr(e) => {
                self.in_stmt(|b| {
                    b.operand(e);
                });
            }
            Stmt::Resume(l, v) => {
                let (result, join, depth) = self.resumes[l];
                self.push_frame(ScopeKind::Block, self.span);
                match (v, result) {
                    (Some(v), Some(r)) => self.assign(Place::local(r), v, false),
                    (Some(v), None) => {
                        self.operand(v);
                    }
                    _ => {}
                }
                self.unwind(depth);
                self.frames.pop();
                self.diverge(TerminatorKind::Goto(join));
            }
            Stmt::Reraise(p) => {
                let pl = self.hir_place(p);
                self.at(p.span);
                let ty = crate::mir::ty::place_ty_in(&self.locals, self.hir, &pl);
                self.push_frame(ScopeKind::Block, self.span);
                let failure = self.new_temp(ty);
                self.assign_stmt(Place::local(failure), Rvalue::Use(Operand::Move(pl)));
                self.return_failure(failure, ty);
                self.frames.pop();
            }
            Stmt::Decl(l) => {
                let local = self.local(*l);
                self.at(self.func.locals[*l].span);
                let scope = self.cur_scope();
                self.locals[local].scope = scope;
                self.locals[local].span = self.span;
                self.push(StatementKind::StorageLive(local));
                self.register(local, true);
                self.just_declared = Some(local);
            }
            Stmt::Unsafe(b) => {
                self.unsafe_depth += 1;
                self.push_frame(ScopeKind::Unsafe, self.span);
                self.block(b);
                self.pop_frame(self.span);
                self.unsafe_depth -= 1;
            }
            Stmt::Forget(p) => {
                let pl = self.hir_place(p);
                self.at(p.span);
                let ty = crate::mir::ty::place_ty_in(&self.locals, self.hir, &pl);
                let gone = self.decl(ty, LocalKind::Temp);
                self.assign_stmt(Place::local(gone), Rvalue::Use(Operand::Move(pl)));
            }
            Stmt::Check { local, kind, span } => {
                let l = self.local(*local);
                self.at(*span);
                self.check_stmt(l, *kind);
            }
            Stmt::InitField(p, e) => {
                // A field given its value by `fill` or a constructor: the local
                // has no value yet, so the write is an initialisation.
                self.at(p.span);
                self.init_local = Some(self.local(p.local));
                // The old value of a `mut` field a def gave is dropped; a field
                // with no value has nothing to drop (the drop flags know).
                self.in_stmt(|b| {
                    let dest = b.hir_place(p);
                    b.assign(dest, e, true)
                });
                self.init_local = None;
            }
            Stmt::Destroy(p) => {
                let pl = self.hir_place(p);
                self.at(p.span);
                let ty = self.locals[pl.local].ty;
                self.push(StatementKind::Drop { place: pl.clone(), flag: None, reason: DropReason::Destroy });
                // `destroy` ends the local like a move; the value goes nowhere.
                let gone = self.decl(ty, LocalKind::Temp);
                self.assign_stmt(Place::local(gone), Rvalue::Use(Operand::Move(pl)));
            }
        }
    }

    /// A condition, as an operand. The temps it makes die before the branch.
    fn cond(&mut self, e: &Expr) -> Operand {
        self.at(expr_span(e));
        self.push_frame(ScopeKind::Block, self.span);
        let mut c = self.operand(e);
        if !self.frames.last().unwrap().owned.is_empty() {
            c = self.freeze(c, Type::Bool);
        }
        self.pop_frame(self.span);
        c
    }

    /// A `match` on an enum with three or more variants whose cases are all
    /// tag tests of one place becomes a single `Switch`. Returns false (and
    /// emits nothing) when the branches are not of that shape.
    fn switch_stmt(&mut self, label: Label, branches: &[hir::Branch]) -> bool {
        fn tag_tests<'e>(e: &'e Expr, out: &mut Vec<(&'e Expr, u128)>) -> bool {
            match &e.kind {
                ExprKind::Binary(hir::BinOp::Eq, a, b) => match (&a.kind, &b.kind) {
                    (ExprKind::Discriminant(p), ExprKind::Int(v)) => {
                        out.push((p, *v));
                        true
                    }
                    _ => false,
                },
                ExprKind::Logic { and: false, lhs, rhs } => tag_tests(lhs, out) && tag_tests(rhs, out),
                _ => false,
            }
        }
        fn place_key(e: &Expr) -> Option<Vec<(u8, usize)>> {
            match &e.kind {
                ExprKind::Local(l) => Some(vec![(0, *l)]),
                ExprKind::Field(i, f) => place_key(i).map(|mut k| {
                    k.push((1, *f));
                    k
                }),
                ExprKind::Deref(i) => place_key(i).map(|mut k| {
                    k.push((2, 0));
                    k
                }),
                ExprKind::Payload { value, variant, field } => place_key(value).map(|mut k| {
                    k.push((3, *variant));
                    k.push((4, *field));
                    k
                }),
                _ => None,
            }
        }
        fn has_next(b: &[Stmt], label: Label) -> bool {
            b.iter().any(|s| match s {
                Stmt::Jump(Jump::Next(l, _)) => *l == label,
                Stmt::If { branches, .. } => branches.iter().any(|br| has_next(&br.body, label)),
                Stmt::Loop { body, step, .. } => has_next(body, label) || has_next(step, label),
                Stmt::Unsafe(b) => has_next(b, label),
                _ => false,
            })
        }
        let mut place: Option<&Expr> = None;
        let mut key = None;
        let mut cases: Vec<(u128, usize)> = Vec::new();
        let mut otherwise = None;
        for (k, br) in branches.iter().enumerate() {
            if has_next(&br.body, label) {
                return false;
            }
            if otherwise.is_some() {
                continue;
            }
            match &br.test {
                None => otherwise = Some(k),
                Some(t) => {
                    let mut found = Vec::new();
                    if !tag_tests(t, &mut found) {
                        return false;
                    }
                    for (p, v) in found {
                        let Some(pk) = place_key(p) else { return false };
                        if key.get_or_insert_with(|| pk.clone()) != &pk {
                            return false;
                        }
                        place.get_or_insert(p);
                        if !cases.iter().any(|(c, _)| *c == v) {
                            cases.push((v, k));
                        }
                    }
                }
            }
        }
        let Some(place) = place else { return false };
        if cases.len() + (otherwise.is_some() as usize) < 3 {
            return false;
        }
        let exit = self.new_block();
        let bodies: Vec<BlockId> = branches.iter().map(|_| self.new_block()).collect();
        let mut tests = bodies.clone();
        tests.push(exit);
        let depth = self.frames.len();
        self.targets.insert(label, Target::If { exit, tests, depth });
        let disc = Expr::new(ExprKind::Discriminant(Box::new(place.clone())), crate::mir::ty::tag_type(self.hir.enum_def(place.ty).tag_size)).at(place.span);
        self.at(expr_span(&disc));
        let d = self.operand(&disc);
        let kind = TerminatorKind::Switch {
            discr: d,
            cases: cases.iter().map(|&(v, k)| (v, bodies[k])).collect(),
            otherwise: otherwise.map_or(exit, |k| bodies[k]),
        };
        self.end(kind);
        for (k, br) in branches.iter().enumerate() {
            self.cur = bodies[k];
            self.push_frame(ScopeKind::IfBranch, br.end);
            self.block(&br.body);
            self.pop_frame(br.end);
            self.goto(exit);
        }
        self.cur = exit;
        true
    }

    fn if_stmt(&mut self, label: Label, branches: &[hir::Branch]) {
        if self.switch_stmt(label, branches) {
            return;
        }
        let exit = self.new_block();
        let bodies: Vec<BlockId> = branches.iter().map(|_| self.new_block()).collect();
        // A branch with no test goes straight to its body.
        let mut tests: Vec<BlockId> =
            branches.iter().zip(&bodies).map(|(b, &body)| if b.test.is_some() { self.new_block() } else { body }).collect();
        tests.push(exit);
        let depth = self.frames.len();
        self.targets.insert(label, Target::If { exit, tests: tests.clone(), depth });

        self.goto(tests[0]);
        for (k, br) in branches.iter().enumerate() {
            if let Some(test) = &br.test {
                self.cur = tests[k];
                let c = self.cond(test);
                self.end(TerminatorKind::If { cond: c, then: bodies[k], els: tests[k + 1] });
            }
            self.cur = bodies[k];
            self.push_frame(ScopeKind::IfBranch, br.end);
            self.block(&br.body);
            self.pop_frame(br.end);
            self.goto(exit);
        }
        self.cur = exit;
    }

    fn loop_stmt(&mut self, label: Label, cond: Option<&Expr>, body: &[Stmt], step: &[Stmt], end: Span) {
        let head = self.new_block();
        let body_bb = self.new_block();
        let step_bb = self.new_block();
        let exit = self.new_block();
        let depth = self.frames.len();
        self.targets.insert(label, Target::Loop { exit, step: step_bb, depth });

        self.goto(head);
        self.cur = head;
        match cond {
            // R7: a loop that cannot end has no edge to its exit.
            Some(c) if !matches!(c.kind, ExprKind::Bool(true)) => {
                let c = self.cond(c);
                self.end(TerminatorKind::If { cond: c, then: body_bb, els: exit });
            }
            _ => self.goto(body_bb),
        }
        self.cur = body_bb;
        self.push_frame(ScopeKind::LoopBody, end);
        self.block(body);
        self.pop_frame(end);
        self.goto(step_bb);
        self.cur = step_bb;
        self.push_frame(ScopeKind::Block, end);
        self.block(step);
        self.pop_frame(end);
        self.goto(head);
        self.cur = exit;
    }

    /// `dest = e`. A call goes through a temp (B7). If the old value of
    /// `dest` needs dropping it is dropped after `e` is evaluated (B6).
    fn assign(&mut self, dest: Place, e: &Expr, drop_old: bool) {
        // Where the assigned place is written (conflicts with a borrow point at it).
        let dest_span = self.span;
        self.at(expr_span(e));
        // Only a declaration whose whole initializer is a borrow hoists (B9).
        self.decl_init = self.decl_init && matches!(e.kind, ExprKind::Borrow { .. });
        let owning = drop_old && needs_drop(self.hir, e.ty);
        let is_rvalue = match &e.kind {
            ExprKind::Neg(_)
            | ExprKind::Not(_)
            | ExprKind::BitNot(_)
            | ExprKind::Binary(..)
            | ExprKind::Shift { .. }
            | ExprKind::IntPow(..)
            | ExprKind::Convert(_)
            | ExprKind::PtrConv(_)
            | ExprKind::FuncRef(_)
            | ExprKind::Unsize(_)
            | ExprKind::Struct(_)
            | ExprKind::Variant { .. }
            | ExprKind::Len(_)
            | ExprKind::Array(_)
            | ExprKind::Borrow { .. }
            | ExprKind::AddressOf(_) => true,
            ExprKind::DeepCopy(_) => !is_copy(self.hir, e.ty),
            _ => false,
        };
        if is_rvalue {
            let rv = self.rvalue(e);
            if owning {
                let t = self.new_temp(e.ty);
                self.assign_stmt(Place::local(t), rv);
                self.at(dest_span);
                self.drop_before_assign(&dest);
                self.assign_stmt(dest, Rvalue::Use(Operand::Move(Place::local(t))));
            } else {
                self.assign_stmt(dest, rv);
            }
        } else {
            let op = self.operand(e);
            if e.ty != Type::None {
                if owning {
                    self.at(dest_span);
                    self.drop_before_assign(&dest);
                    self.at(expr_span(e));
                } else if matches!(op, Operand::Const(_)) {
                    self.at(dest_span);
                }
                self.assign_stmt(dest, Rvalue::Use(op));
            }
        }
    }

    fn drop_before_assign(&mut self, dest: &Place) {
        self.push(StatementKind::Drop { place: dest.clone(), flag: None, reason: DropReason::Assign });
    }

    // ---- expressions -----------------------------------------------------

    /// A place expression as a place, without reading it. Anything else is
    /// evaluated into a temp.
    fn as_place(&mut self, e: &Expr) -> Place {
        match &e.kind {
            ExprKind::Local(l) => self.local_place(*l),
            ExprKind::Field(inner, f) => {
                let mut p = self.as_place(inner);
                if inner.ty.is_borrow() {
                    p.proj.push(Proj::Deref);
                }
                p.proj.push(Proj::Field(*f));
                p
            }
            ExprKind::Deref(inner) => {
                let mut p = self.as_place(inner);
                p.proj.push(Proj::Deref);
                p
            }
            ExprKind::Index(base, idx) => {
                let mut p = self.as_place(base);
                let mut ty = base.ty;
                if ty.is_borrow() {
                    p.proj.push(Proj::Deref);
                    ty = ty.target().unwrap_or(ty);
                }
                let l = self.index_local(&p, ty, idx);
                p.proj.push(Proj::Index(l));
                p
            }
            ExprKind::Payload { value, variant, field } => {
                let mut p = self.as_place(value);
                if value.ty.is_borrow() {
                    p.proj.push(Proj::Deref);
                }
                p.proj.push(Proj::Downcast(*variant));
                p.proj.push(Proj::Field(*field));
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

    /// The text of a place for messages, and the struct types along it.
    fn path_text(&self, hp: &hir::Place) -> String {
        let l = &self.func.locals[hp.local];
        let mut text = l.name.clone();
        let mut t = l.ty;
        for pr in &hp.proj {
            match pr {
                hir::HirProj::Downcast(_) => {}
                hir::HirProj::Index(_) => {
                    text.push_str("[..]");
                    t = t.elem().unwrap_or(t);
                }
                hir::HirProj::Field(_) if t.enum_id().is_some() => return text,
                hir::HirProj::Field(f) => {
                    let def = self.hir.struct_def(t);
                    text.push('.');
                    text.push_str(&def.fields[*f].name);
                    t = def.fields[*f].ty;
                }
                hir::HirProj::Deref => t = t.target().unwrap_or(t),
            }
        }
        text
    }

    /// Read a place of the expression's type: a copy, or a move. B10: moving
    /// a non-Copy field out of an owned path needs `take` (E0324).
    fn read_place(&mut self, p: Place, e: &Expr) -> Operand {
        let ty = e.ty;
        if is_copy(self.hir, ty) {
            return Operand::Copy(p);
        }
        if p.proj.iter().any(|x| matches!(x, Proj::Index(_))) && self.func.span != NO_SPAN {
            let d = Diagnostic::at(&codes::E0622, e.span, format!("cannot move a `{ty}` out of a list or array by index"))
                .note("moved out here")
                .help("borrow it with `&`, or copy it with `copy`");
            self.diags.push(d);
            return Operand::Copy(p);
        }
        let root_hidden = matches!(self.locals[p.local].kind, LocalKind::Temp);
        let field_move = p.proj.iter().any(|x| matches!(x, Proj::Field(_))) && !p.proj.iter().any(|x| matches!(x, Proj::Deref | Proj::Index(_)));
        // The compiler's own functions (the automatic `==`) have no source span.
        if field_move && !root_hidden && self.func.span != NO_SPAN {
            self.field_move_error(&p, e.span);
        }
        Operand::Move(p)
    }

    fn field_move_error(&mut self, p: &Place, span: Span) {
        let hp = hir::Place {
            local: p.local.index() - 1,
            proj: p
                .proj
                .iter()
                .map(|x| match x {
                    Proj::Field(f) => hir::HirProj::Field(*f),
                    Proj::Deref => hir::HirProj::Deref,
                    Proj::Downcast(v) => hir::HirProj::Downcast(*v),
                    Proj::Index(_) => unreachable!("an item of a list is never moved by field"),
                })
                .collect(),
            span,
        };
        let path = self.path_text(&hp);
        let (parent, field) = path.rsplit_once('.').unwrap_or((&path, &path));
        let d = Diagnostic::at(&codes::E0324, span, format!("cannot move `{path}` out of `{parent}` without `take`"))
            .note(format!("this would leave `{parent}` without a {field}"))
            .help(format!("write `take {path}` to move it out, or `copy {path}` to keep both"));
        self.diags.push(d);
    }

    /// E0325: a plain `take` out of a struct with a `drop()`.
    fn check_take(&mut self, hp: &hir::Place, span: Span) {
        let mut t = self.func.locals[hp.local].ty;
        for pr in &hp.proj {
            match pr {
                hir::HirProj::Downcast(_) => {}
                hir::HirProj::Index(_) => t = t.elem().unwrap_or(t),
                hir::HirProj::Field(f) => {
                    let Some(id) = t.struct_id() else { return };
                    let def = &self.hir.structs[id];
                    if let Some(drop_fn) = def.has_drop {
                        let path = self.path_text(hp);
                        let last = &def.fields[*f].ty;
                        let value = match last {
                            Type::Str => "\"\"".to_string(),
                            Type::Bool => "false".to_string(),
                            t if t.is_numeric() => "0".to_string(),
                            _ => "<a new value>".to_string(),
                        };
                        let d = Diagnostic::at(
                            &codes::E0325,
                            span,
                            format!("cannot take `{path}` out of a `{}`: it has a `drop()`", def.name),
                        )
                        .note("taken here")
                        .label(
                            self.hir.funcs[drop_fn].name_span,
                            format!("`drop` runs when a `{}` is destroyed, so it must stay whole", def.name),
                        )
                        .help(format!("put a value back in the same statement: `take {path} leaving {value}`"));
                        self.diags.push(d);
                        return;
                    }
                    t = def.fields[*f].ty;
                }
                hir::HirProj::Deref => t = t.target().unwrap_or(t),
            }
        }
    }

    /// Evaluate `e` and give its value as an operand.
    fn operand(&mut self, e: &Expr) -> Operand {
        self.at(expr_span(e));
        match &e.kind {
            ExprKind::Int(v) => Operand::Const(Constant::Int(*v, e.ty)),
            ExprKind::Float(v) => Operand::Const(Constant::Float(*v, e.ty)),
            ExprKind::Bool(b) => Operand::Const(Constant::Bool(*b)),
            ExprKind::Str(s) => Operand::Const(Constant::Str(self.intern(s))),
            ExprKind::NullPtr => Operand::Const(Constant::Int(0, e.ty)),
            ExprKind::Local(_) | ExprKind::Field(..) | ExprKind::Deref(_) | ExprKind::Payload { .. } | ExprKind::Index(..) => {
                let p = self.as_place(e);
                self.at(expr_span(e));
                self.read_place(p, e)
            }
            ExprKind::Discriminant(inner) => {
                let p = self.as_place(inner);
                self.at(expr_span(e));
                let t = self.decl(e.ty, LocalKind::Temp);
                self.assign_stmt(Place::local(t), Rvalue::Discriminant(p));
                Operand::Copy(Place::local(t))
            }
            ExprKind::List(items) => {
                let elem = e.ty.elem().expect("a list literal has a list type");
                let list = self.new_temp(e.ty);
                self.assign_stmt(Place::local(list), Rvalue::ListNew(e.ty, items.len() as u64));
                for item in items {
                    let ops = self.operands(&[item], &|_| None);
                    let r = self.new_ref_temp(e.ty, true);
                    self.assign_stmt(Place::local(r), Rvalue::Ref(BorrowKind::Mut { two_phase: false }, Place::local(list)));
                    let mut args = vec![Arg { op: Operand::Move(Place::local(r)), span: self.span }];
                    args.extend(ops);
                    self.call_args(Callee::ListPush(elem), args, Type::None);
                }
                Operand::Move(Place::local(list))
            }
            ExprKind::ListCall { op, args } => self.list_call(e, *op, args),
            ExprKind::ListPush { list, value } => {
                let elem = value.ty;
                let ops = self.operands(&[&**list, &**value], &|i| (i == 0).then_some((true, true)));
                self.call(Callee::ListPush(elem), ops, Type::None)
            }
            ExprKind::Neg(_)
            | ExprKind::Not(_)
            | ExprKind::BitNot(_)
            | ExprKind::Binary(..)
            | ExprKind::Shift { .. }
            | ExprKind::IntPow(..)
            | ExprKind::Convert(_)
            | ExprKind::PtrConv(_)
            | ExprKind::FuncRef(_)
            | ExprKind::Unsize(_)
            | ExprKind::Struct(_)
            | ExprKind::Variant { .. }
            | ExprKind::Len(_)
            | ExprKind::Array(_)
            | ExprKind::Borrow { .. }
            | ExprKind::AddressOf(_) => {
                let rv = self.rvalue(e);
                let t = if matches!(e.kind, ExprKind::Borrow { .. }) {
                    self.decl(e.ty, LocalKind::Temp)
                } else {
                    self.new_temp(e.ty)
                };
                self.assign_stmt(Place::local(t), rv);
                self.read(Place::local(t), e.ty)
            }
            ExprKind::DeepCopy(inner) => {
                let p = self.as_place(inner);
                if is_copy(self.hir, e.ty) {
                    Operand::Copy(p)
                } else {
                    let t = self.new_temp(e.ty);
                    self.assign_stmt(Place::local(t), Rvalue::DeepCopy(p));
                    Operand::Move(Place::local(t))
                }
            }
            ExprKind::Take { place: hp, leaving } => {
                if leaving.is_none() {
                    self.check_take(hp, e.span);
                }
                let p = self.hir_place(hp);
                self.at(e.span);
                let t = self.new_temp(e.ty);
                self.assign_stmt(Place::local(t), Rvalue::Use(self.read(p.clone(), e.ty)));
                if let Some(v) = leaving {
                    let v = self.operand(v);
                    self.assign_stmt(p, Rvalue::Use(v));
                }
                self.read(Place::local(t), e.ty)
            }
            ExprKind::Lend(inner) => {
                let mut p = self.as_place(inner);
                p.proj.push(Proj::Deref);
                let t = self.decl(e.ty, LocalKind::Temp);
                self.assign_stmt(Place::local(t), Rvalue::Ref(BorrowKind::Mut { two_phase: false }, p));
                Operand::Move(Place::local(t))
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
                // The right side may not run: its temps die inside it.
                self.push_frame(ScopeKind::Block, self.span);
                let r = self.operand(rhs);
                self.assign_stmt(Place::local(t), Rvalue::Use(r));
                self.pop_frame(self.span);
                self.goto(join);
                self.cur = join;
                Operand::Copy(Place::local(t))
            }
            ExprKind::Maybe { value, fallback } => {
                // The optional goes in a temp: a place is read where it is (only
                // Copy ones get here), an owned value is moved in. Its tag picks
                // the payload (`Some`) or the fallback (`none`).
                let opt = value.ty;
                let tag_ty = crate::mir::ty::tag_type(self.hir.enum_def(opt).tag_size);
                let held = self.decl(opt, LocalKind::Temp);
                let v = self.operand(value);
                self.assign_stmt(Place::local(held), Rvalue::Use(v));
                let tag = self.new_temp(tag_ty);
                self.assign_stmt(Place::local(tag), Rvalue::Discriminant(Place::local(held)));
                let is_some = self.new_temp(Type::Bool);
                let some_tag = Operand::Const(Constant::Int(1, tag_ty));
                self.assign_stmt(Place::local(is_some), Rvalue::Binary(hir::BinOp::Eq, Operand::Copy(Place::local(tag)), some_tag));
                // A temp that dies at the end of its scope unless it is moved on (`(a maybe b)::len()` borrows it).
                let res = self.new_temp(e.ty);
                let some = self.new_block();
                let none = self.new_block();
                let join = self.new_block();
                self.end(TerminatorKind::If { cond: Operand::Copy(Place::local(is_some)), then: some, els: none });
                self.cur = some;
                let mut payload = Place::local(held);
                payload.proj.push(Proj::Downcast(1));
                payload.proj.push(Proj::Field(0));
                let got = self.read(payload, e.ty);
                self.assign_stmt(Place::local(res), Rvalue::Use(got));
                self.goto(join);
                self.cur = none;
                // The fallback may not run: its temps die inside it.
                self.push_frame(ScopeKind::Block, self.span);
                let f = self.operand(fallback);
                self.assign_stmt(Place::local(res), Rvalue::Use(f));
                self.pop_frame(self.span);
                self.goto(join);
                self.cur = join;
                self.read(Place::local(res), e.ty)
            }
            ExprKind::IncDec { place: p, inc, prefix } => {
                let p = self.hir_place(p);
                let one = match e.ty {
                    Type::Float(_) => Constant::Float(1.0, e.ty),
                    _ => Constant::Int(1, e.ty),
                };
                let old = self.decl(e.ty, LocalKind::Temp);
                self.assign_stmt(Place::local(old), Rvalue::Use(Operand::Copy(p.clone())));
                let op = if *inc { hir::BinOp::Add } else { hir::BinOp::Sub };
                self.assign_stmt(p.clone(), Rvalue::Binary(op, Operand::Copy(p.clone()), Operand::Const(one)));
                if *prefix {
                    let new = self.decl(e.ty, LocalKind::Temp);
                    self.assign_stmt(Place::local(new), Rvalue::Use(Operand::Copy(p)));
                    Operand::Copy(Place::local(new))
                } else {
                    Operand::Copy(Place::local(old))
                }
            }
            ExprKind::Call(f, args) => {
                let has_self = self.hir.funcs[*f].has_self;
                let mutable = receiver_mutable(self.modes, *f);
                let es: Vec<&Expr> = args.iter().collect();
                let ops = self.operands(&es, &|i| (i == 0 && has_self).then_some((mutable, mutable)));
                self.call(Callee::Func(*f), ops, e.ty)
            }
            ExprKind::FnCall(args) => {
                let refs: Vec<_> = args.iter().collect();
                let ops = self.operands(&refs, &|_| None);
                self.call(Callee::Fn(args[0].ty), ops, e.ty)
            }
            ExprKind::DynCall(iface, method, args) => {
                let changes = self.hir.dyn_ifaces[*iface].methods[*method].changes;
                let es: Vec<&Expr> = args.iter().collect();
                let ops = self.operands(&es, &|i| (i == 0).then_some((changes, changes)));
                self.call(Callee::Dyn { iface: *iface, method: *method }, ops, e.ty)
            }
            ExprKind::StrCall(f, args) => {
                let params = f.params();
                let es: Vec<&Expr> = args.iter().collect();
                let ops = self
                    .operands(&es, &|i| (params.get(i) == Some(&Type::Str)).then_some((*f == StrFn::Append && i == 0, *f == StrFn::Append && i == 0)));
                self.call(Callee::Str(*f), ops, e.ty)
            }
            ExprKind::Out { newline, arg } => {
                let ops = self.operands(&[&**arg], &|_| Some((false, false)));
                self.call(Callee::Out { newline: *newline }, ops, Type::None)
            }
            ExprKind::Text(v) => {
                let ops = self.operands(&[&**v], &|_| None);
                self.call(Callee::Text, ops, e.ty)
            }
            ExprKind::InLine => self.call(Callee::InLine, Vec::new(), Type::Str),
            ExprKind::InEnded => self.call(Callee::InEnded, Vec::new(), Type::Bool),
            ExprKind::TryParse { .. } => unreachable!("a text parse is always under `maybe` or `fails`"),
            ExprKind::FailMaybe { call, fallback } => self.maybe(e, call, fallback),
            ExprKind::PassUp { call } => self.pass_up(e, call),
            ExprKind::Handled { call, arms, label } => self.handled(e, call, arms, *label),
            ExprKind::CauseWrap { msg, slot } => {
                let ops = self.operands(&[&**msg, &**slot], &|i| Some((i == 1, false)));
                self.call(Callee::CauseWrap, ops, e.ty)
            }
            ExprKind::Report { msg, slot, exit: None } => {
                let ops = self.operands(&[&**msg, &**slot], &|i| Some((i == 1, false)));
                self.call(Callee::Report, ops, Type::None)
            }
            ExprKind::Report { msg, slot, exit: Some(code) } => {
                let ops = self.operands(&[&**msg, &**slot, &**code], &|i| (i < 2).then_some((i == 1, false)));
                self.call(Callee::ReportExit, ops, Type::None)
            }
            ExprKind::Radix { value, base, upper } => {
                let ops = self.operands(&[&**value], &|_| None);
                self.call(Callee::Radix { base: *base, upper: *upper }, ops, e.ty)
            }
            ExprKind::Join(parts) => {
                let es: Vec<&Expr> = parts.iter().collect();
                let ops = self.operands(&es, &|_| Some((false, false)));
                self.call(Callee::Join, ops, e.ty)
            }
        }
    }

    /// The rvalue computing `e` (one of the kinds `assign` hands over).
    fn rvalue(&mut self, e: &Expr) -> Rvalue {
        let none = |_: usize| None;
        let first = |mut v: Vec<Arg>| v.remove(0).op;
        match &e.kind {
            ExprKind::Neg(a) => Rvalue::Neg(self.operand(a)),
            ExprKind::Not(a) => Rvalue::Not(self.operand(a)),
            ExprKind::BitNot(a) => Rvalue::BitNot(self.operand(a)),
            ExprKind::Binary(op, a, b) => {
                let mut o = self.operands(&[&**a, &**b], &none).into_iter();
                let (mut x, mut y) = (o.next().unwrap().op, o.next().unwrap().op);
                // Comparing raw pointers does not use them up.
                if a.ty.is_ptr() {
                    for v in [&mut x, &mut y] {
                        if let Operand::Move(p) = v {
                            *v = Operand::Copy(p.clone());
                        }
                    }
                }
                Rvalue::Binary(*op, x, y)
            }
            ExprKind::Shift { left, value, amount } => {
                let mut o = self.operands(&[&**value, &**amount], &none).into_iter();
                Rvalue::Shift { left: *left, value: o.next().unwrap().op, amount: o.next().unwrap().op }
            }
            ExprKind::IntPow(a, b) => {
                let mut o = self.operands(&[&**a, &**b], &none).into_iter();
                Rvalue::IntPow(o.next().unwrap().op, o.next().unwrap().op)
            }
            ExprKind::Convert(a) => Rvalue::Convert(first(self.operands(&[&**a], &none)), e.ty),
            ExprKind::PtrConv(a) => Rvalue::Use(first(self.operands(&[&**a], &none))),
            ExprKind::FuncRef(id) => Rvalue::MakeFn(*id, e.ty),
            ExprKind::Unsize(a) => Rvalue::Unsize(first(self.operands(&[&**a], &none)), e.ty),
            ExprKind::Struct(fields) => {
                let sid = e.ty.struct_id().expect("a struct value has a struct type");
                let es: Vec<&Expr> = fields.iter().map(|(_, v)| v).collect();
                let ops = self.operands(&es, &none);
                let mut by_field: Vec<Option<Operand>> = vec![None; self.hir.structs[sid].fields.len()];
                for ((f, _), op) in fields.iter().zip(ops) {
                    by_field[*f] = Some(op.op);
                }
                Rvalue::Aggregate(sid, by_field.into_iter().map(|o| o.expect("every field is given")).collect())
            }
            ExprKind::Variant { enum_id, variant, args } => {
                let es: Vec<&Expr> = args.iter().collect();
                let ops = self.operands(&es, &none);
                Rvalue::Variant(*enum_id, *variant, ops.into_iter().map(|a| a.op).collect())
            }
            ExprKind::Borrow { mutable, place: inner } => {
                let is_place = inner.is_place();
                let hoist = std::mem::take(&mut self.decl_init);
                let p = self.as_place(inner);
                if !is_place && hoist {
                    // B9 (Q13): a borrowed temporary in a declaration lives as long as the block.
                    self.hoist(p.local);
                }
                let kind = if *mutable { BorrowKind::Mut { two_phase: false } } else { BorrowKind::Shared };
                Rvalue::Ref(kind, p)
            }
            ExprKind::AddressOf(inner) => Rvalue::RawRef(self.as_place(inner)),
            ExprKind::Len(inner) => {
                let mut p = self.as_place(inner);
                if inner.ty.is_borrow() {
                    p.proj.push(Proj::Deref);
                }
                Rvalue::Len(p)
            }
            ExprKind::Array(items) => {
                let es: Vec<&Expr> = items.iter().collect();
                let ops = self.operands(&es, &none);
                Rvalue::Array(e.ty, ops.into_iter().map(|a| a.op).collect())
            }
            ExprKind::DeepCopy(inner) => Rvalue::DeepCopy(self.as_place(inner)),
            _ => unreachable!("rvalue of {}", kind_name(&e.kind)),
        }
    }

    /// Evaluate operands left to right. `borrow(i)` says operand `i` is passed
    /// as `&place` (`Some((false, _))`) or `&mut place` (`Some((true, two_phase))`).
    /// An operand that reads a place is copied into a temp when a later one
    /// might change that place (`f(i, i++)`).
    fn operands(&mut self, es: &[&Expr], borrow: &dyn Fn(usize) -> Option<(bool, bool)>) -> Vec<Arg> {
        let mut out = Vec::new();
        for (i, e) in es.iter().enumerate() {
            let span = if e.span != NO_SPAN { e.span } else { self.span };
            if let Some((mutable, two_phase)) = borrow(i) {
                let p = self.as_place(e);
                self.at(span);
                let r = self.new_ref_temp(e.ty, mutable);
                let kind = if mutable { BorrowKind::Mut { two_phase } } else { BorrowKind::Shared };
                self.assign_stmt(Place::local(r), Rvalue::Ref(kind, p));
                let rp = Place::local(r);
                out.push(Arg { op: if mutable { Operand::Move(rp) } else { Operand::Copy(rp) }, span });
            } else {
                let mut op = self.operand(e);
                let stable = matches!(&op, Operand::Copy(p) | Operand::Move(p)
                    if p.proj.is_empty() && p.local.index() > self.func.locals.len());
                if !stable && es[i + 1..].iter().any(|l| !pure(l)) {
                    op = self.freeze(op, e.ty);
                }
                out.push(Arg { op, span });
            }
        }
        out
    }

    // ---- failures ----------------------------------------------------------

    /// The temp holding the outcome of the failing call `call`.
    fn outcome_of(&mut self, call: &Expr) -> Place {
        match self.operand(call) {
            Operand::Move(p) | Operand::Copy(p) => p,
            Operand::Const(_) => unreachable!("an outcome is never a constant"),
        }
    }

    /// `if tag(out) != 0 { fail } else { ok }`; returns (fail block, ok block).
    fn branch_on_failure(&mut self, out: &Place) -> (BlockId, BlockId) {
        let ty = crate::mir::ty::place_ty_in(&self.locals, self.hir, out);
        let tag_ty = crate::mir::ty::tag_type(self.hir.enum_def(ty).tag_size);
        let tag = self.decl(tag_ty, LocalKind::Temp);
        self.assign_stmt(Place::local(tag), Rvalue::Discriminant(out.clone()));
        let failed = self.decl(Type::Bool, LocalKind::Temp);
        let zero = Operand::Const(Constant::Int(0, tag_ty));
        self.assign_stmt(Place::local(failed), Rvalue::Binary(hir::BinOp::Ne, Operand::Copy(Place::local(tag)), zero));
        let (fail_bb, ok_bb) = (self.new_block(), self.new_block());
        self.end(TerminatorKind::If { cond: Operand::Copy(Place::local(failed)), then: fail_bb, els: ok_bb });
        (fail_bb, ok_bb)
    }

    /// `out` is finished with: nothing is dropped for it any more (its parts
    /// were moved out, or it was dropped just before).
    fn forget(&mut self, out: &Place) {
        let ty = crate::mir::ty::place_ty_in(&self.locals, self.hir, out);
        let gone = self.decl(ty, LocalKind::Temp);
        self.assign_stmt(Place::local(gone), Rvalue::Use(Operand::Move(out.clone())));
    }

    fn payload(out: &Place, variant: usize) -> Place {
        let mut p = out.clone();
        p.proj.push(Proj::Downcast(variant));
        p.proj.push(Proj::Field(0));
        p
    }

    /// `call maybe fallback` (`e`): the Ok value, or the fallback.
    fn maybe(&mut self, e: &Expr, call: &Expr, fallback: &Expr) -> Operand {
        let result = self.new_temp(e.ty);
        let join = self.new_block();
        let (fail_bb, ok_bb);
        let out;
        if let ExprKind::TryParse { text, to, .. } = &call.kind {
            // Reading text needs no outcome: the status says everything.
            let ops = self.operands(&[&**text], &|_| Some((false, false)));
            let status = self.call(Callee::ParseStatus(*to), ops.clone(), Type::U8);
            let failed = self.decl(Type::Bool, LocalKind::Temp);
            self.assign_stmt(Place::local(failed), Rvalue::Binary(hir::BinOp::Ne, status, Operand::Const(Constant::Int(0, Type::U8))));
            (fail_bb, ok_bb) = (self.new_block(), self.new_block());
            self.end(TerminatorKind::If { cond: Operand::Copy(Place::local(failed)), then: fail_bb, els: ok_bb });
            self.cur = ok_bb;
            let v = self.call(Callee::Parse(*to), ops, *to);
            self.assign_stmt(Place::local(result), Rvalue::Use(v));
            out = None;
        } else {
            let o = self.outcome_of(call);
            (fail_bb, ok_bb) = self.branch_on_failure(&o);
            self.cur = ok_bb;
            let v = self.read(Self::payload(&o, 0), e.ty);
            self.assign_stmt(Place::local(result), Rvalue::Use(v));
            self.forget(&o);
            out = Some(o);
        }
        self.goto(join);
        self.cur = fail_bb;
        if let Some(o) = &out {
            // The failure is dropped before the fallback runs.
            self.push(StatementKind::Drop { place: o.clone(), flag: None, reason: DropReason::Destroy });
            self.forget(o);
        }
        self.push_frame(ScopeKind::Block, self.span);
        let f = self.operand(fallback);
        self.assign_stmt(Place::local(result), Rvalue::Use(f));
        self.pop_frame(self.span);
        self.goto(join);
        self.cur = join;
        self.read(Place::local(result), e.ty)
    }

    /// Can `target` be reached from the start, through the blocks built so far?
    fn reaches(&self, target: BlockId) -> bool {
        let mut seen = vec![false; self.blocks.len()];
        let mut stack = vec![0usize];
        seen[0] = true;
        while let Some(b) = stack.pop() {
            if b == target.index() {
                return true;
            }
            if let Some(t) = &self.blocks[b].terminator {
                for n in traversal::successors(t) {
                    if !seen[n.index()] {
                        seen[n.index()] = true;
                        stack.push(n.index());
                    }
                }
            }
        }
        false
    }

    /// The `ConvertError` for the status `st` (not zero) of reading text.
    fn parse_failure(&mut self, st: Operand, fail: Type) -> Local {
        let arms: Vec<BlockId> = (0..3).map(|_| self.new_block()).collect();
        let merge = self.new_block();
        self.end(TerminatorKind::Switch { discr: st, cases: vec![(1, arms[0]), (2, arms[1])], otherwise: arms[2] });
        let failure = self.new_temp(fail);
        for (k, arm) in arms.into_iter().enumerate() {
            self.cur = arm;
            self.assign_stmt(Place::local(failure), Rvalue::Variant(fail.enum_id().expect("ConvertError"), k, vec![Operand::Const(Constant::Int(0, Type::U64))]));
            self.goto(merge);
        }
        self.cur = merge;
        failure
    }

    /// `call fails with ...` (`e`): the Ok value, or what the handler for the failure gives.
    fn handled(&mut self, e: &Expr, call: &Expr, arms: &[hir::HandlerArm], label: Label) -> Operand {
        let result = (e.ty != Type::None).then(|| self.new_temp(e.ty));
        let join = self.new_block();
        self.resumes.insert(label, (result, join, self.frames.len()));
        let failure;
        let from;
        if let ExprKind::TryParse { text, to, fail } = &call.kind {
            let ops = self.operands(&[&**text], &|_| Some((false, false)));
            let status = self.call(Callee::ParseStatus(*to), ops.clone(), Type::U8);
            let st = self.freeze(status, Type::U8);
            let bad = self.decl(Type::Bool, LocalKind::Temp);
            self.assign_stmt(Place::local(bad), Rvalue::Binary(hir::BinOp::Ne, st.clone(), Operand::Const(Constant::Int(0, Type::U8))));
            let (fail_bb, ok_bb) = (self.new_block(), self.new_block());
            self.end(TerminatorKind::If { cond: Operand::Copy(Place::local(bad)), then: fail_bb, els: ok_bb });
            self.cur = ok_bb;
            let v = self.call(Callee::Parse(*to), ops, *to);
            self.assign_stmt(Place::local(result.expect("a number")), Rvalue::Use(v));
            self.goto(join);
            self.cur = fail_bb;
            self.push_frame(ScopeKind::Block, self.span);
            failure = self.parse_failure(st, *fail);
            from = *fail;
        } else {
            let out = self.outcome_of(call);
            let (fail_bb, ok_bb) = self.branch_on_failure(&out);
            self.cur = ok_bb;
            if let Some(r) = result {
                let v = self.read(Self::payload(&out, 0), e.ty);
                self.assign_stmt(Place::local(r), Rvalue::Use(v));
            }
            self.forget(&out);
            self.goto(join);
            self.cur = fail_bb;
            self.push_frame(ScopeKind::Block, self.span);
            let callee_ty = crate::mir::ty::place_ty_in(&self.locals, self.hir, &out);
            let hir::EnumKind::Outcome { fail: f, .. } = self.hir.enum_def(callee_ty).kind else { unreachable!("a failing call has an outcome") };
            from = f;
            failure = self.new_temp(from);
            self.assign_stmt(Place::local(failure), Rvalue::Use(Operand::Move(Self::payload(&out, 1))));
            self.forget(&out);
        }
        let set = self.hir.enum_def(from).kind == hir::EnumKind::FailSet;
        let blocks: Vec<BlockId> = arms.iter().map(|_| self.new_block()).collect();
        if set && arms.iter().any(|a| a.member.is_some()) {
            let tag_ty = crate::mir::ty::tag_type(self.hir.enum_def(from).tag_size);
            let tag = self.decl(tag_ty, LocalKind::Temp);
            self.assign_stmt(Place::local(tag), Rvalue::Discriminant(Place::local(failure)));
            let mut cases: Vec<(u128, BlockId)> = arms.iter().zip(&blocks).filter_map(|(a, &b)| a.member.map(|m| (m as u128, b))).collect();
            let otherwise = match arms.iter().position(|a| a.member.is_none()) {
                Some(k) => blocks[k],
                None => cases.pop().expect("a typed handler").1,
            };
            self.end(TerminatorKind::Switch { discr: Operand::Copy(Place::local(tag)), cases, otherwise });
        } else {
            self.goto(blocks[0]);
        }
        for (a, &bb) in arms.iter().zip(&blocks) {
            self.cur = bb;
            self.push_frame(ScopeKind::IfBranch, a.end);
            self.stmt(&a.body[0]);
            self.just_declared = None;
            let bind = self.local(a.bind);
            let src = match a.member {
                Some(m) => Self::payload(&Place::local(failure), m),
                None => Place::local(failure),
            };
            self.assign_stmt(Place::local(bind), Rvalue::Use(Operand::Move(src)));
            if a.member.is_some() {
                self.forget(&Place::local(failure));
            }
            self.block(&a.body[1..]);
            self.pop_frame(a.end);
            if result.is_some() && self.reaches(self.cur) {
                self.diags.push(
                    Diagnostic::at(&codes::E0547, a.end, "this handler can end without giving the call's value")
                        .note("it ends here")
                        .help("end the handler with `maybe value;`, `return` or `fail`"),
                );
                self.end(TerminatorKind::Unreachable);
            } else {
                self.goto(join);
            }
        }
        self.frames.pop();
        self.cur = join;
        match result {
            Some(r) => self.read(Place::local(r), e.ty),
            None => Self::unit(),
        }
    }

    /// `call fails` (`e`): the Ok value, or a return of the failure.
    fn pass_up(&mut self, e: &Expr, call: &Expr) -> Operand {
        if let ExprKind::TryParse { text, to, fail } = &call.kind {
            let ops = self.operands(&[&**text], &|_| Some((false, false)));
            let status = self.call(Callee::ParseStatus(*to), ops.clone(), Type::U8);
            let st = self.freeze(status, Type::U8);
            let failed = self.decl(Type::Bool, LocalKind::Temp);
            self.assign_stmt(Place::local(failed), Rvalue::Binary(hir::BinOp::Ne, st.clone(), Operand::Const(Constant::Int(0, Type::U8))));
            let (bad_bb, ok_bb) = (self.new_block(), self.new_block());
            self.end(TerminatorKind::If { cond: Operand::Copy(Place::local(failed)), then: bad_bb, els: ok_bb });
            self.cur = bad_bb;
            // What the failing path makes dies there, not on the way on.
            self.push_frame(ScopeKind::Block, self.span);
            let failure = self.parse_failure(st, *fail);
            self.return_failure(failure, *fail);
            self.frames.pop();
            self.cur = ok_bb;
            return self.call(Callee::Parse(*to), ops, *to);
        }
        let out = self.outcome_of(call);
        let (fail_bb, ok_bb) = self.branch_on_failure(&out);
        self.cur = fail_bb;
        self.push_frame(ScopeKind::Block, self.span);
        let callee_ty = crate::mir::ty::place_ty_in(&self.locals, self.hir, &out);
        let hir::EnumKind::Outcome { fail: from, .. } = self.hir.enum_def(callee_ty).kind else { unreachable!("a failing call has an outcome") };
        let failure = self.new_temp(from);
        self.assign_stmt(Place::local(failure), Rvalue::Use(Operand::Move(Self::payload(&out, 1))));
        self.forget(&out);
        self.return_failure(failure, from);
        self.frames.pop();
        self.cur = ok_bb;
        if e.ty == Type::None {
            self.forget(&out);
            return Self::unit();
        }
        let result = self.new_temp(e.ty);
        let v = self.read(Self::payload(&out, 0), e.ty);
        self.assign_stmt(Place::local(result), Rvalue::Use(v));
        self.forget(&out);
        self.read(Place::local(result), e.ty)
    }

    /// Return the failure in `failure` (of type `from`, one of the failure
    /// types of this function's list or a set of them) as this function's
    /// outcome. Ends the block.
    fn return_failure(&mut self, failure: Local, from: Type) {
        let mine = self.func.ret;
        let hir::EnumKind::Outcome { fail: to, .. } = self.hir.enum_def(mine).kind else { unreachable!("a failing function returns an outcome") };
        let wrapped = if from == to {
            failure
        } else {
            let w = self.decl(to, LocalKind::Temp);
            let to_def = self.hir.enum_def(to);
            let member = |t: Type| to_def.variants.iter().position(|v| v.fields[0].ty == t).expect("the caller lists this failure");
            if self.hir.enum_def(from).kind == hir::EnumKind::FailSet {
                // Re-wrap variant by variant.
                let n = self.hir.enum_def(from).variants.len();
                let tag_ty = crate::mir::ty::tag_type(self.hir.enum_def(from).tag_size);
                let tag = self.decl(tag_ty, LocalKind::Temp);
                self.assign_stmt(Place::local(tag), Rvalue::Discriminant(Place::local(failure)));
                let arms: Vec<BlockId> = (0..n).map(|_| self.new_block()).collect();
                let merge = self.new_block();
                let cases = (0..n - 1).map(|j| (j as u128, arms[j])).collect();
                self.end(TerminatorKind::Switch { discr: Operand::Copy(Place::local(tag)), cases, otherwise: arms[n - 1] });
                for (j, arm) in arms.into_iter().enumerate() {
                    self.cur = arm;
                    let mty = self.hir.enum_def(from).variants[j].fields[0].ty;
                    let inner = self.decl(mty, LocalKind::Temp);
                    self.assign_stmt(Place::local(inner), Rvalue::Use(Operand::Move(Self::payload(&Place::local(failure), j))));
                    self.forget(&Place::local(failure));
                    let k = member(mty);
                    self.assign_stmt(Place::local(w), Rvalue::Variant(to.enum_id().unwrap(), k, vec![Operand::Move(Place::local(inner))]));
                    self.goto(merge);
                }
                self.cur = merge;
            } else {
                let k = member(from);
                self.assign_stmt(Place::local(w), Rvalue::Variant(to.enum_id().unwrap(), k, vec![Operand::Move(Place::local(failure))]));
            }
            w
        };
        self.push_frame(ScopeKind::Block, self.span);
        self.assign_stmt(Place::local(Local(0)), Rvalue::Variant(mine.enum_id().unwrap(), 1, vec![Operand::Move(Place::local(wrapped))]));
        self.unwind(0);
        self.frames.pop();
        self.diverge(TerminatorKind::Return);
    }

    /// A call whose result goes to `dest`, which the caller made (an item
    /// a runtime call writes into, never read before that).
    fn call_into(&mut self, callee: Callee, args: Vec<Arg>, dest: Place) {
        let next = self.new_block();
        self.end(TerminatorKind::Call { callee, args, dest: Some(dest), target: next });
        self.cur = next;
    }

    /// `if test { some } else { none }` into a new temp of the optional type
    /// `opt`: `some` makes the payload, which goes in `Some`. Returns the temp.
    fn optional_if(&mut self, opt: Type, test: Operand, some: impl FnOnce(&mut Self) -> Operand) -> Operand {
        let enum_id = opt.enum_id().expect("an optional is an enum");
        let res = self.new_temp(opt);
        let (some_bb, none_bb, join) = (self.new_block(), self.new_block(), self.new_block());
        self.end(TerminatorKind::If { cond: test, then: some_bb, els: none_bb });
        self.cur = some_bb;
        let got = some(self);
        self.assign_stmt(Place::local(res), Rvalue::Variant(enum_id, 1, vec![got]));
        self.goto(join);
        self.cur = none_bb;
        self.assign_stmt(Place::local(res), Rvalue::Variant(enum_id, 0, Vec::new()));
        self.goto(join);
        self.cur = join;
        self.read(Place::local(res), opt)
    }

    /// A list method with a runtime call (`hir::ListOp`). `pop` and `get` test
    /// first (the count, or `ListHas`) and take the item only when there is
    /// one, so the item is always written before it is read.
    fn list_call(&mut self, e: &Expr, op: hir::ListOp, args: &[Expr]) -> Operand {
        use hir::ListOp;
        let elem = args[0].ty.elem().expect("a list method is called on a list");
        let list_ty = args[0].ty;
        let borrow = move |i: usize| match i {
            0 => Some((op.changes(), op.changes())),
            1 if op == ListOp::Join => Some((false, false)),
            _ => None,
        };
        let es: Vec<&Expr> = args.iter().collect();
        match op {
            ListOp::Pop => {
                // The list place is read once: its count says whether there is a last item.
                let list = self.as_place(&args[0]);
                let count = self.decl(Type::U64, LocalKind::Temp);
                self.assign_stmt(Place::local(count), Rvalue::Len(list.clone()));
                let test = self.decl(Type::Bool, LocalKind::Temp);
                self.assign_stmt(Place::local(test), Rvalue::Binary(hir::BinOp::Ne, Operand::Copy(Place::local(count)), Operand::Const(Constant::Int(0, Type::U64))));
                let last = Operand::Const(Constant::Int(u64::MAX as u128, Type::S64));
                self.optional_if(e.ty, Operand::Copy(Place::local(test)), |b| {
                    let r = b.new_ref_temp(list_ty, true);
                    b.assign_stmt(Place::local(r), Rvalue::Ref(BorrowKind::Mut { two_phase: false }, list.clone()));
                    let out = b.decl(elem, LocalKind::Temp);
                    let args = vec![Arg { op: Operand::Move(Place::local(r)), span: b.span }, Arg { op: last, span: b.span }];
                    b.call_into(Callee::ListOp(ListOp::Remove, elem), args, Place::local(out));
                    b.read(Place::local(out), elem)
                })
            }
            ListOp::Get => {
                // `get(i)`: the index is evaluated once; `ListHas` says whether it is in the list.
                let list = self.as_place(&args[0]);
                let idx = self.operands(&[&args[1]], &|_| None).remove(0).op;
                let r = self.new_ref_temp(list_ty, false);
                self.assign_stmt(Place::local(r), Rvalue::Ref(BorrowKind::Shared, list.clone()));
                let found_args = vec![Arg { op: Operand::Copy(Place::local(r)), span: self.span }, Arg { op: idx.clone(), span: self.span }];
                let found = self.call(Callee::ListHas(elem), found_args, Type::Bool);
                self.optional_if(e.ty, found, |b| {
                    let r = b.new_ref_temp(list_ty, false);
                    b.assign_stmt(Place::local(r), Rvalue::Ref(BorrowKind::Shared, list.clone()));
                    let out = b.decl(elem, LocalKind::Temp);
                    let args = vec![Arg { op: Operand::Copy(Place::local(r)), span: b.span }, Arg { op: idx, span: b.span }];
                    b.call_into(Callee::ListOp(ListOp::Get, elem), args, Place::local(out));
                    b.read(Place::local(out), elem)
                })
            }
            ListOp::Remove | ListOp::SwapRemove => {
                let ops = self.operands(&es, &borrow);
                let out = self.decl(elem, LocalKind::Temp);
                self.call_into(Callee::ListOp(op, elem), ops, Place::local(out));
                self.read(Place::local(out), elem)
            }
            ListOp::Insert | ListOp::Clear => {
                let ops = self.operands(&es, &borrow);
                self.call(Callee::ListOp(op, elem), ops, Type::None)
            }
            ListOp::Join => {
                let ops = self.operands(&es, &borrow);
                self.call(Callee::ListOp(op, elem), ops, e.ty)
            }
        }
    }

    /// `call` with arguments already made.
    fn call_args(&mut self, callee: Callee, args: Vec<Arg>, ret: Type) -> Operand {
        self.call(callee, args, ret)
    }

    /// Emit a call to a fresh target block; its result goes to a fresh temp.
    fn call(&mut self, callee: Callee, args: Vec<Arg>, ret: Type) -> Operand {
        let dest = (ret != Type::None).then(|| Place::local(self.new_temp(ret)));
        let next = self.new_block();
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
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::Bool(_)
        | ExprKind::Str(_)
        | ExprKind::Local(_)
        | ExprKind::NullPtr => true,
        ExprKind::Field(a, _)
        | ExprKind::Len(a)
        | ExprKind::Neg(a)
        | ExprKind::Not(a)
        | ExprKind::BitNot(a)
        | ExprKind::Convert(a)
        | ExprKind::PtrConv(a)
        | ExprKind::Unsize(a)
        | ExprKind::Deref(a)
        | ExprKind::Discriminant(a)
        | ExprKind::Payload { value: a, .. }
        | ExprKind::DeepCopy(a)
        | ExprKind::AddressOf(a) => pure(a),
        ExprKind::Variant { args, .. } => args.iter().all(pure),
        ExprKind::Index(a, b) => pure(a) && pure(b),
        ExprKind::Borrow { place, .. } => pure(place),
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
        ExprKind::Maybe { .. } => "Maybe",
        ExprKind::Convert(_) => "Convert",
        ExprKind::PtrConv(_) => "PtrConv",
        ExprKind::FuncRef(_) => "FuncRef",
        ExprKind::FnCall(_) => "FnCall",
        ExprKind::Unsize(_) => "Unsize",
        ExprKind::DynCall(..) => "DynCall",
        ExprKind::Call(..) => "Call",
        ExprKind::IncDec { .. } => "IncDec",
        ExprKind::Text(_) => "Text",
        ExprKind::Radix { .. } => "Radix",
        ExprKind::StrCall(..) => "StrCall",
        ExprKind::Join(_) => "Join",
        ExprKind::Out { .. } => "Out",
        ExprKind::InLine => "InLine",
        ExprKind::InEnded => "InEnded",
        ExprKind::TryParse { .. } => "TryParse",
        ExprKind::FailMaybe { .. } => "FailMaybe",
        ExprKind::PassUp { .. } => "PassUp",
        ExprKind::Handled { .. } => "Handled",
        ExprKind::CauseWrap { .. } => "CauseWrap",
        ExprKind::Report { .. } => "Report",
        ExprKind::Borrow { .. } => "Borrow",
        ExprKind::Deref(_) => "Deref",
        ExprKind::DeepCopy(_) => "DeepCopy",
        ExprKind::Take { .. } => "Take",
        ExprKind::Lend(_) => "Lend",
        ExprKind::AddressOf(_) => "AddressOf",
        ExprKind::NullPtr => "NullPtr",
        ExprKind::Variant { .. } => "Variant",
        ExprKind::Discriminant(_) => "Discriminant",
        ExprKind::Payload { .. } => "Payload",
        ExprKind::Index(..) => "Index",
        ExprKind::Len(_) => "Len",
        ExprKind::List(_) => "List",
        ExprKind::Array(_) => "Array",
        ExprKind::ListPush { .. } => "ListPush",
        ExprKind::ListCall { .. } => "ListCall",
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
