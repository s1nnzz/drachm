//! `&Shape` and `&mut Shape`: borrows of an interface (S6E).
//!
//! An interface is only ever borrowed (owner, step 6 Q4). A `&Shape` is a fat
//! borrow of two words, the address of the value and the table (`hir::VTable`)
//! of its type for the interface, so `s::area()` runs the method of whatever
//! struct is behind it. Any interface can be borrowed this way (Q5); its
//! `static` methods and the methods that mention `Self` are just not callable
//! through the borrow (E0619). A `&Circle` becomes a `&Shape` where a `&Shape`
//! is wanted, and a `&Shape3D` becomes a `&Shape` the same way (an upcast).
//!
//! The conversion is `hir::ExprKind::Unsize`; the call is `DynCall`; a required
//! field is read as a `Field` of the `Type::Dyn` the borrow points at, whose
//! offset the backends load from the table.

use super::interfaces::{self, conforms, field_for, methods_of, sig_for};
use super::{Checker, Globals, Val, origin};
use crate::ast;
use crate::diag::{Diagnostic, Span, codes};
use crate::hir::{self, ExprKind as K};
use crate::types::{self, Type};

#[derive(Default)]
pub(super) struct State {
    /// The pairs (implementing type, interface) a borrow is converted at,
    /// each once, with the pairs for the interface's ancestors.
    uses: Vec<(Type, usize)>,
}

/// `&Shape` / `&mut Shape` as written: the fat borrow type. Only the plain form;
/// an interface in any other place is for `interfaces::as_value_type` to refuse.
pub(super) fn dyn_borrow(g: &Globals, t: &ast::TypeName) -> Option<Type> {
    let [(ast::TypeMod::Ref { mutable }, _)] = t.mods[..] else { return None };
    if !t.dims.is_empty() || t.optional.is_some() || t.unit.is_some() || !t.args.is_empty() {
        return None;
    }
    if g.tenv.contains_key(&t.name) || g.structs_by_name.contains_key(&t.name) || g.enums_by_name.contains_key(&t.name) {
        return None;
    }
    let &k = g.ifaces.by_name.get(&t.name)?;
    g.ifaces.list[k].usable.then(|| Type::borrow(mutable, dyn_of(g, k)))
}

/// `Type::Dyn` for interface `k`.
pub(super) fn dyn_of(g: &Globals, k: usize) -> Type {
    Type::Dyn { id: k, name: types::intern(&g.ifaces.list[k].name) }
}

/// Does the written type mention `Self` anywhere?
fn mentions_self(t: &ast::TypeName) -> bool {
    t.name == "Self" || t.args.iter().any(mentions_self)
}

/// The non-`static` methods of interface `k`, parents first: the methods a vtable has slots for.
fn dyn_methods(g: &Globals, k: usize) -> Vec<(interfaces::Flat, std::rc::Rc<ast::Interface>)> {
    g.ifaces.list[k]
        .methods
        .iter()
        .filter_map(|&fm| {
            let decl = g.ifaces.list[fm.owner].decl.clone();
            decl.methods[fm.idx].is_static.is_none().then_some((fm, decl))
        })
        .collect()
}

/// Can method `m` be called through a `&Iface`? No when it mentions `Self` or is an operator.
fn callable_shape(m: &ast::IMethod) -> bool {
    m.operator.is_none() && !mentions_self(&m.ret) && !m.params.iter().any(|p| mentions_self(&p.ty))
}

/// A conversion to `&Iface` is made at `ty`: its table (and those of the
/// interface's ancestors, for upcasts) will be emitted.
fn note_use(g: &mut Globals, ty: Type, k: usize) {
    let mut all = vec![k];
    all.extend(g.ifaces.list[k].ancestors.iter().copied().filter(|&a| a != k));
    for a in all {
        if !g.dynamic.uses.contains(&(ty, a)) {
            g.dynamic.uses.push((ty, a));
        }
    }
}

/// The interfaces and tables for the backends, once every body is checked
/// and the structs are laid out.
pub(super) fn finish(g: &mut Globals, structs: &[hir::StructDef]) -> (Vec<hir::DynIface>, Vec<hir::VTable>) {
    let mut ifaces = Vec::new();
    for k in 0..g.ifaces.list.len() {
        let mut methods = Vec::new();
        for (fm, decl) in dyn_methods(g, k) {
            let m = &decl.methods[fm.idx];
            let (params, ret) = sig_for(g, m, Type::Bool);
            let ok = callable_shape(m) && params.iter().all(Option::is_some) && ret.is_some_and(|r| !g.holds_borrow(r));
            methods.push(hir::DynMethod {
                name: m.name.clone(),
                params: params.into_iter().map(|p| p.unwrap_or(Type::Bool)).collect(),
                ret: ret.unwrap_or(Type::None),
                changes: fm.changes,
                callable: ok,
            });
        }
        let mut fields = Vec::new();
        for ff in g.ifaces.list[k].fields.clone() {
            let f = g.ifaces.list[ff.owner].decl.fields[ff.idx].clone();
            fields.push(hir::DynField { name: f.name.clone(), ty: field_for(g, &f, Type::Bool).unwrap_or(Type::Bool) });
        }
        let ancestors = g.ifaces.list[k].ancestors.iter().copied().filter(|&a| a != k).collect();
        ifaces.push(hir::DynIface { name: g.ifaces.list[k].name.clone(), ancestors, methods, fields });
    }
    let mut tables = Vec::new();
    for &(ty, k) in &g.dynamic.uses {
        if ty.struct_id().is_some_and(|s| g.structs[s].placeholder.is_some()) {
            continue;
        }
        let have = methods_of(g, ty);
        let methods = ifaces[k]
            .methods
            .iter()
            .map(|dm| if dm.callable { have.iter().find(|x| x.name == dm.name && x.operator.is_none()).map(|x| x.func) } else { None })
            .collect();
        let fields = ifaces[k]
            .fields
            .iter()
            .map(|f| ty.struct_id().and_then(|s| structs[s].fields.iter().find(|x| x.name == f.name)).map_or(0, |x| x.offset))
            .collect();
        tables.push(hir::VTable { ty, iface: k, methods, fields });
    }
    (ifaces, tables)
}

impl Checker<'_> {
    /// `x`, a borrow, where the fat borrow `to` is wanted: a `&Circle` for a
    /// `&Shape`, or a `&Shape3D` for a `&Shape` (E0640 when there is no way).
    pub(super) fn unsize(&mut self, x: hir::Expr, e: &ast::Expr, to: Type) -> Option<hir::Expr> {
        let Type::Ref { mutable: want_mut, to: &Type::Dyn { id: k, name: iname } } = to else { unreachable!("a fat borrow") };
        let Type::Ref { mutable: have_mut, to: src } = x.ty else { unreachable!("a borrow") };
        let shown = self.text(e.span).to_string();
        if have_mut && !want_mut {
            // The existing rule for a `&mut` where a `&` is wanted (owner, step 3).
            if matches!(e.kind, ast::ExprKind::Borrow { .. } | ast::ExprKind::Lend(_)) || !super::ast_place(e) {
                self.diags.push(Diagnostic::at(&codes::E0901, e.span, "using a `&mut` where a `&` is expected is not supported yet").note(format!("this is `{}`", x.ty)));
            } else {
                self.diags.push(
                    Diagnostic::at(&codes::E0330, e.span, format!("`{shown}` is a `{}`, but a `{to}` is wanted here", x.ty))
                        .note("passes the `&mut` itself")
                        .help(format!("write `&{shown}`: a shared borrow through the `&mut` you hold")),
                );
            }
            return None;
        }
        if !have_mut && want_mut {
            return self.into(Val::Expr(x), to, origin(e));
        }
        match *src {
            Type::Dyn { id: j, name: jname } => {
                if !self.g.ifaces.list[j].ancestors.contains(&k) {
                    self.diags.push(
                        Diagnostic::at(&codes::E0640, e.span, format!("`{jname}` does not extend `{iname}`, so `{shown}` is not a `{to}`"))
                            .note(format!("this is `{}`", x.ty))
                            .label(self.g.ifaces.list[j].name_span, format!("`{jname}` is declared here"))
                            .help(format!("add `extends {iname}` to `interface {jname}`")),
                    );
                    return None;
                }
            }
            src => {
                if !self.can_borrow_as(src, k, e.span, &shown, x.ty, to) {
                    return None;
                }
                note_use(self.g, src, k);
            }
        }
        Some(hir::Expr::new(K::Unsize(Box::new(x)), to).at(e.span))
    }

    /// Does a `&src` become a `&Iface` (`k`)? Reports why not.
    fn can_borrow_as(&mut self, src: Type, k: usize, at: Span, shown: &str, from: Type, to: Type) -> bool {
        let iname = self.g.ifaces.list[k].name.clone();
        if !conforms(self.g, src, k) {
            let mut d = Diagnostic::at(&codes::E0640, at, format!("`{src}` does not implement `{iname}`, so `{shown}` is not a `{to}`")).note(format!("this is `{from}`"));
            d = match src.struct_id() {
                Some(sid) => d.label(self.g.structs[sid].name_span, format!("`struct {src}` is declared here")).help(format!("add `with {iname}` to `struct {src}`")),
                None => d.help(format!("a type promises an interface with `with`: `{src} with {iname} {{ ... }}`")),
            };
            self.diags.push(d);
            return false;
        }
        if self.g.holds_borrow(src) {
            self.diags.push(
                Diagnostic::at(&codes::E0901, at, format!("a `&{src}` cannot become a `&{iname}` yet: `{src}` holds a borrow"))
                    .note("a holder's borrows would be lost in the conversion"),
            );
            return false;
        }
        let have = methods_of(self.g, src);
        let missing = dyn_methods(self.g, k).into_iter().find(|(fm, decl)| {
            let m = &decl.methods[fm.idx];
            callable_shape(m) && !have.iter().any(|x| x.name == m.name && x.operator.is_none())
        });
        if let Some((fm, decl)) = missing {
            let name = &decl.methods[fm.idx].name;
            self.diags.push(
                Diagnostic::at(&codes::E0901, at, format!("a `&{src}` cannot become a `&{iname}` yet: `{name}` is built in for `{src}`"))
                    .note("the compiler has no function to put in the table")
                    .help(format!("write the method yourself: `{src} with {iname} {{ ... }}`")),
            );
            return false;
        }
        true
    }

    /// `recv::name(args)` where `recv` is a place of interface type (behind a `&Iface`).
    #[allow(clippy::too_many_arguments)]
    pub(super) fn dyn_method(&mut self, recv: hir::Expr, recv_span: Span, name: &str, name_span: Span, targs: &[ast::TypeName], args: &[ast::Expr], _span: Span) -> Option<Val> {
        let Type::Dyn { id: k, name: iname } = recv.ty else { unreachable!("an interface") };
        let methods = dyn_methods(self.g, k);
        let found = methods.iter().position(|(fm, decl)| decl.methods[fm.idx].name == name);
        let Some(idx) = found else {
            let statics = self.g.ifaces.list[k].methods.iter().any(|fm| {
                let m = &self.g.ifaces.list[fm.owner].decl.methods[fm.idx];
                m.name == name && m.is_static.is_some()
            });
            for a in args {
                self.expr(a);
            }
            let shown = self.text(recv_span).to_string();
            if statics {
                self.diags.push(
                    Diagnostic::at(&codes::E0619, name_span, format!("`{name}` is `static`, so it needs a type, not a `&{iname}`"))
                        .note("called on a value")
                        .help(format!("call it on the type: `Type::{name}()`")),
                );
            } else {
                let names: Vec<String> = methods.iter().map(|(fm, d)| d.methods[fm.idx].name.clone()).filter(|n| callable_name(n)).collect();
                let mut d = Diagnostic::at(&codes::E0201, name_span, format!("`{iname}` has no method `{name}`")).label(recv_span, format!("this is a `&{iname}`"));
                let near: Vec<&str> = names.iter().map(String::as_str).collect();
                d = if let Some(n) = super::similar(name, &near) {
                    d.help(format!("did you mean `{n}`?"))
                } else if self.g.ifaces.list[k].fields.iter().any(|f| self.g.ifaces.list[f.owner].decl.fields[f.idx].name == name) {
                    d.help(format!("`{name}` is a field: read it with `{shown}.{name}`"))
                } else if names.is_empty() {
                    d.help(format!("`{iname}` lists no methods"))
                } else {
                    d.help(format!("its methods are {}", super::list(&near)))
                };
                self.diags.push(d);
            }
            return None;
        };
        let (fm, decl) = methods[idx].clone();
        let m = &decl.methods[fm.idx];
        self.no_targs(name, targs);
        if !callable_shape(m) {
            for a in args {
                self.expr(a);
            }
            let why = if m.operator.is_some() { "an operator" } else { "`Self` in its signature" };
            self.diags.push(
                Diagnostic::at(&codes::E0619, name_span, format!("`{name}` cannot be called through a `&{iname}`: it has {why}"))
                    .note("the type behind the borrow is not known here")
                    .help("call it on a value of the struct, or in a function constrained with `<T with ...>`"),
            );
            return None;
        }
        let (params, ret) = sig_for(self.g, m, Type::Bool);
        let (Some(ret), true) = (ret, params.iter().all(Option::is_some)) else {
            for a in args {
                self.expr(a);
            }
            return None;
        };
        if self.g.holds_borrow(ret) {
            for a in args {
                self.expr(a);
            }
            self.diags.push(
                Diagnostic::at(&codes::E0901, name_span, format!("`{name}` returns a borrow, which cannot be called through a `&{iname}` yet"))
                    .note("how long the result lives is not known here"),
            );
            return None;
        }
        let params: Vec<Type> = params.into_iter().flatten().collect();
        if params.len() != args.len() {
            let n = params.len();
            self.diags.push(
                Diagnostic::at(&codes::E0103, name_span, format!("`{iname}::{name}` takes {n} argument{}, but {} given", if n == 1 { "" } else { "s" }, args.len()))
                    .label(m.span, format!("`{name}` is declared here")),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        }
        let mut all = vec![recv];
        let mut ok = true;
        for (a, p) in args.iter().zip(&params) {
            let v = if p.is_borrow() { self.coerce_borrow(a, *p, None) } else { self.coerce(a, *p) };
            match v {
                Some(v) => all.push(v),
                None => ok = false,
            }
        }
        if !ok {
            return None;
        }
        Some(Val::Expr(hir::Expr::new(K::DynCall(k, idx, all), ret)))
    }

    /// Required field `name` of interface-typed place of interface `k`: its index and type.
    pub(super) fn dyn_field(&mut self, k: usize, name: &str, name_span: Span, recv_span: Span) -> Option<(usize, Type)> {
        let iname = self.g.ifaces.list[k].name.clone();
        let fields = self.g.ifaces.list[k].fields.clone();
        let Some(i) = fields.iter().position(|ff| self.g.ifaces.list[ff.owner].decl.fields[ff.idx].name == name) else {
            let names: Vec<String> = fields.iter().map(|ff| self.g.ifaces.list[ff.owner].decl.fields[ff.idx].name.clone()).collect();
            let near: Vec<&str> = names.iter().map(String::as_str).collect();
            let shown = self.text(recv_span).to_string();
            let is_method = self.g.ifaces.list[k].methods.iter().any(|fm| self.g.ifaces.list[fm.owner].decl.methods[fm.idx].name == name);
            let mut d = Diagnostic::at(&codes::E0201, name_span, format!("`{iname}` has no field `{name}`")).label(recv_span, format!("this is a `&{iname}`"));
            d = if is_method {
                d.help(format!("`{name}` is a method: call it with `{shown}::{name}()`"))
            } else if let Some(n) = super::similar(name, &near) {
                d.help(format!("did you mean `{n}`?"))
            } else if near.is_empty() {
                d.help(format!("`{iname}` requires no fields, so a `&{iname}` has none to read"))
            } else {
                d.help(format!("its fields are {}", super::list(&near)))
            };
            self.diags.push(d);
            return None;
        };
        let ff = fields[i];
        let f = self.g.ifaces.list[ff.owner].decl.fields[ff.idx].clone();
        let ty = field_for(self.g, &f, Type::Bool)?;
        Some((i, ty))
    }

    /// A write to a required field through a `&mut Iface`: undecided, so refused.
    pub(super) fn dyn_field_write(&mut self, name: &str, at: Span) {
        self.diags.push(
            Diagnostic::at(&codes::E0901, at, format!("changing the field `{name}` through a borrow of an interface is not supported yet"))
                .note("a required field can be read through a `&Iface`")
                .help("change it through the struct, or add a method that does"),
        );
    }
}

fn callable_name(n: &str) -> bool {
    n.chars().next().is_some_and(|c| c.is_alphabetic() || c == '_')
}
