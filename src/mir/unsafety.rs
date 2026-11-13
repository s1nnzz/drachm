//! The `unsafe` pass (WP9). Everything that touches a raw pointer value must
//! be inside an `unsafe { }` block, or in the body of an `unsafe` function,
//! which counts as one unsafe block (owner answers, Q14 and #8): making a
//! pointer, reading, writing, copying, comparing, converting, moving, passing
//! and returning one, and calling an `unsafe` function. Each statement or
//! terminator reports at most one E0316.

use super::ty::{operand_ty, place_ty};
use super::*;
use crate::diag::{codes, Diagnostic};

/// Check one body. The compiler's own functions are skipped by the caller.
pub fn run(hir: &hir::Program, body: &Body, diags: &mut Vec<Diagnostic>) {
    if body.unsafe_fn {
        return;
    }
    for block in body.blocks.iter() {
        for s in &block.statements {
            if s.unsafe_ctx {
                continue;
            }
            if let Some(d) = statement(hir, body, s) {
                diags.push(d);
            }
        }
        let t = &block.terminator;
        if !t.unsafe_ctx {
            if let Some(d) = terminator(hir, body, t) {
                diags.push(d);
            }
        }
    }
}

/// What the pointer operation is, for the message.
#[derive(Clone, Copy)]
enum Op<'a> {
    Making,
    Reading,
    Writing,
    Copying,
    Comparing,
    Null,
    Converting,
    Moving,
    Passing,
    Returning,
    /// `name` is declared `unsafe` at `decl`.
    Calling { name: &'a str, decl: Span },
}

fn report(op: Op, span: Span) -> Diagnostic {
    let (msg, label, help) = match op {
        Op::Making => (
            "making a raw pointer with `address of` needs an `unsafe { }` block",
            "the pointer is made here",
            "declare the pointer first, then `unsafe { p = address of n; }`",
        ),
        Op::Reading => (
            "reading through a raw pointer needs an `unsafe { }` block",
            "the pointer is read here",
            "declare the result first, then `unsafe { v = *p; }`",
        ),
        Op::Writing => (
            "writing through a raw pointer needs an `unsafe { }` block",
            "the pointer is written here",
            "`unsafe { *p = v; }`",
        ),
        Op::Copying => (
            "copying a raw pointer needs an `unsafe { }` block",
            "the pointer is copied here",
            "`unsafe { q = copy p; }`",
        ),
        Op::Comparing => (
            "comparing a raw pointer needs an `unsafe { }` block",
            "the pointer is compared here",
            "`unsafe { gone = p == none; }`",
        ),
        Op::Null => (
            "making a null pointer needs an `unsafe { }` block",
            "the null pointer is made here",
            "`unsafe { p = none; }`",
        ),
        Op::Converting => (
            "converting between `*T` and `*fixed T` needs an `unsafe { }` block",
            "the pointer is converted here",
            "`unsafe { r = w::freeze(); }` or `unsafe { w2 = r::thaw(); }`",
        ),
        Op::Moving => (
            "moving a raw pointer needs an `unsafe { }` block",
            "the pointer is moved here",
            "`unsafe { q = p; }`: pointers move, so `p` is gone after this",
        ),
        Op::Passing => (
            "passing a raw pointer needs an `unsafe { }` block",
            "the pointer is passed here",
            "`unsafe { f(p); }`",
        ),
        Op::Returning => (
            "returning a raw pointer needs an `unsafe { }` block",
            "the pointer is returned here",
            "`unsafe { return p; }`",
        ),
        Op::Calling { name, decl } => {
            let d = Diagnostic::at(
                &codes::E0316,
                span,
                format!("calling `{name}`, an `unsafe` function, needs an `unsafe {{ }}` block"),
            )
            .note(format!("`{name}` is `unsafe`"))
            .label(decl, format!("`{name}` is declared `unsafe` here"))
            .help(format!("`unsafe {{ x = {name}(); }}`"));
            return d;
        }
    };
    Diagnostic::at(&codes::E0316, span, msg).note(label).help(format!("do it inside a block: {help}"))
}

/// Does `p` go through a raw pointer (a `Deref` whose base is `*T` or `*fixed T`)?
fn through_ptr(body: &Body, hir: &hir::Program, p: &Place) -> bool {
    let mut t = match body.locals.get(p.local) {
        Some(d) => d.ty,
        None => return false,
    };
    let mut variant = None;
    for pr in &p.proj {
        match *pr {
            Proj::Downcast(v) => variant = Some(v),
            Proj::Index(_) => match t.elem() {
                Some(e) => t = e,
                None => return false,
            },
            Proj::Field(f) if variant.is_some() => {
                let Type::Enum { id, .. } = t else { return false };
                t = hir.enums[id].variants[variant.take().unwrap()].fields[f].ty;
            }
            Proj::Deref => {
                if t.is_ptr() {
                    return true;
                }
                match t.target() {
                    Some(to) => t = to,
                    None => return false,
                }
            }
            Proj::Field(f) => {
                let Type::Struct { id, .. } = t else { return false };
                match hir.structs.get(id).and_then(|d| d.fields.get(f)) {
                    Some(fd) => t = fd.ty,
                    None => return false,
                }
            }
        }
    }
    false
}

fn is_ptr_ty(body: &Body, hir: &hir::Program, p: &Place) -> bool {
    place_ty(body, hir, p).is_ok_and(Type::is_ptr)
}

/// The first operation on a raw pointer in `op`, if any.
fn operand_op(body: &Body, hir: &hir::Program, op: &Operand) -> Option<Op<'static>> {
    match op {
        Operand::Copy(p) | Operand::Move(p) if through_ptr(body, hir, p) => Some(Op::Reading),
        Operand::Const(Constant::Int(_, t)) if t.is_ptr() => Some(Op::Null),
        Operand::Copy(_) | Operand::Move(_) if operand_ty(body, hir, op).is_ok_and(Type::is_ptr) => Some(Op::Moving),
        _ => None,
    }
}

fn statement(hir: &hir::Program, body: &Body, s: &Statement) -> Option<Diagnostic> {
    let StatementKind::Assign(dst, rv) = &s.kind else { return None };
    let op = rvalue_op(hir, body, dst, rv)?;
    Some(report(op, s.span))
}

fn rvalue_op(hir: &hir::Program, body: &Body, dst: &Place, rv: &Rvalue) -> Option<Op<'static>> {
    // The place written to, through a pointer.
    let dst_ptr = through_ptr(body, hir, dst);
    let dst_is_ptr = is_ptr_ty(body, hir, dst);
    match rv {
        Rvalue::RawRef(_) => return Some(Op::Making),
        Rvalue::Ref(kind, p) if through_ptr(body, hir, p) => {
            return Some(match kind {
                BorrowKind::Shared => Op::Reading,
                BorrowKind::Mut { .. } => Op::Writing,
            })
        }
        Rvalue::DeepCopy(p) if through_ptr(body, hir, p) => return Some(Op::Reading),
        Rvalue::DeepCopy(p) if is_ptr_ty(body, hir, p) => return Some(Op::Copying),
        _ => {}
    }
    if dst_ptr {
        return Some(Op::Writing);
    }
    // Operands of this rvalue, in order.
    let ops: Vec<&Operand> = match rv {
        Rvalue::MakeFn(..) => Vec::new(),
        Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) | Rvalue::Unsize(o, _) => vec![o],
        Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) | Rvalue::Shift { value: a, amount: b, .. } => vec![a, b],
        Rvalue::Aggregate(_, os) | Rvalue::Variant(_, _, os) | Rvalue::Array(_, os) => os.iter().collect(),
        Rvalue::Ref(..) | Rvalue::RawRef(_) | Rvalue::DeepCopy(_) | Rvalue::Discriminant(_) | Rvalue::Len(_) | Rvalue::ListNew(..) | Rvalue::MapNew(_) => Vec::new(),
    };
    if let Rvalue::Binary(op, a, _) = rv {
        if matches!(op, hir::BinOp::Eq | hir::BinOp::Ne) && operand_ty(body, hir, a).is_ok_and(Type::is_ptr) {
            return Some(Op::Comparing);
        }
    }
    for o in &ops {
        if let Some(op) = operand_op(body, hir, o) {
            // A pointer moved into a pointer of the other kind (`freeze`, `thaw`).
            if let (Op::Moving, Rvalue::Use(_)) = (op, rv) {
                let (from, to) = (operand_ty(body, hir, o).ok(), place_ty(body, hir, dst).ok());
                if dst_is_ptr && from != to {
                    return Some(Op::Converting);
                }
            }
            return Some(op);
        }
    }
    None
}

fn terminator(hir: &hir::Program, body: &Body, t: &Terminator) -> Option<Diagnostic> {
    match &t.kind {
        TerminatorKind::Call { callee, args, .. } => {
            if let Callee::Func(f) = callee {
                let func = &hir.funcs[*f];
                if func.is_unsafe {
                    let name = super::check::source_name(&func.symbol).1;
                    let decl = func.name_span;
                    return Some(report(Op::Calling { name: &name, decl }, t.span));
                }
            }
            for a in args {
                if let Some(op) = operand_op(body, hir, &a.op) {
                    let op = if matches!(op, Op::Moving) { Op::Passing } else { op };
                    return Some(report(op, a.span));
                }
            }
            None
        }
        TerminatorKind::Return => {
            let ret = body.locals.iter().find(|d| matches!(d.kind, LocalKind::Return))?;
            ret.ty.is_ptr().then(|| report(Op::Returning, t.span))
        }
        _ => None,
    }
}
