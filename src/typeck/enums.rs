//! Enums: declaring them, laying them out, building values of them, and
//! `match` on them (patterns, bindings and exhaustiveness).
//!
//! An enum is a tag followed by the fields of the variant the tag names
//! (`hir::enum_layout`). The tag of a variant is its index in the
//! declaration. Enums are plain values like structs: Copy when every payload
//! is and the whole is at most 32 bytes, otherwise moved.

use super::FieldInfo;
use super::Globals;
use crate::ast;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, EnumId};
use crate::types::{self, Type, TypeError};

pub(super) struct EnumInfo {
    pub name: String,
    pub name_span: Span,
    pub ty: Type,
    pub variants: Vec<VariantInfo>,
    pub kind: hir::EnumKind,
    /// Some payload owns memory (see `hir::EnumDef::owns`).
    pub owns: bool,
}

pub(super) struct VariantInfo {
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<FieldInfo>,
}

impl EnumInfo {
    pub fn variant(&self, name: &str) -> Option<usize> {
        self.variants.iter().position(|v| v.name == name)
    }

    /// `` `A`, `B` and `C` ``, for messages.
    pub fn variant_list(&self) -> String {
        let names: Vec<String> = self.variants.iter().map(|v| format!("`{}`", v.name)).collect();
        join_and(&names)
    }
}

/// `a`, `a and b`, `a, b and c`.
pub(super) fn join_and(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((only, [])) => only.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
    }
}

/// Register the name of enum `e`. Its variants wait until every type name exists.
pub(super) fn declare_enum(g: &mut Globals, e: &ast::Enum, diags: &mut Vec<Diagnostic>) -> Option<EnumId> {
    if e.name == "none" || !matches!(Type::from_name(&e.name), Err(TypeError::Unknown)) {
        diags.push(
            Diagnostic::at(&codes::E0202, e.name_span, format!("`{}` is the name of a built-in type", e.name))
                .note("already a type")
                .help("give the enum another name"),
        );
        return None;
    }
    let first = g.structs_by_name.get(&e.name).map(|&s| g.structs[s].name_span).or_else(|| g.enums_by_name.get(&e.name).map(|&o| g.enums[o].name_span));
    if let Some(first) = first {
        let what = if g.structs_by_name.contains_key(&e.name) { "a struct and an enum are both named" } else { "enum is defined twice:" };
        let msg = if what.starts_with("enum") { format!("enum `{}` is defined twice", e.name) } else { format!("`{}` is both a struct and an enum", e.name) };
        diags.push(Diagnostic::at(&codes::E0202, e.name_span, msg).note("defined again here").label(first, "first defined here"));
        return None;
    }
    let id = g.enums.len();
    let ty = Type::Enum { id, name: types::intern(&e.name) };
    g.enums.push(EnumInfo { name: e.name.clone(), name_span: e.name_span, ty, variants: Vec::new(), kind: hir::EnumKind::User, owns: false });
    g.enums_by_name.insert(e.name.clone(), id);
    Some(id)
}

/// Resolve the variants of enum `id`: duplicate names (E0520), payload types
/// (a borrow or pointer is E0901).
pub(super) fn declare_variants(g: &mut Globals, id: EnumId, e: &ast::Enum, diags: &mut Vec<Diagnostic>) {
    for v in &e.variants {
        if let Some(first) = g.enums[id].variants.iter().find(|o| o.name == v.name) {
            diags.push(
                Diagnostic::at(&codes::E0520, v.name_span, format!("`{}` has two variants named `{}`", e.name, v.name))
                    .note("defined again here")
                    .label(first.name_span, "first defined here")
                    .help("rename one of them"),
            );
            continue;
        }
        let mut fields: Vec<FieldInfo> = Vec::new();
        for f in &v.fields {
            let ty = g.resolve_binding_type(&f.ty, diags);
            if ty.is_some_and(|t| t.is_borrow() || t.is_ptr() || g.holds_borrow(t)) {
                diags.push(
                    Diagnostic::at(&codes::E0901, f.ty.span, "a variant holding a borrow or a pointer is not supported yet")
                        .note(format!("`{}` holds a `{}`", f.name, ty.unwrap()))
                        .help("hold the value itself in the variant"),
                );
                fields.push(FieldInfo { name: f.name.clone(), ty: None, mutable: false, public: true, span: f.name_span });
                continue;
            }
            if let Some(first) = fields.iter().find(|o| o.name == f.name) {
                diags.push(
                    Diagnostic::at(&codes::E0202, f.name_span, format!("`{}` has two values named `{}`", v.name, f.name))
                        .note("defined again here")
                        .label(first.span, "first defined here"),
                );
                continue;
            }
            fields.push(FieldInfo { name: f.name.clone(), ty, mutable: false, public: true, span: f.name_span });
        }
        g.enums[id].variants.push(VariantInfo { name: v.name.clone(), name_span: v.name_span, fields });
    }
}

/// The `hir` form of the declared enums, given each payload type's size and
/// alignment (`size_of`) and whether it copies.
pub(super) fn lay_out_enums(
    g: &Globals,
    cyclic: &[bool],
    size_of: &mut dyn FnMut(Option<Type>) -> (u32, u32),
) -> Vec<hir::EnumDef> {
    let base = g.structs.len();
    g.enums
        .iter()
        .enumerate()
        .map(|(id, e)| {
            let sizes: Vec<Vec<(u32, u32)>> = if cyclic[base + id] {
                e.variants.iter().map(|_| Vec::new()).collect()
            } else {
                e.variants.iter().map(|v| v.fields.iter().map(|f| size_of(f.ty)).collect()).collect()
            };
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
            hir::EnumDef {
                name: e.name.clone(),
                variants,
                kind: e.kind,
                tag_size,
                size,
                align,
                owns: e.owns,
                copy: false,
                name_span: e.name_span,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use crate::mir::{is_copy, needs_drop};

    fn checked(src: &str) -> crate::hir::Program {
        let (hir, diags) = crate::front(src);
        hir.unwrap_or_else(|| panic!("does not check: {diags:?}"))
    }

    fn errors(src: &str) -> Vec<&'static str> {
        let (_, diags) = crate::front(src);
        diags.iter().map(|d| d.code.id).collect()
    }

    #[test]
    fn layout_of_a_shape() {
        let hir = checked("enum Shape { Circle(float64 r), Rect(float64 w, float64 h), Empty, }\nnone main() { }\n");
        let e = &hir.enums[0];
        assert_eq!((e.tag_size, e.size, e.align), (1, 24, 8));
        assert_eq!(e.variants[0].fields[0].offset, 8);
        assert_eq!(e.variants[1].fields[1].offset, 16);
        assert!(e.copy && !e.owns);
    }

    #[test]
    fn small_payloads_pack_after_the_tag() {
        let hir = checked("enum Pair { Both(uint8 a, uint16 b), One(uint8 a) }\nnone main() { }\n");
        let e = &hir.enums[0];
        // Payload aligned to 2: tag at 0, `a` at 2, `b` at 4.
        assert_eq!((e.variants[0].fields[0].offset, e.variants[0].fields[1].offset), (2, 4));
        assert_eq!((e.size, e.align), (6, 2));
        let wide = checked("enum W { Big(uint128 x), Small }\nnone main() { }\n");
        assert_eq!((wide.enums[0].size, wide.enums[0].align), (32, 16));
    }

    #[test]
    fn owning_enums_move_and_copy_ones_copy() {
        let hir = checked(
            "enum Msg { Text(string s), Quit }\nenum Num { A(uint32 n), B }\nstruct Holder { public: Msg m; }\nenum Big { X(uint64 a, uint64 b, uint64 c, uint64 d), Y }\nnone main() { }\n",
        );
        let t = |i| hir.enum_type(i);
        assert!(hir.enums[0].owns && !is_copy(&hir, t(0)) && needs_drop(&hir, t(0)));
        assert!(is_copy(&hir, t(1)) && !needs_drop(&hir, t(1)));
        assert!(hir.structs[0].owns, "a struct holding an owning enum owns");
        assert!(!is_copy(&hir, t(2)), "40 bytes is over the limit");
    }

    #[test]
    fn declaration_errors() {
        assert_eq!(errors("enum E { A, A }\nnone main() { }\n"), ["E0520"]);
        assert_eq!(errors("enum Tree { Leaf, Node(Tree left) }\nnone main() { }\n"), ["E0210"]);
        assert_eq!(errors("struct S { public: uint32 a; }\nenum S { A }\nnone main() { }\n"), ["E0202"]);
        assert!(errors("struct Circle { public: uint32 a; }\nenum Shape { Circle(uint32 r) }\nnone main() { }\n").is_empty());
        assert_eq!(errors("enum E { A(&uint32 r) }\nnone main() { }\n"), ["E0901"]);
    }
}
