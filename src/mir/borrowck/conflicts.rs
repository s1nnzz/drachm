//! Accesses and conflicts (spec 3.5.6).

use super::constraints::{Generated, Loan};
use super::loans::LoanScopes;
use crate::diag::Span;
use crate::hir;
use crate::mir::index::Idx;
use crate::mir::traversal::reverse_postorder;
use crate::mir::ty::place_ty;
use crate::mir::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessKind {
    Read,
    ShareBorrow,
    MutBorrow,
    Write,
    Move,
    Drop,
    Dead,
}

/// How the access came about, for the wording of the error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Plain,
    /// The call that activates a method receiver's two-phase `&mut`.
    Receiver,
    /// An explicit `&mut x` (or `lend`) taken for an argument.
    MutArgument,
    /// The drop that precedes an assignment.
    ReassignDrop,
    /// A `Drop`, `StorageDead` or return at the end of the owner's life.
    ScopeEnd,
    /// A `destroy x;`.
    Destroy,
}

#[derive(Debug, Clone)]
pub struct Access {
    pub place: Place,
    pub kind: AccessKind,
    pub span: Span,
    /// The point where the access happens.
    pub loc: Location,
    /// The point whose loans in scope it is checked against (the call's
    /// target for the write of a result, otherwise `loc`).
    pub scope_loc: Location,
    pub via: Via,
    /// The loan that is being activated, which the access is not checked against.
    pub skip_loan: Option<usize>,
    /// Shared borrows passed to the activating runtime string call are not checked
    /// (its functions read the other string before changing, so `s += s` is fine).
    pub skip_str_args: bool,
}

#[derive(Debug, Clone)]
pub struct Conflict {
    pub loan: usize,
    pub access: Access,
}

fn operand_access(op: &Operand, span: Span, loc: Location, via: Via, out: &mut Vec<Access>) {
    let (place, kind) = match op {
        Operand::Copy(p) => (p, AccessKind::Read),
        Operand::Move(p) => (p, AccessKind::Move),
        Operand::Const(_) => return,
    };
    out.push(Access { place: place.clone(), kind, span, loc, scope_loc: loc, via, skip_loan: None, skip_str_args: false });
}

/// The accesses of the statement or terminator at `loc`, in evaluation order.
pub fn accesses(hir: &hir::Program, body: &Body, loc: Location) -> Vec<Access> {
    let block = &body.blocks[loc.block];
    let i = loc.index as usize;
    let mut out = Vec::new();
    let acc = |place: &Place, kind, span, via| Access { place: place.clone(), kind, span, loc, scope_loc: loc, via, skip_loan: None, skip_str_args: false };
    if i < block.statements.len() {
        let s = &block.statements[i];
        match &s.kind {
            StatementKind::Assign(dest, rv) => {
                match rv {
                    Rvalue::MakeFn(_, _, Some(p)) => out.push(acc(p, AccessKind::ShareBorrow, s.span, Via::Plain)),
                    Rvalue::MakeFn(_, _, None) => {}

                    Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) | Rvalue::Unsize(o, _) => {
                        operand_access(o, s.span, loc, Via::Plain, &mut out)
                    }
                    Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) | Rvalue::Checked(_, _, a, b) => {
                        operand_access(a, s.span, loc, Via::Plain, &mut out);
                        operand_access(b, s.span, loc, Via::Plain, &mut out);
                    }
                    Rvalue::Shift { value, amount, .. } => {
                        operand_access(value, s.span, loc, Via::Plain, &mut out);
                        operand_access(amount, s.span, loc, Via::Plain, &mut out);
                    }
                    Rvalue::Aggregate(_, ops) | Rvalue::Variant(_, _, ops) | Rvalue::Array(_, ops) => ops.iter().for_each(|o| operand_access(o, s.span, loc, Via::Plain, &mut out)),
                    Rvalue::ListNew(..) | Rvalue::MapNew(_) => {}
                    Rvalue::Ref(BorrowKind::Shared, p) => out.push(acc(p, AccessKind::ShareBorrow, s.span, Via::Plain)),
                    Rvalue::Ref(BorrowKind::Mut { two_phase }, p) => {
                        // A receiver is only reserved here, which reads like a shared borrow.
                        let kind = if *two_phase { AccessKind::ShareBorrow } else { AccessKind::MutBorrow };
                        out.push(acc(p, kind, s.span, Via::MutArgument));
                    }
                    Rvalue::RawRef(p) | Rvalue::DeepCopy(p) | Rvalue::Discriminant(p) | Rvalue::Len(p) => out.push(acc(p, AccessKind::Read, s.span, Via::Plain)),
                }
                // Writing through a borrow (`*r = ..`, `p.f = ..`) reads the borrow variable.
                out.push(acc(dest, AccessKind::Write, s.span, Via::Plain));
            }
            StatementKind::StorageDead(l) => out.push(acc(&Place::local(*l), AccessKind::Dead, s.span, Via::ScopeEnd)),
            StatementKind::Drop { place, reason, .. } => {
                let ty = place_ty(body, hir, place).expect("validated MIR");
                if !ty.is_borrow() && !ty.is_ptr() {
                    match reason {
                        DropReason::Assign => out.push(acc(place, AccessKind::Write, s.span, Via::ReassignDrop)),
                        DropReason::ScopeEnd if crate::mir::needs_drop(hir, ty) => out.push(acc(place, AccessKind::Drop, s.span, Via::ScopeEnd)),
                        DropReason::Destroy => out.push(acc(place, AccessKind::Drop, s.span, Via::Destroy)),
                        DropReason::ScopeEnd => {}
                    }
                }
            }
            StatementKind::FakeRead(p) => out.push(acc(p, AccessKind::Read, s.span, Via::Plain)),
            StatementKind::StorageLive(_) | StatementKind::Nop => {}
        }
    } else {
        let t = &block.terminator;
        match &t.kind {
            TerminatorKind::If { cond, .. } => operand_access(cond, t.span, loc, Via::Plain, &mut out),
            TerminatorKind::Switch { discr, .. } => operand_access(discr, t.span, loc, Via::Plain, &mut out),
            TerminatorKind::Call { args, dest, target, .. } => {
                for a in args {
                    operand_access(&a.op, a.span, loc, Via::Plain, &mut out);
                }
                if let Some(d) = dest {
                    let mut w = acc(d, AccessKind::Write, t.span, Via::Plain);
                    // The result is stored when the call returns.
                    w.scope_loc = Location { block: *target, index: 0 };
                    out.push(w);
                }
            }
            TerminatorKind::Return => {
                // The builder ends every local's storage before returning, except
                // parameters, which live until the function returns.
                for l in 1..=body.arg_count {
                    let l = Local(l as u32);
                    let ty = body.locals[l].ty;
                    if !ty.is_borrow() && !ty.is_ptr() {
                        out.push(acc(&Place::local(l), AccessKind::Dead, t.span, Via::ScopeEnd));
                    }
                }
            }
            TerminatorKind::Goto(_) | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => {}
        }
    }
    out
}

/// Do a loan of `borrowed` and an `access` to `accessed` touch the same memory?
pub fn places_conflict(hir: &hir::Program, body: &Body, borrowed: &Place, accessed: &Place, kind: AccessKind) -> bool {
    if borrowed.local != accessed.local {
        return false;
    }
    // An access through a raw pointer is never checked.
    let mut ty = body.locals[accessed.local].ty;
    for pr in &accessed.proj {
        match *pr {
            Proj::Deref => {
                if ty.is_ptr() {
                    return false;
                }
                ty = ty.target().unwrap_or(ty);
            }
            Proj::Field(f) => {
                if let Type::Struct { id, .. } = ty {
                    ty = hir.structs[id].fields[f].ty;
                }
            }
            // A payload place overlaps every other place through the same enum (conservative).
            Proj::Downcast(_) => {}
            // Any item may be any other (conservative).
            Proj::Index(_) => ty = ty.elem().unwrap_or(ty),
        }
    }
    let n = borrowed.proj.len().min(accessed.proj.len());
    for i in 0..n {
        if let (Proj::Field(a), Proj::Field(b)) = (borrowed.proj[i], accessed.proj[i]) {
            if a != b {
                return false;
            }
        }
    }
    let proper_prefix = accessed.proj.len() < borrowed.proj.len();
    if proper_prefix {
        match kind {
            // Overwriting a borrow variable does not touch what it pointed to.
            AccessKind::Write if borrowed.proj[accessed.proj.len()] == Proj::Deref => return false,
            // The owner dying does not invalidate data it only borrowed.
            AccessKind::Drop | AccessKind::Dead if borrowed.proj[accessed.proj.len()..].contains(&Proj::Deref) => return false,
            _ => {}
        }
    }
    true
}

/// The code of the error when an access meets a loan of `kind`, if it does.
pub fn conflict_code(loan: BorrowKind, access: AccessKind) -> Option<&'static str> {
    use AccessKind::*;
    let shared = matches!(loan, BorrowKind::Shared);
    match (shared, access) {
        (_, Move) => Some("E0305"),
        (_, Drop | Dead) => Some("E0306"),
        (_, Write) => Some("E0302"),
        (true, MutBorrow) => Some("E0302"),
        (true, Read | ShareBorrow) => None,
        (false, MutBorrow) => Some("E0303"),
        (false, Read | ShareBorrow) => Some("E0304"),
    }
}

/// Every loan that an access conflicts with, at most one per loan (the first
/// access in program order).
pub fn find(hir: &hir::Program, body: &Body, cg: &Generated, scopes: &LoanScopes) -> Vec<Conflict> {
    let rpo = reverse_postorder(body);
    let mut rank = vec![usize::MAX; body.blocks.len()];
    for (i, b) in rpo.iter().enumerate() {
        rank[b.index()] = i;
    }
    let order = |l: Location| (rank[l.block.index()], l.index);
    let mut best: Vec<Option<(( usize, u32), usize, Access)>> = vec![None; cg.loans.len()];
    let mut seq = 0usize;
    let mut check = |access: Access, best: &mut Vec<Option<((usize, u32), usize, Access)>>| {
        seq += 1;
        let at = cg.points.index(access.scope_loc);
        for &li in &scopes.by_point[at] {
            if access.skip_loan == Some(li) {
                continue;
            }
            let loan: &Loan = &cg.loans[li];
            if access.skip_str_args && loan.kind == BorrowKind::Shared && call_arg_is(body, access.loc, loan.dest.local) {
                continue;
            }
            let reserved = scopes.reserved[li].contains(at);
            let kind = match loan.kind {
                BorrowKind::Mut { two_phase: true } if reserved => BorrowKind::Shared,
                k => k,
            };
            if conflict_code(kind, access.kind).is_none() || !places_conflict(hir, body, &loan.place, &access.place, access.kind) {
                continue;
            }
            let key = (order(access.loc), seq);
            if best[li].as_ref().is_none_or(|(k, ..)| (k.0, k.1) > key.0) {
                best[li] = Some((key.0, seq, access.clone()));
            }
        }
    };
    for p in 0..cg.points.len() {
        let loc = cg.points.loc(p);
        if rank[loc.block.index()] == usize::MAX {
            continue;
        }
        for a in accesses(hir, body, loc) {
            check(a, &mut best);
        }
        // A two-phase borrow becomes a real `&mut` at its activation.
        for (li, act) in scopes.activation.iter().enumerate() {
            if *act == Some(loc) {
                let loan = &cg.loans[li];
                    let a = Access {
                        place: loan.place.clone(),
                        kind: AccessKind::MutBorrow,
                        span: body.blocks[loc.block].terminator.span,
                        loc,
                        scope_loc: loc,
                        via: Via::Receiver,
                        skip_loan: Some(li),
                        skip_str_args: matches!(&body.blocks[loc.block].terminator.kind, TerminatorKind::Call { callee: Callee::Str(_), .. }),
                    };
                    check(a, &mut best);
            }
        }
    }
    best.into_iter()
        .enumerate()
        .filter_map(|(loan, b)| b.map(|(_, _, access)| Conflict { loan, access }))
        .collect()
}

/// Is `local` (a borrow temp) one of the arguments of the call at `loc`?
fn call_arg_is(body: &Body, loc: Location, local: Local) -> bool {
    match &body.blocks[loc.block].terminator.kind {
        TerminatorKind::Call { args, .. } => args.iter().any(|a| matches!(&a.op, Operand::Copy(p) | Operand::Move(p) if p.local == local && p.proj.is_empty())),
        _ => false,
    }
}
