//! The types of places, operands and rvalues. MIR uses `types::Type`
//! directly, `Ref` and `Ptr` included.

use super::*;
use crate::types;

pub fn local_ty(body: &Body, l: Local) -> Result<Type, String> {
    body.locals.get(l).map(|d| d.ty).ok_or_else(|| format!("{l} does not exist"))
}

/// The type of `p`, or why it is ill-typed (V2).
pub fn place_ty(body: &Body, hir: &hir::Program, p: &Place) -> Result<Type, String> {
    let mut t = local_ty(body, p.local)?;
    for (i, pr) in p.proj.iter().enumerate() {
        match *pr {
            Proj::Deref => {
                t = t.target().ok_or_else(|| format!("Deref of `{t}` in a place based on {}", p.local))?;
            }
            Proj::Field(f) => {
                let Type::Struct { id, .. } = t else {
                    return Err(format!("field {f} (projection {i}) of non-struct `{t}` in a place based on {}", p.local));
                };
                let def = hir.structs.get(id).ok_or_else(|| format!("struct #{id} does not exist"))?;
                let fd = def.fields.get(f).ok_or_else(|| format!("struct {} has no field {f}", def.name))?;
                t = fd.ty;
            }
        }
    }
    Ok(t)
}

pub fn const_ty(c: &Constant) -> Type {
    match c {
        Constant::Int(_, t) | Constant::Float(_, t) => *t,
        Constant::Bool(_) => Type::Bool,
        Constant::Str(_) => Type::Str,
    }
}

pub fn operand_ty(body: &Body, hir: &hir::Program, op: &Operand) -> Result<Type, String> {
    match op {
        Operand::Copy(p) | Operand::Move(p) => place_ty(body, hir, p),
        Operand::Const(c) => Ok(const_ty(c)),
    }
}

/// The struct type with id `sid`.
pub fn struct_type(hir: &hir::Program, sid: StructId) -> Type {
    Type::Struct { id: sid, name: types::intern(&hir.structs[sid].name) }
}

pub fn rvalue_ty(body: &Body, hir: &hir::Program, rv: &Rvalue) -> Result<Type, String> {
    Ok(match rv {
        Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) => operand_ty(body, hir, o)?,
        Rvalue::Binary(op, a, _) => {
            if op.is_comparison() {
                Type::Bool
            } else {
                operand_ty(body, hir, a)?
            }
        }
        Rvalue::Shift { value, .. } => operand_ty(body, hir, value)?,
        Rvalue::IntPow(base, _) => operand_ty(body, hir, base)?,
        Rvalue::Convert(_, t) => *t,
        Rvalue::Ref(kind, p) => {
            let t = place_ty(body, hir, p)?;
            if t.is_borrow() {
                return Err("a reference to a reference".into());
            }
            Type::borrow(matches!(kind, BorrowKind::Mut { .. }), t)
        }
        Rvalue::RawRef(p) => Type::ptr(place_ty(body, hir, p)?),
        Rvalue::Aggregate(sid, _) => struct_type(hir, *sid),
        Rvalue::DeepCopy(p) => place_ty(body, hir, p)?,
    })
}
