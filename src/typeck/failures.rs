//! Failures: `failure` declarations, `fails` in signatures, `fail`, the
//! `maybe` and `fails` suffixes on calls, the enums a failing function
//! returns, the functions that give a failure's message and exit code, and
//! the wrapper that reports a failure leaving `main`.
//!
//! A function that `fails` returns an *outcome*: an enum with `Ok(value)`
//! (tag 0, no field for `none`) and `Fail(failure)` (tag 1). Its `hir::Func`
//! has the outcome as `ret`; the checker keeps the written type in `Sig::ret`.
//! A function that lists several failure types returns a `FailSet`, an enum
//! with one variant for each. Every variant of a failure starts with the
//! hidden cause word, so it sits at the same offset in all of them.

use std::collections::HashMap;

use super::enums::{EnumInfo, VariantInfo};
use super::strings::text_of;
use super::{str_lit, Checker, FieldInfo, Globals, Sig, Val};
use crate::ast::{self, ExprKind, StrPart};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, EnumId, EnumKind, FuncId};
use crate::types::{self, Type};

/// One piece of a failure message.
pub(super) enum MsgPart {
    Text(String),
    /// The value with this index among the variant's own values.
    Value(usize),
}

pub(super) struct FailureVariant {
    pub message: Vec<MsgPart>,
    pub exit: u8,
}

/// Everything the checker knows about failures, beside the enums themselves.
#[derive(Default)]
pub(super) struct FailState {
    /// The messages and exit codes of each failure, by variant.
    pub info: HashMap<EnumId, Vec<FailureVariant>>,
    /// The functions `message(&F)` and `exit_code(&F)` of each failure and failure set.
    pub fns: HashMap<EnumId, (FuncId, FuncId)>,
    /// Failures and sets whose two functions are not built yet.
    pub pending: Vec<EnumId>,
    /// The outcome enum for a written type and a failure list.
    pub outcomes: HashMap<(Type, Vec<EnumId>), Type>,
    /// The failure-set enum for a failure list of two or more.
    pub sets: HashMap<Vec<EnumId>, Type>,
    /// The built-in `ConvertError`.
    pub convert_error: Option<EnumId>,
}

/// The failures every program has.
pub(super) const PRELUDE: &str = r#"
failure ConvertError {
    Empty      "there is no number in the text",
    NotANumber "the text is not a number",
    OutOfRange "the number does not fit the type",
}
"#;

const NOWHERE: Span = Span { start: 0, end: 0 };

fn expr(kind: hir::ExprKind, ty: Type) -> hir::Expr {
    hir::Expr::new(kind, ty)
}

fn local(id: usize, ty: Type) -> hir::Expr {
    expr(hir::ExprKind::Local(id), ty)
}

fn payload(value: hir::Expr, variant: usize, field: usize, ty: Type) -> hir::Expr {
    expr(hir::ExprKind::Payload { value: Box::new(value), variant, field }, ty)
}

fn new_enum(g: &mut Globals, name: String, kind: EnumKind, variants: Vec<VariantInfo>) -> EnumId {
    let id = g.enums.len();
    let ty = Type::Enum { id, name: types::intern(&name) };
    g.enums.push(EnumInfo { name, name_span: NOWHERE, ty, variants, kind, owns: true });
    id
}

fn plain_field(name: &str, ty: Type) -> FieldInfo {
    FieldInfo { name: name.to_string(), ty: Some(ty), mutable: false, public: true, span: NOWHERE }
}

/// Register the name of failure `f` and its two hidden functions. The
/// variants wait until every type name exists.
pub(super) fn declare_failure(g: &mut Globals, f: &ast::Failure, diags: &mut Vec<Diagnostic>) -> Option<EnumId> {
    if f.name == "none" || !matches!(Type::from_name(&f.name), Err(types::TypeError::Unknown)) {
        diags.push(
            Diagnostic::at(&codes::E0202, f.name_span, format!("`{}` is the name of a built-in type", f.name))
                .note("already a type")
                .help("give the failure another name"),
        );
        return None;
    }
    let first = g.structs_by_name.get(&f.name).map(|&s| g.structs[s].name_span).or_else(|| g.enums_by_name.get(&f.name).map(|&o| g.enums[o].name_span));
    if let Some(first) = first {
        diags.push(
            Diagnostic::at(&codes::E0202, f.name_span, format!("`{}` is already the name of a struct, an enum or a failure", f.name))
                .note("defined again here")
                .label(first, "first defined here"),
        );
        return None;
    }
    let id = g.enums.len();
    let ty = Type::Enum { id, name: types::intern(&f.name) };
    g.enums.push(EnumInfo { name: f.name.clone(), name_span: f.name_span, ty, variants: Vec::new(), kind: EnumKind::Failure, owns: true });
    g.enums_by_name.insert(f.name.clone(), id);
    add_fns(g, id, &f.name, f.name_span);
    Some(id)
}

fn add_fns(g: &mut Globals, id: EnumId, name: &str, span: Span) {
    let m = g.add_sig(Sig { name: format!("{name}::message"), params: Vec::new(), ret: Some(Type::Str), span, fails: Vec::new(), outcome: None });
    let x = g.add_sig(Sig { name: format!("{name}::exit_code"), params: Vec::new(), ret: Some(Type::U8), span, fails: Vec::new(), outcome: None });
    g.fail.fns.insert(id, (m, x));
    g.fail.pending.push(id);
}

/// Resolve the variants of failure `id`: the hidden cause, the values, the
/// message and the exit code.
pub(super) fn declare_variants(g: &mut Globals, id: EnumId, f: &ast::Failure, diags: &mut Vec<Diagnostic>) {
    if f.variants.is_empty() {
        diags.push(
            Diagnostic::at(&codes::E0554, f.name_span, format!("failure `{}` has no variants", f.name))
                .note("a failure needs at least one variant")
                .help(format!("write one: `failure {} {{ Failed \"what went wrong\" }}`", f.name)),
        );
    }
    let mut infos = Vec::new();
    for v in &f.variants {
        if let Some(first) = g.enums[id].variants.iter().find(|o| o.name == v.name) {
            diags.push(
                Diagnostic::at(&codes::E0520, v.name_span, format!("`{}` has two variants named `{}`", f.name, v.name))
                    .note("defined again here")
                    .label(first.name_span, "first defined here")
                    .help("rename one of them"),
            );
            continue;
        }
        let mut fields = vec![plain_field(hir::CAUSE_FIELD, Type::U64)];
        for pf in &v.fields {
            let ty = g.resolve_binding_type(&pf.ty, diags);
            let bad = ty.filter(|t| t.is_borrow() || t.is_ptr() || g.holds_borrow(*t) || !(*t == Type::Str || *t == Type::Bool || t.is_numeric()) || t.mentions_unit());
            if let Some(t) = bad {
                diags.push(
                    Diagnostic::at(&codes::E0901, pf.ty.span, "a failure's values are numbers, bools and strings for now")
                        .note(format!("`{}` holds a `{t}`", pf.name))
                        .help("hold the value in a number, a bool or a string"),
                );
                fields.push(FieldInfo { name: pf.name.clone(), ty: None, mutable: false, public: true, span: pf.name_span });
                continue;
            }
            if let Some(first) = fields.iter().find(|o| o.name == pf.name) {
                diags.push(
                    Diagnostic::at(&codes::E0202, pf.name_span, format!("`{}` has two values named `{}`", v.name, pf.name))
                        .note("defined again here")
                        .label(first.span, "first defined here"),
                );
                continue;
            }
            fields.push(FieldInfo { name: pf.name.clone(), ty, mutable: false, public: true, span: pf.name_span });
        }
        let message = message_parts(v, &fields, diags);
        let exit = match v.exit {
            Some((n, span)) if n > 255 => {
                diags.push(
                    Diagnostic::at(&codes::E0552, span, format!("exit code `{n}` is out of range"))
                        .note("exit codes are 0 to 255")
                        .help("write a code from 0 to 255"),
                );
                1
            }
            Some((n, _)) => n as u8,
            None => 1,
        };
        infos.push(FailureVariant { message, exit });
        g.enums[id].variants.push(VariantInfo { name: v.name.clone(), name_span: v.name_span, fields });
    }
    g.fail.info.insert(id, infos);
}

/// The message of variant `v` as text and values. `fields` starts with the
/// hidden cause.
fn message_parts(v: &ast::FailVariant, fields: &[FieldInfo], diags: &mut Vec<Diagnostic>) -> Vec<MsgPart> {
    let ExprKind::Str(parts) = &v.message.kind else { return Vec::new() };
    let mut out = Vec::new();
    let mut text = String::new();
    for part in parts {
        match part {
            StrPart::Raw(t, _) => text.push_str(t),
            StrPart::Escape(c, _) => text.push(*c),
            StrPart::Interp(e) => {
                let ExprKind::Var(name) = &e.kind else {
                    diags.push(
                        Diagnostic::at(&codes::E0901, e.span, "a failure message can only use the names of the variant's values for now")
                            .note("not a value name")
                            .help("write `${name}` with the name of one of the values"),
                    );
                    continue;
                };
                match fields.iter().skip(1).position(|f| &f.name == name) {
                    Some(i) => {
                        if !text.is_empty() {
                            out.push(MsgPart::Text(std::mem::take(&mut text)));
                        }
                        out.push(MsgPart::Value(i));
                    }
                    None => {
                        let have: Vec<String> = fields.iter().skip(1).map(|f| format!("`{}`", f.name)).collect();
                        let help = if have.is_empty() { format!("`{}` has no values", v.name) } else { format!("its values are {}", super::enums::join_and(&have)) };
                        diags.push(
                            Diagnostic::at(&codes::E0551, e.span, format!("the message of `{}` uses `${{{name}}}`, but it has no such value", v.name))
                                .note(format!("no value `{name}`"))
                                .label(v.name_span, format!("`{}` is declared here", v.name))
                                .help(help),
                        );
                    }
                }
            }
        }
    }
    if !text.is_empty() {
        out.push(MsgPart::Text(text));
    }
    out
}

/// The failure types and outcome of a function written `f`, whose written
/// return type is `ret`. Reports problems with the list. `allowed` is false
/// for the functions that may not fail yet (reported by the caller).
pub(super) fn signature(g: &mut Globals, f: &ast::Func, ret: Option<Type>, allowed: bool, diags: &mut Vec<Diagnostic>) -> (Vec<EnumId>, Option<Type>) {
    let mut ids: Vec<EnumId> = Vec::new();
    if !allowed {
        return (ids, None);
    }
    for (name, span) in &f.fails {
        match g.enums_by_name.get(name).copied() {
            Some(id) if g.enums[id].kind == EnumKind::Failure => {
                if ids.contains(&id) {
                    diags.push(
                        Diagnostic::at(&codes::E0554, *span, format!("`{}` lists `{name}` twice", f.name))
                            .note("listed again here")
                            .help("remove one"),
                    );
                } else {
                    ids.push(id);
                }
            }
            other => {
                let what = if other.is_some() || g.structs_by_name.contains_key(name) { "is not a failure type" } else { "is not a type" };
                let help = if what == "is not a type" {
                    format!("declare it: `failure {name} {{ Failed \"what went wrong\" }}`")
                } else {
                    "only a `failure` can be listed after `fails`".to_string()
                };
                diags.push(Diagnostic::at(&codes::E0540, *span, format!("`{name}` {what}")).note("not a failure").help(help));
            }
        }
    }
    let Some(ret) = ret else { return (ids, None) };
    if ids.is_empty() {
        return (ids, None);
    }
    if ret.is_borrow() || ret.is_ptr() {
        diags.push(Diagnostic::at(&codes::E0901, f.ret.span, "a function that fails cannot return a borrow or a pointer yet").note("returns a borrow"));
        return (Vec::new(), None);
    }
    let outcome = outcome_type(g, ret, &ids);
    (ids, Some(outcome))
}

/// The enum a function returning `ok` and failing with `ids` returns.
pub(super) fn outcome_type(g: &mut Globals, ok: Type, ids: &[EnumId]) -> Type {
    let key = (ok, ids.to_vec());
    if let Some(&t) = g.fail.outcomes.get(&key) {
        return t;
    }
    let fail = if ids.len() == 1 { g.enums[ids[0]].ty } else { failure_set(g, ids) };
    let ok_fields = if ok == Type::None { Vec::new() } else { vec![plain_field("value", ok)] };
    let variants = vec![
        VariantInfo { name: "Ok".into(), name_span: NOWHERE, fields: ok_fields },
        VariantInfo { name: "Fail".into(), name_span: NOWHERE, fields: vec![plain_field("failure", fail)] },
    ];
    let name = format!("{ok} or {fail}");
    let id = new_enum(g, name, EnumKind::Outcome { ok, fail }, variants);
    let t = g.enums[id].ty;
    g.fail.outcomes.insert(key, t);
    t
}

fn failure_set(g: &mut Globals, ids: &[EnumId]) -> Type {
    if let Some(&t) = g.fail.sets.get(ids) {
        return t;
    }
    let variants = ids
        .iter()
        .map(|&i| VariantInfo { name: g.enums[i].name.clone(), name_span: NOWHERE, fields: vec![plain_field("failure", g.enums[i].ty)] })
        .collect();
    let names: Vec<String> = ids.iter().map(|&i| g.enums[i].name.clone()).collect();
    let name = names.join(" or ");
    let id = new_enum(g, name.clone(), EnumKind::FailSet, variants);
    add_fns(g, id, &name, NOWHERE);
    let t = g.enums[id].ty;
    g.fail.sets.insert(ids.to_vec(), t);
    t
}

// ---- the synthesised functions ------------------------------------------------

fn synth(symbol: String, ret: Type, self_ty: Type, body: hir::Block) -> hir::Func {
    hir::Func {
        symbol,
        params: 1,
        has_self: true,
        locals: vec![hir::Local { name: "e".into(), ty: self_ty, span: NOWHERE, mutability: hir::Mutability::Fixed, is_param: true, written: None }],
        ret,
        body,
        name_span: NOWHERE,
        span: NOWHERE,
        body_end: NOWHERE,
        is_unsafe: false,
        lives: None,
        lives_span: None,
    }
}

/// `if tag == 0 { return a } else if tag == 1 { return b } else { return c }`:
/// one `Return` for each variant.
fn dispatch(g: &Globals, ty: Type, arms: Vec<hir::Expr>) -> hir::Block {
    let n = arms.len();
    if n == 1 {
        return vec![hir::Stmt::Return(arms.into_iter().next())];
    }
    let tag_ty = crate::mir::ty::tag_type(hir::tag_size_for(g.enums[ty.enum_id().unwrap()].variants.len()));
    let branches = arms
        .into_iter()
        .enumerate()
        .map(|(vi, value)| {
            let test = (vi + 1 < n).then(|| {
                let tag = expr(hir::ExprKind::Discriminant(Box::new(local(0, ty))), tag_ty);
                let want = expr(hir::ExprKind::Int(vi as u128), tag_ty);
                expr(hir::ExprKind::Binary(hir::BinOp::Eq, Box::new(tag), Box::new(want)), Type::Bool)
            });
            hir::Branch { test, body: vec![hir::Stmt::Return(Some(value))], end: NOWHERE }
        })
        .collect();
    vec![hir::Stmt::If { label: 0, branches }]
}

/// The bodies of the `message` and `exit_code` functions still to build.
pub(super) fn finish(g: &mut Globals) -> Vec<(FuncId, hir::Func)> {
    let mut out = Vec::new();
    for id in std::mem::take(&mut g.fail.pending) {
        let (mf, xf) = g.fail.fns[&id];
        let ty = g.enums[id].ty;
        let (msgs, exits): (Vec<hir::Expr>, Vec<hir::Expr>) = if g.enums[id].kind == EnumKind::FailSet {
            let mut msgs = Vec::new();
            let mut exits = Vec::new();
            for (vi, v) in g.enums[id].variants.iter().enumerate() {
                let member = v.fields[0].ty.unwrap();
                let (m, x) = g.fail.fns[&member.enum_id().unwrap()];
                let arg = payload(local(0, ty), vi, 0, member);
                msgs.push(expr(hir::ExprKind::Call(m, vec![arg.clone()]), Type::Str));
                exits.push(expr(hir::ExprKind::Call(x, vec![arg]), Type::U8));
            }
            (msgs, exits)
        } else {
            let mut msgs = Vec::new();
            let mut exits = Vec::new();
            let infos = &g.fail.info[&id];
            for (vi, (v, info)) in g.enums[id].variants.iter().zip(infos).enumerate() {
                let mut pieces = Vec::new();
                for part in &info.message {
                    match part {
                        MsgPart::Text(t) => pieces.push(str_lit(t.clone())),
                        MsgPart::Value(i) => {
                            let fty = v.fields[i + 1].ty.unwrap_or(Type::Str);
                            let p = payload(local(0, ty), vi, i + 1, fty);
                            pieces.push(if fty == Type::Str { p } else { text_of(p) });
                        }
                    }
                }
                let msg = match pieces.len() {
                    0 => str_lit(String::new()),
                    1 if !pieces[0].is_place() => pieces.pop().unwrap(),
                    _ => expr(hir::ExprKind::Join(pieces), Type::Str),
                };
                msgs.push(msg);
                exits.push(expr(hir::ExprKind::Int(info.exit as u128), Type::U8));
            }
            (msgs, exits)
        };
        if msgs.is_empty() {
            // A failure with no variants was reported; keep the functions well formed.
            out.push((mf, synth(format!("drachm_fail_msg_{id}"), Type::Str, ty, vec![hir::Stmt::Return(Some(str_lit(String::new())))])));
            out.push((xf, synth(format!("drachm_fail_exit_{id}"), Type::U8, ty, vec![hir::Stmt::Return(Some(expr(hir::ExprKind::Int(1), Type::U8)))])));
            continue;
        }
        out.push((mf, synth(format!("drachm_fail_msg_{id}"), Type::Str, ty, dispatch(g, ty, msgs))));
        out.push((xf, synth(format!("drachm_fail_exit_{id}"), Type::U8, ty, dispatch(g, ty, exits))));
    }
    out
}

/// The function `main` really is when the written one fails: it calls it,
/// and on a failure prints the report and returns the failure's exit code.
pub(super) fn main_wrapper(g: &mut Globals, main: FuncId) -> Option<(FuncId, hir::Func)> {
    let sig = &g.sigs[main];
    let outcome = sig.outcome?;
    let ok = sig.ret.unwrap_or(Type::None);
    let EnumKind::Outcome { fail, .. } = g.enums[outcome.enum_id()?].kind else { return None };
    let (mf, xf) = *g.fail.fns.get(&fail.enum_id()?)?;
    let id = g.add_sig(Sig { name: "main".into(), params: Vec::new(), ret: Some(Type::S32), span: NOWHERE, fails: Vec::new(), outcome: None });
    let out = |i: usize| local(i, outcome);
    let failure = payload(out(0), 1, 0, fail);
    // The cause word is at the same place in every failure of the set.
    let first = if g.enums[fail.enum_id()?].kind == EnumKind::FailSet {
        let inner = g.enums[fail.enum_id()?].variants[0].fields[0].ty?;
        payload(failure.clone(), 0, 0, inner)
    } else {
        failure.clone()
    };
    let slot = payload(first, 0, 0, Type::U64);
    let place = |i: usize| hir::Place { local: i, proj: Vec::new(), span: NOWHERE };
    let tag_ty = crate::mir::ty::tag_type(hir::tag_size_for(2));
    let failed = expr(
        hir::ExprKind::Binary(
            hir::BinOp::Ne,
            Box::new(expr(hir::ExprKind::Discriminant(Box::new(out(0))), tag_ty)),
            Box::new(expr(hir::ExprKind::Int(0), tag_ty)),
        ),
        Type::Bool,
    );
    let code = expr(hir::ExprKind::Call(xf, vec![failure.clone()]), Type::U8);
    let on_failure = vec![
        hir::Stmt::Decl(1),
        hir::Stmt::Assign(place(1), expr(hir::ExprKind::Call(mf, vec![failure]), Type::Str)),
        hir::Stmt::Expr(expr(hir::ExprKind::Report { msg: Box::new(local(1, Type::Str)), slot: Box::new(slot) }, Type::None)),
        hir::Stmt::Return(Some(expr(hir::ExprKind::Convert(Box::new(code)), Type::S32))),
    ];
    let value = if ok == Type::S32 { payload(out(0), 0, 0, Type::S32) } else { expr(hir::ExprKind::Int(0), Type::S32) };
    let body = vec![
        hir::Stmt::Decl(0),
        hir::Stmt::Assign(place(0), expr(hir::ExprKind::Call(main, Vec::new()), outcome)),
        hir::Stmt::If { label: 0, branches: vec![hir::Branch { test: Some(failed), body: on_failure, end: NOWHERE }] },
        hir::Stmt::Return(Some(value)),
    ];
    let local_decl = |name: &str, ty, mutability| hir::Local { name: name.into(), ty, span: NOWHERE, mutability, is_param: false, written: None };
    let func = hir::Func {
        symbol: "drachm_main_wrapper".into(),
        params: 0,
        has_self: false,
        locals: vec![local_decl("outcome", outcome, hir::Mutability::Mut), local_decl("message", Type::Str, hir::Mutability::Fixed)],
        ret: Type::S32,
        body,
        name_span: NOWHERE,
        span: NOWHERE,
        body_end: NOWHERE,
        is_unsafe: false,
        lives: None,
        lives_span: None,
    };
    Some((id, func))
}

// ---- in function bodies ---------------------------------------------------------

/// A failing call, found under `maybe` or `fails`.
pub(super) struct Failing {
    /// The call, typed with its outcome (or the text parse, typed with its number).
    call: hir::Expr,
    /// The type of the call's value.
    ok: Type,
    fails: Vec<EnumId>,
    name: String,
}

impl Checker<'_> {
    /// `v` as the value of a `return` in this function: `Ok(v)` when it fails.
    pub(super) fn wrap_ok(&self, v: Option<hir::Expr>) -> Option<hir::Expr> {
        let Some(outcome) = self.outcome else { return v };
        Some(expr(hir::ExprKind::Variant { enum_id: outcome.enum_id().unwrap(), variant: 0, args: v.into_iter().collect() }, outcome))
    }

    /// The names of the failures of this function, for messages.
    fn fail_names(&self, ids: &[EnumId]) -> String {
        let names: Vec<String> = ids.iter().map(|&i| format!("`{}`", self.g.enums[i].name)).collect();
        super::enums::join_and(&names)
    }

    /// `fail X;`, `fail X(values);`, `fail Type::X;`, with `because e`.
    pub(super) fn fail_stmt(&mut self, value: &ast::Expr, because: Option<&ast::Expr>, span: Span, out: &mut hir::Block) {
        let Some(outcome) = self.outcome else {
            let help = "add `fails` and the failure type after the parameters: `none check() fails ParseError { ... }`";
            self.diags.push(
                Diagnostic::at(&codes::E0542, span, "`fail` is used in a function that does not say `fails`").note("the function fails here").help(help),
            );
            return;
        };
        let (owner, vname, args, parens): (Option<&String>, &String, &[ast::Expr], bool) = match &value.kind {
            ExprKind::Var(n) => (None, n, &[], false),
            ExprKind::Call { name, args } => (None, name, args, true),
            ExprKind::Path { owner, name, .. } => (Some(owner), name, &[], false),
            ExprKind::Method { recv, name, args, targs, .. } if targs.is_empty() && matches!(recv.kind, ExprKind::Var(_)) => {
                let ExprKind::Var(o) = &recv.kind else { unreachable!() };
                (Some(o), name, args, true)
            }
            _ => {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, value.span, "`fail` takes the name of a failure for now")
                        .note("not the name of a failure")
                        .help("write `fail Name;` or `fail Type::Name;`"),
                );
                return;
            }
        };
        let fails = self.fails.clone();
        let found: Option<(EnumId, usize)> = match owner {
            Some(o) => match self.g.enums_by_name.get(o).copied().filter(|&id| self.g.enums[id].kind == EnumKind::Failure) {
                Some(id) if fails.contains(&id) => self.find_variant(id, vname, value.span).map(|vi| (id, vi)),
                Some(id) => {
                    self.diags.push(
                        Diagnostic::at(&codes::E0541, value.span, format!("this function does not list `{}` after `fails`", self.g.enums[id].name))
                            .note("this failure is not in the list")
                            .help(format!("add it: `fails {}`", self.fail_list_with(id))),
                    );
                    None
                }
                None => {
                    self.diags.push(Diagnostic::at(&codes::E0541, value.span, format!("`{o}` is not a failure type")).note("not a failure"));
                    None
                }
            },
            None => {
                let owners: Vec<EnumId> = fails.iter().copied().filter(|&id| self.g.enums[id].variant(vname).is_some()).collect();
                match owners.as_slice() {
                    [id] => Some((*id, self.g.enums[*id].variant(vname).unwrap())),
                    [] => {
                        let listed = self.fail_names(&fails);
                        self.diags.push(
                            Diagnostic::at(&codes::E0541, value.span, format!("none of {listed} has a variant `{vname}`"))
                                .note("this failure is not in the list")
                                .help("fail with a variant of a listed failure, or list the failure that has it after `fails`"),
                        );
                        None
                    }
                    several => {
                        let choices: Vec<String> = several.iter().map(|&id| format!("`fail {}::{vname};`", self.g.enums[id].name)).collect();
                        self.diags.push(
                            Diagnostic::at(&codes::E0543, value.span, format!("`{vname}` is a variant of more than one failure this function lists"))
                                .note("ambiguous")
                                .help(format!("write {}", super::enums::join_or(&choices))),
                        );
                        None
                    }
                }
            }
        };
        let Some((fid, vi)) = found else {
            for a in args {
                self.expr(a);
            }
            if let Some(b) = because {
                self.expr(b);
            }
            return;
        };
        let Some(mut fv) = self.variant_expr(fid, vi, args, parens, value.span, value.span) else {
            if let Some(b) = because {
                self.expr(b);
            }
            return;
        };
        if let Some(b) = because {
            let Some(cause) = self.because(b, out) else { return };
            if let hir::ExprKind::Variant { args, .. } = &mut fv.kind {
                args[0] = cause;
            }
        }
        let EnumKind::Outcome { fail, .. } = self.g.enums[outcome.enum_id().unwrap()].kind else { unreachable!("an outcome") };
        let wrapped = if self.fails.len() > 1 {
            let k = self.fails.iter().position(|&i| i == fid).unwrap();
            expr(hir::ExprKind::Variant { enum_id: fail.enum_id().unwrap(), variant: k, args: vec![fv] }, fail)
        } else {
            fv
        };
        let result = expr(hir::ExprKind::Variant { enum_id: outcome.enum_id().unwrap(), variant: 1, args: vec![wrapped] }, outcome).at(span);
        out.push(hir::Stmt::Return(Some(result)));
    }

    /// The list after `fails` with `extra` added, for a fix-it.
    fn fail_list_with(&self, extra: EnumId) -> String {
        let mut names: Vec<&str> = self.fails.iter().map(|&i| self.g.enums[i].name.as_str()).collect();
        names.push(&self.g.enums[extra].name);
        names.join(", ")
    }

    /// `because e`: the cause word of a new failure. `e` is a failure value,
    /// moved into a hidden local whose message goes in front of its own
    /// causes.
    fn because(&mut self, e: &ast::Expr, out: &mut hir::Block) -> Option<hir::Expr> {
        let v = self.expr(e)?;
        let v = self.materialise(v, e.span)?;
        let is_failure = v.ty.enum_id().is_some_and(|id| self.g.enums[id].kind == EnumKind::Failure);
        if !is_failure {
            self.diags.push(
                Diagnostic::at(&codes::E0549, e.span, format!("`because` needs a failure, but this is a `{}`", v.ty))
                    .note("not a failure")
                    .help("`because` takes a failure value: `fail LoadFailed because cause;`"),
            );
            return None;
        }
        let ty = v.ty;
        let tmp = self.hidden_named("because", ty);
        out.push(hir::Stmt::Decl(tmp));
        out.push(hir::Stmt::Assign(hir::Place { local: tmp, proj: Vec::new(), span: e.span }, v));
        let (m, _) = self.g.fail.fns[&ty.enum_id().unwrap()];
        let msg = expr(hir::ExprKind::Call(m, vec![local(tmp, ty)]), Type::Str);
        let slot = payload(local(tmp, ty), 0, 0, Type::U64);
        Some(expr(hir::ExprKind::CauseWrap { msg: Box::new(msg), slot: Box::new(slot) }, Type::U64))
    }

    /// The call under a `fails`, with what it can fail with.
    fn failing_call(&mut self, call: &ast::Expr, word: &str) -> Option<Failing> {
        let saved = self.suffix_ok.replace(call as *const ast::Expr as usize);
        let v = self.expr_raw(call);
        self.suffix_ok = saved;
        let v = self.read(v?);
        let Val::Expr(x) = v else {
            self.cannot_fail(call, word);
            return None;
        };
        match self.classify(x) {
            Ok(f) => Some(f),
            Err(_) => {
                self.cannot_fail(call, word);
                None
            }
        }
    }

    /// `x` as a failing call, or itself back.
    pub(super) fn classify(&self, mut x: hir::Expr) -> Result<Failing, hir::Expr> {
        match x.kind {
            hir::ExprKind::Call(fid, _) if !self.g.sigs[fid].fails.is_empty() => {
                let sig = &self.g.sigs[fid];
                let (fails, name, outcome) = (sig.fails.clone(), sig.name.clone(), sig.outcome.unwrap());
                let ok = x.ty;
                x.ty = outcome;
                Ok(Failing { call: x, ok, fails, name })
            }
            hir::ExprKind::TryParse { to, fail, .. } => {
                let fails = vec![fail.enum_id().unwrap()];
                Ok(Failing { ok: to, call: x, fails, name: "to".into() })
            }
            _ => Err(x),
        }
    }

    fn cannot_fail(&mut self, call: &ast::Expr, word: &str) {
        self.diags.push(
            Diagnostic::at(&codes::E0003, call.span, format!("`{word}` needs a call that can fail"))
                .note("this cannot fail")
                .help("`maybe` and `fails` go right after a call to a function that says `fails`"),
        );
    }

    /// `call maybe fallback` for a call `f` that can fail.
    pub(super) fn maybe_failing(&mut self, f: Failing, fallback: &ast::Expr) -> Option<Val> {
        if f.ok == Type::None {
            self.diags.push(
                Diagnostic::at(&codes::E0003, fallback.span, format!("`{}` returns nothing, so there is no value to fall back to", f.name))
                    .note("no value")
                    .help("pass the failure up with `fails` instead"),
            );
            return None;
        }
        let fb = self.coerce(fallback, f.ok)?;
        Some(Val::Expr(expr(hir::ExprKind::FailMaybe { call: Box::new(f.call), fallback: Box::new(fb) }, f.ok)))
    }

    /// `call fails`: every failure of the call must be in this function's list.
    pub(super) fn pass_up_expr(&mut self, e: &ast::Expr, call: &ast::Expr) -> Option<Val> {
        let f = self.failing_call(call, "fails")?;
        if let Some(&missing) = f.fails.iter().find(|id| !self.fails.contains(id)) {
            let name = self.g.enums[missing].name.clone();
            let mut d = Diagnostic::at(&codes::E0548, e.span, format!("`{}` can fail with `{name}`, but this function does not list it", f.name))
                .note("the failure is passed up here");
            d = if self.fails.is_empty() {
                d.help(format!("add `fails {name}` after the parameters, or give a fallback with `maybe`"))
            } else {
                d.help(format!("add it: `fails {}`, or give a fallback with `maybe`", self.fail_list_with(missing)))
            };
            self.diags.push(d);
            return None;
        }
        Some(Val::Expr(expr(hir::ExprKind::PassUp { call: Box::new(f.call) }, f.ok)))
    }

    /// E0544: a call that can fail, with nothing after it.
    pub(super) fn check_unhandled(&mut self, e: &ast::Expr, x: &hir::Expr) {
        if self.suffix_ok == Some(e as *const ast::Expr as usize) {
            return;
        }
        // `a != b` with a failing `==` is the negation of the call.
        if let hir::ExprKind::Not(inner) = &x.kind
            && matches!(&inner.kind, hir::ExprKind::Call(fid, _) if !self.g.sigs[*fid].fails.is_empty())
        {
            self.diags.push(
                Diagnostic::at(&codes::E0901, e.span, "`!=` with an `==` that can fail is not supported yet")
                    .note("the `==` can fail here")
                    .help("write the negation yourself: `!((a == b) maybe false)`"),
            );
            return;
        }
        self.report_unhandled(e.span, x);
    }

    /// E0544 for the failing call `x`, if it is one.
    pub(super) fn report_unhandled(&mut self, span: Span, x: &hir::Expr) {
        let (name, fails): (String, Vec<EnumId>) = match &x.kind {
            hir::ExprKind::Call(fid, _) if !self.g.sigs[*fid].fails.is_empty() => (self.g.sigs[*fid].name.clone(), self.g.sigs[*fid].fails.clone()),
            hir::ExprKind::TryParse { fail, .. } => ("to".to_string(), vec![fail.enum_id().unwrap()]),
            _ => return,
        };
        let first = self.g.enums[fails[0]].name.clone();
        let text = short_call(self.src, span);
        let nothing = matches!(&x.kind, hir::ExprKind::Call(fid, _) if self.g.sigs[*fid].ret == Some(Type::None));
        let pass = format!("pass the failure up with `{text} fails` (and add `fails {first}` to the function)");
        let help = if nothing { pass } else { format!("give a fallback with `{text} maybe 0`, or {pass}") };
        self.diags.push(
            Diagnostic::at(&codes::E0544, span, format!("`{name}` can fail, and nothing says what to do when it does"))
                .note(format!("this call can fail with {}", self.fail_names(&fails)))
                .help(help),
        );
    }
}

fn short_call(src: &str, span: Span) -> &str {
    super::short(src, span)
}
