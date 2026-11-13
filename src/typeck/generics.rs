//! Generic functions, structs and enums by monomorphisation (S6A.1, S6A.2).
//!
//! `T max<T>(T a, T b)` is a template. It is not declared or checked where it
//! is written: each call works out what `T` stands for (from the type
//! arguments written at the call, or from the arguments), and the pair of
//! template and types is an instance: an ordinary function with its own
//! symbol (`drachm_f_max_Luint32_R`), whose body is checked as if the types
//! had been written out. Everything after the checker sees plain functions.
//!
//! An error inside an instance is reported at its own place in the template,
//! with a note saying which instance it came from and a label at the call
//! that made it. The same place is reported once, however many instances
//! hit it.
//!
//! `struct Stack<T> { ... }` and `enum Maybe<T> { ... }` are templates in the
//! same way. `Stack<uint32>` written anywhere makes (once) an instance: an
//! ordinary struct called `Stack<uint32>` with its own fields, methods,
//! constructors and `drop()`, whose method bodies are checked as if the type
//! had been written out. The parser lists every type written with arguments
//! (`Program::generics`); `ensure_written` makes the instances such a list
//! names before anything resolves a type, because resolving only looks them
//! up. A template's list holds the types written inside it, and is used
//! whenever the template is instantiated, with the parameters bound.

use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use super::{
    check_body, check_one_struct_lives, check_signature_extras, declare_methods, failures, interfaces, method_symbol, never_uses_self, new_struct_info,
    optionals, origin, units, Checker, Globals, MethodCtx, Sig, Val,
};
use crate::ast::{self, ExprKind};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, FuncId};
use crate::types::{self, Type};
use crate::units::Unit;

/// How many instances may be nested: the body of one makes another.
pub(super) const MAX_DEPTH: usize = 64;

/// The key in a `tenv` that holds the type of the struct being instantiated:
/// what a constructor's return type stands for.
pub(super) const SELF: &str = "\u{0}self";

pub(super) struct Template {
    /// Its index in `Program::funcs`.
    pub index: usize,
    /// The declaration without its body.
    pub head: ast::Func,
    /// False after an error or an unsupported feature in its type parameters.
    pub usable: bool,
    /// How many calls name it (counted even when the call is wrong).
    pub instances: usize,
}

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Struct,
    Enum,
}

/// A generic struct or enum.
pub(super) struct TypeTemplate {
    pub kind: Kind,
    /// Its index in `Program::structs` or `Program::enums`.
    pub index: usize,
    pub name_span: Span,
    pub tparams: Vec<ast::TypeParam>,
    /// False after an error or an unsupported feature in its type parameters.
    pub usable: bool,
    /// How many types written in the file name it.
    pub instances: usize,
    /// The declaration without its method bodies.
    pub head: Head,
}

pub(super) enum Head {
    Struct(Rc<ast::Struct>),
    Enum(Rc<ast::Enum>),
}

/// What a pending instance checks.
enum Body {
    Func(usize),
    /// Method `mi` of struct template `si`.
    Method { si: usize, mi: usize, ctx: MethodCtx, symbol: String, id: FuncId },
}

/// An instance whose body is still to be checked.
pub(super) struct Pending {
    body: Body,
    id: FuncId,
    scope: Rc<Vec<ast::TypeName>>,
    tenv: HashMap<String, Type>,
    depth: usize,
    call: Span,
    display: String,
    symbol: String,
}

#[derive(Default)]
pub(super) struct State {
    pub templates: HashMap<String, Template>,
    /// Generic structs and enums by name.
    pub type_templates: HashMap<String, TypeTemplate>,
    /// The type each `(template, arguments)` made.
    pub type_instances: HashMap<(String, Vec<Type>), Type>,
    /// What an instance was made from.
    pub origin: HashMap<Type, (String, Vec<Type>)>,
    /// Instances that were refused (already reported).
    refused: HashSet<(String, Vec<Type>)>,
    /// The types written with arguments in the item being resolved or checked.
    pub scope: Rc<Vec<ast::TypeName>>,
    /// The ones written outside every template.
    pub global: Rc<Vec<ast::TypeName>>,
    /// The ones inside each template: `fn name` or `type name`.
    scopes: HashMap<String, Rc<Vec<ast::TypeName>>>,
    /// How many struct and enum instances are being made inside each other.
    type_depth: usize,
    instances: HashMap<(String, Vec<Type>), FuncId>,
    pending: VecDeque<Pending>,
    /// How deep the body being checked is in instances (0: not in one).
    depth: usize,
    /// Every `T?` the file writes, to make the optionals an instance names.
    pub written_optionals: Vec<ast::TypeName>,
    /// The (code, place) pairs already reported from instances.
    reported: HashSet<(&'static str, usize, usize)>,
    /// The template being checked at its definition (`definition_pass`).
    defn: Option<String>,
    /// Templates whose definition has errors: their instances are not reported again.
    defn_failed: HashSet<String>,
}

/// The declaration of `f` without its body.
fn func_head(f: &ast::Func) -> ast::Func {
    ast::Func {
        ret: f.ret.clone(),
        name: f.name.clone(),
        name_span: f.name_span,
        tparams: f.tparams.clone(),
        params: f.params.clone(),
        body: ast::Block { stmts: Vec::new(), end: f.body.end },
        is_unsafe: f.is_unsafe,
        lives: f.lives.clone(),
        lives_span: f.lives_span,
        lives_from: f.lives_from,
        fails: f.fails.clone(),
        fails_span: f.fails_span,
        span: f.span,
        test: None,
    }
}

/// A single letter that no unit, struct, enum, type or other parameter of
/// `tparams` has: the one E0605 suggests for a parameter that clashes.
fn free_type_name(g: &Globals, tparams: &[ast::TypeParam]) -> String {
    let taken = |name: &str| {
        units::is_unit_name(g, name)
            || g.structs_by_name.contains_key(name)
            || g.enums_by_name.contains_key(name)
            || g.mono.type_templates.contains_key(name)
            || name == "none"
            || Type::from_name(name).is_ok()
            || tparams.iter().any(|p| p.name == name)
    };
    // Common parameter names first, then any capital letter.
    "UXYZPQRBDEFGHJKLMNOSTVW"
        .chars()
        .chain('A'..='Z')
        .map(|c| c.to_string())
        .find(|name| !taken(name))
        .unwrap_or_else(|| "T".to_string())
}

/// E0605 and E0901 for a type parameter list; false if the template cannot be used.
pub(super) fn check_tparams(g: &Globals, tparams: &[ast::TypeParam], diags: &mut Vec<Diagnostic>) -> bool {
    let mut usable = true;
    let mut seen: Vec<(&str, Span)> = Vec::new();
    for p in tparams {
        if let Some((_, first)) = seen.iter().find(|(n, _)| *n == p.name) {
            diags.push(
                Diagnostic::at(&codes::E0605, p.span, format!("type parameter `{}` is declared twice", p.name))
                    .note("declared again here")
                    .label(*first, "first declared here")
                    .help("give each type parameter its own name"),
            );
            usable = false;
        } else {
            let what = if g.structs_by_name.contains_key(&p.name) || g.mono.type_templates.get(&p.name).is_some_and(|t| t.kind == Kind::Struct) {
                Some("struct")
            } else if g.enums_by_name.contains_key(&p.name) || g.mono.type_templates.get(&p.name).is_some_and(|t| t.kind == Kind::Enum) {
                Some("enum")
            } else if p.name == "none" || Type::from_name(&p.name).is_ok() {
                Some("type")
            } else {
                None
            };
            if let Some(what) = what {
                let art = if what == "enum" { "an" } else { "a" };
                diags.push(
                    Diagnostic::at(&codes::E0605, p.span, format!("type parameter `{}` is also the name of {art} {what}", p.name))
                        .note(format!("{art} {what} is already called `{}`", p.name))
                        .help(format!("a type parameter takes no other name's place: rename it, for example `{}`", free_type_name(g, tparams))),
                );
                usable = false;
            }
            if units::is_unit_name(g, &p.name) {
                diags.push(
                    Diagnostic::at(&codes::W0618, p.span, format!("type parameter `{}` has the same name as a unit", p.name))
                        .note(format!("inside its scope `{}` means the type parameter", p.name))
                        .help(format!("write `{0} as unit` to mean the unit, or rename the parameter", p.name)),
                );
            }
        }
        seen.push((&p.name, p.span));
        if let Some((ty, at)) = &p.value {
            if !matches!(Type::from_name(ty), Ok(Type::Int { .. })) {
                diags.push(
                    Diagnostic::at(&codes::E0605, *at, format!("value parameter `{}` has type `{ty}`, which is not a whole-number type", p.name))
                        .help("a value parameter is a whole number: `uint32`, `sint64` and so on"),
                );
                usable = false;
            }
            if !p.bounds.is_empty() {
                diags.push(Diagnostic::at(&codes::E0901, p.span, "a constraint on a value parameter is not supported yet").help(format!("write `{ty} {}` with no `with`", p.name)));
                usable = false;
            }
        }
        if p.is_unit && !p.bounds.is_empty() {
            diags.push(Diagnostic::at(&codes::E0901, p.span, "a constraint on a unit parameter is not supported yet").help(format!("write `unit {}` with no `with`", p.name)));
            usable = false;
        }
    }
    if !interfaces::check_param_bounds(g, tparams, diags) {
        usable = false;
    }
    usable
}

/// Does any of these parameters stand for a unit?
fn has_unit_params(tparams: &[ast::TypeParam]) -> bool {
    tparams.iter().any(|p| p.is_unit || p.value.is_some())
}

/// The unit parameter a unit written `spec` is, when the unit is just that
/// parameter (`U`, not `U/s`).
fn bare_unit_param(tparams: &[ast::TypeParam], spec: &ast::UnitSpec) -> Option<usize> {
    match &spec.expr {
        ast::UnitExpr::Name(n, _) if !spec.delta => tparams.iter().position(|tp| tp.is_unit && tp.name == *n),
        _ => None,
    }
}

/// Does a unit expression name a unit parameter anywhere?
fn unit_mentions(tparams: &[ast::TypeParam], e: &ast::UnitExpr) -> bool {
    match e {
        ast::UnitExpr::Name(n, _) => tparams.iter().any(|tp| tp.is_unit && tp.name == *n),
        ast::UnitExpr::Explicit(..) | ast::UnitExpr::One(_) => false,
        ast::UnitExpr::Mul(a, b) | ast::UnitExpr::Div(a, b) => unit_mentions(tparams, a) || unit_mentions(tparams, b),
        ast::UnitExpr::Pow(b, _, _) => unit_mentions(tparams, b),
    }
}

/// A unit spec that puts a unit parameter inside a bigger unit (`U/s`), as
/// opposed to the bare parameter (`U`).
fn compound_spec(tparams: &[ast::TypeParam], s: &ast::UnitSpec) -> bool {
    unit_mentions(tparams, &s.expr) && bare_unit_param(tparams, s).is_none()
}

/// A parameter type that puts a unit parameter inside a `delta` (`delta U`),
/// which is not worked out yet.
fn delta_unit(tparams: &[ast::TypeParam], p: &ast::TypeName) -> bool {
    let delta = |spec: &Option<ast::UnitSpec>| spec.as_ref().is_some_and(|s| s.delta && unit_mentions(tparams, &s.expr));
    delta(&p.unit) || delta(&p.unit_arg) || p.args.iter().any(|a| delta_unit(tparams, a))
}

/// The unit parameters a unit expression names, each once, in order.
fn unit_param_names(tparams: &[ast::TypeParam], e: &ast::UnitExpr, out: &mut Vec<String>) {
    match e {
        ast::UnitExpr::Name(n, _) => {
            if tparams.iter().any(|tp| tp.is_unit && tp.name == *n) && !out.contains(n) {
                out.push(n.clone());
            }
        }
        ast::UnitExpr::Explicit(..) | ast::UnitExpr::One(_) => {}
        ast::UnitExpr::Mul(a, b) | ast::UnitExpr::Div(a, b) => {
            unit_param_names(tparams, a, out);
            unit_param_names(tparams, b, out);
        }
        ast::UnitExpr::Pow(b, _, _) => unit_param_names(tparams, b, out),
    }
}

/// The power a unit expression raises the unit parameter `name` to: `U/s` is 1,
/// `U*U/s` is 2, `U/U` is 0.
fn unit_exp(e: &ast::UnitExpr, name: &str) -> i32 {
    match e {
        ast::UnitExpr::Name(n, _) => i32::from(n == name),
        ast::UnitExpr::Explicit(..) | ast::UnitExpr::One(_) => 0,
        ast::UnitExpr::Mul(a, b) => unit_exp(a, name) + unit_exp(b, name),
        ast::UnitExpr::Div(a, b) => unit_exp(a, name) - unit_exp(b, name),
        ast::UnitExpr::Pow(b, n, _) => n * unit_exp(b, name),
    }
}

/// The unit parameters among `names` that are bound so far, as a message
/// says them (`` `U` is `m` ``).
fn bound_names(tparams: &[ast::TypeParam], bound: &[Option<(Type, usize)>], names: &[String]) -> String {
    let parts: Vec<String> = names
        .iter()
        .filter_map(|n| {
            let k = tparams.iter().position(|tp| tp.name == *n)?;
            let (t, _) = bound[k]?;
            Some(format!("`{n}` = `{}`", show_arg(&t)))
        })
        .collect();
    if parts.is_empty() { "no unit parameter given yet".to_string() } else { parts.join(" and ") }
}

/// A unit expression as written, for messages (`U/s`, `U*V^2`).
fn unit_text(e: &ast::UnitExpr) -> String {
    match e {
        ast::UnitExpr::Name(n, _) => n.clone(),
        ast::UnitExpr::Explicit(n, _) => format!("{n} as unit"),
        ast::UnitExpr::One(_) => "1".to_string(),
        ast::UnitExpr::Mul(a, b) => format!("{}*{}", unit_text(a), paren_unit(b)),
        ast::UnitExpr::Div(a, b) => format!("{}/{}", unit_text(a), paren_unit(b)),
        ast::UnitExpr::Pow(b, n, _) => format!("{}^{n}", paren_unit(b)),
    }
}

fn paren_unit(e: &ast::UnitExpr) -> String {
    match e {
        ast::UnitExpr::Mul(..) | ast::UnitExpr::Div(..) => format!("({})", unit_text(e)),
        _ => unit_text(e),
    }
}

/// The compound unit parameters inside a parameter's type, for an argument of
/// type `ty` at position `i`: `(i, spec, unit)` for each that has a unit.
fn compound_of<'a>(g: &Globals, tparams: &[ast::TypeParam], p: &'a ast::TypeName, ty: Type, i: usize, out: &mut Vec<(usize, &'a ast::UnitSpec, &'static Unit)>) {
    let inner = peel(g, p, ty);
    if let Some(s) = p.unit.as_ref().filter(|s| compound_spec(tparams, s))
        && let Some(u) = inner.unit()
    {
        out.push((i, s, u));
    }
    if let Some(s) = p.unit_arg.as_ref().filter(|s| compound_spec(tparams, s))
        && let Some(u) = inner.as_unit_arg()
    {
        out.push((i, s, u));
    }
    if !p.args.is_empty()
        && let Some((name, targs)) = g.mono.origin.get(&inner)
        && *name == p.name
        && targs.len() == p.args.len()
    {
        for (pa, ta) in p.args.iter().zip(targs) {
            compound_of(g, tparams, pa, *ta, i, out);
        }
    }
}

/// A type or unit as a message shows it: a unit by its name.
fn show_arg(t: &Type) -> String {
    match t.as_unit_arg() {
        Some(u) => types::unit_name(u),
        None => t.to_string(),
    }
}

/// A type argument as written in an instance's name: `uint32`, `unit m/s`.
fn written_arg(t: &Type) -> String {
    match t.as_unit_arg() {
        Some(u) => format!("unit {}", types::unit_name(u)),
        None => t.to_string(),
    }
}

/// Records function `f` (at `index` in the program) as a template.
pub(super) fn declare_template(g: &mut Globals, index: usize, f: &ast::Func, diags: &mut Vec<Diagnostic>) {
    if let Some(first) = g.funcs_by_name.get(&f.name).map(|&id| g.sigs[id].span).or_else(|| g.mono.templates.get(&f.name).map(|t| t.head.span)) {
        diags.push(
            Diagnostic::at(&codes::E0202, f.name_span, format!("function `{}` is defined twice", f.name))
                .note("defined again here")
                .label(first, "first defined here"),
        );
        return;
    }
    let usable = check_tparams(g, &f.tparams, diags);
    let head = func_head(f);
    g.mono.templates.insert(f.name.clone(), Template { index, head, usable, instances: 0 });
}

/// Checks the bodies of the instances made so far, and of the ones they make.
pub(super) fn drain(g: &mut Globals, src: &str, prog: &ast::Program, diags: &mut Vec<Diagnostic>, emit: &mut dyn FnMut(FuncId, hir::Func)) {
    while let Some(p) = g.mono.pending.pop_front() {
        g.tenv = p.tenv.clone();
        g.mono.depth = p.depth;
        g.mono.scope = p.scope.clone();
        let mut local = Vec::new();
        ensure_written(g, &mut local);
        let mut unused_self = None;
        let f = match &p.body {
            Body::Func(index) => check_body(g, src, &mut local, &prog.funcs[*index], None, p.symbol.clone()),
            Body::Method { si, mi, ctx, symbol, id } => {
                let m = &prog.structs[*si].methods[*mi];
                let f = check_body(g, src, &mut local, &m.func, Some(*ctx), symbol.clone());
                if m.is_static.is_none() && m.operator.is_none() && !m.ctor && !g.self_use[*id].used && m.func.name != "drop" && !g.ifaces.implementing.contains(id) {
                    unused_self = Some(never_uses_self(src, &prog.structs[*si].name, m));
                }
                f
            }
        };
        g.tenv.clear();
        g.mono.depth = 0;
        local.extend(unused_self);
        let template = p.display.split('<').next().unwrap_or_default().to_string();
        let failed = g.mono.defn.is_none() && g.mono.defn_failed.contains(&template);
        for d in local {
            // `copy` that does nothing for this `T` is needed for another one.
            if d.code.id == "W0322" || (failed && d.is_error()) || !first_report(g, &d) {
                continue;
            }
            diags.push(instance_note(g, d, &p.display, p.call));
        }
        emit(p.id, f);
    }
}

/// W0618 for each usable template nothing instantiated.
pub(super) fn warn_unused(g: &Globals, diags: &mut Vec<Diagnostic>) {
    let bounded = |tps: &[ast::TypeParam]| tps.iter().any(|p| !p.bounds.is_empty());
    let mut unused: Vec<&Template> = g.mono.templates.values().filter(|t| t.usable && t.instances == 0 && !bounded(&t.head.tparams)).collect();
    unused.sort_by_key(|t| t.index);
    for t in unused {
        diags.push(
            Diagnostic::at(&codes::W0600, t.head.name_span, format!("generic function `{}` is never used", t.head.name))
                .note("nothing calls it")
                .help("a generic function is only checked when it is called with a type: call it, or remove it"),
        );
    }
    let mut unused: Vec<(&String, &TypeTemplate)> = g.mono.type_templates.iter().filter(|(_, t)| t.usable && t.instances == 0 && !bounded(&t.tparams)).collect();
    unused.sort_by_key(|(_, t)| t.name_span.start);
    for (name, t) in unused {
        let what = if t.kind == Kind::Struct { "struct" } else { "enum" };
        diags.push(
            Diagnostic::at(&codes::W0600, t.name_span, format!("generic {what} `{name}` is never used"))
                .note("no type names it")
                .help(format!("a generic {what} is only checked when a type like `{name}<uint32>` is used: use it, or remove it")),
        );
    }
}

/// Makes the optional types that the written `T?` name under the current `tenv`.
fn declare_written_optionals(g: &mut Globals) {
    let written = std::mem::take(&mut g.mono.written_optionals);
    optionals::declare_optionals(g, &written);
    g.mono.written_optionals = written;
}

/// Makes the instances and optionals that the types written in the current
/// scope name, under the current `tenv`. A name that does not resolve here
/// (a type parameter of another template, an error) is left for `resolve` to
/// report where it is used.
pub(super) fn ensure_written(g: &mut Globals, diags: &mut Vec<Diagnostic>) {
    let scope = g.mono.scope.clone();
    loop {
        let before = (g.structs.len(), g.enums.len());
        for t in scope.iter() {
            ensure_type(g, t, diags);
        }
        declare_written_optionals(g);
        if (g.structs.len(), g.enums.len()) == before {
            break;
        }
    }
}

/// True the first time a diagnostic's code and place are seen.
fn first_report(g: &mut Globals, d: &Diagnostic) -> bool {
    match d.labels.first() {
        Some((s, _)) => g.mono.reported.insert((d.code.id, s.start, s.end)),
        None => true,
    }
}

/// Says where a diagnostic from inside an instance comes from: the instance
/// and the place that made it, or, when a template is being checked at its
/// definition, just that.
fn instance_note(g: &Globals, d: Diagnostic, display: &str, call: Span) -> Diagnostic {
    match &g.mono.defn {
        Some(name) => d.note_line(format!("in the definition of `{name}`")),
        None => {
            let same = d.labels.first().is_some_and(|l| l.0 == call);
            let d = d.note_line(format!("while checking `{display}`, instantiated here"));
            if same { d } else { d.label(call, "instantiated here") }
        }
    }
}

fn ordinal(i: usize) -> String {
    match i {
        0 => "first".into(),
        1 => "second".into(),
        2 => "third".into(),
        3 => "fourth".into(),
        4 => "fifth".into(),
        n => format!("{}th", n + 1),
    }
}

/// What a parameter written as `p` binds its type parameter to, given an
/// argument of type `ty`: `&T` takes the target of a borrow, `T?` the inside
/// of an optional.
fn peel(g: &Globals, p: &ast::TypeName, ty: Type) -> Type {
    let mut t = ty;
    for (m, _) in &p.mods {
        t = match (m, t) {
            (ast::TypeMod::Ref { .. }, Type::Ref { to, .. }) => *to,
            (ast::TypeMod::Ptr, Type::Ptr { to }) | (ast::TypeMod::FixedPtr, Type::FixedPtr { to }) => *to,
            _ => t,
        };
    }
    if p.optional.is_some() && let Some(inner) = optionals::inner_of(g, t) {
        t = inner;
    }
    // `T[?]` and `T[4]` bind `T` to the item type.
    for d in p.dims.iter().rev() {
        t = match (d, t) {
            (ast::Dim::List(_), Type::List { elem }) | (ast::Dim::Array(..) | ast::Dim::Param(..) | ast::Dim::Calc(..), Type::Array { elem, .. }) => *elem,
            _ => t,
        };
    }
    t
}

/// Does the written type `p` name one of the type parameters, anywhere?
fn mentions(tparams: &[ast::TypeParam], p: &ast::TypeName) -> bool {
    p.dims.iter().any(|d| match d {
        ast::Dim::Param(n, _) => tparams.iter().any(|tp| tp.name == *n),
        ast::Dim::Calc(e, _) => tparams.iter().any(|tp| calc_names(e, &tp.name)),
        _ => false,
    }) || (p.args.is_empty() && tparams.iter().any(|tp| tp.name == p.name))
        || p.unit.as_ref().is_some_and(|s| unit_mentions(tparams, &s.expr))
        || p.unit_arg.as_ref().is_some_and(|s| unit_mentions(tparams, &s.expr))
        || p.args.iter().any(|a| mentions(tparams, a))
}

/// Does the length calculation `e` name `n`?
fn calc_names(e: &ast::ConstExpr, n: &str) -> bool {
    use ast::ConstExpr as C;
    match e {
        C::Num(..) => false,
        C::Name(m, _) => m == n,
        C::Neg(i, _) | C::Not(i, _) => calc_names(i, n),
        C::Bin(_, l, r, _) => calc_names(l, n) || calc_names(r, n),
    }
}

/// The one name `e` has and the value it must have for `e` to be `len`, when
/// that is plain to see: `N + 3`, `3 + N`, `N - 3`, `3 - N`, `N * 3`, `3 * N`.
fn solve_calc(e: &ast::ConstExpr, len: i128) -> Option<(String, i128)> {
    use ast::ConstExpr as C;
    let C::Bin(op, l, r, _) = e else { return None };
    let num = |x: &C| match x {
        C::Num(v, _) => num_traits::ToPrimitive::to_i128(v),
        _ => None,
    };
    let name = |x: &C| match x {
        C::Name(n, _) => Some(n.clone()),
        _ => None,
    };
    match (op, name(l), num(r), num(l), name(r)) {
        (ast::BinOp::Add, Some(n), Some(k), ..) | (ast::BinOp::Add, _, _, Some(k), Some(n)) => Some((n, len.checked_sub(k)?)),
        (ast::BinOp::Sub, Some(n), Some(k), ..) => Some((n, len.checked_add(k)?)),
        (ast::BinOp::Sub, _, _, Some(k), Some(n)) => Some((n, k.checked_sub(len)?)),
        (ast::BinOp::Mul, Some(n), Some(k), ..) | (ast::BinOp::Mul, _, _, Some(k), Some(n)) if k != 0 && len % k == 0 => Some((n, len / k)),
        _ => None,
    }
}

/// What a parameter written `p` makes of the type parameters, given an
/// argument of type `ty`: `(parameter index, type)` for each one it shows.
fn bindings(g: &Globals, tparams: &[ast::TypeParam], p: &ast::TypeName, ty: Type, out: &mut Vec<(usize, Type)>) {
    // `T[N]` binds `N` to the length of the array.
    if p.dims.iter().any(|d| matches!(d, ast::Dim::Param(..) | ast::Dim::Calc(..))) {
        let mut t = ty;
        for (m, _) in &p.mods {
            t = match (m, t) {
                (ast::TypeMod::Ref { .. }, Type::Ref { to, .. }) => *to,
                (ast::TypeMod::Ptr, Type::Ptr { to }) | (ast::TypeMod::FixedPtr, Type::FixedPtr { to }) => *to,
                _ => t,
            };
        }
        if p.optional.is_some() && let Some(inner) = optionals::inner_of(g, t) {
            t = inner;
        }
        for d in p.dims.iter().rev() {
            match (d, t) {
                (ast::Dim::Param(n, _), Type::Array { elem, len }) => {
                    if let Some(k) = tparams.iter().position(|tp| tp.name == *n)
                        && let Some(vt) = tparams[k].value.as_ref().and_then(|(t, _)| Type::from_name(t).ok())
                    {
                        out.push((k, Type::val(vt, i128::from(len))));
                    }
                    t = *elem;
                }
                (ast::Dim::Calc(e, _), Type::Array { elem, len }) => {
                    if let Some((n, v)) = solve_calc(e, i128::from(len))
                        && let Some(k) = tparams.iter().position(|tp| tp.name == n)
                        && let Some(vt) = tparams[k].value.as_ref().and_then(|(t, _)| Type::from_name(t).ok())
                    {
                        out.push((k, Type::val(vt, v)));
                    }
                    t = *elem;
                }
                (ast::Dim::List(_), Type::List { elem }) | (ast::Dim::Array(..), Type::Array { elem, .. }) => t = *elem,
                _ => {}
            }
        }
    }
    let inner = peel(g, p, ty);
    // `float64<U>` binds `U` to the unit of the value; `unit U` as a type
    // argument binds it to the unit that argument is.
    if let Some(k) = p.unit.as_ref().and_then(|s| bare_unit_param(tparams, s))
        && let Some(u) = inner.unit()
    {
        out.push((k, g.unit_arg(u)));
    }
    if let Some(k) = p.unit_arg.as_ref().and_then(|s| bare_unit_param(tparams, s))
        && let Some(u) = inner.as_unit_arg()
    {
        out.push((k, g.unit_arg(u)));
    }
    if p.args.is_empty() {
        if let Some(k) = tparams.iter().position(|tp| tp.name == p.name) {
            out.push((k, inner));
        }
        return;
    }
    if let Some((name, targs)) = g.mono.origin.get(&inner)
        && *name == p.name
        && targs.len() == p.args.len()
    {
        for (pa, ta) in p.args.iter().zip(targs) {
            bindings(g, tparams, pa, *ta, out);
        }
    }
}

/// An argument of a generic call, looked at before the instance is known.
enum Pre {
    /// Its parameter names no type parameter, or it has to wait for the
    /// parameter's type (`none`).
    Later,
    /// Checked already.
    Done(Option<Val>),
}

impl Checker<'_> {
    /// A call `f(args)` or `f<A, B>(args)` of a generic function.
    pub(super) fn generic_call(&mut self, name: &str, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        let (params, tparams, usable) = {
            let t = &self.g.mono.templates[name];
            (t.head.params.iter().map(|p| p.ty.clone()).collect::<Vec<_>>(), t.head.tparams.clone(), t.usable)
        };
        let n = tparams.len();
        if let Some(t) = self.g.mono.templates.get_mut(name) {
            t.instances += 1;
        }
        let fail = |cx: &mut Self| {
            for a in args {
                cx.expr(a);
            }
            None
        };
        if !usable {
            return fail(self);
        }
        if params.len() != args.len() {
            let np = params.len();
            self.diags.push(
                Diagnostic::at(&codes::E0103, span, format!("`{name}` takes {np} argument{}, but {} given", if np == 1 { "" } else { "s" }, args.len()))
                    .label(self.g.mono.templates[name].head.name_span, format!("`{name}` is defined here")),
            );
            return fail(self);
        }
        if let Some(p) = params.iter().find(|p| delta_unit(&tparams, p)) {
            self.diags.push(
                Diagnostic::at(&codes::E0901, p.span, "a delta of a unit parameter is not supported yet")
                    .help("write the parameter as the unit itself, for example `float64<U>`"),
            );
            return fail(self);
        }
        let mut bound: Vec<Option<(Type, usize)>> = vec![None; n];
        let mut explicit = false;
        if !targs.is_empty() {
            if targs.len() > n {
                self.diags.push(
                    Diagnostic::at(&codes::E0602, targs[n].span, format!("`{name}` takes {n} type argument{}, but {} given", if n == 1 { "" } else { "s" }, targs.len()))
                        .note("one too many")
                        .label(self.g.mono.templates[name].head.name_span, format!("`{name}` is declared with {n} here")),
                );
                return fail(self);
            }
            if targs.len() < n {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, targs[0].span, "writing only some of the type arguments is not supported yet")
                        .help(format!("`{name}` has {n}: write all of them, or none to have them worked out")),
                );
                return fail(self);
            }
            for (i, t) in targs.iter().enumerate() {
                if kind_mismatch(self.g, &tparams[i], t) {
                    self.diags.push(unit_kind_error(name, &tparams[i], t));
                    return fail(self);
                }
                let ty = if tparams[i].value.is_some() { value_targ(self.g, name, &tparams[i], t, self.diags) } else { self.g.resolve_binding_type(t, self.diags) };
                let Some(ty) = ty else { return fail(self) };
                bound[i] = Some((ty, usize::MAX));
            }
            explicit = true;
        }
        // Arguments that show a type parameter are checked first, without an expected type.
        let mut pre: Vec<Pre> = Vec::new();
        for (a, p) in args.iter().zip(&params) {
            let shows = !explicit && mentions(&tparams, p);
            if !shows || matches!(a.kind, ExprKind::NoneValue) {
                pre.push(Pre::Later);
                continue;
            }
            let v = if matches!(p.mods.first(), Some((ast::TypeMod::Ref { .. }, _))) { self.expr_raw(a) } else { self.expr(a) };
            pre.push(Pre::Done(v));
        }
        if !explicit {
            // Units inside a bigger unit (`float64<U/s>`), solved below.
            let mut compound: Vec<(usize, &ast::UnitSpec, &'static Unit)> = Vec::new();
            // The first argument that has a type decides.
            for (i, (a, p)) in args.iter().zip(&params).enumerate() {
                let mut found = Vec::new();
                match &pre[i] {
                    Pre::Done(Some(Val::Expr(x))) => {
                        bindings(self.g, &tparams, p, x.ty, &mut found);
                        compound_of(self.g, &tparams, p, x.ty, i, &mut compound);
                    }
                    // A constant with a unit binds a unit parameter the same way.
                    Pre::Done(Some(Val::UConst(_, u))) => {
                        found.extend(p.unit.as_ref().and_then(|s| bare_unit_param(&tparams, s)).map(|k| (k, self.g.unit_arg(u))));
                        if let Some(s) = p.unit.as_ref().filter(|s| compound_spec(&tparams, s)) {
                            compound.push((i, s, u));
                        }
                    }
                    _ => continue,
                }
                for (k, ty) in found {
                match bound[k] {
                    None => bound[k] = Some((ty, i)),
                    Some((first, from)) if first != ty => {
                        self.diags.push(
                            Diagnostic::at(
                                &codes::E0603,
                                a.span,
                                format!("`{}` is `{}` from the {} argument, but `{}` from the {}", tparams[k].name, show_arg(&first), ordinal(from), show_arg(&ty), ordinal(i)),
                            )
                            .note(format!("this one is `{}`", show_arg(&ty)))
                            .label(args[from].span, format!("this one is `{}`", show_arg(&first)))
                            .help(if ty.as_val().is_some() {
                                format!("`{}` is one length: give the arguments the same length", tparams[k].name)
                            } else {
                                format!("convert one of them, or write the type: `{name}<{}>(...)`", written_arg(&first))
                            }),
                        );
                        for (j, a) in args.iter().enumerate() {
                            if j > i && matches!(pre[j], Pre::Later) {
                                self.expr(a);
                            }
                        }
                        return None;
                    }
                    _ => {}
                }
                }
            }
            // Compound unit parameters: each is solved once all but one of its unit
            // parameters are known. Solving may bind more, so repeat while it goes on.
            let mut todo = compound;
            loop {
                let before = todo.len();
                let mut rest = Vec::new();
                for (i, spec, u) in std::mem::take(&mut todo) {
                    let mut names = Vec::new();
                    unit_param_names(&tparams, &spec.expr, &mut names);
                    let unknown: Vec<usize> = names
                        .iter()
                        .filter_map(|n| tparams.iter().position(|tp| tp.name == *n))
                        .filter(|&k| bound[k].is_none())
                        .collect();
                    if unknown.len() > 1 {
                        rest.push((i, spec, u));
                        continue;
                    }
                    let mut map: HashMap<String, Type> = HashMap::new();
                    for (k, tp) in tparams.iter().enumerate() {
                        if let Some((t, _)) = &bound[k] {
                            map.insert(tp.name.clone(), *t);
                        }
                    }
                    let x = unknown.first().copied();
                    if let Some(k) = x {
                        let one = self.g.units.borrow_mut().table.dimensionless();
                        map.insert(tparams[k].name.clone(), self.g.unit_arg(one));
                    }
                    let Some(r) = self.g.resolve_unit_spec_in(spec, &map, args[i].span, self.diags) else { continue };
                    let shown = self.g.units.borrow().table.display(u);
                    let text = unit_text(&spec.expr);
                    match x {
                        None => {
                            if *r != *u {
                                let got = self.g.units.borrow().table.display(r);
                                self.diags.push(
                                    Diagnostic::at(&codes::E0603, args[i].span, format!("the {} argument's unit `{shown}` does not fit `{text}`", ordinal(i)))
                                        .note(format!("with {}, `{text}` is `{got}`", bound_names(&tparams, &bound, &names))),
                                );
                                return None;
                            }
                        }
                        Some(k) => {
                            let exp = unit_exp(&spec.expr, &tparams[k].name);
                            let root = if exp == 0 {
                                None
                            } else {
                                let q = self.g.units.borrow_mut().table.div(u, r).ok();
                                q.and_then(|q| self.g.units.borrow_mut().table.root(q, exp))
                            };
                            match root {
                                Some(v) => bound[k] = Some((self.g.unit_arg(v), i)),
                                None if exp == 0 => {}
                                None => {
                                    let name = &tparams[k].name;
                                    self.diags.push(
                                        Diagnostic::at(&codes::E0603, args[i].span, format!("`{name}` cannot be worked out from the {} argument: `{shown}` does not fit `{text}`", ordinal(i)))
                                            .note(format!("`{name}` is raised to the power {exp} in `{text}`, and `{shown}` does not split into {} equal parts", exp.unsigned_abs()))
                                            .help(format!("the unit of this argument must be a power of `{name}` times the other units of `{text}`")),
                                    );
                                    return None;
                                }
                            }
                        }
                    }
                }
                todo = rest;
                if todo.is_empty() || todo.len() == before {
                    break;
                }
            }
            if let Some((i, spec, _)) = todo.first() {
                let mut names = Vec::new();
                unit_param_names(&tparams, &spec.expr, &mut names);
                let text = unit_text(&spec.expr);
                self.diags.push(
                    Diagnostic::at(&codes::E0601, args[*i].span, format!("cannot tell what the unit parameters of `{text}` are in this call of `{name}`"))
                        .note(format!("`{text}` has {} unit parameters, and no other argument settles all but one", names.len()))
                        .label(self.g.mono.templates[name].head.name_span, format!("`{name}` is declared here"))
                        .help(format!("write them out: `{name}<{}>(...)`", names.iter().map(|n| format!("unit {n}")).collect::<Vec<_>>().join(", "))),
                );
                for (a, p) in args.iter().zip(&pre) {
                    if matches!(p, Pre::Later) {
                        self.expr(a);
                    }
                }
                return None;
            }
            // A parameter only constants show takes the type a constant has by itself.
            for (k, tp) in tparams.iter().enumerate() {
                if bound[k].is_some() {
                    continue;
                }
                for (i, p) in params.iter().enumerate() {
                    if p.name == tp.name && let Pre::Done(Some(Val::Const(c))) = &pre[i] && let Some(ty) = c.default_type() {
                        bound[k] = Some((ty, i));
                        break;
                    }
                }
            }
            if let Some(k) = bound.iter().position(Option::is_none) {
                let tp = &tparams[k].name;
                let sample = if args.is_empty() { "()" } else { "(...)" };
                let example = if tparams[k].is_unit {
                    "unit m"
                } else if tparams[k].value.is_some() {
                    "8"
                } else {
                    "uint32"
                };
                let in_calc = params.iter().any(|p| p.dims.iter().any(|d| matches!(d, ast::Dim::Calc(e, _) if calc_names(e, tp))));
                let why = if in_calc {
                    format!("`{tp}` is only used in a calculation here, and the compiler solves only `{tp} + 1`, `{tp} - 1`, `{tp} * 2` and the like")
                } else {
                    format!("no argument shows `{tp}`")
                };
                self.diags.push(
                    Diagnostic::at(&codes::E0601, span, format!("cannot tell what `{tp}` is in this call of `{name}`"))
                        .note(why)
                        .label(self.g.mono.templates[name].head.name_span, format!("`{tp}` is declared here"))
                        .help(format!("write it out: `{name}<{example}>{sample}`")),
                );
                for (a, p) in args.iter().zip(&pre) {
                    if matches!(p, Pre::Later) {
                        self.expr(a);
                    }
                }
                return None;
            }
        }
        let types: Vec<Type> = bound.iter().map(|b| b.unwrap().0).collect();
        if types.iter().any(|t| t.is_func()) {
            self.diags.push(Diagnostic::at(&codes::E0901, span, "a function type as a type argument is not supported yet"));
            return fail(self);
        }
        // A borrow as a type argument is allowed by the design but not built yet.
        if let Some(bad) = types.iter().position(|t| t.is_borrow() || self.g.holds_borrow(*t)) {
            self.diags.push(
                Diagnostic::at(&codes::E0901, span, "a type argument cannot be a borrow yet")
                    .note(format!("`{}` is `{}` here", tparams[bad].name, types[bad]))
                    .help("pass the value itself, or write the borrow as a parameter: `&T`"),
            );
            return fail(self);
        }
        let id = self.instantiate_fn(name, types, span);
        let id = match id {
            Some(id) => id,
            None => {
                for (a, p) in args.iter().zip(&pre) {
                    if matches!(p, Pre::Later) {
                        self.expr(a);
                    }
                }
                return None;
            }
        };
        let sig_params = self.g.sigs[id].params.clone();
        let mut out = Vec::new();
        let mut ok = true;
        for ((a, p), pre) in args.iter().zip(&sig_params).zip(pre) {
            let v = match (pre, p) {
                (Pre::Done(v), Some(p)) => v.and_then(|v| if p.is_borrow() { self.borrow_value(v, a, *p, Some(id)) } else { self.into(v, *p, origin(a)) }),
                (Pre::Later, Some(p)) if p.is_borrow() => self.coerce_borrow(a, *p, Some(id)),
                (Pre::Later, Some(p)) => self.coerce(a, *p),
                (_, None) => {
                    self.expr(a);
                    None
                }
            };
            match v {
                Some(v) => out.push(v),
                None => ok = false,
            }
        }
        let ty = self.g.sigs[id].ret?;
        if !ok {
            return None;
        }
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Call(id, out), ty)))
    }

    /// The instance of generic function `name` for `types`, made on first use.
    /// Its body waits on the worklist.
    pub(super) fn instantiate_fn(&mut self, name: &str, types: Vec<Type>, call: Span) -> Option<FuncId> {
        make_fn_instance(self.g, self.diags, name, types, call)
    }

    /// `name<types>` used as a function or a type that takes no type arguments.
    pub(super) fn not_generic(&mut self, name: &str, targs: &[ast::TypeName], args: Vec<&ast::Expr>, span: Span) {
        let at = targs.first().map_or(span, |t| t.span.to(targs.last().unwrap().span));
        let known = self.g.funcs_by_name.contains_key(name) || self.g.structs_by_name.contains_key(name) || self.g.enums_by_name.contains_key(name);
        if known {
            self.diags.push(
                Diagnostic::at(&codes::E0602, at, format!("`{name}` takes no type arguments"))
                    .note("type arguments written here")
                    .help(format!("`{name}` is not generic: write just `{name}`")),
            );
        } else {
            self.diags.push(Diagnostic::at(&codes::E0201, span, format!("unknown name `{name}`")));
        }
        for a in args {
            self.expr(a);
        }
    }

    /// E0604: the name of a generic function where a value is wanted.
    pub(super) fn generic_by_name(&mut self, name: &str, span: Span) {
        let def = self.g.mono.templates[name].head.name_span;
        self.diags.push(
            Diagnostic::at(&codes::E0604, span, format!("`{name}` is generic, so it needs a call"))
                .note(format!("`{name}` alone is not a value"))
                .label(def, format!("`{name}` is declared with type parameters here"))
                .help(format!("call it: `{name}(...)`, or `{name}<uint32>(...)` to say what the type parameters are")),
        );
    }
}

/// Is `a` the wrong kind of argument for `tp`: a unit for a type parameter, a
/// type for a unit or a value parameter, a number for a type parameter?
pub(super) fn kind_mismatch(g: &Globals, tp: &ast::TypeParam, a: &ast::TypeName) -> bool {
    if tp.is_unit != a.unit_arg.is_some() {
        return true;
    }
    if tp.value.is_some() {
        return !is_value_arg(g, a);
    }
    a.is_number
}

/// Can `a` be read as a value: a number or calculation, or a bare name that is
/// not a type?
fn is_value_arg(g: &Globals, a: &ast::TypeName) -> bool {
    if a.is_number {
        return true;
    }
    let plain = a.args.is_empty()
        && a.mods.is_empty()
        && a.dims.is_empty()
        && a.optional.is_none()
        && a.unit.is_none()
        && a.unit_arg.is_none()
        && a.func.is_none()
        && !a.self_ty;
    if !plain {
        return false;
    }
    match g.tenv.get(&a.name) {
        Some(t) => t.as_val().is_some(),
        None => g.resolve_base_quiet(a).is_none() && !g.mono.type_templates.contains_key(&a.name),
    }
}

/// A type argument of the wrong kind: a unit for a type parameter, or a type for a unit one.
pub(super) fn unit_kind_error(name: &str, tp: &ast::TypeParam, t: &ast::TypeName) -> Diagnostic {
    if tp.is_unit {
        Diagnostic::at(&codes::E0602, t.span, format!("`{name}` takes a unit for `{}`, not a type", tp.name))
            .note(format!("`{}` stands for a unit", tp.name))
            .help("write the unit with `unit` in front, for example `unit m`")
    } else if let Some((ty, _)) = &tp.value {
        Diagnostic::at(&codes::E0602, t.span, format!("`{name}` takes a value for `{}`, not a type", tp.name))
            .note(format!("`{}` stands for a whole number of type `{ty}`", tp.name))
            .help("write a number here, for example `16`")
    } else if t.is_number {
        Diagnostic::at(&codes::E0602, t.span, format!("`{name}` takes a type for `{}`, not a number", tp.name))
            .note(format!("`{}` stands for a type", tp.name))
            .help(format!("write a type here, for example `uint32`; to take a number, declare `{}` as a value parameter: `uint32 {}`", tp.name, tp.name))
    } else {
        Diagnostic::at(&codes::E0602, t.span, format!("`{name}` takes a type for `{}`, not a unit", tp.name))
            .note(format!("`{}` stands for a type", tp.name))
            .help("write a type here, for example `uint32`")
    }
}

/// The value a whole-number constant expression has (names are value parameters
/// in scope). The flag says whether a name took part.
pub(super) fn eval_value(g: &Globals, e: &ast::ConstExpr) -> Result<(num_bigint::BigInt, bool), Diagnostic> {
    use crate::consts::{Const, FoldError, Folded};
    let whole = |c: Const| match c {
        Const::Int(v) => v,
        _ => unreachable!("whole numbers stay whole"),
    };
    match e {
        ast::ConstExpr::Num(v, _) => Ok((v.clone(), false)),
        ast::ConstExpr::Name(n, span) => match g.tenv.get(n) {
            Some(t) => match t.as_val() {
                Some((_, v)) => Ok((num_bigint::BigInt::from(v), true)),
                None => Err(Diagnostic::at(&codes::E0602, *span, format!("`{n}` is a type, not a value"))
                    .help("a value argument is a number or a value parameter declared as `uint32 N`")),
            },
            None => match g.consts.get(n.as_str()) {
                Some(info) => match &info.value {
                    Const::Int(v) => Ok((v.clone(), false)),
                    _ => Err(Diagnostic::at(&codes::E0634, *span, format!("`{n}` is not a whole number"))),
                },
                None => Err(Diagnostic::at(&codes::E0634, *span, format!("`{n}` is not a constant"))
                    .note(g.run_value_note(n).unwrap_or_else(|| "a value argument must be known when the program is compiled".to_string()))
                    .help("write a number, a calculation on numbers, a constant (`uint32 SIZE = 8;` at the top of the file) or a value parameter of the enclosing generic")),
            },
        },
        ast::ConstExpr::Neg(inner, _) => {
            let (v, named) = eval_value(g, inner)?;
            Ok((-v, named))
        }
        ast::ConstExpr::Not(inner, span) => {
            let (v, named) = eval_value(g, inner)?;
            Ok((whole(Const::Int(v).bit_not().map_err(|_| Diagnostic::at(&codes::E0634, *span, "`~` needs a whole number"))?), named))
        }
        ast::ConstExpr::Bin(op, l, r, span) => {
            let ((a, an), (b, bn)) = (eval_value(g, l)?, eval_value(g, r)?);
            match Const::binary(*op, &Const::Int(a), &Const::Int(b)) {
                Ok(Folded::Value(c)) => Ok((whole(c), an || bn)),
                Ok(Folded::Bool(_)) => Err(Diagnostic::at(&codes::E0634, *span, "a value argument is a whole number")),
                Err(FoldError::DivisionByZero) => Err(Diagnostic::at(&codes::E0107, *span, "this constant expression divides by zero").label(r.span(), "this is zero")),
                Err(FoldError::TooLarge) => Err(Diagnostic::at(&codes::E0002, *span, "this constant expression is too large to evaluate")),
                Err(_) => Err(Diagnostic::at(&codes::E0634, *span, "this calculation has no whole-number result")),
            }
        }
    }
}

/// The stand-in type for the value of argument `a` of value parameter `tp`.
/// Reports why not.
pub(super) fn value_targ(g: &Globals, name: &str, tp: &ast::TypeParam, a: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
    use num_bigint::BigInt;
    use num_traits::ToPrimitive;
    if !is_value_arg(g, a) {
        diags.push(unit_kind_error(name, tp, a));
        return None;
    }
    let expr = match &a.val {
        Some(e) => (**e).clone(),
        None => ast::ConstExpr::Name(a.name.clone(), a.span),
    };
    let v = match eval_value(g, &expr) {
        Ok((v, _)) => v,
        Err(d) => {
            diags.push(d);
            return None;
        }
    };
    let (ty_name, _) = tp.value.as_ref()?;
    let ty = Type::from_name(ty_name).ok()?;
    let Type::Int { bits, signed } = ty else { return None };
    let fits = if signed {
        let lim = BigInt::from(1) << (bits - 1);
        v >= -lim.clone() && v < lim
    } else {
        v >= BigInt::from(0) && v < (BigInt::from(1) << bits)
    };
    if !fits {
        let range = if signed {
            format!("{} to {}", -(BigInt::from(1) << (bits - 1)), (BigInt::from(1) << (bits - 1)) - 1)
        } else {
            format!("0 to {}", (BigInt::from(1) << bits) - 1)
        };
        diags.push(
            Diagnostic::at(&codes::E0635, a.span, format!("{v} does not fit `{ty}`, the type of `{}`", tp.name))
                .note(format!("`{}` is a `{ty}`: {range}", tp.name))
                .help(format!("use a value in range, or declare `{}` with a wider type", tp.name)),
        );
        return None;
    }
    match v.to_i128() {
        Some(v) => Some(Type::val(ty, v)),
        None => {
            diags.push(Diagnostic::at(&codes::E0901, a.span, "a value above the largest `sint128` is not supported yet"));
            None
        }
    }
}

/// The types the arguments `args` of generic `name` stand for. `quiet` reports nothing.
pub(super) fn resolve_targs(g: &Globals, name: &str, tparams: &[ast::TypeParam], args: &[ast::TypeName], quiet: bool, diags: &mut Vec<Diagnostic>) -> Option<Vec<Type>> {
    let mut out = Vec::with_capacity(args.len());
    let mut ok = true;
    for (i, a) in args.iter().enumerate() {
        let ty = match tparams.get(i) {
            Some(tp) if tp.value.is_some() => {
                let mut own = Vec::new();
                let t = value_targ(g, name, tp, a, &mut own);
                if !quiet {
                    diags.extend(own);
                }
                t
            }
            _ if a.is_number => None,
            _ if quiet => g.resolve_quiet(a),
            _ => g.resolve(a, diags),
        };
        match ty {
            Some(t) => out.push(t),
            None => ok = false,
        }
    }
    ok.then_some(out)
}

/// The instance of generic function `name` for `types`, made on first use.
pub(super) fn make_fn_instance(g: &mut Globals, diags: &mut Vec<Diagnostic>, name: &str, types: Vec<Type>, call: Span) -> Option<FuncId> {
    let key = (name.to_string(), types);
    if let Some(&id) = g.mono.instances.get(&key) {
        return Some(id);
    }
    let types = key.1.clone();
    let list: Vec<String> = types.iter().map(written_arg).collect();
    let display = format!("{name}<{}>", list.join(", "));
    let depth = g.mono.depth + 1;
    if depth > MAX_DEPTH {
        diags.push(
            Diagnostic::at(&codes::E0608, call, format!("`{name}` keeps instantiating itself"))
                .note("instantiated here again with a bigger type")
                .note_line(format!("{MAX_DEPTH} instances deep, the last is `{display}`"))
                .help("make the recursive call use the same type parameters"),
        );
        return None;
    }
    let tps = g.mono.templates[name].head.tparams.clone();
    if !interfaces::check_bounds(g, name, &tps, &key.1, call, diags) {
        return None;
    }
    let t = g.mono.templates.remove(name).expect("a template");
    let tenv: HashMap<String, Type> = t.head.tparams.iter().map(|p| p.name.clone()).zip(types.iter().copied()).collect();
    let saved = std::mem::replace(&mut g.tenv, tenv.clone());
    let scope = g.mono.scopes.get(&format!("fn {name}")).cloned().unwrap_or_default();
    let saved_scope = std::mem::replace(&mut g.mono.scope, scope.clone());
    let mut local = Vec::new();
    ensure_written(g, &mut local);
    let params: Vec<Option<Type>> = t.head.params.iter().map(|p| g.resolve_binding_type(&p.ty, &mut local)).collect();
    let ret = g.resolve_return(&t.head.ret, &mut local);
    let (fails, outcome) = failures::signature(g, &t.head, ret, true, &mut local);
    let sig = Sig { name: display.clone(), params, ret, span: t.head.span, fails, outcome };
    check_signature_extras(g, &t.head, &sig.params, false, &mut local);
    g.tenv = saved;
    g.mono.scope = saved_scope;
    for d in local {
        if first_report(g, &d) {
            diags.push(instance_note(g, d, &display, call));
        }
    }
    let broken = sig.params.iter().any(Option::is_none) || sig.ret.is_none();
    let index = t.index;
    let id = g.add_sig(sig);
    g.tie_jobs.push(super::ties::Job {
        id,
        tparams: t.head.tparams.iter().map(|p| p.name.clone()).collect(),
        has_self: false,
        owner: None,
        params: t.head.params.iter().map(|p| p.ty.clone()).collect(),
        ret: t.head.ret.clone(),
    });
    g.mono.templates.insert(name.to_string(), t);
    g.mono.instances.insert(key, id);
    if !broken {
        let symbol = format!("drachm_g_{}", types::mangle(&display));
        g.mono.pending.push_back(Pending { body: Body::Func(index), id, scope, tenv, depth, call, display, symbol });
    }
    Some(id)
}

// ---- generic structs and enums -------------------------------------------

impl State {
    /// The `defaults` clause of generic struct `name`, if it has one.
    pub fn struct_defaults(&self, name: &str) -> Option<(String, Span)> {
        match &self.type_templates.get(name)?.head {
            Head::Struct(s) => s.defaults.clone(),
            Head::Enum(_) => None,
        }
    }
}

/// Sorts the types written with arguments into the template that contains
/// them (by position) or the global list.
pub(super) fn split_written(g: &mut Globals, prog: &ast::Program) {
    let mut ranges: Vec<(usize, usize, String)> = Vec::new();
    for f in prog.funcs.iter().filter(|f| !f.tparams.is_empty()) {
        ranges.push((f.span.start, f.body.end.end, format!("fn {}", f.name)));
    }
    for s in prog.structs.iter().filter(|s| !s.tparams.is_empty()) {
        ranges.push((s.span.start, s.span.end, format!("type {}", s.name)));
    }
    for e in prog.enums.iter().filter(|e| !e.tparams.is_empty()) {
        ranges.push((e.span.start, e.span.end, format!("type {}", e.name)));
    }
    let mut global = Vec::new();
    let mut scopes: HashMap<String, Vec<ast::TypeName>> = HashMap::new();
    for t in &prog.generics {
        match ranges.iter().find(|(a, b, _)| t.span.start >= *a && t.span.start <= *b) {
            Some((_, _, key)) => scopes.entry(key.clone()).or_default().push(t.clone()),
            None => global.push(t.clone()),
        }
    }
    g.mono.global = Rc::new(global);
    g.mono.scopes = scopes.into_iter().map(|(k, v)| (k, Rc::new(v))).collect();
}

/// E0202 when a struct or enum called `name` clashes with a generic type.
pub(super) fn type_name_taken(g: &Globals, name: &str, span: Span, _what: &str, diags: &mut Vec<Diagnostic>) -> bool {
    if let Some(first) = g.mono.type_templates.get(name) {
        diags.push(
            Diagnostic::at(&codes::E0202, span, format!("`{name}` is defined twice"))
                .note("defined again here")
                .label(first.name_span, "first defined here"),
        );
        return true;
    }
    false
}

fn declare_type_template(g: &mut Globals, kind: Kind, index: usize, name: &str, name_span: Span, tparams: &[ast::TypeParam], head: Head, diags: &mut Vec<Diagnostic>) {
    if name == "none" || super::maps::is_map_name(name) || !matches!(Type::from_name(name), Err(types::TypeError::Unknown)) {
        diags.push(
            Diagnostic::at(&codes::E0202, name_span, format!("`{name}` is the name of a built-in type"))
                .note("already a type")
                .help("give it another name"),
        );
        return;
    }
    let first = g
        .mono
        .type_templates
        .get(name)
        .map(|t| t.name_span)
        .or_else(|| g.structs_by_name.get(name).map(|&s| g.structs[s].name_span))
        .or_else(|| g.enums_by_name.get(name).map(|&e| g.enums[e].name_span));
    if let Some(first) = first {
        diags.push(Diagnostic::at(&codes::E0202, name_span, format!("`{name}` is defined twice")).note("defined again here").label(first, "first defined here"));
        return;
    }
    g.mono.type_templates.insert(name.to_string(), TypeTemplate { kind, index, name_span, tparams: tparams.to_vec(), usable: true, instances: 0, head });
}

pub(super) fn declare_struct_template(g: &mut Globals, index: usize, s: &ast::Struct, diags: &mut Vec<Diagnostic>) -> Option<usize> {
    let head = ast::Struct {
        name: s.name.clone(),
        name_span: s.name_span,
        tparams: s.tparams.clone(),
        span: s.span,
        with: s.with.clone(),
        lives: s.lives.clone(),
        stays: s.stays,
        defaults: s.defaults.clone(),
        auto_ctor: s.auto_ctor,
        fields: s.fields.clone(),
        methods: s
            .methods
            .iter()
            .map(|m| ast::Method { public: m.public, is_static: m.is_static, operator: m.operator, ctor: m.ctor, func: func_head(&m.func) })
            .collect(),
    };
    declare_type_template(g, Kind::Struct, index, &s.name, s.name_span, &s.tparams, Head::Struct(Rc::new(head)), diags);
    None
}

pub(super) fn declare_enum_template(g: &mut Globals, index: usize, e: &ast::Enum, diags: &mut Vec<Diagnostic>) -> Option<usize> {
    let head = ast::Enum { name: e.name.clone(), name_span: e.name_span, tparams: e.tparams.clone(), span: e.span, variants: e.variants.clone() };
    declare_type_template(g, Kind::Enum, index, &e.name, e.name_span, &e.tparams, Head::Enum(Rc::new(head)), diags);
    None
}

/// Checks the type parameter lists of the generic structs and enums, once
/// every type name exists.
pub(super) fn validate_type_templates(g: &mut Globals, diags: &mut Vec<Diagnostic>) {
    let mut names: Vec<(usize, String)> = g.mono.type_templates.iter().map(|(n, t)| (t.name_span.start, n.clone())).collect();
    names.sort();
    for (_, n) in names {
        let params = g.mono.type_templates[&n].tparams.clone();
        let ok = check_tparams(g, &params, diags);
        g.mono.type_templates.get_mut(&n).unwrap().usable = ok;
    }
}

fn user_optional(g: &Globals) -> bool {
    g.structs_by_name.contains_key("Optional") || g.enums_by_name.contains_key("Optional")
}

/// Is `t` (written with arguments) a use of a generic type? `Some(true)`:
/// yes and the arguments fit; `Some(false)`: it was wrong and is reported;
/// `None`: not a generic type.
pub(super) fn check_type_args(g: &Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<bool> {
    let at = t.span.to(t.args.last().map_or(t.span, |a| a.span));
    if t.name == "Optional" && !user_optional(g) {
        if t.args.len() != 1 || t.args[0].is_number {
            diags.push(Diagnostic::at(&codes::E0602, at, "`Optional` takes 1 type argument").note("type arguments written here").help("write `Optional<uint32>`, or `uint32?`"));
            return Some(false);
        }
        if t.optional.is_some() {
            diags.push(Diagnostic::at(&codes::E0901, t.span, "an optional of an optional is not supported yet"));
            return Some(false);
        }
        return Some(true);
    }
    let tpl = g.mono.type_templates.get(&t.name)?;
    if !tpl.usable {
        return Some(false);
    }
    let n = tpl.tparams.len();
    if t.args.len() != n {
        diags.push(
            Diagnostic::at(&codes::E0602, at, format!("`{}` takes {n} type argument{}, but {} given", t.name, if n == 1 { "" } else { "s" }, t.args.len()))
                .note("type arguments written here")
                .label(tpl.name_span, format!("`{}` is declared with {n} here", t.name)),
        );
        return Some(false);
    }
    if let Some((p, a)) = tpl.tparams.iter().zip(&t.args).find(|(p, a)| kind_mismatch(g, p, a)) {
        diags.push(unit_kind_error(&t.name, p, a));
        return Some(false);
    }
    Some(true)
}

/// E0604: a generic type written without its arguments.
pub(super) fn type_by_name(g: &Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) {
    let tpl = &g.mono.type_templates[&t.name];
    if !tpl.usable {
        return;
    }
    let params: Vec<&str> = tpl.tparams.iter().map(|p| p.name.as_str()).collect();
    let sample: Vec<&str> = params.iter().map(|_| "uint32").collect();
    diags.push(
        Diagnostic::at(&codes::E0604, t.span, format!("`{}` is generic, so it needs type arguments", t.name))
            .note(format!("`{}` alone is not a type", t.name))
            .label(tpl.name_span, format!("`{}` is declared with `<{}>` here", t.name, params.join(", ")))
            .help(format!("say what they are: `{}<{}>`", t.name, sample.join(", "))),
    );
}

/// The instance `t` names was not made: report why, or say it is not supported.
pub(super) fn missing_instance(g: &Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) {
    let types = match g.mono.type_templates.get(&t.name) {
        Some(tpl) => resolve_targs(g, &t.name, &tpl.tparams, &t.args, false, diags),
        None => t.args.iter().map(|a| g.resolve(a, diags)).collect::<Option<Vec<_>>>(),
    };
    let Some(types) = types else { return };
    if t.name == "Optional" && types.first().is_some_and(|x| x.is_borrow()) {
        return;
    }
    if g.mono.refused.contains(&(t.name.clone(), types)) || g.mono.type_templates.get(&t.name).is_some_and(|x| !x.usable) {
        return;
    }
    diags.push(Diagnostic::at(&codes::E0901, t.span, format!("`{}` cannot be made here yet", t.name)));
}

/// Prepare a type written outside the ordinary parsed type scopes.
pub(super) fn prepare_type(g: &mut Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) {
    for arg in &t.args {
        prepare_type(g, arg, diags);
    }
    ensure_type(g, t, diags);
    if t.optional.is_some() {
        let inner = ast::TypeName { optional: None, mods: Vec::new(), ..t.clone() };
        optionals::declare_optionals(g, &[inner]);
    }
}

/// Makes the instance a type written with arguments names, and the ones
/// inside its arguments.
fn ensure_type(g: &mut Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) {
    if t.args.is_empty() || t.is_number {
        return;
    }
    for a in &t.args {
        ensure_type(g, a, diags);
    }
    let args: Option<Vec<Type>> = match g.mono.type_templates.get(&t.name) {
        Some(tpl) => resolve_targs(g, &t.name, &tpl.tparams, &t.args, true, &mut Vec::new()),
        None => t.args.iter().map(|a| if a.is_number { None } else { g.resolve_quiet(a) }).collect(),
    };
    let Some(args) = args else { return };
    if t.name == "Optional" && !user_optional(g) {
        if let [inner] = args[..] {
            let _ = inner;
            optionals::declare_optionals(g, &t.args[..1]);
        }
        return;
    }
    let Some(tpl) = g.mono.type_templates.get(&t.name) else { return };
    if !tpl.usable || tpl.tparams.len() != args.len() {
        return;
    }
    // A type where a unit goes (or the reverse) is reported when it is resolved.
    if tpl.tparams.iter().zip(&t.args).any(|(p, a)| kind_mismatch(g, p, a)) {
        return;
    }
    let key = (t.name.clone(), args.clone());
    if g.mono.type_instances.contains_key(&key) || g.mono.refused.contains(&key) {
        return;
    }
    instantiate_type(g, &t.name, args, t.span, diags);
}

/// The instance of generic struct or enum `name` for `types`, made on first use.
pub(super) fn instantiate_type(g: &mut Globals, name: &str, types: Vec<Type>, at: Span, diags: &mut Vec<Diagnostic>) -> Option<Type> {
    let key = (name.to_string(), types.clone());
    if let Some(&t) = g.mono.type_instances.get(&key) {
        return Some(t);
    }
    let list: Vec<String> = types.iter().map(written_arg).collect();
    let display = format!("{name}<{}>", list.join(", "));
    if g.mono.type_depth + 1 > MAX_DEPTH {
        g.mono.refused.insert(key);
        diags.push(
            Diagnostic::at(&codes::E0608, at, format!("`{name}` keeps instantiating itself"))
                .note("instantiated here again with a bigger type")
                .note_line(format!("{MAX_DEPTH} instances deep, the last is `{display}`"))
                .help("make the field or method use the same type parameters"),
        );
        return None;
    }
    let tps = g.mono.type_templates.get(name)?.tparams.clone();
    if !interfaces::check_type_bounds(g, name, &tps, &types, at, &display, diags) {
        g.mono.refused.insert(key);
        return None;
    }
    let (index, tparams, head) = {
        let t = g.mono.type_templates.get_mut(name)?;
        t.instances += 1;
        let head = match &t.head {
            Head::Struct(s) => Head::Struct(s.clone()),
            Head::Enum(e) => Head::Enum(e.clone()),
        };
        (t.index, t.tparams.clone(), head)
    };
    let mut tenv: HashMap<String, Type> = tparams.iter().map(|p| p.name.clone()).zip(types.iter().copied()).collect();
    let scope = g.mono.scopes.get(&format!("type {name}")).cloned().unwrap_or_default();
    let mut local = Vec::new();
    let ty;
    match &head {
        Head::Struct(s) => {
            let id = new_struct_info(g, &display, s);
            ty = g.structs[id].ty;
            tenv.insert(SELF.to_string(), ty);
            g.mono.type_instances.insert(key.clone(), ty);
            g.mono.origin.insert(ty, key);
            let saved = std::mem::replace(&mut g.tenv, tenv.clone());
            let saved_scope = std::mem::replace(&mut g.mono.scope, scope.clone());
            g.mono.type_depth += 1;
            ensure_written(g, &mut local);
            super::declare_fields(g, id, s, &mut local);
            check_one_struct_lives(g, id, s, &mut local);
            let ids = declare_methods(g, id, s, &mut local);
            interfaces::on_instance(g, ty, &s.with, &display, at, &mut local);
            g.mono.type_depth -= 1;
            g.tenv = saved;
            g.mono.scope = saved_scope;
            for (j, mid) in ids.into_iter().enumerate() {
                let Some(mid) = mid else { continue };
                let m = &s.methods[j];
                g.tie_jobs.push(super::ties::Job {
                    id: mid,
                    tparams: tparams.iter().map(|p| p.name.clone()).collect(),
                    has_self: m.is_static.is_none() && !m.ctor,
                    owner: Some((ty, s.fields.iter().map(|f| (f.name.clone(), f.ty.clone())).collect())),
                    params: m.func.params.iter().map(|p| p.ty.clone()).collect(),
                    ret: m.func.ret.clone(),
                });
                let ctx = MethodCtx { struct_id: id, func: mid, static_span: m.is_static, ctor: m.ctor, no_fails: false, ext: None, trusted: true };
                let symbol = method_symbol(&display, m, j, &s.methods);
                g.mono.pending.push_back(Pending {
                    body: Body::Method { si: index, mi: j, ctx, symbol: symbol.clone(), id: mid },
                    id: mid,
                    scope: scope.clone(),
                    tenv: tenv.clone(),
                    depth: 0,
                    call: at,
                    display: display.clone(),
                    symbol,
                });
            }
        }
        Head::Enum(e) => {
            let id = g.enums.len();
            ty = Type::Enum { id, name: types::intern(&display) };
            g.enums.push(super::enums::EnumInfo { name: display.clone(), name_span: e.name_span, ty, variants: Vec::new(), kind: hir::EnumKind::User, owns: false });
            g.mono.type_instances.insert(key.clone(), ty);
            g.mono.origin.insert(ty, key);
            let saved = std::mem::replace(&mut g.tenv, tenv.clone());
            let saved_scope = std::mem::replace(&mut g.mono.scope, scope.clone());
            g.mono.type_depth += 1;
            ensure_written(g, &mut local);
            super::enums::declare_variants(g, id, e, &mut local);
            g.mono.type_depth -= 1;
            g.tenv = saved;
            g.mono.scope = saved_scope;
        }
    }
    for d in local {
        // Inside another instantiation the outer one decides what is repeated.
        if g.mono.type_depth > 0 {
            diags.push(d);
        } else if first_report(g, &d) {
            diags.push(instance_note(g, d, &display, at));
        }
    }
    Some(ty)
}

impl Checker<'_> {
    /// The name of a generic struct or enum where a type or a value is
    /// wanted, written without arguments.
    pub(super) fn generic_type_alone(&mut self, name: &str, span: Span) {
        let t = ast::TypeName::plain(name.to_string(), span);
        type_by_name(self.g, &t, self.diags);
    }

    /// The instance `name<targs>` names.
    fn instance_for(&mut self, name: &str, targs: &[ast::TypeName], span: Span) -> Option<Type> {
        let t = ast::TypeName { args: targs.to_vec(), ..ast::TypeName::plain(name.to_string(), span) };
        self.g.resolve(&t, self.diags)
    }

    /// `new Stack<uint32> of (...)`.
    pub(super) fn new_generic(&mut self, name: &str, name_span: Span, targs: &[ast::TypeName], values: &[ast::FieldValue], span: Span) -> Option<Val> {
        let ty = self.instance_for(name, targs, name_span);
        match ty.and_then(Type::struct_id) {
            Some(s) => self.new_struct(s, values, span),
            None => {
                if let Some(t) = ty {
                    self.diags.push(Diagnostic::at(&codes::E0101, name_span, format!("`new` builds a struct, and `{t}` is not one")));
                }
                for v in values {
                    self.expr(&v.value);
                }
                None
            }
        }
    }

    /// `Stack<uint32>::name(args)`: a static method of the instance, or a
    /// variant of a generic enum.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn generic_static(&mut self, tname: &str, ttargs: &[ast::TypeName], tspan: Span, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        if !self.g.mono.type_templates.contains_key(tname) {
            self.not_generic(tname, ttargs, args.iter().collect(), span);
            return None;
        }
        let ty = self.instance_for(tname, ttargs, tspan);
        let fail = |cx: &mut Self| {
            for a in args {
                cx.expr(a);
            }
            None
        };
        match ty {
            Some(Type::Struct { id, .. }) => self.static_call(id, name, name_span, targs, args, span),
            Some(Type::Enum { id, .. }) => self.qualified_variant(id, name, name_span, args, span),
            _ => fail(self),
        }
    }
}

// ---- the definition pass ---------------------------------------------------

/// Is `t` (or what it points at) a struct or enum made after `ns`, `ne` existed?
fn is_new(t: Type, ns: usize, ne: usize) -> bool {
    match t {
        Type::Struct { id, .. } => id >= ns,
        Type::Enum { id, .. } => id >= ne,
        Type::Ref { to, .. } | Type::Ptr { to } | Type::FixedPtr { to } => is_new(*to, ns, ne),
        Type::Unit { of, .. } => is_new(*of, ns, ne),
        _ => false,
    }
}

/// What to put back after a template was checked with stand-ins.
struct Mark {
    sigs: usize,
    structs: usize,
    enums: usize,
    wrappers: usize,
    records: usize,
    interfaces: usize,
    pending: VecDeque<Pending>,
    counts: Vec<(String, usize)>,
    type_counts: Vec<(String, usize)>,
}

fn mark(g: &mut Globals) -> Mark {
    Mark {
        sigs: g.sigs.len(),
        structs: g.structs.len(),
        enums: g.enums.len(),
        wrappers: g.fail.drop_wrappers.len(),
        records: g.ifaces.records.len(),
        interfaces: g.ifaces.list.len(),
        pending: std::mem::take(&mut g.mono.pending),
        counts: g.mono.templates.iter().map(|(n, t)| (n.clone(), t.instances)).collect(),
        type_counts: g.mono.type_templates.iter().map(|(n, t)| (n.clone(), t.instances)).collect(),
    }
}

/// Forgets every function, struct and enum made since `m`, and the tables that point at them.
fn rollback(g: &mut Globals, m: Mark) {
    let (nf, ns, ne) = (m.sigs, m.structs, m.enums);
    g.sigs.truncate(nf);
    g.tie_jobs.retain(|j| j.id < nf);
    g.self_use.truncate(nf);
    g.structs.truncate(ns);
    g.enums.truncate(ne);
    g.optional_ids.retain(|k, &mut v| v < ne && !is_new(*k, ns, ne));
    g.eq_funcs.retain(|&s, &mut f| s < ns && f < nf);
    g.eq_pending.retain(|&s| s < ns);
    super::maps::rollback(g, nf, &|t| is_new(t, ns, ne));
    g.fail.info.retain(|&k, _| k < ne);
    g.fail.fns.retain(|&k, _| k < ne);
    g.fail.pending.retain(|&k| k < ne);
    g.fail.outcomes.retain(|(t, list), v| !is_new(*t, ns, ne) && list.iter().all(|&e| e < ne) && !is_new(*v, ns, ne));
    g.fail.sets.retain(|list, v| list.iter().all(|&e| e < ne) && !is_new(*v, ns, ne));
    g.fail.drop_wrappers.truncate(m.wrappers);
    g.ifaces.conforms.retain(|(t, k)| *k < m.interfaces && !is_new(*t, ns, ne));
    g.ifaces.instances.retain(|_, id| *id < m.interfaces);
    g.ifaces.list.truncate(m.interfaces);
    g.ifaces.records.truncate(m.records);
    g.ifaces.implementing.retain(|&f| f < nf);
    g.ifaces.changing.retain(|&f| f < nf);
    g.mono.instances.retain(|k, &mut v| v < nf && !k.1.iter().any(|t| is_new(*t, ns, ne)));
    g.mono.type_instances.retain(|k, v| !is_new(*v, ns, ne) && !k.1.iter().any(|t| is_new(*t, ns, ne)));
    g.mono.origin.retain(|k, _| !is_new(*k, ns, ne));
    g.mono.refused.retain(|k| !k.1.iter().any(|t| is_new(*t, ns, ne)));
    g.mono.pending = m.pending;
    g.mono.depth = 0;
    g.mono.type_depth = 0;
    for (n, c) in m.counts {
        if let Some(t) = g.mono.templates.get_mut(&n) {
            t.instances = c;
        }
    }
    for (n, c) in m.type_counts {
        if let Some(t) = g.mono.type_templates.get_mut(&n) {
            t.instances = c;
        }
    }
    g.reset_layout_answers();
}

fn mentions_name(d: &Diagnostic, name: &str) -> bool {
    let quoted = format!("`{name}`");
    let mentions = |text: &str| {
        text.contains(&quoted) || text.split('`').skip(1).step_by(2).any(|part| {
            part.contains('<') && part.split(|c: char| !(c.is_alphanumeric() || c == '_')).any(|word| word == name)
        })
    };
    if d.code.id == "E0606" {
        // Only the constrained name it is about counts; a loose parameter inside
        // its constraint (`R with Read<T>`) does not hide it.
        return d.message.split('`').nth(1) == Some(name);
    }
    mentions(&d.message) || d.labels.iter().any(|(_, t)| mentions(t))
}

/// Checks each generic that has a constrained type parameter once, where it is
/// written: its body is checked with each parameter standing for a struct that
/// has only what the parameter's interfaces promise. What it makes is thrown
/// away. A parameter with no constraint in the same list stands for a struct
/// with nothing, and what depends on it is left to the instances (Q1).
pub(super) fn definition_pass(g: &mut Globals, src: &str, prog: &ast::Program, diags: &mut Vec<Diagnostic>) {
    let bounded = |tps: &[ast::TypeParam]| tps.iter().any(|p| !p.bounds.is_empty());
    let mut jobs: Vec<(usize, String, bool)> = Vec::new();
    // A template with a unit parameter is not checked here: the definition
    // pass would need a stand-in unit for it. Its instances are checked.
    for (n, t) in &g.mono.templates {
        if t.usable && bounded(&t.head.tparams) && !has_unit_params(&t.head.tparams) {
            jobs.push((t.head.name_span.start, n.clone(), true));
        }
    }
    for (n, t) in &g.mono.type_templates {
        if t.usable && bounded(&t.tparams) && !has_unit_params(&t.tparams) {
            jobs.push((t.name_span.start, n.clone(), false));
        }
    }
    jobs.sort();
    for (_, name, is_fn) in jobs {
        let (tparams, at) = if is_fn {
            let t = &g.mono.templates[&name];
            (t.head.tparams.clone(), t.head.name_span)
        } else {
            let t = &g.mono.type_templates[&name];
            (t.tparams.clone(), t.name_span)
        };
        let m = mark(g);
        let types: Vec<Type> = tparams.iter().map(|tp| interfaces::placeholder(g, tp)).collect();
        let loose: Vec<&str> = tparams.iter().filter(|tp| tp.bounds.is_empty()).map(|tp| tp.name.as_str()).collect();
        g.mono.defn = Some(name.clone());
        let mut local = Vec::new();
        let env = tparams.iter().zip(&types).map(|(p, &ty)| (p.name.clone(), ty)).collect();
        let saved = std::mem::replace(&mut g.tenv, env);
        for (tp, &ty) in tparams.iter().zip(&types) {
            interfaces::populate_placeholder(g, ty, tp, &mut local);
        }
        g.tenv = saved;
        if is_fn {
            make_fn_instance(g, &mut local, &name, types, at);
        } else {
            instantiate_type(g, &name, types, at, &mut local);
        }
        drain(g, src, prog, &mut local, &mut |_, _| {});
        g.mono.defn = None;
        rollback(g, m);
        local.retain(|d| !loose.iter().any(|n| mentions_name(d, n)));
        if local.iter().any(Diagnostic::is_error) {
            g.mono.defn_failed.insert(name.clone());
        }
        diags.extend(local);
    }
}
