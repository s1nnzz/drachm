//! Whether a method needs `&self` (`Shared`) or `&mut self` (`Mut`), from the
//! HIR alone (spec 3.7). Direct evidence is a write to a part of `self`, or a
//! `&mut` / `lend` of one; a call on `self` to a `Mut` method is evidence too.

use std::collections::VecDeque;

use crate::diag::Span;
use crate::hir::{self, Block, Expr, ExprKind, FuncId, HirProj, Place, Stmt};
use crate::types::Type;

#[derive(Debug, Clone, PartialEq)]
pub enum SelfMode {
    Shared,
    Mut(MutReason),
}

#[derive(Debug, Clone, PartialEq)]
pub struct MutReason {
    pub span: Span,
    pub kind: MutKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MutKind {
    /// Assignment, compound assignment, `++`/`--`, or `take ... leaving`: the
    /// place as the user writes it ("self.size").
    Writes(String),
    /// `&mut self.items`, or `lend self.out`.
    LendsMut(String),
    /// A call of a method that changes `self`, on a self place.
    Calls(FuncId),
    /// The method is `drop` (reserved name: always called with exclusive access).
    IsDrop,
}

/// One entry per `FuncId`; `None` for functions without a `self`.
pub fn infer(hir: &hir::Program) -> Vec<Option<SelfMode>> {
    let n = hir.funcs.len();
    let mut modes: Vec<Option<SelfMode>> = vec![None; n];
    let mut evidence: Vec<Vec<Evidence>> = (0..n).map(|_| Vec::new()).collect();
    let mut callers: Vec<Vec<FuncId>> = vec![Vec::new(); n];
    for (id, f) in hir.funcs.iter().enumerate() {
        if !f.has_self {
            continue;
        }
        let mut found = Vec::new();
        walk(hir, f, &mut |e| found.push(e));
        for e in &found {
            if let Evidence::Calls(_, callee) = e {
                callers[*callee].push(id);
            }
        }
        evidence[id] = found;
    }
    for c in &mut callers {
        c.sort_unstable();
        c.dedup();
    }
    for s in &hir.structs {
        if let Some(d) = s.has_drop {
            modes[d] = Some(SelfMode::Mut(MutReason { span: hir.funcs[d].name_span, kind: MutKind::IsDrop }));
        }
    }
    // Least fixpoint: modes only go from Shared to Mut. A method's reason is
    // the first evidence in source order that holds when it becomes Mut.
    let mut queue: VecDeque<FuncId> = (0..n).filter(|&id| hir.funcs[id].has_self).collect();
    while let Some(m) = queue.pop_front() {
        if modes[m].is_some() {
            continue;
        }
        let reason = evidence[m].iter().find_map(|e| match e {
            Evidence::Writes(span, text, _) => Some(MutReason { span: *span, kind: MutKind::Writes(text.clone()) }),
            Evidence::LendsMut(span, text) => Some(MutReason { span: *span, kind: MutKind::LendsMut(text.clone()) }),
            Evidence::Calls(span, callee) => {
                is_mut(&modes, *callee).then(|| MutReason { span: *span, kind: MutKind::Calls(*callee) })
            }
        });
        if let Some(reason) = reason {
            modes[m] = Some(SelfMode::Mut(reason));
            queue.extend(callers[m].iter().copied().filter(|&c| modes[c].is_none()));
        }
    }
    modes
        .into_iter()
        .enumerate()
        .map(|(id, m)| if hir.funcs[id].has_self { Some(m.unwrap_or(SelfMode::Shared)) } else { None })
        .collect()
}

/// The chain of reasons for diagnostics: `[heal: Writes "self.hp"]` or
/// `[grow: Calls(resize), resize: Writes "self.cap"]`. Stops at the first
/// non-`Calls` reason; cannot loop (the reason of a `Mut` method points at a
/// method that was made `Mut` earlier).
pub fn reason_chain(modes: &[Option<SelfMode>], f: FuncId) -> Vec<(FuncId, MutReason)> {
    let mut chain: Vec<(FuncId, MutReason)> = Vec::new();
    let mut cur = f;
    while let Some(Some(SelfMode::Mut(reason))) = modes.get(cur) {
        if chain.iter().any(|(id, _)| *id == cur) {
            break;
        }
        let next = match reason.kind {
            MutKind::Calls(callee) => Some(callee),
            _ => None,
        };
        chain.push((cur, reason.clone()));
        match next {
            Some(callee) => cur = callee,
            None => break,
        }
    }
    chain
}

fn is_mut(modes: &[Option<SelfMode>], f: FuncId) -> bool {
    matches!(modes[f], Some(SelfMode::Mut(_)))
}

/// Whether `f` is a method that changes `self` only by writing `mut` fields
/// (WP5, owner Q-MF): it is `&mut self`, but may be called on an owned value
/// that is not `mut`. Recomputes the evidence; the MIR asks only when a
/// receiver check fails.
pub fn writes_only_mut_fields(hir: &hir::Program, f: FuncId) -> bool {
    let modes = infer(hir);
    is_mut(&modes, f) && mut_field_only(hir, &modes)[f]
}

/// By `FuncId`: every change of `self` the method makes is a write to a `mut`
/// field, or a call of such a method on `self`. A `drop()` never is.
fn mut_field_only(hir: &hir::Program, modes: &[Option<SelfMode>]) -> Vec<bool> {
    let evidence: Vec<Vec<Evidence>> = hir
        .funcs
        .iter()
        .map(|f| {
            let mut found = Vec::new();
            if f.has_self {
                walk(hir, f, &mut |e| found.push(e));
            }
            found
        })
        .collect();
    let mut only: Vec<bool> = hir.funcs.iter().map(|f| f.has_self).collect();
    for s in &hir.structs {
        if let Some(d) = s.has_drop {
            only[d] = false;
        }
    }
    // Greatest fixpoint: a call of a method that is only-mut-fields keeps the caller so.
    loop {
        let mut changed = false;
        for id in 0..only.len() {
            if !only[id] {
                continue;
            }
            let holds = evidence[id].iter().all(|e| match e {
                Evidence::Writes(_, _, mut_path) => *mut_path,
                Evidence::LendsMut(..) => false,
                Evidence::Calls(_, callee) => !is_mut(modes, *callee) || only[*callee],
            });
            if !holds {
                only[id] = false;
                changed = true;
            }
        }
        if !changed {
            return only;
        }
    }
}

/// Evidence found in a body.
enum Evidence {
    /// A write to a part of `self`; the flag says the path reaches a `mut`
    /// field before any explicit dereference.
    Writes(Span, String, bool),
    LendsMut(Span, String),
    /// A call with a self place as its receiver (`args[0]`).
    Calls(Span, FuncId),
}

/// Every piece of evidence in `func`'s body, in source order. Reads, shared
/// borrows, moves and calls on places not rooted at `self` are not evidence.
fn walk(hir: &hir::Program, func: &hir::Func, out: &mut dyn FnMut(Evidence)) {
    let cx = Cx { hir, func, root: 0 };
    cx.block(&func.body, out);
}

pub(crate) fn changes_local(hir: &hir::Program, code: FuncId, root: hir::LocalId, modes: &[Option<SelfMode>]) -> bool {
    let cx = Cx { hir, func: &hir.funcs[code], root };
    let mut changes = false;
    cx.block(&cx.func.body, &mut |e| {
        changes |= match e {
            Evidence::Calls(_, f) => is_mut(modes, f),
            _ => true,
        };
    });
    changes
}

struct Cx<'a> {
    hir: &'a hir::Program,
    func: &'a hir::Func,
    root: hir::LocalId,
}

impl Cx<'_> {
    fn block(&self, b: &Block, out: &mut dyn FnMut(Evidence)) {
        for s in b {
            self.stmt(s, out);
        }
    }

    fn stmt(&self, s: &Stmt, out: &mut dyn FnMut(Evidence)) {
        match s {
            Stmt::Assign(place, e) => {
                self.place_write(place, out);
                self.expr(e, out);
            }
            Stmt::If { branches, .. } => {
                for b in branches {
                    if let Some(t) = &b.test {
                        self.expr(t, out);
                    }
                    self.block(&b.body, out);
                }
            }
            Stmt::Loop { cond, body, step, .. } => {
                if let Some(c) = cond {
                    self.expr(c, out);
                }
                self.block(body, out);
                self.block(step, out);
            }
            Stmt::Jump(_) | Stmt::Decl(_) => {}
            Stmt::Return(e) => {
                if let Some(e) = e {
                    self.expr(e, out);
                }
            }
            Stmt::Expr(e) => self.expr(e, out),
            Stmt::Unsafe(b) => self.block(b, out),
            Stmt::Resume(_, e) => {
                if let Some(e) = e {
                    self.expr(e, out);
                }
            }
            Stmt::Reraise(_) => {}
            // Moving a value out through a borrow is an error later (E0310),
            // so `destroy` of a part of `self` is not evidence.
            Stmt::Destroy(_) | Stmt::Forget(_) | Stmt::Check { .. } | Stmt::Stop(_) => {}
            Stmt::InitField(place, e) => {
                self.place_write(place, out);
                self.expr(e, out);
            }
        }
    }

    fn expr(&self, e: &Expr, out: &mut dyn FnMut(Evidence)) {
        match &e.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Local(_)
            | ExprKind::FuncRef(_)
            | ExprKind::InLine
            | ExprKind::InEnded
            | ExprKind::NullPtr => {}
            ExprKind::Field(inner, _)
            | ExprKind::Neg(inner)
            | ExprKind::Not(inner)
            | ExprKind::BitNot(inner)
            | ExprKind::Convert(inner)
            | ExprKind::PtrConv(inner)
            | ExprKind::Unsize(inner)
            | ExprKind::Sleep(inner)
            | ExprKind::Text(inner)
            | ExprKind::DeepCopy(inner)
            | ExprKind::AddressOf(inner)
            | ExprKind::Deref(inner)
            | ExprKind::Discriminant(inner)
            | ExprKind::Payload { value: inner, .. }
            | ExprKind::Len(inner)
            | ExprKind::Out { arg: inner, .. }
            | ExprKind::TryParse { text: inner, .. }
            | ExprKind::PassUp { call: inner }
            | ExprKind::Radix { value: inner, .. } => self.expr(inner, out),
            ExprKind::Index(a, b) => {
                self.expr(a, out);
                self.expr(b, out);
            }
            ExprKind::List(items) | ExprKind::Array(items) => {
                for i in items {
                    self.expr(i, out);
                }
            }
            ExprKind::System(_, args, _) => {
                for arg in args {
                    self.expr(arg, out);
                }
            }
            ExprKind::ListCall { op, args } => {
                // A method that changes the list writes to it when it is part of `self`.
                if op.changes()
                    && let Some(place) = args.first().and_then(|list| self.place_of(list))
                {
                    self.place_write(&place, out);
                }
                for a in args {
                    self.expr(a, out);
                }
            }
            ExprKind::MapCall { op, args } => {
                // A map method that changes the map writes to it when it is part of `self`.
                if op.changes()
                    && let Some(place) = args.first().and_then(|map| self.place_of(map))
                {
                    self.place_write(&place, out);
                }
                for a in args {
                    self.expr(a, out);
                }
            }
            ExprKind::Map(pairs) => {
                for (k, v) in pairs {
                    self.expr(k, out);
                    self.expr(v, out);
                }
            }
            ExprKind::ListPush { list, value } => {
                // Pushing changes the list: a write to it when it is part of `self`.
                if let Some(place) = self.place_of(list) {
                    self.place_write(&place, out);
                }
                self.expr(list, out);
                self.expr(value, out);
            }
            ExprKind::Handled { call, arms, .. } => {
                self.expr(call, out);
                for a in arms {
                    self.block(&a.body, out);
                }
            }
            ExprKind::FailMaybe { call: a, fallback: b } | ExprKind::CauseWrap { msg: a, slot: b } => {
                self.expr(a, out);
                self.expr(b, out);
            }
            ExprKind::Report { msg, slot, exit } => {
                self.expr(msg, out);
                self.expr(slot, out);
                if let Some(x) = exit {
                    self.expr(x, out);
                }
            }
            ExprKind::Struct(fields) => {
                for (_, v) in fields {
                    self.expr(v, out);
                }
            }
            ExprKind::Binary(_, a, b) | ExprKind::IntPow(a, b) | ExprKind::Logic { lhs: a, rhs: b, .. } | ExprKind::Maybe { value: a, fallback: b } => {
                self.expr(a, out);
                self.expr(b, out);
            }
            ExprKind::Shift { value, amount, .. } => {
                self.expr(value, out);
                self.expr(amount, out);
            }
            ExprKind::Call(callee, args) => {
                for a in args {
                    self.expr(a, out);
                }
                if self.hir.funcs[*callee].has_self && args.first().is_some_and(|r| self.self_text(r).is_some()) {
                    out(Evidence::Calls(e.span, *callee));
                }
            }
            ExprKind::IncDec { place, .. } => self.place_write(place, out),
            ExprKind::StrCall(f, args) => {
                // `s += t` appends to `s` in place: a write to it.
                if *f == hir::StrFn::Append
                    && let Some(place) = args.first().and_then(|a| self.place_of(a))
                {
                    self.place_write(&place, out);
                }
                for a in args {
                    self.expr(a, out);
                }
            }
            ExprKind::Closure(code, captures) => {
                for (value, capture) in captures.iter().zip(&self.hir.closures[code].captures) {
                    if capture.borrow == Some(true)
                        && let Some(text) = self.self_text(value)
                    {
                        out(Evidence::LendsMut(value.span, text));
                    }
                    self.expr(value, out);
                }
            }
            ExprKind::DynCall(iface, method, args) => {
                if self.hir.dyn_ifaces[*iface].methods[*method].changes
                    && let Some(place) = args.first().and_then(|recv| self.place_of(recv))
                {
                    self.place_write(&place, out);
                }
                for a in args {
                    self.expr(a, out);
                }
            }
            ExprKind::Join(args) | ExprKind::Variant { args, .. } | ExprKind::FnCall(args) => {
                for a in args {
                    self.expr(a, out);
                }
            }
            ExprKind::Borrow { mutable, place } => {
                if *mutable {
                    if let Some(text) = self.self_text(place) {
                        out(Evidence::LendsMut(place.span, text));
                    }
                }
                self.expr(place, out);
            }
            ExprKind::Take { place, leaving } => {
                // `take p leaving v` writes the field it takes from (R15).
                if let Some(v) = leaving {
                    self.place_write(place, out);
                    self.expr(v, out);
                }
            }
            ExprKind::Lend(inner) => {
                if let Some(text) = self.self_text(inner) {
                    out(Evidence::LendsMut(inner.span, text));
                }
                self.expr(inner, out);
            }
        }
    }

    /// A write to `place` is evidence when its root is `self`.
    fn place_write(&self, place: &Place, out: &mut dyn FnMut(Evidence)) {
        for step in &place.proj {
            if let HirProj::Index(index) = step {
                self.expr(index, out);
            }
        }
        if let Some(text) = self.place_text(place) {
            out(Evidence::Writes(place.span, text, self.mut_path(place)));
        }
    }

    /// Does the path from `self` reach a `mut` field with no explicit `*` before it?
    fn mut_path(&self, place: &Place) -> bool {
        let mut ty = self.func.locals[self.root].ty;
        let mut derefed = false;
        for p in &place.proj {
            match p {
                HirProj::Downcast(_) => return false,
                HirProj::Index(_) => ty = ty.elem().unwrap_or(ty),
                HirProj::Field(i) => {
                    let i = *i;
                    let pointee = if ty.is_borrow() { ty.target().unwrap_or(ty) } else { ty };
                    let Some(_) = pointee.struct_id() else { return false };
                    let f = &self.hir.struct_def(pointee).fields[i];
                    if f.mutable && !derefed {
                        return true;
                    }
                    ty = f.ty;
                }
                HirProj::Deref => {
                    derefed = true;
                    ty = ty.target().unwrap_or(ty);
                }
            }
        }
        false
    }

    /// The HIR place an expression names, if it is `self` or a part of it.
    fn place_of(&self, e: &Expr) -> Option<Place> {
        match &e.kind {
            ExprKind::Local(id) if *id == self.root => Some(Place { local: *id, proj: Vec::new(), span: e.span }),
            ExprKind::Field(inner, i) => {
                let mut p = self.place_of(inner)?;
                p.proj.push(HirProj::Field(*i));
                Some(p)
            }
            ExprKind::Deref(inner) if !inner.ty.is_ptr() => {
                let mut p = self.place_of(inner)?;
                p.proj.push(HirProj::Deref);
                Some(p)
            }
            ExprKind::Index(inner, idx) => {
                let mut p = self.place_of(inner)?;
                p.proj.push(HirProj::Index(idx.clone()));
                Some(p)
            }
            _ => None,
        }
    }

    /// The text of a self place as the user writes it, or `None` if the
    /// expression is not rooted at `self` or goes through a raw pointer.
    fn self_text(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Local(id) if *id == self.root => Some(self.func.locals[*id].name.clone()),
            ExprKind::Field(inner, i) => {
                let base = self.self_text(inner)?;
                Some(self.extend(base, inner.ty, *i))
            }
            ExprKind::Deref(inner) if !inner.ty.is_ptr() => self.self_text(inner),
            ExprKind::Index(inner, _) => Some(format!("{}[..]", self.self_text(inner)?)),
            _ => None,
        }
    }

    fn place_text(&self, place: &Place) -> Option<String> {
        if place.local != self.root {
            return None;
        }
        let mut text = self.func.locals[self.root].name.clone();
        let mut ty = self.func.locals[self.root].ty;
        for p in &place.proj {
            match p {
                HirProj::Downcast(_) => return None,
                HirProj::Index(_) => {
                    text.push_str("[..]");
                    ty = ty.elem().unwrap_or(ty);
                }
                HirProj::Field(i) => {
                    let i = *i;
                    text = self.extend(text, ty, i);
                    ty = self.field_ty(ty, i).unwrap_or(Type::None);
                }
                HirProj::Deref => {
                    if ty.is_ptr() {
                        return None;
                    }
                    ty = ty.target().unwrap_or(ty);
                }
            }
        }
        Some(text)
    }

    /// `text` plus the name of field `i` of a value of type `ty` (a borrow
    /// is read through). Where the name cannot be found, the place is named
    /// by `self` alone.
    fn extend(&self, text: String, ty: Type, i: usize) -> String {
        match self.field_ty(ty, i) {
            Some(_) => {
                let pointee = if ty.is_borrow() { ty.target().unwrap_or(ty) } else { ty };
                let name = &self.hir.struct_def(pointee).fields[i].name;
                format!("{text}.{name}")
            }
            None => "self".to_string(),
        }
    }

    fn field_ty(&self, ty: Type, i: usize) -> Option<Type> {
        let pointee = if ty.is_borrow() { ty.target()? } else { ty };
        pointee.struct_id()?;
        self.hir.struct_def(pointee).fields.get(i).map(|f| f.ty)
    }
}
