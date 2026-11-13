//! Region slots (spec 2.4, 3.5.1, 3.5.3): where the borrows inside a type are,
//! how a struct's region parameters vary, and the universal regions of a body.
//!
//! The slots of a type, outermost first:
//!   * `&T` / `&mut T`: one slot for the borrow itself, then `T`'s slots;
//!   * a struct: one slot per name in its `lives` list, in that order (the
//!     slot is the outer region of that borrow field);
//!   * everything else (numbers, `string`, raw pointers): none.
//!
//! A `lives` field is `&T` / `&mut T` with a slot-free `T` (E0314 and E0901),
//! so a struct's slots are exactly its borrow fields' outer regions.

use std::ops::Range;

use crate::hir::{self, FuncId, StructId};
use crate::mir::{Body, Local, Place, Proj};
use crate::types::Type;

/// The number of region slots in a value of type `ty`.
pub fn region_slots(hir: &hir::Program, ty: Type) -> usize {
    match ty {
        Type::Func(_) => 1,
        Type::Ref { to, .. } => 1 + region_slots(hir, *to),
        Type::Struct { id, .. } => hir.structs[id].lives.len(),
        _ => 0,
    }
}

/// How a region parameter of a struct may vary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Variance {
    Covariant,
    Invariant,
}

/// Per slot of `sid`: covariant unless the borrow field is `&mut` or a `mut`
/// field (a `mut &string cur;` can be pointed elsewhere through a shared path).
pub fn struct_variance(hir: &hir::Program, sid: StructId) -> Vec<Variance> {
    let def = &hir.structs[sid];
    def.lives
        .iter()
        .map(|&f| {
            let field = &def.fields[f];
            if field.mutable || matches!(field.ty, Type::Ref { mutable: true, .. }) { Variance::Invariant } else { Variance::Covariant }
        })
        .collect()
}

/// The slots of the value at `place`, as a range into its local's slot vector.
pub fn slots_of_place(hir: &hir::Program, body: &Body, place: &Place) -> Range<usize> {
    slots_of_projection(hir, body.locals[place.local].ty, &place.proj)
}

pub fn slots_of_projection(hir: &hir::Program, local_ty: Type, proj: &[Proj]) -> Range<usize> {
    let mut ty = local_ty;
    let mut r = 0..region_slots(hir, ty);
    for pr in proj {
        match *pr {
            Proj::Deref => match ty {
                Type::Ref { to, .. } => {
                    r = (r.start + 1)..r.end;
                    ty = *to;
                }
                // Raw pointers are not tracked.
                _ => {
                    r = r.end..r.end;
                    ty = ty.target().unwrap_or(ty);
                }
            },
            // Enums have no region slots, and neither does anything inside one.
            Proj::Downcast(_) => return r.end..r.end,
            // Lists and arrays cannot hold borrows (E0901), so nothing inside has slots.
            Proj::Index(_) => return r.end..r.end,
            Proj::Field(f) => {
                let Type::Struct { id, .. } = ty else { return r.end..r.end };
                let def = &hir.structs[id];
                r = match def.lives.iter().position(|&x| x == f) {
                    Some(k) => (r.start + k)..(r.start + k + 1),
                    None => r.start..r.start,
                };
                ty = def.fields[f].ty;
            }
        }
    }
    r
}

/// The variance of slot `k` of a value of type `ty`: slots under a `&mut` are
/// invariant, struct parameters have their struct's variance.
pub fn slot_variance(hir: &hir::Program, ty: Type, k: usize) -> Variance {
    fn walk(hir: &hir::Program, ty: Type, base: usize, k: usize, under_mut: bool) -> Variance {
        match ty {
            Type::Ref { mutable, to } => {
                if k == base {
                    // The borrow's own region: covariant in its place.
                    return if under_mut { Variance::Invariant } else { Variance::Covariant };
                }
                walk(hir, *to, base + 1, k, under_mut || mutable)
            }
            Type::Struct { id, .. } => {
                let v = struct_variance(hir, id);
                match v.get(k.wrapping_sub(base)) {
                    Some(_) if under_mut => Variance::Invariant,
                    Some(&v) => v,
                    None => Variance::Covariant,
                }
            }
            _ => Variance::Covariant,
        }
    }
    walk(hir, ty, 0, k, false)
}

/// A universal region: a region slot of a parameter or of the return value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Universal {
    /// The parameter (counting from 0, `self` first), or `None` for a return slot.
    pub param: Option<usize>,
    /// The slot within that parameter's type, or within the return type.
    pub slot: usize,
    /// For messages: `a`, `p.src`, or `the result`.
    pub name: String,
}

/// The number of slots of parameter `i` of `f`. A method's receiver is a
/// borrow of its struct, so it has one slot more than the struct's own.
pub fn param_slots(hir: &hir::Program, f: FuncId, i: usize) -> usize {
    let func = &hir.funcs[f];
    let ty = func.locals[i].ty;
    if i == 0 && func.has_self { 1 + region_slots(hir, ty) } else { region_slots(hir, ty) }
}

/// The type a parameter has inside the callee's body: the receiver is a borrow.
pub fn param_type(hir: &hir::Program, f: FuncId, i: usize, shared: bool) -> Type {
    let func = &hir.funcs[f];
    let ty = func.locals[i].ty;
    if i == 0 && func.has_self { Type::borrow(!shared, ty) } else { ty }
}

fn slot_name(hir: &hir::Program, base: &str, ty: Type, k: usize) -> String {
    // Find the struct field behind the slot, if any.
    fn walk(hir: &hir::Program, ty: Type, at: usize, k: usize) -> Option<String> {
        match ty {
            Type::Ref { to, .. } => walk(hir, *to, at + 1, k),
            Type::Struct { id, .. } => {
                let def = &hir.structs[id];
                let j = k.checked_sub(at)?;
                def.lives.get(j).map(|&f| def.fields[f].name.clone())
            }
            _ => None,
        }
    }
    match walk(hir, ty, 0, k) {
        Some(field) => format!("{base}.{field}"),
        None => base.to_string(),
    }
}

/// Every universal region of `body`, in order: the slots of each parameter
/// (parameters in order, `self` first), then the slots of the return value.
pub fn universals(hir: &hir::Program, body: &Body) -> Vec<Universal> {
    let mut out = Vec::new();
    for i in 0..body.arg_count {
        let l = Local(i as u32 + 1);
        let ty = body.locals[l].ty;
        let name = match &body.locals[l].kind {
            crate::mir::LocalKind::Param { name, .. } => name.clone(),
            _ => format!("argument {}", i + 1),
        };
        for k in 0..region_slots(hir, ty) {
            out.push(Universal { param: Some(i), slot: k, name: slot_name(hir, &name, ty, k) });
        }
    }
    let ret = body.locals[Local(0)].ty;
    for k in 0..region_slots(hir, ret) {
        out.push(Universal { param: None, slot: k, name: slot_name(hir, "the result", ret, k) });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn front(src: &str) -> hir::Program {
        let (hir, diags) = crate::front(src);
        hir.unwrap_or_else(|| panic!("does not check: {diags:?}"))
    }

    fn ty_of(hir: &hir::Program, name: &str) -> Type {
        let id = hir.structs.iter().position(|s| s.name == name).unwrap();
        Type::Struct { id, name: crate::types::intern(name) }
    }

    const SRC: &str = "
struct Parser lives src { public: &string src; uint32 pos; }
struct Cur lives c { public: mut &string c; }
struct Pair lives a, b { public: &string a; &mut string b; }
struct Plain { public: string s; }
none main() { }
";

    #[test]
    fn slots_are_counted() {
        let hir = front(SRC);
        let s = Type::Str;
        assert_eq!(region_slots(&hir, Type::U32), 0);
        assert_eq!(region_slots(&hir, s), 0);
        assert_eq!(region_slots(&hir, Type::borrow(false, s)), 1);
        assert_eq!(region_slots(&hir, Type::borrow(true, Type::U32)), 1);
        assert_eq!(region_slots(&hir, ty_of(&hir, "Parser")), 1);
        assert_eq!(region_slots(&hir, ty_of(&hir, "Pair")), 2);
        assert_eq!(region_slots(&hir, ty_of(&hir, "Plain")), 0);
        assert_eq!(region_slots(&hir, Type::borrow(false, ty_of(&hir, "Pair"))), 3);
        assert_eq!(region_slots(&hir, Type::ptr(Type::U32)), 0);
    }

    #[test]
    fn projections_pick_sub_ranges() {
        let hir = front(SRC);
        let pair = ty_of(&hir, "Pair");
        let r = Type::borrow(true, pair);
        assert_eq!(slots_of_projection(&hir, r, &[]), 0..3);
        assert_eq!(slots_of_projection(&hir, r, &[Proj::Deref]), 1..3);
        assert_eq!(slots_of_projection(&hir, r, &[Proj::Deref, Proj::Field(0)]), 1..2);
        assert_eq!(slots_of_projection(&hir, r, &[Proj::Deref, Proj::Field(1)]), 2..3);
        let parser = ty_of(&hir, "Parser");
        // `pos` has no slots.
        assert_eq!(slots_of_projection(&hir, parser, &[Proj::Field(1)]), 0..0);
    }

    #[test]
    fn variance_follows_the_field() {
        let hir = front(SRC);
        let id = |n: &str| ty_of(&hir, n).struct_id().unwrap();
        assert_eq!(struct_variance(&hir, id("Parser")), [Variance::Covariant]);
        assert_eq!(struct_variance(&hir, id("Cur")), [Variance::Invariant]);
        assert_eq!(struct_variance(&hir, id("Pair")), [Variance::Covariant, Variance::Invariant]);
        // Under a `&mut`, everything inside is invariant.
        let under = Type::borrow(true, ty_of(&hir, "Parser"));
        assert_eq!(slot_variance(&hir, under, 0), Variance::Covariant);
        assert_eq!(slot_variance(&hir, under, 1), Variance::Invariant);
        let shared = Type::borrow(false, ty_of(&hir, "Parser"));
        assert_eq!(slot_variance(&hir, shared, 1), Variance::Covariant);
    }

    #[test]
    fn universals_are_named() {
        let hir = front("struct Parser lives src { public: &string src; uint32 pos; }\n&string pick(&string a, Parser p) lives a { return a; }\nnone main() { }\n");
        let mir = crate::mir::build_program(&hir, &mut Vec::new());
        let body = mir.bodies.iter().flatten().find(|b| b.name == "pick").unwrap();
        let u = universals(&hir, body);
        let names: Vec<&str> = u.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["a", "p.src", "the result"]);
        assert_eq!(u[1].param, Some(1));
        assert_eq!(u[2].param, None);
    }
}
