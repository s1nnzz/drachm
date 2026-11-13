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
    slots_deep(hir, ty, 0)
}

/// A type that contains itself (a generic struct holding a list of itself) is
/// cut off here; the checker refuses such an instance before it gets so far.
const MAX_DEPTH: usize = 12;

fn slots_deep(hir: &hir::Program, ty: Type, depth: usize) -> usize {
    if depth > MAX_DEPTH {
        return 0;
    }
    match ty {
        Type::Func(_) => 1,
        Type::Ref { to, .. } => 1 + slots_deep(hir, *to, depth + 1),
        Type::Struct { id, .. } => {
            let def = &hir.structs[id];
            def.lives.iter().map(|&f| slots_deep(hir, def.fields[f].ty, depth + 1)).sum()
        }
        Type::List { elem } | Type::Array { elem, .. } => slots_deep(hir, *elem, depth + 1),
        Type::Map { key, val, .. } => slots_deep(hir, *key, depth + 1) + slots_deep(hir, *val, depth + 1),
        Type::Enum { id, .. } => hir.enums[id].variants.iter().flat_map(|v| &v.fields).map(|f| slots_deep(hir, f.ty, depth + 1)).sum(),
        _ => 0,
    }
}

/// The slots of field `f` of struct `sid`, as a range into the struct's slots.
pub fn struct_field_slots(hir: &hir::Program, sid: StructId, f: usize) -> Range<usize> {
    let def = &hir.structs[sid];
    let mut at = 0;
    for &l in &def.lives {
        let n = slots_deep(hir, def.fields[l].ty, 1);
        if l == f {
            return at..at + n;
        }
        at += n;
    }
    at..at
}

/// The slots of field `f` of variant `v` of enum `eid`, as a range into the enum's slots.
pub fn enum_field_slots(hir: &hir::Program, eid: usize, v: usize, f: usize) -> Range<usize> {
    let def = &hir.enums[eid];
    let mut at = 0;
    for (vi, var) in def.variants.iter().enumerate() {
        for (fi, field) in var.fields.iter().enumerate() {
            let n = slots_deep(hir, field.ty, 1);
            if (vi, fi) == (v, f) {
                return at..at + n;
            }
            at += n;
        }
    }
    at..at
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
    let mut out = Vec::new();
    for &f in &def.lives {
        let field = &def.fields[f];
        let n = slots_deep(hir, field.ty, 1);
        for k in 0..n {
            let v = if field.mutable || matches!(field.ty, Type::Ref { mutable: true, .. }) { Variance::Invariant } else { slot_variance(hir, field.ty, k) };
            out.push(v);
        }
    }
    out
}

/// The slots of the value at `place`, as a range into its local's slot vector.
pub fn slots_of_place(hir: &hir::Program, body: &Body, place: &Place) -> Range<usize> {
    slots_of_projection(hir, body.locals[place.local].ty, &place.proj)
}

pub fn slots_of_projection(hir: &hir::Program, local_ty: Type, proj: &[Proj]) -> Range<usize> {
    let mut ty = local_ty;
    let mut r = 0..region_slots(hir, ty);
    let mut variant = None;
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
            Proj::Downcast(v) => variant = Some(v),
            // Every item of a list or array shares the item type's slots.
            Proj::Index(_) => {
                let Some(elem) = ty.elem() else { return r.end..r.end };
                ty = elem;
            }
            Proj::Field(f) => match ty {
                Type::Struct { id, .. } => {
                    let sub = struct_field_slots(hir, id, f);
                    r = (r.start + sub.start)..(r.start + sub.end);
                    ty = hir.structs[id].fields[f].ty;
                }
                Type::Enum { id, .. } => {
                    let Some(v) = variant.take() else { return r.end..r.end };
                    let sub = enum_field_slots(hir, id, v, f);
                    r = (r.start + sub.start)..(r.start + sub.end);
                    ty = hir.enums[id].variants[v].fields[f].ty;
                }
                _ => return r.end..r.end,
            },
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
            Type::List { elem } | Type::Array { elem, .. } => walk(hir, *elem, base, k, under_mut),
            Type::Map { key, val, .. } => {
                let nk = region_slots(hir, *key);
                if k < base + nk { walk(hir, *key, base, k, under_mut) } else { walk(hir, *val, base + nk, k, under_mut) }
            }
            Type::Enum { id, .. } => {
                let mut at = base;
                for var in &hir.enums[id].variants {
                    for f in &var.fields {
                        let n = slots_deep(hir, f.ty, 1);
                        if (at..at + n).contains(&k) {
                            return walk(hir, f.ty, at, k, under_mut);
                        }
                        at += n;
                    }
                }
                Variance::Covariant
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
    /// The group of slots of this function that are one region (they come from
    /// the same type parameter of an instance, `hir::Program::ties`).
    pub tie: Option<usize>,
}

/// The tie group of a slot of function `f`, if it has one.
pub fn tie_class(hir: &hir::Program, f: FuncId, param: Option<usize>, slot: usize) -> Option<usize> {
    hir.ties.get(&f)?.iter().position(|group| group.contains(&(param, slot)))
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
                def.lives.iter().find(|&&f| struct_field_slots(hir, id, f).contains(&j)).map(|&f| def.fields[f].name.clone())
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
            out.push(Universal { param: Some(i), slot: k, name: slot_name(hir, &name, ty, k), tie: tie_class(hir, body.func, Some(i), k) });
        }
    }
    let ret = body.locals[Local(0)].ty;
    for k in 0..region_slots(hir, ret) {
        out.push(Universal { param: None, slot: k, name: slot_name(hir, "the result", ret, k), tie: tie_class(hir, body.func, None, k) });
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
        assert!(slots_of_projection(&hir, parser, &[Proj::Field(1)]).is_empty());
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
