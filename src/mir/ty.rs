//! The types of places, operands and rvalues.
//!
//! PHASE A: `types::Type` has no references yet, so a local that holds a
//! `&T` is a `T` with `LocalDecl::borrow` set, and `Ty` pairs the two. Phase B
//! replaces `Ty` by `Type` (with `Ref`/`Ptr`) and handles `Deref` properly.

use super::*;
use crate::types;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ty {
    pub ty: Type,
    /// `Some(mutable)`: a reference to `ty`.
    pub borrow: Option<bool>,
}

impl Ty {
    pub fn plain(ty: Type) -> Ty {
        Ty { ty, borrow: None }
    }
}

impl std::fmt::Display for Ty {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self.borrow {
            None => write!(f, "{}", self.ty),
            Some(false) => write!(f, "&{}", self.ty),
            Some(true) => write!(f, "&mut {}", self.ty),
        }
    }
}

pub fn local_ty(body: &Body, l: Local) -> Result<Ty, String> {
    let d = body.locals.get(l).ok_or_else(|| format!("{l} does not exist"))?;
    Ok(Ty { ty: d.ty, borrow: d.borrow })
}

/// The type of `p`, or why it is ill-typed (V2).
pub fn place_ty(body: &Body, hir: &hir::Program, p: &Place) -> Result<Ty, String> {
    let mut t = local_ty(body, p.local)?;
    for (i, pr) in p.proj.iter().enumerate() {
        match *pr {
            Proj::Deref => {
                if t.borrow.is_none() {
                    return Err(format!("Deref of `{t}` in a place based on {}", p.local));
                }
                t.borrow = None;
            }
            Proj::Field(f) => {
                if t.borrow.is_some() {
                    return Err(format!("field {f} of the reference `{t}` without a Deref ({})", p.local));
                }
                let Type::Struct { id, .. } = t.ty else {
                    return Err(format!("field {f} (projection {i}) of non-struct `{}` in a place based on {}", t.ty, p.local));
                };
                let def = hir.structs.get(id).ok_or_else(|| format!("struct #{id} does not exist"))?;
                let fd = def.fields.get(f).ok_or_else(|| format!("struct {} has no field {f}", def.name))?;
                t = Ty::plain(fd.ty);
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

pub fn operand_ty(body: &Body, hir: &hir::Program, op: &Operand) -> Result<Ty, String> {
    match op {
        Operand::Copy(p) | Operand::Move(p) => place_ty(body, hir, p),
        Operand::Const(c) => Ok(Ty::plain(const_ty(c))),
    }
}

/// The struct type with id `sid`.
pub fn struct_type(hir: &hir::Program, sid: StructId) -> Type {
    Type::Struct { id: sid, name: types::intern(&hir.structs[sid].name) }
}

pub fn rvalue_ty(body: &Body, hir: &hir::Program, rv: &Rvalue) -> Result<Ty, String> {
    Ok(match rv {
        Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) => operand_ty(body, hir, o)?,
        Rvalue::Binary(op, a, _) => {
            if op.is_comparison() {
                Ty::plain(Type::Bool)
            } else {
                operand_ty(body, hir, a)?
            }
        }
        Rvalue::Shift { value, .. } => operand_ty(body, hir, value)?,
        Rvalue::IntPow(base, _) => operand_ty(body, hir, base)?,
        Rvalue::Convert(_, t) => Ty::plain(*t),
        Rvalue::Ref(kind, p) => {
            let t = place_ty(body, hir, p)?;
            if t.borrow.is_some() {
                return Err("a reference to a reference".into());
            }
            Ty { ty: t.ty, borrow: Some(matches!(kind, BorrowKind::Mut { .. })) }
        }
        Rvalue::RawRef(_) => return Err("RawRef needs raw pointer types (phase B)".into()),
        Rvalue::Aggregate(sid, _) => Ty::plain(struct_type(hir, *sid)),
        Rvalue::DeepCopy(p) => place_ty(body, hir, p)?,
    })
}
