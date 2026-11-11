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
pub fn display_name(symbol: &str) -> &str {
    symbol.strip_prefix("drachm_f_").unwrap_or(symbol)
}

const NO_SPAN: Span = Span { start: 0, end: 0 };

/// A HIR place as a MIR place (`self` of a method is dealt with by the builder).
fn place(p: &hir::Place) -> Place {
    let proj = p
        .proj
        .iter()
        .map(|pr| match pr {
            hir::HirProj::Field(f) => Proj::Field(*f),
            hir::HirProj::Deref => Proj::Deref,
        })
        .collect();
    Place { local: Local::new(p.local + 1), proj }
}

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
        lives: hir.funcs.iter().map(|_| LivesSig::Unknown).collect(),
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

/// The locals declared by `Stmt::Decl` anywhere in a block.
fn decls(b: &[Stmt], out: &mut HashSet<usize>) {
    for s in b {
        match s {
            Stmt::Decl(l) => {
                out.insert(*l);
            }
            Stmt::If { branches, .. } => branches.iter().for_each(|br| decls(&br.body, out)),
            Stmt::Loop { body, step, .. } => {
                decls(body, out);
                decls(step, out);
            }
            Stmt::Unsafe(b) => decls(b, out),
            _ => {}
        }
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
    span: Span,
    unsafe_depth: u32,
    /// The local a `Decl` just declared, until the next statement.
    just_declared: Option<Local>,
    /// The statement being lowered initialises the local `Decl` just declared.
    decl_init: bool,
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
            span: NO_SPAN,
            unsafe_depth: 0,
            just_declared: None,
            decl_init: false,
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
                LocalKind::Param { name: l.name.clone(), mutability }
            } else if declared.contains(&i) || !is_hidden(&l.name) {
                LocalKind::User { name: l.name.clone(), mutability }
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

    fn hir_place(&self, p: &hir::Place) -> Place {
        let mut out = place(p);
        if p.local == 0 && self.func.has_self {
            out.proj.insert(0, Proj::Deref);
        }
        out
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
        let s = Statement { kind, span: self.span, unsafe_ctx: self.unsafe_depth > 0 };
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
                let dest = self.hir_place(p);
                self.at(p.span);
                let fresh = declared == Some(dest.local) && dest.proj.is_empty();
                self.decl_init = fresh;
                // A just-declared local holds nothing: nothing to drop first.
                self.in_stmt(|b| b.assign(dest, e, !fresh));
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

    fn if_stmt(&mut self, label: Label, branches: &[hir::Branch]) {
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
            | ExprKind::Struct(_)
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
                self.drop_before_assign(&dest);
                self.assign_stmt(dest, Rvalue::Use(Operand::Move(Place::local(t))));
            } else {
                self.assign_stmt(dest, rv);
            }
        } else {
            let op = self.operand(e);
            if e.ty != Type::None {
                if owning {
                    self.drop_before_assign(&dest);
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
        let root_hidden = matches!(self.locals[p.local].kind, LocalKind::Temp);
        let field_move = p.proj.iter().any(|x| matches!(x, Proj::Field(_))) && !p.proj.contains(&Proj::Deref);
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
            ExprKind::Local(_) | ExprKind::Field(..) | ExprKind::Deref(_) => {
                let p = self.as_place(e);
                self.at(expr_span(e));
                self.read_place(p, e)
            }
            ExprKind::Neg(_)
            | ExprKind::Not(_)
            | ExprKind::BitNot(_)
            | ExprKind::Binary(..)
            | ExprKind::Shift { .. }
            | ExprKind::IntPow(..)
            | ExprKind::Convert(_)
            | ExprKind::PtrConv(_)
            | ExprKind::Struct(_)
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
            ExprKind::StrCall(f, args) => {
                let params = f.params();
                let es: Vec<&Expr> = args.iter().collect();
                let ops = self
                    .operands(&es, &|i| (params.get(i) == Some(&Type::Str)).then_some((*f == StrFn::Append && i == 0, false)));
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
        | ExprKind::Neg(a)
        | ExprKind::Not(a)
        | ExprKind::BitNot(a)
        | ExprKind::Convert(a)
        | ExprKind::PtrConv(a)
        | ExprKind::Deref(a)
        | ExprKind::DeepCopy(a)
        | ExprKind::AddressOf(a) => pure(a),
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
        ExprKind::Convert(_) => "Convert",
        ExprKind::PtrConv(_) => "PtrConv",
        ExprKind::Call(..) => "Call",
        ExprKind::IncDec { .. } => "IncDec",
        ExprKind::Text(_) => "Text",
        ExprKind::Radix { .. } => "Radix",
        ExprKind::StrCall(..) => "StrCall",
        ExprKind::Join(_) => "Join",
        ExprKind::Out { .. } => "Out",
        ExprKind::Borrow { .. } => "Borrow",
        ExprKind::Deref(_) => "Deref",
        ExprKind::DeepCopy(_) => "DeepCopy",
        ExprKind::Take { .. } => "Take",
        ExprKind::Lend(_) => "Lend",
        ExprKind::AddressOf(_) => "AddressOf",
        ExprKind::NullPtr => "NullPtr",
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
