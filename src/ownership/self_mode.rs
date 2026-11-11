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
            Evidence::Writes(span, text) => Some(MutReason { span: *span, kind: MutKind::Writes(text.clone()) }),
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

/// Evidence found in a body.
enum Evidence {
    Writes(Span, String),
    LendsMut(Span, String),
    /// A call with a self place as its receiver (`args[0]`).
    Calls(Span, FuncId),
}

/// Every piece of evidence in `func`'s body, in source order. Reads, shared
/// borrows, moves and calls on places not rooted at `self` are not evidence.
fn walk(hir: &hir::Program, func: &hir::Func, out: &mut dyn FnMut(Evidence)) {
    let cx = Cx { hir, func };
    cx.block(&func.body, out);
}

struct Cx<'a> {
    hir: &'a hir::Program,
    func: &'a hir::Func,
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
            // Moving a value out through a borrow is an error later (E0310),
            // so `destroy` of a part of `self` is not evidence.
            Stmt::Destroy(_) => {}
        }
    }

    fn expr(&self, e: &Expr, out: &mut dyn FnMut(Evidence)) {
        match &e.kind {
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::Bool(_)
            | ExprKind::Str(_)
            | ExprKind::Local(_)
            | ExprKind::NullPtr => {}
            ExprKind::Field(inner, _)
            | ExprKind::Neg(inner)
            | ExprKind::Not(inner)
            | ExprKind::BitNot(inner)
            | ExprKind::Convert(inner)
            | ExprKind::Text(inner)
            | ExprKind::DeepCopy(inner)
            | ExprKind::AddressOf(inner)
            | ExprKind::Deref(inner)
            | ExprKind::Out { arg: inner, .. }
            | ExprKind::Radix { value: inner, .. } => self.expr(inner, out),
            ExprKind::Struct(fields) => {
                for (_, v) in fields {
                    self.expr(v, out);
                }
            }
            ExprKind::Binary(_, a, b) | ExprKind::IntPow(a, b) | ExprKind::Logic { lhs: a, rhs: b, .. } => {
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
            ExprKind::StrCall(_, args) | ExprKind::Join(args) => {
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
        if let Some(text) = self.place_text(place) {
            out(Evidence::Writes(place.span, text));
        }
    }

    /// The text of a self place as the user writes it, or `None` if the
    /// expression is not rooted at `self` or goes through a raw pointer.
    fn self_text(&self, e: &Expr) -> Option<String> {
        match &e.kind {
            ExprKind::Local(0) if self.func.has_self => Some("self".to_string()),
            ExprKind::Field(inner, i) => {
                let base = self.self_text(inner)?;
                Some(self.extend(base, inner.ty, *i))
            }
            ExprKind::Deref(inner) if !inner.ty.is_ptr() => self.self_text(inner),
            _ => None,
        }
    }

    fn place_text(&self, place: &Place) -> Option<String> {
        if !self.func.has_self || place.local != 0 {
            return None;
        }
        let mut text = "self".to_string();
        let mut ty = self.func.locals[0].ty;
        for p in &place.proj {
            match *p {
                HirProj::Field(i) => {
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
