//! Generic functions by monomorphisation (S6A.1).
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

use std::collections::{HashMap, HashSet, VecDeque};

use super::{check_body, check_signature_extras, failures, optionals, origin, units, Checker, Globals, Sig, Val};
use crate::ast::{self, ExprKind};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, FuncId};
use crate::types::{self, Type};

/// How many instances may be nested: the body of one makes another.
pub(super) const MAX_DEPTH: usize = 64;

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

/// An instance whose body is still to be checked.
pub(super) struct Pending {
    index: usize,
    id: FuncId,
    tenv: HashMap<String, Type>,
    depth: usize,
    call: Span,
    display: String,
    symbol: String,
}

#[derive(Default)]
pub(super) struct State {
    pub templates: HashMap<String, Template>,
    instances: HashMap<(String, Vec<Type>), FuncId>,
    pending: VecDeque<Pending>,
    /// How deep the body being checked is in instances (0: not in one).
    depth: usize,
    /// Every `T?` the file writes, to make the optionals an instance names.
    pub written_optionals: Vec<ast::TypeName>,
    /// The (code, place) pairs already reported from instances.
    reported: HashSet<(&'static str, usize, usize)>,
}

/// Records function `f` (at `index` in the program) as a template.
pub(super) fn declare_template(g: &mut Globals, index: usize, f: &ast::Func, diags: &mut Vec<Diagnostic>) {
    let mut usable = true;
    if let Some(first) = g.funcs_by_name.get(&f.name).map(|&id| g.sigs[id].span).or_else(|| g.mono.templates.get(&f.name).map(|t| t.head.span)) {
        diags.push(
            Diagnostic::at(&codes::E0202, f.name_span, format!("function `{}` is defined twice", f.name))
                .note("defined again here")
                .label(first, "first defined here"),
        );
        return;
    }
    let mut seen: Vec<(&str, Span)> = Vec::new();
    for p in &f.tparams {
        if let Some((_, first)) = seen.iter().find(|(n, _)| *n == p.name) {
            diags.push(
                Diagnostic::at(&codes::E0605, p.span, format!("type parameter `{}` is declared twice", p.name))
                    .note("declared again here")
                    .label(*first, "first declared here")
                    .help("give each type parameter its own name"),
            );
            usable = false;
        } else {
            let what = if g.structs_by_name.contains_key(&p.name) {
                Some("struct")
            } else if g.enums_by_name.contains_key(&p.name) {
                Some("enum")
            } else if units::is_unit_name(g, &p.name) {
                Some("unit")
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
                        .help("a type parameter takes no other name's place: rename it, for example `T`"),
                );
                usable = false;
            }
        }
        seen.push((&p.name, p.span));
        if p.is_unit {
            diags.push(Diagnostic::at(&codes::E0901, p.span, "unit parameters (`unit U`) are not supported yet"));
            usable = false;
        }
        if let Some(b) = p.bounds.first() {
            diags.push(
                Diagnostic::at(&codes::E0901, b.span, "constraints (`with`) on a type parameter are not supported yet")
                    .help("leave `with` out: the function is checked for each type it is called with"),
            );
            usable = false;
        }
    }
    let head = ast::Func {
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
    };
    g.mono.templates.insert(f.name.clone(), Template { index, head, usable, instances: 0 });
}

/// Checks the bodies of the instances made so far, and of the ones they make.
pub(super) fn drain(g: &mut Globals, src: &str, prog: &ast::Program, diags: &mut Vec<Diagnostic>, emit: &mut dyn FnMut(FuncId, hir::Func)) {
    while let Some(p) = g.mono.pending.pop_front() {
        g.tenv = p.tenv.clone();
        g.mono.depth = p.depth;
        declare_written_optionals(g);
        let mut local = Vec::new();
        let f = check_body(g, src, &mut local, &prog.funcs[p.index], None, p.symbol.clone());
        g.tenv.clear();
        g.mono.depth = 0;
        for d in local {
            // `copy` that does nothing for this `T` is needed for another one.
            if d.code.id == "W0322" || !first_report(g, &d) {
                continue;
            }
            let same = d.labels.first().is_some_and(|l| l.0 == p.call);
            let d = d.note_line(format!("while checking `{}`, instantiated here", p.display));
            diags.push(if same { d } else { d.label(p.call, "instantiated here") });
        }
        emit(p.id, f);
    }
}

/// W0601 for each usable template nothing instantiated.
pub(super) fn warn_unused(g: &Globals, diags: &mut Vec<Diagnostic>) {
    let mut unused: Vec<&Template> = g.mono.templates.values().filter(|t| t.usable && t.instances == 0).collect();
    unused.sort_by_key(|t| t.index);
    for t in unused {
        diags.push(
            Diagnostic::at(&codes::W0600, t.head.name_span, format!("generic function `{}` is never used", t.head.name))
                .note("nothing calls it")
                .help("a generic function is only checked when it is called with a type: call it, or remove it"),
        );
    }
}

/// Makes the optional types that the written `T?` name under the current `tenv`.
fn declare_written_optionals(g: &mut Globals) {
    let written = std::mem::take(&mut g.mono.written_optionals);
    optionals::declare_optionals(g, &written);
    g.mono.written_optionals = written;
}

/// True the first time a diagnostic's code and place are seen.
fn first_report(g: &mut Globals, d: &Diagnostic) -> bool {
    match d.labels.first() {
        Some((s, _)) => g.mono.reported.insert((d.code.id, s.start, s.end)),
        None => true,
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
            (ast::Dim::List(_), Type::List { elem }) | (ast::Dim::Array(..), Type::Array { elem, .. }) => *elem,
            _ => t,
        };
    }
    t
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
                let Some(ty) = self.g.resolve_binding_type(t, self.diags) else { return fail(self) };
                bound[i] = Some((ty, usize::MAX));
            }
            explicit = true;
        }
        // Arguments that show a type parameter are checked first, without an expected type.
        let mut pre: Vec<Pre> = Vec::new();
        for (a, p) in args.iter().zip(&params) {
            let shows = !explicit && tparams.iter().any(|tp| tp.name == p.name) && p.args.is_empty();
            if !shows || matches!(a.kind, ExprKind::NoneValue) {
                pre.push(Pre::Later);
                continue;
            }
            let v = if matches!(p.mods.first(), Some((ast::TypeMod::Ref { .. }, _))) { self.expr_raw(a) } else { self.expr(a) };
            pre.push(Pre::Done(v));
        }
        if !explicit {
            // The first argument that has a type decides.
            for (i, (a, p)) in args.iter().zip(&params).enumerate() {
                let Pre::Done(Some(Val::Expr(x))) = &pre[i] else { continue };
                let k = tparams.iter().position(|tp| tp.name == p.name).unwrap();
                let ty = peel(self.g, p, x.ty);
                match bound[k] {
                    None => bound[k] = Some((ty, i)),
                    Some((first, from)) if first != ty => {
                        self.diags.push(
                            Diagnostic::at(
                                &codes::E0603,
                                a.span,
                                format!("`{}` is `{first}` from the {} argument, but `{ty}` from the {}", tparams[k].name, ordinal(from), ordinal(i)),
                            )
                            .note(format!("this one is `{ty}`"))
                            .label(args[from].span, format!("this one is `{first}`"))
                            .help(format!("convert one of them, or write the type: `{name}<{first}>(...)`")),
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
                self.diags.push(
                    Diagnostic::at(&codes::E0601, span, format!("cannot tell what `{tp}` is in this call of `{name}`"))
                        .note(format!("no argument shows `{tp}`"))
                        .label(self.g.mono.templates[name].head.name_span, format!("`{tp}` is declared here"))
                        .help(format!("write it out: `{name}<uint32>{sample}`")),
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
        let key = (name.to_string(), types);
        if let Some(&id) = self.g.mono.instances.get(&key) {
            return Some(id);
        }
        let types = key.1.clone();
        let list: Vec<String> = types.iter().map(Type::to_string).collect();
        let display = format!("{name}<{}>", list.join(", "));
        let depth = self.g.mono.depth + 1;
        if depth > MAX_DEPTH {
            self.diags.push(
                Diagnostic::at(&codes::E0608, call, format!("`{name}` keeps instantiating itself"))
                    .note("instantiated here again with a bigger type")
                    .note_line(format!("{MAX_DEPTH} instances deep, the last is `{display}`"))
                    .help("make the recursive call use the same type parameters"),
            );
            return None;
        }
        let t = self.g.mono.templates.remove(name).expect("a template");
        let tenv: HashMap<String, Type> = t.head.tparams.iter().map(|p| p.name.clone()).zip(types.iter().copied()).collect();
        let saved = std::mem::replace(&mut self.g.tenv, tenv.clone());
        declare_written_optionals(self.g);
        let mut local = Vec::new();
        let params: Vec<Option<Type>> = t.head.params.iter().map(|p| self.g.resolve_binding_type(&p.ty, &mut local)).collect();
        let ret = self.g.resolve(&t.head.ret, &mut local);
        let (fails, outcome) = failures::signature(self.g, &t.head, ret, true, &mut local);
        let sig = Sig { name: display.clone(), params, ret, span: t.head.span, fails, outcome };
        check_signature_extras(self.g, &t.head, &sig.params, false, &mut local);
        self.g.tenv = saved;
        for d in local {
            if first_report(self.g, &d) {
                self.diags.push(d.note_line(format!("while checking `{display}`, instantiated here")).label(call, "instantiated here"));
            }
        }
        let broken = sig.params.iter().any(Option::is_none) || sig.ret.is_none();
        let index = t.index;
        let id = self.g.add_sig(sig);
        self.g.mono.templates.insert(name.to_string(), t);
        self.g.mono.instances.insert(key, id);
        if !broken {
            let symbol = format!("drachm_g_{}", types::mangle(&display));
            self.g.mono.pending.push_back(Pending { index, id, tenv, depth, call, display, symbol });
        }
        Some(id)
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
