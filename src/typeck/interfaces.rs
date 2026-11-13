//! Interfaces and the structs that implement them (S6B).
//!
//! `interface Shape { float64 area(); static Self unit(); float64 x; }` lists
//! method signatures, `static` signatures and required fields. `Self` in a
//! signature stands for the implementing type: a signature is resolved once
//! for each implementing type, with `Self` bound to that type in the type
//! environment, and then compared with the struct's own method.
//!
//! `struct Circle with Shape and Drawable` is checked when every signature
//! exists (`finish`), and records one `conforms` pair for the struct and the
//! interface and each interface it extends. The types a constrained type
//! parameter is used with are checked against those pairs (`check_bounds`).
//! A generic instance is checked when it is made, or, if it is made while the
//! declarations are still being read, as soon as they are all read.
//!
//! The interfaces `Zero`, `Equal`, `Ordered` and `Text` are built in for
//! numbers and `string` (`PRELUDE`): a program that declares a type or
//! interface of the same name uses its own instead.
//!
//! A constrained type parameter is checked once where it is declared (the
//! definition pass, `definition.rs` below): the body is checked with the
//! parameter standing for a placeholder struct that has only what the
//! interfaces promise.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use super::{check_signature_extras, declare_method_list, failures, generics, similar, Checker, FieldInfo, Globals, MethodInfo, Sig, StructInfo, Val};
use crate::ast;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, StructId};
use crate::types::{Type, TypeError};
use crate::{lexer, parser};

/// The interfaces that numbers and `string` implement without saying so.
pub(super) const PRELUDE: &str = r#"
interface Zero { static Self zero(); }
interface Equal { bool ==(Self o); }
interface Ordered extends Equal { bool <(Self o); bool <=(Self o); bool >(Self o); bool >=(Self o); }
interface Text { string text(); }
interface Iterable<T> { T? at(uint64 i); }
"#;

pub(super) struct IfaceInfo {
    pub name: String,
    pub name_span: Span,
    pub decl: Rc<ast::Interface>,
    /// Type arguments of this instance, in declaration order.
    args: Vec<Type>,
    /// One of `PRELUDE`: its spans are not in the file.
    pub builtin: bool,
    /// False after an error in its declaration.
    pub usable: bool,
    pub parents: Vec<usize>,
    /// Itself and everything it extends, parents first.
    pub ancestors: Vec<usize>,
    /// Every method, its own and inherited, parents first.
    pub methods: Vec<Flat>,
    pub fields: Vec<Flat>,
}

/// A member as an interface sees it: declared in `owner`, at `idx` in its list.
#[derive(Clone, Copy)]
pub(super) struct Flat {
    pub owner: usize,
    pub idx: usize,
    /// Some interface that has it says `changes self`.
    pub changes: bool,
}

enum Deferred {
    Conform { ty: Type, with: Vec<(usize, Span)>, display: String, at: Span },
    Bound { tname: String, tparams: Vec<ast::TypeParam>, types: Vec<Type>, at: Span, display: String },
}

#[derive(Default)]
pub(super) struct State {
    pub list: Vec<IfaceInfo>,
    pub by_name: HashMap<String, usize>,
    pub instances: HashMap<(usize, Vec<Type>), usize>,
    /// The types that implement an interface (each with its ancestors).
    pub conforms: HashSet<(Type, usize)>,
    /// The struct methods whose `self` mode is compared with the interface.
    pub records: Vec<hir::Conformance>,
    /// Methods that stand for an interface's: they need a `self` even if unused.
    pub implementing: HashSet<hir::FuncId>,
    /// For each type, the `at` method that gives its `Iterable<T>`: (instance, method).
    pub iterable: HashMap<Type, Vec<(usize, hir::FuncId)>>,
    /// The placeholder methods whose interface says `changes self`.
    pub changing: HashSet<hir::FuncId>,
    /// Every signature exists: conformance can be checked at once.
    pub ready: bool,
    deferred: Vec<Deferred>,
    /// The text of the file, to quote interface lines in messages.
    pub src: String,
    /// Methods added to built-in types by `uint32 with Zero { }`.
    pub ext: HashMap<Type, Vec<MethodInfo>>,
}

// ---- declarations ----------------------------------------------------------

/// Registers the file's interfaces and the built-in ones, links `extends`
/// and flattens the members.
pub(super) fn declare(g: &mut Globals, src: &str, user: &[ast::Interface], diags: &mut Vec<Diagnostic>) {
    g.ifaces.src = src.to_string();
    let mut core_diags = Vec::new();
    let core = parser::parse(lexer::lex(PRELUDE, &mut core_diags), &mut core_diags);
    debug_assert!(core_diags.is_empty(), "the interface prelude must parse");
    for i in user {
        add(g, i, false, diags);
    }
    for i in &core.interfaces {
        let taken = g.ifaces.by_name.contains_key(&i.name)
            || g.structs_by_name.contains_key(&i.name)
            || g.enums_by_name.contains_key(&i.name)
            || g.mono.type_templates.contains_key(&i.name);
        if !taken {
            add(g, i, true, &mut Vec::new());
        }
    }
    // Check the declaration graph before expanding any arguments. An inheritance
    // cycle remains a cycle even if each edge changes the arguments.
    link_declarations(g, diags);
}

fn add(g: &mut Globals, i: &ast::Interface, builtin: bool, diags: &mut Vec<Diagnostic>) {
    if i.name == "none" || !matches!(Type::from_name(&i.name), Err(TypeError::Unknown)) {
        diags.push(
            Diagnostic::at(&codes::E0202, i.name_span, format!("`{}` is the name of a built-in type", i.name))
                .note("already a type")
                .help("give the interface another name"),
        );
        return;
    }
    let first = g
        .ifaces
        .by_name
        .get(&i.name)
        .map(|&k| g.ifaces.list[k].name_span)
        .or_else(|| g.structs_by_name.get(&i.name).map(|&s| g.structs[s].name_span))
        .or_else(|| g.enums_by_name.get(&i.name).map(|&e| g.enums[e].name_span))
        .or_else(|| g.mono.type_templates.get(&i.name).map(|t| t.name_span));
    if let Some(first) = first {
        diags.push(Diagnostic::at(&codes::E0202, i.name_span, format!("`{}` is defined twice", i.name)).note("defined again here").label(first, "first defined here"));
        return;
    }
    let k = g.ifaces.list.len();
    g.ifaces.by_name.insert(i.name.clone(), k);
    g.ifaces.list.push(IfaceInfo {
        name: i.name.clone(),
        name_span: i.name_span,
        decl: Rc::new(i.clone()),
        args: Vec::new(),
        builtin,
        usable: true,
        parents: Vec::new(),
        ancestors: Vec::new(),
        methods: Vec::new(),
        fields: Vec::new(),
    });
}

/// The text of a written type, ignoring where it is written: equal text, equal type.
fn tkey(t: &ast::TypeName, env: &HashMap<String, Type>) -> String {
    let mut s = String::new();
    for (m, _) in &t.mods {
        s.push_str(match m {
            ast::TypeMod::Ref { mutable: true } => "&mut ",
            ast::TypeMod::Ref { mutable: false } => "&",
            ast::TypeMod::Ptr => "*",
            ast::TypeMod::FixedPtr => "*fixed ",
        });
    }
    if let Some(f) = &t.func {
        s.push('(');
        s.push_str(&f.params.iter().map(|p| tkey(p, env)).collect::<Vec<_>>().join(","));
        s.push_str(")=>");
        s.push_str(&tkey(&f.ret, env));
        return s;
    }
    if let Some(ty) = env.get(&t.name).filter(|_| t.args.is_empty()) {
        s.push_str(&ty.to_string());
    } else {
        s.push_str(&t.name);
    }
    if t.unit.is_some() {
        s.push_str("<unit>");
    }
    if !t.args.is_empty() {
        s.push('<');
        s.push_str(&t.args.iter().map(|a| tkey(a, env)).collect::<Vec<_>>().join(","));
        s.push('>');
    }
    for d in &t.dims {
        match d {
            ast::Dim::List(_) => s.push_str("[?]"),
            ast::Dim::Array(n, _) => s.push_str(&format!("[{n}]")),
        }
    }
    if t.optional.is_some() {
        s.push('?');
    }
    s
}

fn mkey(m: &ast::IMethod, env: &HashMap<String, Type>) -> String {
    let params: Vec<String> = m.params.iter().map(|p| tkey(&p.ty, env)).collect();
    format!("{}({}){}", m.is_static.is_some(), params.join(","), tkey(&m.ret, env))
}

/// Checks names, arity and cycles in the declaration graph.
fn link_declarations(g: &mut Globals, diags: &mut Vec<Diagnostic>) {
    let n = g.ifaces.list.len();
    for i in 0..n {
        let decl = g.ifaces.list[i].decl.clone();
        for parent in &decl.extends {
            if let Some(p) = interface_head(g, parent, diags) {
                g.ifaces.list[i].parents.push(p);
            } else {
                g.ifaces.list[i].usable = false;
            }
        }
    }
    let mut done = vec![false; n];
    for start in 0..n {
        find_cycles(g, start, &mut Vec::new(), &mut done, diags);
    }
    for i in &mut g.ifaces.list {
        i.parents.clear();
    }
}

fn link_instance(g: &mut Globals, id: usize, diags: &mut Vec<Diagnostic>) {
    let decl = g.ifaces.list[id].decl.clone();
    let env = member_env(g, id, None);
    let saved = std::mem::replace(&mut g.tenv, env);
    let mut parents = Vec::new();
    for written in &decl.extends {
        if let Some(parent) = lookup(g, written, diags) {
            if !parents.contains(&parent) {
                parents.push(parent);
            }
        } else {
            g.ifaces.list[id].usable = false;
        }
    }
    g.tenv = saved;
    g.ifaces.list[id].parents = parents;
    if g.ifaces.list[id].usable {
        flatten(g, id, diags);
    }
}

fn find_cycles(g: &mut Globals, at: usize, path: &mut Vec<usize>, done: &mut [bool], diags: &mut Vec<Diagnostic>) {
    if done[at] {
        return;
    }
    path.push(at);
    let parents = g.ifaces.list[at].parents.clone();
    for p in parents {
        if let Some(pos) = path.iter().position(|&x| x == p) {
            let names: Vec<String> = path[pos..].iter().map(|&k| format!("`{}`", g.ifaces.list[k].name)).collect();
            let head = &g.ifaces.list[p];
            let msg = if names.len() == 1 {
                format!("`{}` extends itself", head.name)
            } else {
                format!("{} extend each other", names.join(" and "))
            };
            let at_span = g.ifaces.list[at].decl.extends.iter().find(|e| e.name == head.name).map_or(g.ifaces.list[at].name_span, |e| e.span);
            diags.push(
                Diagnostic::at(&codes::E0615, at_span, msg)
                    .note("this closes the cycle")
                    .label(head.name_span, format!("`{}` is declared here", head.name))
                    .help("an interface can only extend ones that do not extend it back"),
            );
            for &k in &path[pos..] {
                g.ifaces.list[k].usable = false;
            }
        } else if !done[p] {
            find_cycles(g, p, path, done, diags);
        }
    }
    path.pop();
    done[at] = true;
}

fn flatten(g: &mut Globals, i: usize, diags: &mut Vec<Diagnostic>) {
    let parents = g.ifaces.list[i].parents.clone();
    let mut ancestors: Vec<usize> = Vec::new();
    for &p in &parents {
        if !g.ifaces.list[p].usable {
            g.ifaces.list[i].usable = false;
        }
        for &a in &g.ifaces.list[p].ancestors {
            if !ancestors.contains(&a) {
                ancestors.push(a);
            }
        }
    }
    ancestors.push(i);
    let mut methods: Vec<Flat> = Vec::new();
    let mut fields: Vec<Flat> = Vec::new();
    let name = g.ifaces.list[i].name.clone();
    let name_span = g.ifaces.list[i].name_span;
    for &a in &ancestors {
        let decl = g.ifaces.list[a].decl.clone();
        for (idx, m) in decl.methods.iter().enumerate() {
            match methods.iter().position(|f| g.ifaces.list[f.owner].decl.methods[f.idx].name == m.name) {
                None => methods.push(Flat { owner: a, idx, changes: m.changes_self.is_some() }),
                Some(k) => {
                    let first = methods[k];
                    let other = &g.ifaces.list[first.owner].decl.methods[first.idx];
                    if first.owner == a {
                        diags.push(
                            Diagnostic::at(&codes::E0202, m.name_span, format!("`{}` lists `{}` twice", decl.name, m.name))
                                .note("listed again here")
                                .label(other.name_span, "first listed here"),
                        );
                    } else if mkey(other, &member_env(g, first.owner, None)) != mkey(m, &member_env(g, a, None)) {
                        diags.push(
                            Diagnostic::at(&codes::E0615, name_span, format!("`{name}` gets two different `{}` methods", m.name))
                                .note("clashes here")
                                .label(other.name_span, format!("`{}` has it like this", g.ifaces.list[first.owner].name))
                                .label(m.name_span, format!("`{}` has it like this", g.ifaces.list[a].name))
                                .help("make the signatures equal"),
                        );
                        g.ifaces.list[i].usable = false;
                    } else {
                        methods[k].changes |= m.changes_self.is_some();
                    }
                }
            }
        }
        for (idx, f) in decl.fields.iter().enumerate() {
            if decl.methods.iter().any(|m| m.name == f.name) {
                diags.push(Diagnostic::at(&codes::E0202, f.name_span, format!("`{}` has a field and a method both named `{}`", decl.name, f.name)).note("defined again here"));
                continue;
            }
            match fields.iter().position(|x| g.ifaces.list[x.owner].decl.fields[x.idx].name == f.name) {
                None => fields.push(Flat { owner: a, idx, changes: false }),
                Some(k) => {
                    let first = fields[k];
                    let other = &g.ifaces.list[first.owner].decl.fields[first.idx];
                    if first.owner == a {
                        diags.push(
                            Diagnostic::at(&codes::E0202, f.name_span, format!("`{}` lists the field `{}` twice", decl.name, f.name))
                                .note("listed again here")
                                .label(other.name_span, "first listed here"),
                        );
                    } else if tkey(&other.ty, &member_env(g, first.owner, None)) != tkey(&f.ty, &member_env(g, a, None)) {
                        diags.push(
                            Diagnostic::at(&codes::E0615, name_span, format!("`{name}` gets two different fields named `{}`", f.name))
                                .note("clashes here")
                                .label(other.name_span, format!("`{}` has it like this", g.ifaces.list[first.owner].name))
                                .label(f.name_span, format!("`{}` has it like this", g.ifaces.list[a].name))
                                .help("make the types equal"),
                        );
                        g.ifaces.list[i].usable = false;
                    }
                }
            }
        }
    }
    let info = &mut g.ifaces.list[i];
    info.ancestors = ancestors;
    info.methods = methods;
    info.fields = fields;
}

/// E0201 for a name after `with` or `extends` that is not an interface.
fn not_an_interface(g: &Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) {
    let names: Vec<&str> = g.ifaces.list.iter().map(|i| i.name.as_str()).collect();
    let mut d = Diagnostic::at(&codes::E0201, t.span, format!("unknown interface `{}`", t.name));
    if g.structs_by_name.contains_key(&t.name) || g.enums_by_name.contains_key(&t.name) {
        d = d.note("not an interface").help(format!("`{}` is a type; `with` and `extends` take interfaces", t.name));
    } else if let Some(near) = similar(&t.name, &names) {
        d = d.help(format!("did you mean `{near}`?"));
    } else {
        d = d.help(format!("declare it: `interface {} {{ ... }}`", t.name));
    }
    diags.push(d);
}

/// The declaration named in a promise, bound or parent list.
fn interface_head(g: &Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<usize> {
    let Some(&id) = g.ifaces.by_name.get(&t.name) else {
        not_an_interface(g, t, diags);
        return None;
    };
    let expected = g.ifaces.list[id].decl.tparams.len();
    if expected != t.args.len() {
        let code = if t.args.is_empty() { &codes::E0604 } else { &codes::E0602 };
        diags.push(Diagnostic::at(code, t.span, format!("`{}` takes {expected} type arguments, but {} given", t.name, t.args.len()))
            .note("type arguments expected here"));
        return None;
    }
    Some(id)
}

fn member_env(g: &Globals, id: usize, self_ty: Option<Type>) -> HashMap<String, Type> {
    let iface = &g.ifaces.list[id];
    let mut env: HashMap<_, _> = iface.decl.tparams.iter().zip(&iface.args).map(|(p, &ty)| (p.name.clone(), ty)).collect();
    env.extend(self_ty.map(|ty| ("Self".to_string(), ty)));
    env
}

pub(super) fn lookup(g: &mut Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<usize> {
    let head = interface_head(g, t, diags)?;
    if !g.ifaces.list[head].usable {
        return None;
    }
    if t.args.is_empty() {
        if g.ifaces.list[head].ancestors.is_empty() {
            link_instance(g, head, diags);
        }
        return g.ifaces.list[head].usable.then_some(head);
    }
    let mut args = Vec::with_capacity(t.args.len());
    for arg in &t.args {
        if arg.is_number {
            diags.push(Diagnostic::at(&codes::E0901, arg.span, "a number as a type argument is not supported yet"));
            return None;
        }
        generics::prepare_type(g, arg, diags);
        let ty = g.resolve(arg, diags)?;
        if ty.is_borrow() || g.holds_borrow(ty) {
            diags.push(Diagnostic::at(&codes::E0901, arg.span, "a type argument cannot be a borrow yet"));
            return None;
        }
        args.push(ty);
    }
    let key = (head, args.clone());
    if let Some(&id) = g.ifaces.instances.get(&key) {
        return g.ifaces.list[id].usable.then_some(id);
    }
    let template = &g.ifaces.list[head];
    let name = format!("{}<{}>", template.name, args.iter().map(Type::to_string).collect::<Vec<_>>().join(", "));
    let id = g.ifaces.list.len();
    g.ifaces.list.push(IfaceInfo {
        name,
        name_span: template.name_span,
        decl: template.decl.clone(),
        args,
        builtin: template.builtin,
        usable: true,
        parents: Vec::new(),
        ancestors: Vec::new(),
        methods: Vec::new(),
        fields: Vec::new(),
    });
    g.ifaces.instances.insert(key, id);
    link_instance(g, id, diags);
    g.ifaces.list[id].usable.then_some(id)
}

/// Validates declarations and prepares the members of ordinary interfaces.
pub(super) fn validate(g: &mut Globals, diags: &mut Vec<Diagnostic>) {
    let n = g.ifaces.list.len();
    for i in 0..n {
        let decl = g.ifaces.list[i].decl.clone();
        if !g.ifaces.list[i].builtin && !generics::check_tparams(g, &decl.tparams, diags) {
            g.ifaces.list[i].usable = false;
        }
    }
    for i in 0..n {
        if !g.ifaces.list[i].usable || !g.ifaces.list[i].decl.tparams.is_empty() {
            continue;
        }
        if g.ifaces.list[i].ancestors.is_empty() {
            link_instance(g, i, diags);
        }
        if !g.ifaces.list[i].builtin {
            prepare_members(g, i, Type::Bool, diags);
        }
    }
}

fn prepare_members(g: &mut Globals, id: usize, self_ty: Type, diags: &mut Vec<Diagnostic>) {
    let decl = g.ifaces.list[id].decl.clone();
    let env = member_env(g, id, Some(self_ty));
    let saved = std::mem::replace(&mut g.tenv, env);
    for m in &decl.methods {
        for p in &m.params {
            generics::prepare_type(g, &p.ty, diags);
            g.resolve_binding_type(&p.ty, diags);
        }
        generics::prepare_type(g, &m.ret, diags);
        g.resolve(&m.ret, diags);
    }
    for f in &decl.fields {
        generics::prepare_type(g, &f.ty, diags);
        g.resolve_storable(&f.ty, diags);
    }
    g.tenv = saved;
}

// ---- signatures and conformance ------------------------------------------

/// An interface method's parameter and result types for implementing type `self_ty`.
pub(super) fn sig_for(g: &mut Globals, owner: usize, m: &ast::IMethod, self_ty: Type) -> (Vec<Option<Type>>, Option<Type>) {
    let env = member_env(g, owner, Some(self_ty));
    let saved = std::mem::replace(&mut g.tenv, env);
    let params = m.params.iter().map(|p| g.resolve_quiet(&p.ty)).collect();
    let ret = g.resolve_quiet(&m.ret);
    g.tenv = saved;
    (params, ret)
}

pub(super) fn field_for(g: &mut Globals, owner: usize, f: &ast::IField, self_ty: Type) -> Option<Type> {
    let env = member_env(g, owner, Some(self_ty));
    let saved = std::mem::replace(&mut g.tenv, env);
    let t = g.resolve_quiet(&f.ty);
    g.tenv = saved;
    t
}

fn param_list(ps: &[Option<Type>]) -> String {
    let v: Vec<String> = ps.iter().map(|p| p.map_or("?".to_string(), |t| t.to_string())).collect();
    format!("({})", v.join(", "))
}

/// The label text for where an interface member is declared.
fn promised(g: &Globals, owner: usize, what: &str) -> String {
    format!("`{}` promises {what}", g.ifaces.list[owner].name)
}

/// Checks that the methods and fields of `ty` (struct `sid`) are what the
/// interfaces in `with` list, and records that it implements them.
fn resolve_interfaces(g: &mut Globals, with: &[ast::TypeName], diags: &mut Vec<Diagnostic>) -> Vec<(usize, Span)> {
    with.iter().filter_map(|w| lookup(g, w, diags).map(|k| (k, w.span))).collect()
}

fn check_conformance(g: &mut Globals, ty: Type, with: &[(usize, Span)], diags: &mut Vec<Diagnostic>) {
    let sname = ty.to_string();
    for &(k, at) in with {
        for owner in g.ifaces.list[k].ancestors.clone() {
            if !g.ifaces.list[owner].args.is_empty() {
                prepare_members(g, owner, ty, diags);
            }
        }
        if g.ifaces.list[k].builtin && builtin_has(ty, &g.ifaces.list[k].name) {
            diags.push(
                Diagnostic::at(&codes::E0202, at, format!("`{sname}` already implements `{}`", g.ifaces.list[k].name))
                    .note("it is built in for this type")
                    .help("leave this one out"),
            );
            continue;
        }
        let methods = g.ifaces.list[k].methods.clone();
        let fields = g.ifaces.list[k].fields.clone();
        let iname = g.ifaces.list[k].name.clone();
        for fm in methods {
            let decl = g.ifaces.list[fm.owner].decl.clone();
            let m = &decl.methods[fm.idx];
            let builtin = g.ifaces.list[fm.owner].builtin;
            let owner_name = g.ifaces.list[fm.owner].name.clone();
            let secondary = |d: Diagnostic, what: String| if builtin { d } else { d.label(m.span, what) };
            let same: Vec<MethodInfo> = methods_of(g, ty).into_iter().filter(|x| x.name == m.name).collect();
            // Several methods may differ only in their result: the interface's own result picks.
            let found = if same.len() > 1 {
                let (_, iret) = sig_for(g, fm.owner, m, ty);
                same.iter().find(|x| iret.is_some() && g.sigs[x.func].ret == iret).or(same.first()).cloned()
            } else {
                same.into_iter().next()
            };
            let Some(found) = found else {
                // Every struct has `==` and `!=` by itself.
                if matches!(m.operator, Some(ast::BinOp::Eq | ast::BinOp::Ne)) {
                    continue;
                }
                let sig = if m.is_static.is_some() { "a `static` method" } else { "a method" };
                let d = Diagnostic::at(&codes::E0610, at, format!("`{sname}` says `with {iname}`, but has no method `{}`", m.name)).note(format!("`{iname}` needs {sig} `{}`", m.name));
                let d = secondary(d, promised(g, fm.owner, &format!("`{}`", m.name)));
                diags.push(d.help(format!("add `{}` to `{sname}`", signature_help(g, m, &sname))));
                continue;
            };
            g.ifaces.implementing.insert(found.func);
            if is_iterable(g, fm.owner) {
                let list = g.ifaces.iterable.entry(ty).or_default();
                if !list.iter().any(|x| x.0 == fm.owner) {
                    list.push((fm.owner, found.func));
                }
            }
            if !found.public {
                let d = Diagnostic::at(&codes::E0610, at, format!("`{sname}` says `with {iname}`, but `{}` is private", m.name)).note("an interface's methods are public");
                let d = secondary(d, promised(g, fm.owner, &format!("`{}`", m.name))).label(g.sigs[found.func].span, "declared in a private section");
                diags.push(d.help(format!("move `{}` above the `private:` label, or under `public:`", m.name)));
                continue;
            }
            if found.is_static != m.is_static.is_some() {
                let (has, wants) = if found.is_static { ("static", "not static") } else { ("not static", "static") };
                let d = Diagnostic::at(&codes::E0611, g.sigs[found.func].span, format!("`{sname}::{}` is {has}, but `{owner_name}::{}` is {wants}", m.name, m.name))
                    .note(format!("{has} here"));
                diags.push(secondary(d, format!("{wants} here")).help(if found.is_static { "remove `static`".to_string() } else { "add `static`: it is called on the type, like `Type::name()`".to_string() }));
                continue;
            }
            let (ips, iret) = sig_for(g, fm.owner, m, ty);
            let (sps, sret) = {
                let s = &g.sigs[found.func];
                (s.params.clone(), s.ret)
            };
            let span = g.sigs[found.func].span;
            if ips.len() != sps.len() || ips.iter().zip(&sps).any(|(a, b)| a.is_some() && b.is_some() && a != b) {
                let d = Diagnostic::at(&codes::E0611, span, format!("`{sname}::{}` takes `{}`, but `{owner_name}::{}` takes `{}`", m.name, param_list(&sps), m.name, param_list(&ips)))
                    .note("these parameters");
                diags.push(secondary(d, "the interface's line".to_string()).help(format!("write `{}`", signature_help(g, m, &sname))));
                continue;
            }
            if let (Some(a), Some(b)) = (sret, iret)
                && a != b
            {
                let at = span;
                let d = Diagnostic::at(&codes::E0611, at, format!("`{sname}::{}` returns `{a}`, but `{owner_name}::{}` returns `{b}`", m.name, m.name))
                    .note(format!("returns `{a}`"));
                diags.push(secondary(d, format!("returns `{b}`")).help(format!("change the result to `{b}`")));
                continue;
            }
            if !fm.changes && m.is_static.is_none() {
                let signature = if builtin { String::new() } else { short_text(&g.ifaces.src, m.span).to_string() };
                g.ifaces.records.push(hir::Conformance {
                    func: found.func,
                    method: format!("{sname}::{}", m.name),
                    promised: format!("{owner_name}::{}", m.name),
                    promised_at: (!builtin).then_some(m.span),
                    signature,
                });
            }
        }
        for ff in fields {
            let decl = g.ifaces.list[ff.owner].decl.clone();
            let f = &decl.fields[ff.idx];
            let builtin = g.ifaces.list[ff.owner].builtin;
            let want = field_for(g, ff.owner, f, ty);
            let want_text = want.map_or_else(|| f.ty.name.clone(), |t| t.to_string());
            let have = ty.struct_id().and_then(|sid| g.structs[sid].fields.iter().find(|x| x.name == f.name).map(|x| (x.ty, x.public, x.span)));
            let owner_name = g.ifaces.list[ff.owner].name.clone();
            let secondary = |d: Diagnostic, what: String| if builtin { d } else { d.label(f.name_span, what) };
            match have {
                None => {
                    let d = Diagnostic::at(&codes::E0614, at, format!("`{sname}` says `with {iname}`, but has no field `{}` of type `{want_text}`", f.name)).note(format!("`{iname}` needs `{}`", f.name));
                    diags.push(secondary(d, format!("`{owner_name}` requires `{}` here", f.name)).help(format!("add `public {want_text} {};` to `{sname}`", f.name)));
                }
                Some((fty, public, fspan)) => {
                    if let (Some(a), Some(b)) = (fty, want)
                        && a != b
                    {
                        let d = Diagnostic::at(&codes::E0614, fspan, format!("`{sname}::{}` is `{a}`, but `{owner_name}` requires `{b}`", f.name)).note(format!("`{a}` here"));
                        diags.push(secondary(d, format!("`{b}` there")).help(format!("make the field `{b}`")));
                    } else if !public {
                        let d = Diagnostic::at(&codes::E0614, fspan, format!("`{sname}::{}` is private, but `{owner_name}` requires it to be public", f.name)).note("declared in a private section");
                        diags.push(secondary(d, format!("`{owner_name}` requires it here")).help(format!("put `{}` under `public:`", f.name)));
                    }
                }
            }
        }
        let ancestors = g.ifaces.list[k].ancestors.clone();
        for a in ancestors {
            g.ifaces.conforms.insert((ty, a));
        }
    }
}

fn short_text(src: &str, span: Span) -> &str {
    src.get(span.start..span.end).unwrap_or("")
}

/// `float64 area()`, for a help line.
fn signature_help(g: &mut Globals, m: &ast::IMethod, sname: &str) -> String {
    let _ = g;
    let params: Vec<String> = m.params.iter().map(|p| format!("{} {}", written(&p.ty, sname), p.name)).collect();
    let st = if m.is_static.is_some() { "static " } else { "" };
    format!("{st}{} {}({}) {{ ... }}", written(&m.ret, sname), m.name, params.join(", "))
}

fn written(t: &ast::TypeName, sname: &str) -> String {
    let k = tkey(t, &HashMap::new());
    if t.name == "Self" { k.replace("Self", sname) } else { k }
}

// ---- when to check ----------------------------------------------------------

/// Checks the structs that name interfaces, once every signature exists, and
/// then whatever was waiting for that.
pub(super) fn finish(g: &mut Globals, prog: &ast::Program, struct_ids: &[Option<StructId>], impls: &[Option<ImplTarget>], diags: &mut Vec<Diagnostic>) {
    for (s, id) in prog.structs.iter().zip(struct_ids) {
        if let Some(id) = *id
            && !s.with.is_empty()
        {
            let ty = g.structs[id].ty;
            let with = resolve_interfaces(g, &s.with, diags);
            check_conformance(g, ty, &with, diags);
        }
    }
    for (decl, target) in prog.impls.iter().zip(impls) {
        if let Some(t) = target {
            let with = resolve_interfaces(g, &decl.with, diags);
            check_conformance(g, t.ty, &with, diags);
        }
    }
    g.ifaces.ready = true;
    for d in std::mem::take(&mut g.ifaces.deferred) {
        match d {
            Deferred::Conform { ty, with, display, at } => {
                let mut local = Vec::new();
                check_conformance(g, ty, &with, &mut local);
                for d in local {
                    diags.push(d.note_line(format!("while checking `{display}`, instantiated here")).label(at, "instantiated here"));
                }
            }
            Deferred::Bound { tname, tparams, types, at, display } => {
                let mut local = Vec::new();
                check_bounds(g, &tname, &tparams, &types, at, &mut local);
                for d in local {
                    diags.push(d.note_line(format!("while checking `{display}`, instantiated here")));
                }
            }
        }
    }
}

/// A generic struct instance was made: check its `with` list (now, or later).
pub(super) fn on_instance(g: &mut Globals, ty: Type, with: &[ast::TypeName], display: &str, at: Span, diags: &mut Vec<Diagnostic>) {
    if with.is_empty() {
        return;
    }
    let with = resolve_interfaces(g, with, diags);
    if g.ifaces.ready {
        check_conformance(g, ty, &with, diags);
    } else {
        g.ifaces.deferred.push(Deferred::Conform { ty, with, display: display.to_string(), at });
    }
}

/// Is interface instance `k` the built-in `Iterable<T>`?
pub(super) fn is_iterable(g: &Globals, k: usize) -> bool {
    let i = &g.ifaces.list[k];
    i.builtin && i.decl.name == "Iterable" && i.args.len() == 1
}

/// The item type of `Iterable<T>` instance `k`.
pub(super) fn item_of(g: &Globals, k: usize) -> Type {
    g.ifaces.list[k].args[0]
}

/// Does `ty` implement interface `k`?
pub(super) fn conforms(g: &Globals, ty: Type, k: usize) -> bool {
    if g.ifaces.conforms.contains(&(ty, k)) {
        return true;
    }
    let info = &g.ifaces.list[k];
    info.builtin && builtin_has(ty, &info.name)
}

fn builtin_has(ty: Type, interface: &str) -> bool {
    let num = ty.is_numeric();
    match interface {
        "Zero" => num || ty == Type::Str,
        "Equal" => num || ty == Type::Bool || ty == Type::Str,
        "Ordered" => num,
        "Text" => num || ty == Type::Bool,
        _ => false,
    }
}

/// The bounds a type parameter lists, as interface indexes with their spans.


/// E0607 for each type argument that lacks an interface its parameter asks for.
/// Returns false if there was one.
pub(super) fn check_bounds(g: &mut Globals, tname: &str, tparams: &[ast::TypeParam], types: &[Type], at: Span, diags: &mut Vec<Diagnostic>) -> bool {
    let env = tparams.iter().zip(types).map(|(p, &ty)| (p.name.clone(), ty)).collect();
    let saved = std::mem::replace(&mut g.tenv, env);
    let mut ok = true;
    for (tp, &ty) in tparams.iter().zip(types) {
        let bounds = resolve_interfaces(g, &tp.bounds, diags);
        ok &= bounds.len() == tp.bounds.len();
        for (k, bspan) in bounds {
            if conforms(g, ty, k) {
                continue;
            }
            ok = false;
            let iname = &g.ifaces.list[k].name;
            if let Some(p) = ty.struct_id().and_then(|id| g.structs[id].placeholder.as_ref()) {
                // `ty` stands for a type parameter of the generic being checked.
                let have: Vec<String> = p.bounds.iter().map(|(b, _)| format!("`{}`", g.ifaces.list[*b].name)).collect();
                let (note, help) = if have.is_empty() {
                    (format!("`{ty}` has no constraint"), format!("constrain it: `{ty} with {iname}`"))
                } else {
                    (format!("`{ty}` is only known to implement {}", have.join(" and ")), format!("add the interface to the constraint: `{ty} with {} and {iname}`", have.join(" and ").replace('`', "")))
                };
                diags.push(
                    Diagnostic::at(&codes::E0607, at, format!("`{ty}` does not implement `{iname}`, which `{tname}` asks of `{}`", tp.name))
                        .note(note)
                        .label(bspan, format!("`{}` must implement `{iname}`", tp.name))
                        .help(help),
                );
                continue;
            }
            let help = if ty.struct_id().is_some() {
                format!("write `struct {ty} with {iname}` and give it what `{iname}` lists")
            } else if g.ifaces.list[k].builtin {
                format!("`{iname}` is built in for some number types and `string`; others can say `{ty} with {iname} {{ ... }}`")
            } else {
                format!("say it with a separate declaration: `{ty} with {iname} {{ ... }}`")
            };
            diags.push(
                Diagnostic::at(&codes::E0607, at, format!("`{ty}` does not implement `{iname}`, which `{tname}` asks of `{}`", tp.name))
                    .note(format!("`{}` is `{ty}` here", tp.name))
                    .label(bspan, format!("`{}` must implement `{iname}`", tp.name))
                    .help(help),
            );
        }
    }
    g.tenv = saved;
    ok
}

/// The bounds check of a generic struct instance: now, or once the
/// declarations are all read.
pub(super) fn check_type_bounds(g: &mut Globals, tname: &str, tparams: &[ast::TypeParam], types: &[Type], at: Span, display: &str, diags: &mut Vec<Diagnostic>) -> bool {
    if tparams.iter().all(|p| p.bounds.is_empty()) {
        return true;
    }
    if g.ifaces.ready {
        return check_bounds(g, tname, tparams, types, at, diags);
    }
    g.ifaces.deferred.push(Deferred::Bound {
        tname: tname.to_string(),
        tparams: tparams.to_vec(),
        types: types.to_vec(),
        at,
        display: display.to_string(),
    });
    true
}

/// Validates the bounds written on a type parameter list (names that are not
/// interfaces); false if one is wrong.
pub(super) fn check_param_bounds(g: &Globals, tparams: &[ast::TypeParam], diags: &mut Vec<Diagnostic>) -> bool {
    let mut ok = true;
    for tp in tparams {
        for b in &tp.bounds {
            if !b.mods.is_empty() || !b.dims.is_empty() || b.optional.is_some() || b.unit.is_some() {
                diags.push(Diagnostic::at(&codes::E0003, b.span, "a constraint is the name of an interface").help("write `T with Shape`"));
                ok = false;
            } else if interface_head(g, b, diags).is_none() {
                ok = false;
            }
        }
    }
    ok
}

/// What a placeholder method would be, see `MethodInfo`.
pub(super) fn method_info(name: &str, operator: Option<ast::BinOp>, func: hir::FuncId, is_static: bool) -> MethodInfo {
    MethodInfo { name: name.to_string(), operator, func, is_static, public: true }
}

/// An interface written where a value's type goes: E0617, or E0901 for a
/// borrow of one (`&Shape`, dynamic dispatch, is not built yet). True if reported.
pub(super) fn as_value_type(g: &Globals, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> bool {
    if g.tenv.contains_key(&t.name) || g.structs_by_name.contains_key(&t.name) || g.enums_by_name.contains_key(&t.name) {
        return false;
    }
    let Some(&k) = g.ifaces.by_name.get(&t.name) else { return false };
    let name = &g.ifaces.list[k].name;
    let borrowed = matches!(t.mods.first(), Some((ast::TypeMod::Ref { .. }, _)));
    if borrowed && t.mods.len() == 1 && t.dims.is_empty() && t.optional.is_none() && t.unit.is_none() {
        if !g.ifaces.list[k].decl.tparams.is_empty() || !t.args.is_empty() {
            if interface_head(g, t, diags).is_some() {
                diags.push(Diagnostic::at(&codes::E0901, t.span, "borrowing a generic interface is not supported yet"));
            }
            return true;
        }
        // `&Shape`: a borrow of an interface (`dynamic.rs`), unless its declaration was wrong.
        return !g.ifaces.list[k].usable;
    }
    if borrowed && !t.dims.is_empty() {
        diags.push(
            Diagnostic::at(&codes::E0901, t.span, format!("a list of borrows of `{name}` is not supported yet"))
                .note("a borrow as a list item is not built")
                .help("until then, use an enum when the choices are known, or a list of the struct itself"),
        );
    } else {
        diags.push(
            Diagnostic::at(&codes::E0617, t.span, format!("`{name}` is an interface, so a value cannot be one"))
                .note("not a type for a value")
                .help(format!("borrow it: `&{name}`; or use an enum when the choices are known")),
        );
    }
    true
}

impl Checker<'_> {
    /// `T::name(args)` where `T` stands for a type here (a type parameter of
    /// the instance being checked), or `uint32::name(args)`: a static call on
    /// a type that is not necessarily a struct.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn static_on_type(&mut self, ty: Type, tname: &str, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        match ty {
            Type::Struct { id, .. } => self.static_call(id, name, name_span, targs, args, span),
            Type::Enum { id, .. } => self.qualified_variant(id, name, name_span, args, span),
            _ => {
                if self.g.ifaces.ext.get(&ty).is_some_and(|ms| ms.iter().any(|m| m.name == name && m.is_static)) {
                    return self.ext_static(ty, name, args, span);
                }
                for a in args {
                    self.expr(a);
                }
                let builtin_zero = name == "zero" && builtin_has(ty, "Zero") && self.g.ifaces.by_name.get("Zero").is_some_and(|&k| self.g.ifaces.list[k].builtin);
                if builtin_zero {
                    if let Some(a) = args.first() {
                        self.diags.push(Diagnostic::at(&codes::E0103, a.span, "`zero` takes no arguments"));
                        return None;
                    }
                    let e = match ty {
                        Type::Str => super::str_lit(String::new()),
                        Type::Int { .. } => super::int_lit(0, ty),
                        _ => hir::Expr::new(hir::ExprKind::Float(0.0), ty),
                    };
                    return Some(Val::Expr(e.at(span)));
                }
                let shown = if tname == ty.to_string() { format!("`{ty}`") } else { format!("`{tname}` (which is `{ty}` here)") };
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("{shown} has no static method `{name}`"))
                        .help("numbers and `string` have `zero()` through the built-in `Zero`"),
                );
                None
            }
        }
    }
}

/// What a type parameter stands for while its generic is checked at its definition.
pub(super) struct Placeholder {
    /// It has at least one constraint: what it lacks is E0606.
    pub constrained: bool,
    pub bounds: Vec<(usize, Span)>,
}

/// A struct that has only what the constraints of `tp` promise: its required
/// fields, its methods (as signatures), and the interfaces themselves.
pub(super) fn placeholder(g: &mut Globals, tp: &ast::TypeParam) -> Type {
    let id = g.structs.len();
    let ty = Type::Struct { id, name: crate::types::intern(&tp.name) };
    g.structs.push(StructInfo {
        name: tp.name.clone(),
        name_span: tp.span,
        ty,
        fields: Vec::new(),
        methods: Vec::new(),
        // Nothing is known about copying a `T`, so it moves.
        stays: true,
        lives: Vec::new(),
        has_drop: None,
        ctors: Vec::new(),
        auto_ctor: false,
        default_def: None,
        declared_lives: false,
        placeholder: Some(Placeholder { constrained: !tp.bounds.is_empty(), bounds: Vec::new() }),
    });
    ty
}

pub(super) fn populate_placeholder(g: &mut Globals, ty: Type, tp: &ast::TypeParam, diags: &mut Vec<Diagnostic>) {
    let id = ty.struct_id().expect("placeholder struct");
    let bounds = resolve_interfaces(g, &tp.bounds, diags);
    g.structs[id].placeholder.as_mut().unwrap().bounds = bounds.clone();
    for (k, _) in bounds {
        for owner in g.ifaces.list[k].ancestors.clone() {
            if !g.ifaces.list[owner].args.is_empty() {
                prepare_members(g, owner, ty, diags);
            }
        }
        let methods = g.ifaces.list[k].methods.clone();
        let fields = g.ifaces.list[k].fields.clone();
        for fm in methods {
            let decl = g.ifaces.list[fm.owner].decl.clone();
            let m = &decl.methods[fm.idx];
            let (params, ret) = sig_for(g, fm.owner, m, ty);
            if let Some(x) = g.structs[id].methods.iter().find(|x| x.name == m.name && !(is_iterable(g, fm.owner) && g.sigs[x.func].ret != ret)) {
                if is_iterable(g, fm.owner) {
                    let f = x.func;
                    let list = g.ifaces.iterable.entry(ty).or_default();
                    if !list.iter().any(|x| x.0 == fm.owner) {
                        list.push((fm.owner, f));
                    }
                }
                continue;
            }
            let func = g.add_sig(Sig { name: format!("{}::{}", tp.name, m.name), params, ret, span: m.span, fails: Vec::new(), outcome: None });
            if fm.changes {
                g.ifaces.changing.insert(func);
            }
            if is_iterable(g, fm.owner) {
                g.ifaces.iterable.entry(ty).or_default().push((fm.owner, func));
            }
            g.structs[id].methods.push(method_info(&m.name, m.operator, func, m.is_static.is_some()));
        }
        for ff in fields {
            let decl = g.ifaces.list[ff.owner].decl.clone();
            let f = &decl.fields[ff.idx];
            if g.structs[id].fields.iter().any(|x| x.name == f.name) {
                continue;
            }
            let fty = field_for(g, ff.owner, f, ty);
            g.structs[id].fields.push(FieldInfo { name: f.name.clone(), ty: fty, mutable: true, public: true, span: f.name_span });
        }
        let ancestors = g.ifaces.list[k].ancestors.clone();
        for a in ancestors {
            g.ifaces.conforms.insert((ty, a));
        }
    }
}

impl Checker<'_> {
    /// E0606 when `s` stands for a constrained type parameter and the program
    /// asks for something its constraints do not promise. True if reported.
    pub(super) fn beyond_constraint(&mut self, s: StructId, what: &str, at: Span) -> bool {
        let info = &self.g.structs[s];
        let Some(p) = info.placeholder.as_ref().filter(|p| p.constrained) else { return false };
        let name = info.name.clone();
        let names: Vec<String> = p.bounds.iter().map(|(k, _)| format!("`{}`", self.g.ifaces.list[*k].name)).collect();
        let first = p.bounds.first().map_or(at, |b| b.1);
        let all = names.join(" and ");
        let (message, help) = if what.ends_with("operator") {
            let sym = what.trim_end_matches(" operator").trim_matches('`');
            (
                format!("`{name}` has no `{sym}`: operators need an interface that declares them"),
                format!("add `bool {sym}(Self o);` (or the operator you need) to an interface, and constrain `{name}` with it"),
            )
        } else {
            let plural = if names.len() == 1 { "constraint" } else { "constraints" };
            let verb = if what.starts_with("field") { "requires" } else { "promises" };
            (
                format!("`{name}` has no {what} that its {plural} {all} {verb}"),
                format!("add it to {all}, or constrain `{name}` with an interface that has it"),
            )
        };
        let d = Diagnostic::at(&codes::E0606, at, message).note(format!("not offered by {all}")).label(first, format!("`{name}` is constrained here")).help(help);
        self.diags.push(d);
        true
    }
}

/// The methods `ty` has: a struct's own and added ones, or those added to a built-in type.
pub(super) fn methods_of(g: &Globals, ty: Type) -> Vec<MethodInfo> {
    match ty.struct_id() {
        Some(sid) => g.structs[sid].methods.clone(),
        None => g.ifaces.ext.get(&ty).cloned().unwrap_or_default(),
    }
}

/// A type that `T with I { methods }` adds methods to.
pub(super) struct ImplTarget {
    pub ty: Type,
    /// Set when it is a struct: the methods are added to it.
    pub struct_id: Option<StructId>,
    pub ids: Vec<Option<hir::FuncId>>,
}

/// Declares the signatures of the methods that `T with I { }` blocks add.
pub(super) fn declare_impls(g: &mut Globals, prog: &ast::Program, diags: &mut Vec<Diagnostic>) -> Vec<Option<ImplTarget>> {
    let mut out = Vec::new();
    for decl in &prog.impls {
        let t = &decl.ty;
        if !t.mods.is_empty() || t.optional.is_some() || !t.dims.is_empty() || t.unit.is_some() {
            diags.push(Diagnostic::at(&codes::E0901, t.span, "only a plain type name can promise an interface this way yet"));
            out.push(None);
            continue;
        }
        let Some(ty) = g.resolve(t, diags) else {
            out.push(None);
            continue;
        };
        let target = match ty {
            Type::Struct { id, .. } if !g.mono.origin.contains_key(&ty) => {
                let ids = declare_method_list(g, id, &decl.methods, diags);
                Some(ImplTarget { ty, struct_id: Some(id), ids })
            }
            Type::Int { .. } | Type::Float(_) | Type::Bool | Type::Str => {
                let mut ids = Vec::new();
                for m in &decl.methods {
                    ids.push(declare_ext_method(g, ty, m, diags));
                }
                Some(ImplTarget { ty, struct_id: None, ids })
            }
            _ => {
                diags.push(
                    Diagnostic::at(&codes::E0901, t.span, format!("`{ty}` cannot promise an interface this way yet"))
                        .help("structs, numbers, `bool` and `string` can"),
                );
                None
            }
        };
        out.push(target);
    }
    out
}

fn declare_ext_method(g: &mut Globals, ty: Type, m: &ast::Method, diags: &mut Vec<Diagnostic>) -> Option<hir::FuncId> {
    let f = &m.func;
    if m.ctor || m.operator.is_some() {
        diags.push(Diagnostic::at(&codes::E0901, f.name_span, format!("`{ty}` cannot be given {} yet", if m.ctor { "constructors" } else { "operators" })));
        return None;
    }
    let params = f.params.iter().map(|p| g.resolve_binding_type(&p.ty, diags)).collect();
    let ret = g.resolve_return(&f.ret, diags);
    let (fails, outcome) = failures::signature(g, f, ret, true, diags);
    let sig = Sig { name: format!("{ty}::{}", f.name), params, ret, span: f.span, fails, outcome };
    check_signature_extras(g, f, &sig.params, m.is_static.is_none(), diags);
    if let Some(first) = g.ifaces.ext.get(&ty).and_then(|ms| ms.iter().find(|o| o.name == f.name)) {
        diags.push(
            Diagnostic::at(&codes::E0202, f.name_span, format!("`{ty}` has two methods named `{}`", f.name))
                .note("defined again here")
                .label(g.sigs[first.func].span, "first defined here"),
        );
        return None;
    }
    let func = g.add_sig(sig);
    g.ifaces.ext.entry(ty).or_default().push(MethodInfo { name: f.name.clone(), operator: None, func, is_static: m.is_static.is_some(), public: true });
    Some(func)
}

impl Checker<'_> {
    /// `value::name(args)` for a method that `T with I { }` added to a built-in type.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn ext_method(&mut self, recv: hir::Expr, recv_span: Span, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        let m = self.g.ifaces.ext.get(&recv.ty)?.iter().find(|m| m.name == name)?.clone();
        if m.is_static {
            self.diags.push(
                Diagnostic::at(&codes::E0213, name_span, format!("`{name}` is a static method: it is called on the type, not on a value"))
                    .note("called on a value")
                    .label(self.g.sigs[m.func].span, "declared `static`")
                    .help(format!("write `{}::{name}(...)`", recv.ty)),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let _ = (recv_span, targs);
        let args = self.args(m.func, args, span);
        let ty = self.g.sigs[m.func].ret?;
        let mut all = vec![recv];
        all.extend(args?);
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Call(m.func, all), ty)))
    }

    /// `uint32::name(args)` for a `static` method that `T with I { }` added.
    pub(super) fn ext_static(&mut self, ty: Type, name: &str, args: &[ast::Expr], span: Span) -> Option<Val> {
        let m = self.g.ifaces.ext.get(&ty)?.iter().find(|m| m.name == name && m.is_static)?.clone();
        let args = self.args(m.func, args, span);
        let ret = self.g.sigs[m.func].ret?;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Call(m.func, args?), ret)))
    }
}

impl Checker<'_> {
    /// In the definition of a generic: a call of an interface method that says
    /// `changes self` on a value that cannot be changed (a `&T`, or a `T` binding
    /// that is neither `mut` nor `open`). The instance could not tell, because
    /// its own method may change less than the interface allows.
    pub(super) fn changing_call(&mut self, recv: &hir::Expr, name: &str, name_span: Span) {
        let (code, message, note) = match &recv.kind {
            hir::ExprKind::Deref(inner) if matches!(inner.ty, Type::Ref { mutable: false, .. }) => (
                &codes::E0311,
                format!("cannot call `{name}` through a `&` borrow: `{name}` changes `self`"),
                "call attempted here".to_string(),
            ),
            hir::ExprKind::Local(id) if !self.locals[*id].mutable && !self.locals[*id].open && !self.locals[*id].is_self => (
                &codes::E0312,
                format!("cannot call `{name}` on `{}`: `{name}` changes `self`, and it is neither `mut` nor `open`", self.locals[*id].name),
                "call attempted here".to_string(),
            ),
            _ => return,
        };
        self.diags.push(Diagnostic::at(code, name_span, message).note(note).help("the interface says `changes self`: borrow it as `&mut T`, or declare the binding `mut`"));
    }
}
