//! Layout, ownership and copy answers as memoised queries on `Globals`.
//! Each is computed when first asked, so a type made while bodies are being
//! checked (an optional, later an instance) can be asked about at once. A
//! query that meets itself again is in a cycle (reported as E0210 by
//! `report_cycles`) and answers as an empty, owning-nothing, non-Copy type.

use std::collections::{HashMap, HashSet};

use super::{FieldInfo, Globals};
use crate::hir::{self, EnumId, StructId};
use crate::types::Type;

#[derive(Default)]
pub(super) struct Cache {
    cyclic_structs: HashSet<StructId>,
    cyclic_enums: HashSet<EnumId>,
    owns: HashMap<Type, bool>,
    size: HashMap<Type, (u32, u32)>,
    copy: HashMap<Type, bool>,
    /// The list and array types made so far, elements first (`hir::Program::collections`).
    collections: Vec<Type>,
}

impl Globals {
    /// Forgets the memoised answers (after types were removed again).
    pub(super) fn reset_layout_answers(&self) {
        let mut c = self.layouts.borrow_mut();
        c.owns.clear();
        c.size.clear();
        c.copy.clear();
    }

    /// Records which types `report_cycles` found in a cycle: the structs by id,
    /// then the enums (`structs.len() + id`).
    pub(super) fn mark_cyclic(&self, cyclic: &[bool]) {
        let ns = self.structs.len();
        let mut c = self.layouts.borrow_mut();
        for (i, &is) in cyclic.iter().enumerate() {
            if is {
                if i < ns {
                    c.cyclic_structs.insert(i);
                } else {
                    c.cyclic_enums.insert(i - ns);
                }
            }
        }
    }

    /// Was node `i` (a struct id, then `structs.len() + enum id`) found in a cycle?
    pub(super) fn was_cyclic(&self, i: usize) -> bool {
        let ns = self.structs.len();
        let c = self.layouts.borrow();
        if i < ns { c.cyclic_structs.contains(&i) } else { c.cyclic_enums.contains(&(i - ns)) }
    }

    fn is_cyclic(&self, t: Type) -> bool {
        let c = self.layouts.borrow();
        match t {
            Type::Struct { id, .. } => c.cyclic_structs.contains(&id),
            Type::Enum { id, .. } => c.cyclic_enums.contains(&id),
            _ => false,
        }
    }

    /// The payload and field types of a struct or enum.
    fn parts(&self, t: Type) -> Vec<&FieldInfo> {
        match t {
            Type::Struct { id, .. } => self.structs[id].fields.iter().collect(),
            Type::Enum { id, .. } => self.enums[id].variants.iter().flat_map(|v| &v.fields).collect(),
            _ => Vec::new(),
        }
    }

    /// Does a value of type `t` own memory: is it a `string`, or does it hold
    /// one somewhere inside? Types in a cycle own nothing.
    pub(super) fn owns_of(&self, t: Type) -> bool {
        match t {
            Type::Str | Type::List { .. } => return true,
            Type::Array { elem, .. } => return self.owns_of(*elem),
            Type::Struct { .. } | Type::Enum { .. } => {}
            _ => return false,
        }
        if self.is_cyclic(t) {
            return false;
        }
        if let Some(&o) = self.layouts.borrow().owns.get(&t) {
            return o;
        }
        self.layouts.borrow_mut().owns.insert(t, false);
        let forced = matches!(t, Type::Enum { id, .. } if self.enums[id].owns);
        let owns = forced || self.parts(t).iter().any(|f| f.ty.is_some_and(|ft| self.owns_of(ft)));
        self.layouts.borrow_mut().owns.insert(t, owns);
        owns
    }

    /// Size and alignment of a field or payload of type `t` (`None`: it did
    /// not resolve, which was reported).
    pub(super) fn size_align(&self, t: Option<Type>) -> (u32, u32) {
        let Some(t) = t else { return (0, 1) };
        if t == Type::Str || t.is_list() {
            return (hir::STR_SIZE, 8);
        }
        if t.is_fat() {
            return (16, 8);
        }
        if let Type::Array { elem, len } = t {
            let (size, align) = self.size_align(Some(*elem));
            return ((size as u64).saturating_mul(len).min(u32::MAX as u64 / 2) as u32, align);
        }
        if let Some(n) = hir::scalar_size(t) {
            return (n, n);
        }
        if !matches!(t, Type::Struct { .. } | Type::Enum { .. }) || self.is_cyclic(t) {
            return (0, 1);
        }
        if let Some(&m) = self.layouts.borrow().size.get(&t) {
            return m;
        }
        // Met again while being sized: in a cycle.
        self.layouts.borrow_mut().size.insert(t, (0, 1));
        let m = match t {
            Type::Enum { id, .. } => {
                let sizes = self.enum_sizes(id);
                let (_, size, align) = hir::enum_layout(hir::tag_size_for(sizes.len()), &sizes);
                (size, align)
            }
            Type::Struct { id, .. } => {
                let sizes: Vec<(u32, u32)> = self.structs[id].fields.iter().map(|f| self.size_align(f.ty)).collect();
                let (_, size, align) = hir::layout(&sizes);
                (size, align)
            }
            _ => unreachable!(),
        };
        self.layouts.borrow_mut().size.insert(t, m);
        m
    }

    fn enum_sizes(&self, id: EnumId) -> Vec<Vec<(u32, u32)>> {
        self.enums[id].variants.iter().map(|v| v.fields.iter().map(|f| self.size_align(f.ty)).collect()).collect()
    }

    /// Is a field or element of type `ty` plain data that copies by bits?
    pub(super) fn plain_copy(&self, ty: Option<Type>) -> bool {
        match ty.map(Type::bare) {
            Some(Type::Int { .. } | Type::Float(_) | Type::Bool | Type::Ref { mutable: false, .. }) => true,
            Some(ft @ (Type::Struct { .. } | Type::Enum { .. } | Type::Array { .. })) => self.copy_of(ft),
            _ => false,
        }
    }

    /// Remember a list or array type for the backends (elements are noted first by their own makers).
    pub(super) fn note_collection(&self, t: Type) {
        let mut c = self.layouts.borrow_mut();
        if !c.collections.contains(&t) {
            c.collections.push(t);
        }
    }

    pub(super) fn collections(&self) -> Vec<Type> {
        self.layouts.borrow().collections.clone()
    }

    /// Does a struct or enum copy automatically (R11)? A struct is not
    /// `stays`, has no `drop()`, is at most 32 bytes and every field is Copy;
    /// an enum is not a failure and has only Copy payloads.
    pub(super) fn copy_of(&self, t: Type) -> bool {
        match t {
            Type::Struct { .. } | Type::Enum { .. } => {}
            Type::Array { elem, .. } => return self.plain_copy(Some(*elem)) && self.size_align(Some(t)).0 <= 32,
            _ => return false,
        }
        if self.is_cyclic(t) {
            return false;
        }
        if let Some(&c) = self.layouts.borrow().copy.get(&t) {
            return c;
        }
        self.layouts.borrow_mut().copy.insert(t, false);
        let own_ok = match t {
            Type::Struct { id, .. } => !self.structs[id].stays && self.structs[id].has_drop.is_none(),
            Type::Enum { id, .. } => self.enums[id].kind != hir::EnumKind::Failure,
            _ => false,
        };
        let copy = own_ok && self.size_align(Some(t)).0 <= 32 && self.parts(t).into_iter().all(|f| self.plain_copy(f.ty));
        self.layouts.borrow_mut().copy.insert(t, copy);
        copy
    }
}

/// The `hir` form of every struct and enum, from the queries.
pub(super) fn lay_out(g: &Globals) -> (Vec<hir::StructDef>, Vec<hir::EnumDef>) {
    let structs = g
        .structs
        .iter()
        .map(|s| {
            let sizes: Vec<(u32, u32)> = if g.is_cyclic(s.ty) { Vec::new() } else { s.fields.iter().map(|f| g.size_align(f.ty)).collect() };
            let (offsets, size, align) = hir::layout(&sizes);
            let fields = s
                .fields
                .iter()
                .zip(offsets.into_iter().chain(std::iter::repeat(0)))
                .map(|(f, offset)| hir::FieldDef { name: f.name.clone(), ty: f.ty.unwrap_or(Type::None), offset, mutable: f.mutable })
                .collect();
            hir::StructDef {
                name: s.name.clone(),
                fields,
                size,
                align,
                owns: g.owns_of(s.ty),
                copy: g.copy_of(s.ty),
                stays: s.stays,
                has_drop: s.has_drop,
                lives: s.lives.clone(),
            }
        })
        .collect();
    let enums = g
        .enums
        .iter()
        .map(|e| {
            let sizes: Vec<Vec<(u32, u32)>> =
                if g.is_cyclic(e.ty) { e.variants.iter().map(|_| Vec::new()).collect() } else { e.variants.iter().map(|v| v.fields.iter().map(|f| g.size_align(f.ty)).collect()).collect() };
            let tag_size = hir::tag_size_for(e.variants.len());
            let (offsets, size, align) = hir::enum_layout(tag_size, &sizes);
            let variants = e
                .variants
                .iter()
                .zip(offsets)
                .map(|(v, offs)| hir::VariantDef {
                    name: v.name.clone(),
                    name_span: v.name_span,
                    fields: v
                        .fields
                        .iter()
                        .zip(offs.into_iter().chain(std::iter::repeat(0)))
                        .map(|(f, offset)| hir::FieldDef { name: f.name.clone(), ty: f.ty.unwrap_or(Type::None), offset, mutable: false })
                        .collect(),
                })
                .collect();
            hir::EnumDef { name: e.name.clone(), variants, kind: e.kind, tag_size, size, align, owns: g.owns_of(e.ty), copy: g.copy_of(e.ty), name_span: e.name_span }
        })
        .collect();
    (structs, enums)
}
