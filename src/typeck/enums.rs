//! Enums: declaring them, laying them out, building values of them, and
//! `match` on them (patterns, bindings and exhaustiveness).
//!
//! An enum is a tag followed by the fields of the variant the tag names
//! (`hir::enum_layout`). The tag of a variant is its index in the
//! declaration. Enums are plain values like structs: Copy when every payload
//! is and the whole is at most 32 bytes, otherwise moved.

use super::control::TextPat;
use super::{Checker, FieldInfo, Globals, Origin, Val};
use crate::ast::{self, ExprKind};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, EnumId, StrFn};
use crate::types::{self, Type, TypeError};

pub(super) struct EnumInfo {
    pub name: String,
    pub name_span: Span,
    pub ty: Type,
    pub variants: Vec<VariantInfo>,
    pub kind: hir::EnumKind,
    /// Owns memory whatever its payloads are (failures); other enums own
    /// when a payload does (`Globals::owns_of`).
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
    if e.name == "none" || super::maps::is_map_name(&e.name) || !matches!(Type::from_name(&e.name), Err(TypeError::Unknown)) {
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
    if super::generics::type_name_taken(g, &e.name, e.name_span, "enum", diags) {
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
            let ty = g.resolve_storable(&f.ty, diags);
            // In an instance of a generic enum the payload may hold a borrow that came in through a type
            // argument (`Maybe<&string>`); one written as a borrow in the template is still refused.
            let via_argument = !g.tenv.is_empty() && !f.ty.mods.iter().any(|(m, _)| matches!(m, ast::TypeMod::Ref { .. } | ast::TypeMod::Ptr | ast::TypeMod::FixedPtr));
            if ty.is_some_and(|t| t.is_ptr() || ((t.is_borrow() || g.holds_borrow(t)) && !via_argument)) {
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

impl Checker<'_> {
    /// The text `Shape::Rect(float64 w, float64 h)` of a variant, for messages.
    fn variant_shape(&self, en: EnumId, vi: usize) -> String {
        let e = &self.g.enums[en];
        let v = &e.variants[vi];
        let hidden = (e.kind == hir::EnumKind::Failure) as usize;
        if v.fields.len() <= hidden {
            return v.name.clone();
        }
        let fields: Vec<String> = v.fields.iter().skip(hidden).map(|f| format!("{} {}", f.ty.map_or("?".into(), |t| t.to_string()), f.name)).collect();
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
        // A failure's first field is the hidden cause, which the program never writes.
        let hidden = (self.g.enums[en].kind == hir::EnumKind::Failure) as usize;
        let fields: Vec<Option<Type>> = self.g.enums[en].variants[vi].fields.iter().skip(hidden).map(|f| f.ty).collect();
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
        if hidden == 1 {
            out.insert(0, hir::Expr::new(hir::ExprKind::Int(0), Type::U64));
        }
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
            ExprKind::Call { name, args, .. } => (name, args, true),
            _ => return None,
        };
        let vi = self.g.enums[en].variant(name)?;
        Some(self.variant_expr(en, vi, args, parens, e.span, e.span))
    }

    /// E0523: a bare variant name with nothing to say which enum it is from.
    /// Returns true if `name` is a variant of some enum (and reports it).
    pub(super) fn ambiguous_bare(&mut self, name: &str, span: Span) -> bool {
        // An optional's `Some` is not a name anyone writes bare.
        let owners: Vec<usize> = (0..self.g.enums.len())
            .filter(|&i| self.g.enums[i].variant(name).is_some() && self.g.enums[i].kind == hir::EnumKind::User)
            .collect();
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
        Some((last, rest)) if rest.len() == 1 => format!("{} or {last}", rest[0]),
        Some((last, rest)) => format!("{}, or {last}", rest.join(", ")),
    }
}

// ---- match ----------------------------------------------------------------

/// A pattern as far as covering values goes.
#[derive(Clone, Debug)]
pub(super) enum Pat {
    Wild,
    /// A variant and its payload patterns.
    Variant(usize, Vec<Pat>),
    /// A constant (its key in `control::Lit::key`): covers nothing on its own.
    Lit(u128),
}

#[derive(Clone, Copy, PartialEq)]
enum Ctor {
    Variant(usize),
    Lit(u128),
}

/// The key of a text constant (`Pat::Lit`): equal texts get equal keys. A glob
/// and an exact text with the same characters differ. (A FNV-1a hash: a clash
/// could only make one pattern look redundant.)
fn text_key(glob: bool, text: &str) -> u128 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    (h as u128) | ((glob as u128) << 64) | (1u128 << 127)
}

fn wilds(n: usize) -> Vec<Pat> {
    vec![Pat::Wild; n]
}

fn field_types(g: &Globals, ty: Type, vi: usize) -> Vec<Type> {
    let Type::Enum { id, .. } = ty else { return Vec::new() };
    g.enums[id].variants[vi].fields.iter().map(|f| f.ty.unwrap_or(Type::None)).collect()
}

fn specialize(rows: &[Vec<Pat>], ctor: Ctor, arity: usize) -> Vec<Vec<Pat>> {
    let mut out = Vec::new();
    for r in rows {
        match (&r[0], ctor) {
            (Pat::Wild, _) => out.push(wilds(arity).into_iter().chain(r[1..].iter().cloned()).collect()),
            (Pat::Variant(v, args), Ctor::Variant(c)) if *v == c => out.push(args.iter().cloned().chain(r[1..].iter().cloned()).collect()),
            (Pat::Lit(k), Ctor::Lit(c)) if *k == c => out.push(r[1..].to_vec()),
            _ => {}
        }
    }
    out
}

/// Can `row` match some value that no row of `rows` matches? (`tys` are the
/// types of the columns.)
fn useful(g: &Globals, rows: &[Vec<Pat>], tys: &[Type], row: &[Pat]) -> bool {
    let Some(head) = row.first() else { return rows.is_empty() };
    let rest = &row[1..];
    match head {
        Pat::Variant(vi, args) => {
            let ftys = field_types(g, tys[0], *vi);
            let spec = specialize(rows, Ctor::Variant(*vi), ftys.len());
            let ntys: Vec<Type> = ftys.into_iter().chain(tys[1..].iter().copied()).collect();
            let nrow: Vec<Pat> = args.iter().cloned().chain(rest.iter().cloned()).collect();
            useful(g, &spec, &ntys, &nrow)
        }
        Pat::Lit(k) => useful(g, &specialize(rows, Ctor::Lit(*k), 0), &tys[1..], rest),
        Pat::Wild => {
            if let Type::Enum { id, .. } = tys[0] {
                let n = g.enums[id].variants.len();
                let mut present = vec![false; n];
                for r in rows {
                    if let Pat::Variant(v, _) = &r[0] {
                        present[*v] = true;
                    }
                }
                if present.iter().all(|p| *p) {
                    return (0..n).any(|vi| {
                        let ftys = field_types(g, tys[0], vi);
                        let spec = specialize(rows, Ctor::Variant(vi), ftys.len());
                        let ntys: Vec<Type> = ftys.iter().copied().chain(tys[1..].iter().copied()).collect();
                        let nrow: Vec<Pat> = wilds(ftys.len()).into_iter().chain(rest.iter().cloned()).collect();
                        useful(g, &spec, &ntys, &nrow)
                    });
                }
            }
            let default: Vec<Vec<Pat>> = rows.iter().filter(|r| matches!(r[0], Pat::Wild)).map(|r| r[1..].to_vec()).collect();
            useful(g, &default, &tys[1..], rest)
        }
    }
}

/// Where a binding's value comes from.
enum Source {
    /// A Copy payload, read where it is.
    Read(hir::Expr),
    /// `copy Type name`: a deep copy of the payload.
    Copy(hir::Expr),
    /// `take Type name`: moves the payload out.
    Take(hir::Place),
}

struct Binding {
    name: String,
    span: Span,
    ty: Type,
    source: Source,
}

/// One alternative of a `case` (`case A(..), B(..)` has two), compiled.
#[derive(Default)]
struct Alt {
    /// All of these hold when the alternative matches, in order.
    tests: Vec<hir::Expr>,
    binds: Vec<Binding>,
    /// Payloads that own memory and stay in the matched value: they have to
    /// be dropped when another payload is taken out of the same value.
    leftovers: Vec<hir::Place>,
    has_take: bool,
}

/// A `case`, compiled for `chain`.
pub(super) struct PatCase<'e> {
    pub test: Option<hir::Expr>,
    /// The names the case binds; the last says the value is taken out only after the guard.
    pub binds: Vec<(String, hir::LocalId, bool)>,
    /// Before the guard: declarations and the bindings that copy.
    pub pre: hir::Block,
    /// After the guard: the `take` bindings.
    pub post: hir::Block,
    pub guard: Option<&'e ast::Expr>,
}

fn and_all(mut tests: Vec<hir::Expr>) -> Option<hir::Expr> {
    if tests.is_empty() {
        return None;
    }
    let first = tests.remove(0);
    Some(tests.into_iter().fold(first, |a, b| hir::Expr::new(hir::ExprKind::Logic { and: true, lhs: Box::new(a), rhs: Box::new(b) }, Type::Bool)))
}

fn or_all(tests: Vec<hir::Expr>) -> hir::Expr {
    let mut it = tests.into_iter();
    let first = it.next().unwrap();
    it.fold(first, |a, b| hir::Expr::new(hir::ExprKind::Logic { and: false, lhs: Box::new(a), rhs: Box::new(b) }, Type::Bool))
}

/// A place expression as a `hir::Place`, if it is rooted at a local.
fn to_place(e: &hir::Expr, span: Span) -> Option<hir::Place> {
    use hir::ExprKind as K;
    match &e.kind {
        K::Local(l) => Some(hir::Place { local: *l, proj: Vec::new(), span }),
        K::Field(inner, f) => {
            let mut p = to_place(inner, span)?;
            p.proj.push(hir::HirProj::Field(*f));
            Some(p)
        }
        K::Deref(inner) => {
            let mut p = to_place(inner, span)?;
            p.proj.push(hir::HirProj::Deref);
            Some(p)
        }
        K::Payload { value, variant, field } => {
            let mut p = to_place(value, span)?;
            p.proj.push(hir::HirProj::Downcast(*variant));
            p.proj.push(hir::HirProj::Field(*field));
            Some(p)
        }
        _ => None,
    }
}

impl Checker<'_> {
    /// A local for a pattern binding (not in any scope yet).
    fn declare_binding(&mut self, name: &str, ty: Type, span: Span) -> hir::LocalId {
        let id = self.locals.len();
        self.locals.push(super::LocalInfo { name: name.to_string(), ty: Some(ty), mutable: false, is_param: false, is_self: false, open: false, span, written: None });
        id
    }

    pub(super) fn tag_test(&self, en: EnumId, vi: usize, place: &hir::Expr) -> hir::Expr {
        let ty = crate::mir::ty::tag_type(hir::tag_size_for(self.g.enums[en].variants.len()));
        let tag = hir::Expr::new(hir::ExprKind::Discriminant(Box::new(place.clone())), ty);
        let want = hir::Expr::new(hir::ExprKind::Int(vi as u128), ty);
        hir::Expr::new(hir::ExprKind::Binary(hir::BinOp::Eq, Box::new(tag), Box::new(want)), Type::Bool)
    }

    /// A pattern for the enum value at `place`.
    #[allow(clippy::too_many_arguments)]
    fn variant_pat(
        &mut self,
        place: &hir::Expr,
        enum_name: Option<&(String, Span)>,
        name: &str,
        name_span: Span,
        args: &[ast::SubPat],
        parens: bool,
        span: Span,
        alt: &mut Alt,
    ) -> Option<Pat> {
        let ty = place.ty;
        let Some(en) = ty.enum_id() else {
            self.diags.push(
                Diagnostic::at(&codes::E0525, span, format!("this pattern is a variant, but the value is a `{ty}`"))
                    .note(format!("cannot match a `{ty}`"))
                    .help("only an enum is matched with variants"),
            );
            return None;
        };
        let ename = self.g.enums[en].name.clone();
        let wrong_enum = enum_name.filter(|(n, _)| *n != ename);
        let vi = if wrong_enum.is_some() { None } else { self.g.enums[en].variant(name) };
        let Some(vi) = vi else {
            let shown = match enum_name {
                Some((n, _)) => format!("{n}::{name}"),
                None => name.to_string(),
            };
            self.diags.push(
                Diagnostic::at(&codes::E0525, span, format!("`{shown}` is not a variant of `{ename}`"))
                    .note(format!("this `case` cannot match a `{ename}`"))
                    .help(format!("the variants of `{ename}` are {}", self.g.enums[en].variant_list())),
            );
            return None;
        };
        // A failure's first field is the hidden cause, which a pattern never names.
        let skip = usize::from(self.g.enums[en].kind == hir::EnumKind::Failure);
        let fields: Vec<(String, Type)> = self.g.enums[en].variants[vi].fields.iter().skip(skip).map(|f| (f.name.clone(), f.ty.unwrap_or(Type::None))).collect();
        if parens && args.len() != fields.len() {
            self.diags.push(
                Diagnostic::at(&codes::E0526, span, format!("`{name}` holds {} value{}, but this pattern names {}", fields.len(), if fields.len() == 1 { "" } else { "s" }, args.len()))
                    .note("the wrong number of values")
                    .label(self.g.enums[en].variants[vi].name_span, format!("`{name}` is declared here"))
                    .help(format!("write `{}`, or `{name}` alone to ignore its values", self.variant_shape(en, vi))),
            );
            return None;
        }
        let _ = name_span;
        alt.tests.push(self.tag_test(en, vi, place));
        let mut ok = true;
        let mut subs = vec![Pat::Wild; skip];
        for (i, (fname, fty)) in fields.iter().enumerate() {
            let payload = hir::Expr::new(hir::ExprKind::Payload { value: Box::new(place.clone()), variant: vi, field: i + skip }, *fty);
            let sub = args.get(i);
            match self.sub_pat(&payload, *fty, name, sub, alt) {
                Some(p) => subs.push(p),
                None => {
                    ok = false;
                    subs.push(Pat::Wild);
                }
            }
            let _ = fname;
        }
        ok.then_some(Pat::Variant(vi, subs))
    }

    /// The pattern for one payload value (`payload`, of type `fty`).
    fn sub_pat(&mut self, payload: &hir::Expr, fty: Type, vname: &str, sub: Option<&ast::SubPat>, alt: &mut Alt) -> Option<Pat> {
        let place = to_place(payload, payload.span);
        let owning = !self.copies_anyway(fty);
        match sub {
            None | Some(ast::SubPat::Any(_)) => {
                if owning && let Some(p) = place {
                    alt.leftovers.push(p);
                }
                Some(Pat::Wild)
            }
            Some(ast::SubPat::Bind { mode, ty: tname, name, name_span }) => {
                let want = self.g.resolve_binding_type(tname, self.diags)?;
                if want != fty {
                    self.diags.push(
                        Diagnostic::at(&codes::E0526, tname.span, format!("`{vname}` holds `{fty}` here, not `{want}`"))
                            .note(format!("this binding is `{want}`"))
                            .help(format!("write `{fty} {name}`")),
                    );
                    return None;
                }
                if alt.binds.iter().any(|b| b.name == *name) {
                    self.diags.push(Diagnostic::at(&codes::E0529, *name_span, format!("`{name}` is bound twice in one pattern")).note("bound again here"));
                    return None;
                }
                let source = match mode {
                    ast::BindMode::Plain if owning => {
                        self.diags.push(
                            Diagnostic::at(&codes::E0528, *name_span, format!("binding `{name}` would take memory out of the matched value"))
                                .note(format!("this payload is a `{fty}`, which owns memory"))
                                .help(format!("write `take {fty} {name}` to move it out (the matched value is used up), or `copy {fty} {name}` to copy it")),
                        );
                        return None;
                    }
                    ast::BindMode::Plain => Source::Read(payload.clone()),
                    ast::BindMode::Copy => {
                        if owning && let Some(p) = place {
                            alt.leftovers.push(p);
                        }
                        Source::Copy(payload.clone())
                    }
                    ast::BindMode::Take => {
                        let Some(p) = place else { return None };
                        if owning {
                            alt.has_take = true;
                        }
                        Source::Take(p)
                    }
                };
                alt.binds.push(Binding { name: name.clone(), span: *name_span, ty: fty, source });
                Some(Pat::Wild)
            }
            Some(sub @ (ast::SubPat::Value(e) | ast::SubPat::Glob(e))) if fty == Type::Str => {
                // Text: exact text (`"hi"`), or a glob (`globbed "*.png"`). The test
                // reads the payload where it is, so an owning payload stays in the value.
                let glob = matches!(sub, ast::SubPat::Glob(_));
                let pat = self.text_pattern(e, glob)?;
                if owning && let Some(p) = place {
                    alt.leftovers.push(p);
                }
                let (f, text) = match pat {
                    TextPat::Exact(t) => (StrFn::Eq, t),
                    TextPat::Glob(g) => (StrFn::Glob, g),
                };
                let key = text_key(glob, &text);
                let lit = hir::Expr::new(hir::ExprKind::Str(text), Type::Str);
                alt.tests.push(hir::Expr::new(hir::ExprKind::StrCall(f, vec![payload.clone(), lit]), f.ret()));
                Some(Pat::Lit(key))
            }
            Some(ast::SubPat::Glob(e)) => {
                self.diags.push(Diagnostic::at(&codes::E0109, e.span, format!("`globbed` is only for text, and `{vname}` holds `{fty}`")));
                return None;
            }
            Some(ast::SubPat::Value(e)) => {
                if !(fty.is_numeric() || fty == Type::Bool) {
                    self.diags.push(Diagnostic::at(&codes::E0526, e.span, format!("`{vname}` holds `{fty}` here, which a constant cannot match")).note("this is a constant"));
                    return None;
                }
                let lit = self.pattern_value(e, Some(fty))?;
                alt.tests.push(hir::Expr::new(
                    hir::ExprKind::Binary(hir::BinOp::Eq, Box::new(payload.clone()), Box::new(lit.expr(fty))),
                    Type::Bool,
                ));
                Some(Pat::Lit(lit.key(fty)))
            }
            Some(ast::SubPat::Variant(p)) => {
                let ast::Pattern::Variant { enum_name, name, name_span, args, parens, span } = &**p else { unreachable!() };
                self.variant_pat(payload, enum_name.as_ref(), name, *name_span, args, *parens, *span, alt)
            }
        }
    }

    /// One top-level pattern of a `case`, for the enum value `scrut`.
    fn top_pat(&mut self, scrut: &hir::Expr, p: &ast::Pattern, alt: &mut Alt) -> Option<Pat> {
        match p {
            ast::Pattern::Any(_) => Some(Pat::Wild),
            ast::Pattern::Variant { enum_name, name, name_span, args, parens, span } => {
                self.variant_pat(scrut, enum_name.as_ref(), name, *name_span, args, *parens, *span, alt)
            }
            ast::Pattern::Expr(e) => match &e.kind {
                ExprKind::NoneValue if super::optionals::is_optional(self.g, scrut.ty) => {
                    self.variant_pat(scrut, None, "none", e.span, &[], false, e.span, alt)
                }
                ExprKind::Var(n) => self.variant_pat(scrut, None, n, e.span, &[], false, e.span, alt),
                ExprKind::Path { owner, owner_span, name, name_span } => {
                    self.variant_pat(scrut, Some(&(owner.clone(), *owner_span)), name, *name_span, &[], false, e.span, alt)
                }
                _ => {
                    let ty = scrut.ty;
                    self.diags.push(
                        Diagnostic::at(&codes::E0525, e.span, format!("this `case` cannot match a `{ty}`"))
                            .note("this is not one of its variants")
                            .help(format!("the variants of `{ty}` are {}", self.g.enums[ty.enum_id().unwrap()].variant_list())),
                    );
                    None
                }
            },
            ast::Pattern::Glob(g) => {
                let ty = scrut.ty;
                self.diags.push(Diagnostic::at(&codes::E0109, g.span, format!("`globbed` is only for a `match` on `string`, not a `{ty}`")));
                None
            }
            ast::Pattern::Bind { mode, ty, name, name_span } => {
                let scrut_ty = scrut.ty;
                let Some(inner) = super::optionals::inner_of(self.g, scrut_ty) else {
                    self.diags.push(Diagnostic::at(&codes::E0525, *name_span, format!("this `case` cannot match a `{scrut_ty}`")).note("a binding like this is for optionals"));
                    return None;
                };
                // `case uint32 a` is the `Some` of an optional, with the value bound to `a`.
                let want = self.g.resolve_binding_type(ty, self.diags)?;
                if want != inner {
                    self.diags.push(
                        Diagnostic::at(&codes::E0526, ty.span, format!("`{scrut_ty}` holds a `{inner}`, not a `{want}`"))
                            .note(format!("this binding is a `{want}`"))
                            .help(format!("write `{inner} {name}`")),
                    );
                    return None;
                }
                let sub = ast::SubPat::Bind { mode: *mode, ty: ty.clone(), name: name.clone(), name_span: *name_span };
                self.variant_pat(scrut, None, "Some", *name_span, std::slice::from_ref(&sub), true, *name_span, alt)
            }
        }
    }

    /// `match` on an enum value `v`.
    pub(super) fn enum_match(&mut self, v: hir::Expr, value: &ast::Expr, cases: &[ast::Case], span: Span, out: &mut hir::Block) {
        let ty = v.ty;
        let en = ty.enum_id().unwrap();
        // The value is matched where it is when it is a place (it is not
        // moved by `match`); anything else is kept in a hidden local.
        let rooted = {
            fn root(e: &hir::Expr) -> bool {
                match &e.kind {
                    hir::ExprKind::Local(_) => true,
                    hir::ExprKind::Field(i, _) | hir::ExprKind::Deref(i) | hir::ExprKind::Payload { value: i, .. } => root(i),
                    _ => false,
                }
            }
            root(&v)
        };
        let scrut = if rooted {
            v
        } else {
            let id = self.hidden_named("match value", ty);
            out.push(hir::Stmt::Decl(id));
            out.push(hir::Stmt::Assign(hir::Place::local(id), v));
            hir::Expr::new(hir::ExprKind::Local(id), ty)
        };

        let mut rows: Vec<Vec<Pat>> = Vec::new();
        let mut any: Option<Span> = None;
        let mut clean = true;
        let mut last_guarded = false;
        let mut tests: Vec<super::Test> = Vec::new();
        for case in cases {
            let mut alts: Vec<(Alt, Pat)> = Vec::new();
            let mut wildcard = false;
            let mut ok = true;
            for p in &case.patterns {
                if let ast::Pattern::Any(s) = p {
                    wildcard = true;
                    if case.patterns.len() > 1 {
                        self.diags.push(
                            Diagnostic::at(&codes::E0109, *s, "`_` must be the only pattern in its `case`")
                                .note("this matches everything")
                                .help("`case _` already matches every value; remove the other patterns"),
                        );
                        ok = false;
                    }
                    continue;
                }
                let mut alt = Alt::default();
                match self.top_pat(&scrut, p, &mut alt) {
                    Some(pat) => alts.push((alt, pat)),
                    None => ok = false,
                }
            }
            // Every alternative binds the same names with the same types.
            if ok && alts.len() > 1 {
                let sig = |a: &Alt| {
                    let mut s: Vec<(String, Type)> = a.binds.iter().map(|b| (b.name.clone(), b.ty)).collect();
                    s.sort_by(|x, y| x.0.cmp(&y.0));
                    s
                };
                let first = sig(&alts[0].0);
                if alts[1..].iter().any(|(a, _)| sig(a) != first) {
                    self.diags.push(
                        Diagnostic::at(&codes::E0529, case.span, "the patterns of this `case` bind different names or types")
                            .note("they must bind the same names, with the same types")
                            .help("bind the same names in every pattern, or split them into separate `case`s"),
                    );
                    ok = false;
                }
            }
            let guarded = case.guard.is_some();
            clean &= ok;
            // Reachability and coverage.
            let case_rows: Vec<Vec<Pat>> = if wildcard { vec![vec![Pat::Wild]] } else { alts.iter().map(|(_, p)| vec![p.clone()]).collect() };
            if ok {
                let why = if let Some(_) = any {
                    Some("it comes after `case _`, which matches everything")
                } else if case_rows.iter().all(|r| !useful(self.g, &rows, &[ty], r)) {
                    Some("every value it matches is handled by an earlier case")
                } else {
                    None
                };
                if let Some(why) = why {
                    let mut d = Diagnostic::at(&codes::W0111, case.span, "this case can never run").note(why);
                    if let Some(a) = any {
                        d = d.label(a, "this matches everything");
                    }
                    self.diags.push(d.help("remove it, or move it above the case that handles its values"));
                }
                if !guarded {
                    rows.extend(case_rows);
                }
                if wildcard && !guarded && any.is_none() {
                    any = Some(case.span);
                }
            }
            last_guarded = guarded;
            // The branch.
            if wildcard && !guarded {
                tests.push(super::Test::Always);
                continue;
            }
            if !ok {
                tests.push(super::Test::Value(None));
                continue;
            }
            tests.push(super::Test::Pattern(Box::new(self.case_plan(&scrut, alts, wildcard, case.guard.as_ref()))));
        }

        // Is every value handled by the cases without a guard?
        let missing: Vec<usize> = (0..self.g.enums[en].variants.len())
            .filter(|&vi| {
                let ftys = field_types(self.g, ty, vi);
                let spec = specialize(&rows, Ctor::Variant(vi), ftys.len());
                useful(self.g, &spec, &ftys, &wilds(ftys.len()))
            })
            .collect();
        let exhaustive = missing.is_empty() && !last_guarded;
        if clean && !missing.is_empty() {
            self.warn_missing_variants(en, &missing, value, span);
        }
        let label = self.new_label();
        let parts = tests.into_iter().zip(cases).map(|(t, c)| (t, &c.body)).collect();
        if let Some(branches) = self.chain(label, span, true, parts, exhaustive)
            && !branches.is_empty()
        {
            out.push(hir::Stmt::If { label, branches });
        }
    }

    /// W0110 for a `match` on an enum that leaves variants out.
    fn warn_missing_variants(&mut self, en: EnumId, missing: &[usize], value: &ast::Expr, span: Span) {
        let _ = value;
        if let hir::EnumKind::Optional(inner) = self.g.enums[en].kind {
            // `none` is named as written; `Some` is "a value", which is what `case T name` binds.
            let names: Vec<String> = missing.iter().map(|&v| if v == super::optionals::SOME { "a value".to_string() } else { "`none`".to_string() }).collect();
            let mut help: Vec<String> = missing
                .iter()
                .map(|&v| if v == super::optionals::SOME { format!("`case {inner} value {{ }}`") } else { "`case none { }`".to_string() })
                .collect();
            help.push("`case _ { }`".to_string());
            self.diags.push(
                Diagnostic::at(&codes::W0110, Span::new(span.start, span.start + 5), format!("this `match` does not handle {}", join_and(&names)))
                    .note(if missing.len() == 1 { "this case is missing" } else { "these cases are missing" })
                    .help(format!("add {}", join_or(&help))),
            );
            return;
        }
        let names: Vec<String> = missing.iter().map(|&v| format!("`{}`", self.g.enums[en].variants[v].name)).collect();
        let cases: Vec<String> = missing.iter().map(|&v| format!("`case {} {{ }}`", self.variant_shape(en, v))).collect();
        let mut help = cases;
        help.push("`case _ { }`".to_string());
        let d = Diagnostic::at(&codes::W0110, Span::new(span.start, span.start + 5), format!("this `match` does not handle {}", join_and(&names)))
            .note(if missing.len() == 1 { "this variant is missing" } else { "these variants are missing" })
            .help(format!("add {}", join_or(&help)));
        self.diags.push(d);
    }

    /// Turn the compiled alternatives of a `case` into what `chain` needs.
    fn case_plan<'e>(&mut self, scrut: &hir::Expr, alts: Vec<(Alt, Pat)>, wildcard: bool, guard: Option<&'e ast::Expr>) -> PatCase<'e> {
        if wildcard {
            return PatCase { test: None, binds: Vec::new(), pre: Vec::new(), post: Vec::new(), guard };
        }
        // One local for each bound name, shared by the alternatives.
        let mut binds: Vec<(String, hir::LocalId, Type, Span)> = Vec::new();
        for b in &alts[0].0.binds {
            let id = self.declare_binding(&b.name, b.ty, b.span);
            binds.push((b.name.clone(), id, b.ty, b.span));
        }
        let id_of = |name: &str| binds.iter().find(|b| b.0 == name).map(|b| b.1).unwrap();
        let span = Span::new(0, 0);
        let mut pre_for = Vec::new();
        let mut post_for = Vec::new();
        let mut tests = Vec::new();
        for (alt, _) in &alts {
            let mut pre = Vec::new();
            let mut post = Vec::new();
            for b in &alt.binds {
                let id = id_of(&b.name);
                let local = hir::Place { local: id, proj: Vec::new(), span: b.span };
                match &b.source {
                    Source::Read(e) => pre.push(hir::Stmt::Assign(local, e.clone())),
                    Source::Copy(e) => pre.push(hir::Stmt::Assign(local, hir::Expr::new(hir::ExprKind::DeepCopy(Box::new(e.clone())), b.ty))),
                    Source::Take(p) => {
                        post.push(hir::Stmt::Assign(local, hir::Expr::new(hir::ExprKind::Take { place: p.clone(), leaving: None }, b.ty)))
                    }
                }
            }
            if alt.has_take {
                // The rest of the value goes with it: whatever else owns memory is dropped
                // with this branch.
                for p in &alt.leftovers {
                    let ty = self.place_type(p);
                    let h = self.hidden_named("rest of the value", ty);
                    post.push(hir::Stmt::Decl(h));
                    post.push(hir::Stmt::Assign(hir::Place::local(h), hir::Expr::new(hir::ExprKind::Take { place: p.clone(), leaving: None }, ty)));
                }
                if let Some(root) = to_place(scrut, Span::new(0, 0)) {
                    post.push(hir::Stmt::Forget(root));
                }
            }
            tests.push(and_all(alt.tests.clone()));
            pre_for.push(pre);
            post_for.push(post);
        }
        let test = if tests.iter().any(|t| t.is_none()) { None } else { Some(or_all(tests.iter().cloned().map(Option::unwrap).collect())) };
        let mut pre: hir::Block = binds.iter().map(|b| hir::Stmt::Decl(b.1)).collect();
        let mut post = Vec::new();
        if alts.len() == 1 {
            pre.append(&mut pre_for[0]);
            post.append(&mut post_for[0]);
        } else {
            // Each alternative assigns the shared locals from its own payloads.
            for (phase, lists) in [(0, pre_for), (1, post_for)] {
                let label = self.new_label();
                let n = lists.len();
                let branches: Vec<hir::Branch> = lists
                    .into_iter()
                    .zip(&tests)
                    .enumerate()
                    .map(|(i, (body, t))| hir::Branch { test: if i + 1 == n { None } else { t.clone() }, body, end: span })
                    .collect();
                if branches.iter().all(|b| b.body.is_empty()) {
                    continue;
                }
                let stmt = hir::Stmt::If { label, branches };
                if phase == 0 { pre.push(stmt) } else { post.push(stmt) }
            }
        }
        let taken: Vec<&str> = alts[0].0.binds.iter().filter(|b| matches!(b.source, Source::Take(_))).map(|b| b.name.as_str()).collect();
        PatCase { test, binds: binds.iter().map(|b| (b.0.clone(), b.1, taken.contains(&b.0.as_str()))).collect(), pre, post, guard }
    }

    /// The type of the value at a place of this function.
    fn place_type(&self, p: &hir::Place) -> Type {
        let mut t = self.locals[p.local].ty.unwrap();
        let mut variant = None;
        for pr in &p.proj {
            t = match pr {
                hir::HirProj::Deref => t.target().unwrap_or(t),
                hir::HirProj::Index(_) => t.elem().unwrap_or(t),
                hir::HirProj::Downcast(v) => {
                    variant = Some(*v);
                    t
                }
                hir::HirProj::Field(f) => match variant.take() {
                    Some(v) => self.g.enums[t.enum_id().unwrap()].variants[v].fields[*f].ty.unwrap_or(Type::None),
                    None => self.g.structs[t.struct_id().unwrap()].fields[*f].ty.unwrap_or(Type::None),
                },
            };
        }
        t
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
