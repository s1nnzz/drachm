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
    let mut variant: Option<usize> = None;
    for (i, pr) in p.proj.iter().enumerate() {
        match *pr {
            Proj::Deref => {
                t = t.target().ok_or_else(|| format!("Deref of `{t}` in a place based on {}", p.local))?;
            }
            Proj::Downcast(v) => {
                let Type::Enum { id, .. } = t else {
                    return Err(format!("Downcast (projection {i}) of non-enum `{t}` in a place based on {}", p.local));
                };
                let def = hir.enums.get(id).ok_or_else(|| format!("enum #{id} does not exist"))?;
                if v >= def.variants.len() {
                    return Err(format!("enum {} has no variant {v}", def.name));
                }
                if !matches!(p.proj.get(i + 1), Some(Proj::Field(_))) {
                    return Err(format!("Downcast in a place based on {} is not followed by a Field (V10)", p.local));
                }
                variant = Some(v);
            }
            Proj::Field(f) => {
                if let Some(v) = variant.take() {
                    let Type::Enum { id, .. } = t else { unreachable!() };
                    let fd = hir.enums[id].variants[v].fields.get(f).ok_or_else(|| format!("variant {v} of {} has no field {f}", hir.enums[id].name))?;
                    t = fd.ty;
                    continue;
                }
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

/// The type of `p` while a body is being built (its locals are not a `Body` yet).
pub fn place_ty_in(locals: &IndexVec<Local, LocalDecl>, hir: &hir::Program, p: &Place) -> Type {
    let mut t = locals[p.local].ty;
    let mut variant = None;
    for pr in &p.proj {
        t = match *pr {
            Proj::Deref => t.target().unwrap_or(t),
            Proj::Downcast(v) => {
                variant = Some(v);
                t
            }
            Proj::Field(f) => match variant.take() {
                Some(v) => hir.enum_def(t).variants[v].fields[f].ty,
                None => hir.struct_def(t).fields[f].ty,
            },
        };
    }
    t
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

/// The unsigned integer type of an enum tag of `size` bytes.
pub fn tag_type(size: u32) -> Type {
    Type::Int { bits: (size * 8) as u16, signed: false }
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
        Rvalue::Variant(eid, ..) => hir.enum_type(*eid),
        Rvalue::Discriminant(p) => {
            let t = place_ty(body, hir, p)?;
            let Type::Enum { id, .. } = t else { return Err(format!("Discriminant of non-enum `{t}`")) };
            tag_type(hir.enums[id].tag_size)
        }
        Rvalue::DeepCopy(p) => place_ty(body, hir, p)?,
    })
}
