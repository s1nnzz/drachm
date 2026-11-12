//! Enums: declaring them, laying them out, building values of them, and
//! `match` on them (patterns, bindings and exhaustiveness).
//!
//! An enum is a tag followed by the fields of the variant the tag names
//! (`hir::enum_layout`). The tag of a variant is its index in the
//! declaration. Enums are plain values like structs: Copy when every payload
//! is and the whole is at most 32 bytes, otherwise moved.

use super::{Checker, FieldInfo, Globals, Origin, Val};
use crate::ast::{self, ExprKind};
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

impl Checker<'_> {
    /// The text `Shape::Rect(float64 w, float64 h)` of a variant, for messages.
    fn variant_shape(&self, en: EnumId, vi: usize) -> String {
        let e = &self.g.enums[en];
        let v = &e.variants[vi];
        if v.fields.is_empty() {
            return v.name.clone();
        }
        let fields: Vec<String> = v.fields.iter().map(|f| format!("{} {}", f.ty.map_or("?".into(), |t| t.to_string()), f.name)).collect();
        format!("{}({})", v.name, fields.join(", "))
    }

    /// The variant `name` of enum `en`, or E0521 listing the variants.
    pub(super) fn find_variant(&mut self, en: EnumId, name: &str, at: Span) -> Option<usize> {
        let e = &self.g.enums[en];
        if let Some(i) = e.variant(name) {
            return Some(i);
        }
        let names: Vec<&str> = e.variants.iter().map(|v| v.name.as_str()).collect();
        let mut d = Diagnostic::at(&codes::E0521, at, format!("`{}` has no variant `{name}`", e.name)).note("no such variant");
        d = match super::similar(name, &names) {
            Some(near) => d.help(format!("did you mean `{near}`? its variants are {}", e.variant_list())),
            None => d.help(format!("its variants are {}", e.variant_list())),
        };
        self.diags.push(d);
        None
    }

    /// A value of variant `vi` of enum `en`, built from `args` (one for each
    /// value the variant declares). `parens` says the call was written with `(`.
    pub(super) fn variant_expr(&mut self, en: EnumId, vi: usize, args: &[ast::Expr], parens: bool, span: Span, name_span: Span) -> Option<hir::Expr> {
        let fields: Vec<Option<Type>> = self.g.enums[en].variants[vi].fields.iter().map(|f| f.ty).collect();
        let vname = self.g.enums[en].variants[vi].name.clone();
        if fields.len() != args.len() {
            let n = fields.len();
            let given = if args.is_empty() && !parens { "none is given".to_string() } else { format!("{} {} given", args.len(), if args.len() == 1 { "is" } else { "are" }) };
            let takes = if n == 0 { format!("`{vname}` takes no values") } else { format!("`{vname}` takes {n} value{}", if n == 1 { "" } else { "s" }) };
            let decl = self.g.enums[en].variants[vi].name_span;
            let mut d = Diagnostic::at(&codes::E0522, span, format!("{takes}, but {given}"))
                .note(if n == 0 { "this variant has no values".to_string() } else { format!("needs {}", self.variant_shape(en, vi).trim_start_matches(&vname).trim_matches(|c| c == '(' || c == ')')) })
                .label(decl, format!("`{vname}` is declared here"));
            d = d.help(if n == 0 { format!("write `{vname}` without values") } else { format!("give a value for each: `{}`", self.variant_shape(en, vi)) });
            self.diags.push(d);
            for a in args {
                self.expr(a);
            }
            let _ = name_span;
            return None;
        }
        let mut out = Vec::new();
        let mut ok = true;
        for (a, fty) in args.iter().zip(&fields) {
            match fty {
                Some(t) => match self.coerce(a, *t) {
                    Some(x) => out.push(x),
                    None => ok = false,
                },
                None => {
                    self.expr(a);
                    ok = false;
                }
            }
        }
        let ty = self.g.enums[en].ty;
        ok.then(|| hir::Expr::new(hir::ExprKind::Variant { enum_id: en, variant: vi, args: out }, ty))
    }

    /// `Enum::Variant` written without parentheses.
    pub(super) fn path_expr(&mut self, owner: &str, owner_span: Span, name: &str, name_span: Span, span: Span) -> Option<Val> {
        if self.lookup(owner).is_none()
            && let Some(&en) = self.g.enums_by_name.get(owner)
        {
            let vi = self.find_variant(en, name, name_span)?;
            return self.variant_expr(en, vi, &[], false, span, name_span).map(Val::Expr);
        }
        let _ = owner_span;
        self.diags.push(
            Diagnostic::at(&codes::E0003, name_span, format!("expected `(` after `{owner}::{name}`"))
                .note("a call needs its parentheses")
                .help(format!("write `{owner}::{name}()`")),
        );
        None
    }

    /// `Enum::Variant(args)` (the `Method` form) when `recv` names an enum.
    pub(super) fn qualified_variant(&mut self, en: EnumId, name: &str, name_span: Span, args: &[ast::Expr], span: Span) -> Option<Val> {
        let Some(vi) = self.find_variant(en, name, name_span) else {
            for a in args {
                self.expr(a);
            }
            return None;
        };
        self.variant_expr(en, vi, args, true, span, name_span).map(Val::Expr)
    }

    /// `e` where a value of type `to` is wanted, if `e` is a bare variant
    /// name (`Circle(1.0)`, `Empty`) of enum `to`: the expected type decides
    /// first, even when a struct or function has the same name. `None` when
    /// `e` is not one; `Some(None)` when it is one but had errors.
    pub(super) fn bare_variant(&mut self, e: &ast::Expr, to: Type) -> Option<Option<hir::Expr>> {
        let en = to.enum_id()?;
        let (name, args, parens): (&String, &[ast::Expr], bool) = match &e.kind {
            ExprKind::Var(n) if self.lookup(n).is_none() => (n, &[], false),
            ExprKind::Call { name, args } => (name, args, true),
            _ => return None,
        };
        let vi = self.g.enums[en].variant(name)?;
        Some(self.variant_expr(en, vi, args, parens, e.span, e.span))
    }

    /// E0523: a bare variant name with nothing to say which enum it is from.
    /// Returns true if `name` is a variant of some enum (and reports it).
    pub(super) fn ambiguous_bare(&mut self, name: &str, span: Span) -> bool {
        let owners: Vec<usize> = (0..self.g.enums.len()).filter(|&i| self.g.enums[i].variant(name).is_some()).collect();
        if owners.is_empty() {
            return false;
        }
        let first = &self.g.enums[owners[0]].name;
        let help = if owners.len() == 1 {
            format!("write `{first}::{name}`, or give the type: `{first} s = {name}...;`")
        } else {
            let names: Vec<String> = owners.iter().map(|&i| format!("`{}::{name}`", self.g.enums[i].name)).collect();
            format!("write the enum: {}", super::enums::join_or(&names))
        };
        self.diags.push(
            Diagnostic::at(&codes::E0523, span, format!("cannot tell which enum `{name}` belongs to"))
                .note(format!("`{name}` needs a type to come from"))
                .help(help),
        );
        true
    }

    /// `a == b` / `a != b` on enums (owner answer Q6): only for enums whose
    /// variants carry no values; compares the tags.
    pub(super) fn enum_compare(&mut self, op: ast::BinOp, lhs: hir::Expr, lo: Origin, rv: Val, ro: Origin) -> Option<Val> {
        let t = lhs.ty;
        let rhs = self.into(rv, t, ro)?;
        if self.g.enums[t.enum_id().unwrap()].variants.iter().any(|v| !v.fields.is_empty()) {
            self.diags.push(
                Diagnostic::at(&codes::E0527, lo.span.to(ro.span), format!("`{t}` has variants with values, so it cannot be compared with `{}`", op.symbol()))
                    .label(lo.span, format!("this is a `{t}`"))
                    .help("take it apart with `match`: `match (value) { case Name { } case _ { } }`"),
            );
            return None;
        }
        let tag = |e: hir::Expr| {
            let ty = crate::mir::ty::tag_type(hir::tag_size_for(self.g.enums[t.enum_id().unwrap()].variants.len()));
            hir::Expr::new(hir::ExprKind::Discriminant(Box::new(e)), ty)
        };
        let hop = if op == ast::BinOp::Eq { hir::BinOp::Eq } else { hir::BinOp::Ne };
        let (a, b) = (tag(lhs), tag(rhs));
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Binary(hop, Box::new(a), Box::new(b)), Type::Bool)))
    }

    /// An enum where text is wanted.
    pub(super) fn enum_not_text(&mut self, ty: Type, span: Span) {
        let what = self.quote(span, "this value");
        self.diags.push(
            Diagnostic::at(&codes::E0101, span, format!("{what} is a `{ty}`, which cannot be turned into text by itself"))
                .note(format!("this is `{ty}`"))
                .help("take it apart with `match` and print what each variant holds"),
        );
    }
}

/// `a`, `a or b`, `a, b or c`.
pub(super) fn join_or(items: &[String]) -> String {
    match items.split_last() {
        None => String::new(),
        Some((only, [])) => only.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
    }
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
