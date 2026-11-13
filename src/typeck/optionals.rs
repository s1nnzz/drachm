//! Optionals (`T?`, S5A.2). An optional is an enum with two variants: `none`
//! (tag 0, no value) and `Some` (tag 1, one `T`). The checker makes one for
//! each distinct `T` the file writes (`declare_optionals`), so layout, copy
//! and drop glue, `match` and the backends all come from the enum code.
//!
//! A `T` goes to a `T?` by itself wherever a `T?` is wanted (`Checker::into`),
//! and `none` is the variant `none`. `maybe`, `has value` and `is empty` read
//! the tag.

use super::{Checker, FieldInfo, Globals, Origin, Val};
use super::enums::VariantInfo;
use crate::ast;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, EnumId};
use crate::types::{self, Type};

/// The tag of `none`.
pub(super) const NONE: usize = 0;
/// The tag of `Some`.
pub(super) const SOME: usize = 1;

/// The `T` of an optional type `T?`, if `ty` is one.
pub(super) fn inner_of(g: &Globals, ty: Type) -> Option<Type> {
    match g.enums[ty.enum_id()?].kind {
        hir::EnumKind::Optional(inner) => Some(inner),
        _ => None,
    }
}

pub(super) fn is_optional(g: &Globals, ty: Type) -> bool {
    inner_of(g, ty).is_some()
}

/// Make the optional types the file needs: one for each distinct `T` written
/// as `T?`. Runs after the enum names exist and before any field or payload
/// is resolved. A `T` that does not resolve is reported where it is used.
pub(super) fn declare_optionals(g: &mut Globals, written: &[ast::TypeName]) {
    for t in written {
        let Some(inner) = g.resolve_quiet(t) else { continue };
        if inner == Type::None || g.optional_ids.contains_key(&inner) {
            continue;
        }
        make_optional(g, inner, t.span);
    }
}

/// Makes the optional of `inner` (`T?`) and returns its type.
fn make_optional(g: &mut Globals, inner: Type, span: Span) -> Type {
    let id = g.enums.len();
    let name = format!("{inner}?");
    let ty = Type::Enum { id, name: types::intern(&name) };
    let none = VariantInfo { name: "none".to_string(), name_span: span, fields: Vec::new() };
    let some = VariantInfo {
        name: "Some".to_string(),
        name_span: span,
        fields: vec![FieldInfo { name: "value".to_string(), ty: Some(inner), mutable: false, public: true, span }],
    };
    g.enums.push(super::enums::EnumInfo {
        name,
        name_span: span,
        ty,
        variants: vec![none, some],
        kind: hir::EnumKind::Optional(inner),
        owns: false,
    });
    g.optional_ids.insert(inner, id);
    ty
}

/// The optional of `inner`, made now if the file never wrote it: what
/// `pop()` and `get(i)` give for any item type (S6C.2).
pub(super) fn optional_of(g: &mut Globals, inner: Type) -> Option<Type> {
    if inner == Type::None {
        return None;
    }
    match g.optional_ids.get(&inner) {
        Some(&id) => Some(g.enums[id].ty),
        None => Some(make_optional(g, inner, Span::new(0, 0))),
    }
}

/// `ty` with the `?` of `t` applied: the optional of `ty` when `t` has one.
/// `None` when that optional was never made (its error is reported elsewhere).
pub(super) fn wrap(g: &Globals, t: &ast::TypeName, ty: Type) -> Option<Type> {
    if t.optional.is_none() {
        return Some(ty);
    }
    let id: EnumId = *g.optional_ids.get(&ty)?;
    Some(g.enums[id].ty)
}

/// `none` as a value of optional type `opt`.
pub(super) fn none_value(opt: Type, span: Span) -> hir::Expr {
    let enum_id = opt.enum_id().expect("an optional is an enum");
    hir::Expr::new(hir::ExprKind::Variant { enum_id, variant: NONE, args: Vec::new() }, opt).at(span)
}

/// `Some(x)` of optional type `opt`.
pub(super) fn some_value(opt: Type, x: hir::Expr, span: Span) -> hir::Expr {
    let enum_id = opt.enum_id().expect("an optional is an enum");
    hir::Expr::new(hir::ExprKind::Variant { enum_id, variant: SOME, args: vec![x] }, opt).at(span)
}

/// A sample value of `ty` for a fix-it (`0`, `false`, `""`).
fn sample(ty: Type) -> &'static str {
    match ty {
        Type::Bool => "false",
        Type::Str => "\"\"",
        _ if ty.is_numeric() => "0",
        _ => "fallback",
    }
}

impl Checker<'_> {
    /// E0531: an optional where its value is wanted (`what` is the place or
    /// operand as written, `to` the type wanted, `None` for an operator).
    pub(super) fn optional_as_value(&mut self, ty: Type, to: Option<Type>, at: Origin) {
        let Some(inner) = inner_of(self.g, ty) else { return };
        let what = self.quote(at.span, "this value");
        let name = self.text(at.span).to_string();
        let mut help = format!("write `{name} maybe {}` to give a `{inner}` for `none`, or test it with `{name} has value`", sample(to.unwrap_or(inner)));
        // A view of an item wanted as an owned optional: `copy` makes the copy.
        if inner.is_borrow() && let Some(t) = to && (Some(t) == inner.target() || inner_of(self.g, t) == inner.target()) {
            help = format!("`{name}` only lends the item (`{ty}`); write `copy {name}` for an owned `{}?`, or `{name} maybe &fallback` to keep it a view", inner.target().unwrap_or(inner));
        }
        let msg = match to {
            Some(t) => format!("{what} is a `{ty}`, which may be `none`, so it is not a `{t}`"),
            None => format!("{what} is a `{ty}`, which may be `none`, so this operator cannot use it"),
        };
        self.diags.push(
            Diagnostic::at(&codes::E0531, at.span, msg)
                .note(format!("this is a `{ty}`"))
                .help(help),
        );
    }

    /// `value has value` / `value is empty`: the tag test of an optional.
    pub(super) fn optional_test(&mut self, value: &ast::Expr, has: bool) -> Option<Val> {
        let v = self.expr(value)?;
        let Val::Expr(x) = v else {
            self.not_optional(value, None, has);
            return None;
        };
        let Some(en) = optional_enum(self.g, x.ty) else {
            self.not_optional(value, Some(x.ty), has);
            return None;
        };
        let tag = if has { SOME } else { NONE };
        Some(Val::Expr(self.tag_test(en, tag, &x)))
    }

    /// E0532 for `has value` / `is empty` on something that is not optional.
    fn not_optional(&mut self, value: &ast::Expr, ty: Option<Type>, has: bool) {
        let what = self.quote(value.span, "this value");
        let name = self.text(value.span).to_string();
        let word = if has { "has value" } else { "is empty" };
        let mut d = Diagnostic::at(&codes::E0532, value.span, format!("{what} is not optional, so `{word}` cannot test it"));
        d = match ty {
            Some(t) => d.note(format!("this is a `{t}`")),
            None => d.note("this is a number"),
        };
        self.diags.push(d.help(format!("only an optional value (`T?`) can be `{word}`; `{name} {word}` needs one")));
    }

    /// `value maybe fallback`: the payload of an optional, or `fallback` when it is `none`.
    pub(super) fn maybe_expr(&mut self, value: &ast::Expr, fallback: &ast::Expr) -> Option<Val> {
        let saved = self.suffix_ok.replace(value as *const ast::Expr as usize);
        let v = self.expr(value);
        self.suffix_ok = saved;
        let v = v?;
        let v = match v {
            Val::Expr(x) => match self.classify(x) {
                Ok(f) => return self.maybe_failing(f, fallback),
                Err(x) => Val::Expr(x),
            },
            v => v,
        };
        let Val::Expr(x) = v else {
            self.coerce(fallback, Type::U32);
            self.not_optional_maybe(value, None);
            return None;
        };
        if optional_enum(self.g, x.ty).is_none() {
            self.coerce(fallback, x.ty);
            self.not_optional_maybe(value, Some(x.ty));
            return None;
        }
        let inner = inner_of(self.g, x.ty)?;
        let fb = self.coerce(fallback, inner)?;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Maybe { value: Box::new(x), fallback: Box::new(fb) }, inner)))
    }

    /// E0532 for `maybe` on something that is not optional.
    fn not_optional_maybe(&mut self, value: &ast::Expr, ty: Option<Type>) {
        let what = self.quote(value.span, "this value");
        let name = self.text(value.span).to_string();
        let mut d = Diagnostic::at(&codes::E0532, value.span, format!("`maybe` needs an optional, and {what} is not one"));
        d = match ty {
            Some(t) => d.note(format!("this is a `{t}`")),
            None => d.note("this is a number"),
        };
        self.diags.push(d.help(format!("`{name}` already has a value; `maybe` is for a `T?` that may be `none`")));
    }

    /// E0530: `none` where no optional type is expected.
    pub(super) fn none_without_type(&mut self, span: Span) {
        self.diags.push(
            Diagnostic::at(&codes::E0530, span, "`none` needs an optional type to be empty in")
                .note("no optional type is expected here")
                .help("give the place an optional type, such as `uint32? x = none;`"),
        );
    }

    /// `x == none` / `x != none` on an optional (E0533).
    pub(super) fn optional_vs_none(&mut self, op: ast::BinOp, value: &ast::Expr, span: Span) {
        let name = self.text(value.span).to_string();
        let fix = if op == ast::BinOp::Eq { format!("{name} is empty") } else { format!("{name} has value") };
        self.diags.push(
            Diagnostic::at(&codes::E0533, value.span.to(span), format!("an optional is not compared with `none`"))
                .note("`none` is the empty value, and `==` would compare the whole value")
                .help(format!("write `{fix}`")),
        );
    }
}

/// The enum id of `ty` when it is an optional.
fn optional_enum(g: &Globals, ty: Type) -> Option<EnumId> {
    let en = ty.enum_id()?;
    is_optional(g, ty).then_some(en)
}
