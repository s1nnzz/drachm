//! Names across modules (step 7, WP2).
//!
//! The checker knows one flat set of names. This pass gives it one: it reads
//! the module graph, works out what every name means in the file that writes
//! it, rewrites each name to the program-wide name of the thing it stands for,
//! and joins the files into one `ast::Program`.
//!
//! Program-wide names (engineering choice): the entry file's declarations keep
//! their names, so a one-file program is unchanged; a declaration of another
//! module is `MODULE::name` with the module's dotted path (`game.physics::Vector3`).
//! Backends turn that into a symbol with `types::mangle`; diagnostics show it
//! as written, which is also how a qualified name reads in the source.
//!
//! What each file sees (owner, step 7): its own declarations; the modules it
//! imports, under the last part of the path or the `as` name; the exported
//! items it imports by name (`in X as Y from m`); and with `in * from m` every
//! exported name (`as y` puts them under `y::` only, `also bare` also bare).
//! Only exported names are reachable from another file. A local declaration
//! beats an imported name; between `in *` imports the first wins, with a
//! warning at the later import. `X with I { }` blocks and `def`s are global
//! once joined, which is "visible wherever the type is visible".
//!
//! Also checked here, because the call graph is at hand: functions of
//! different files must not call each other in a circle (owner, Q8).

use std::collections::{HashMap, HashSet, VecDeque};

use crate::ast::*;
use crate::diag::{codes, Diagnostic, Span};
use crate::modules::{ModuleGraph, Target};

#[derive(Clone)]
struct Item {
    global: String,
    func: bool,
}

#[derive(Clone)]
enum Bind {
    /// Something declared in this file (`import` is `None`) or imported
    /// (the index of the `in` line).
    Item { item: Item, import: Option<usize> },
    Module(Target),
}

struct Decl {
    name: String,
    global: String,
    func: bool,
    span: Span,
    /// For a `def`: the struct it defines on.
    def_of: Option<String>,
}

struct Info {
    name: String,
    prefix: String,
    decls: Vec<Decl>,
    /// `shown name -> declared name`.
    exports: Vec<(String, String)>,
}

impl Info {
    /// The names other files can use: `(shown, global, is a function)`.
    fn exported(&self) -> Vec<(String, Item)> {
        let mut out: Vec<(String, Item)> = Vec::new();
        for (shown, declared) in &self.exports {
            for d in self.decls.iter().filter(|d| d.name == *declared) {
                out.push((shown.clone(), Item { global: d.global.clone(), func: d.func }));
            }
            // The defs of an exported struct go with it.
            for d in self.decls.iter().filter(|d| d.def_of.as_deref() == Some(declared) && d.name != *declared) {
                out.push((d.name.clone(), Item { global: d.global.clone(), func: false }));
            }
        }
        out
    }

    fn find(&self, name: &str) -> Option<Item> {
        self.exported().into_iter().find(|(s, _)| s == name).map(|(_, i)| i)
    }
}

struct Ctx {
    infos: Vec<Info>,
    scopes: Vec<HashMap<String, Bind>>,
    /// Imports of each module that are the standard `io`.
    io_imported: Vec<bool>,
    /// Imports of each module that are the standard `si` (its units are per file).
    si_imported: Vec<bool>,
    /// The names of the units `in si;` brings.
    si_names: HashSet<String>,
}

/// Resolve the names of every module and join them into one program.
pub fn resolve(graph: &mut ModuleGraph, diags: &mut Vec<Diagnostic>) -> Program {
    let n = graph.modules.len();
    let mut infos = Vec::new();
    for (i, m) in graph.modules.iter().enumerate() {
        infos.push(info_of(i, &m.name, &m.program));
    }
    let si_names = crate::typeck::si_unit_names().into_iter().collect();
    let mut cx = Ctx { infos, scopes: Vec::new(), io_imported: Vec::new(), si_imported: Vec::new(), si_names };
    for i in 0..n {
        let (scope, io, si) = build_scope(&cx.infos, i, graph, diags);
        cx.scopes.push(scope);
        cx.io_imported.push(io);
        cx.si_imported.push(si);
    }

    let mut merged = Program::default();
    merged.unsafe_allowed = graph.modules[0].program.unsafe_allowed;
    let mut edges: Vec<(String, String, Span)> = Vec::new();
    let mut func_module: HashMap<String, usize> = HashMap::new();
    for i in 0..n {
        let mut prog = std::mem::take(&mut graph.modules[i].program);
        let mut w = Walker { cx: &cx, m: i, diags, locals: Vec::new(), tparams: Vec::new(), calls: Vec::new(), extra_generics: Vec::new(), edges: Vec::new(), module_spans: Vec::new(), si_reported: HashSet::new() };
        w.program(&mut prog);
        // A `::` after a call on a module is the chain's first call (`m::f()::g()`);
        // on anything else it is the E0007 the parser held back.
        for slip in std::mem::take(&mut prog.chain_slips) {
            if !w.module_spans.contains(&slip.owner_span) {
                w.diags.push(slip.diag);
            }
        }
        for f in &prog.funcs {
            func_module.insert(f.name.clone(), i);
        }
        edges.extend(std::mem::take(&mut w.edges));
        let extra = std::mem::take(&mut w.extra_generics);
        // Standard module imports stay, once each.
        for (k, imp) in prog.imports.iter().enumerate() {
            let std = matches!(graph.modules[i].targets.get(k), Some(Some(Target::Std(_))));
            if std && imp.plain_name().is_some() && !merged.imports.iter().any(|x| x.path == imp.path) {
                merged.imports.push(Import { path: imp.path.clone(), path_span: imp.path_span, word_form: false, items: Vec::new(), glob: false, alias: None, also_bare: false, file: None, span: imp.span });
            }
        }
        merged.units.append(&mut prog.units);
        merged.funcs.append(&mut prog.funcs);
        merged.structs.append(&mut prog.structs);
        merged.enums.append(&mut prog.enums);
        merged.interfaces.append(&mut prog.interfaces);
        merged.impls.append(&mut prog.impls);
        merged.failures.append(&mut prog.failures);
        merged.defs.append(&mut prog.defs);
        merged.consts.append(&mut prog.consts);
        merged.optionals.append(&mut prog.optionals);
        merged.generics.append(&mut prog.generics);
        merged.generics.extend(extra);
    }
    let names: Vec<String> = cx.infos.iter().map(|i| i.name.clone()).collect();
    cross_file_recursion(&edges, &func_module, &names, diags);
    merged
}

fn info_of(index: usize, name: &str, p: &Program) -> Info {
    let prefix = if index == 0 { String::new() } else { format!("{name}::") };
    let g = |n: &str| format!("{prefix}{n}");
    let mut decls = Vec::new();
    for f in &p.funcs {
        decls.push(Decl { name: f.name.clone(), global: g(&f.name), func: true, span: f.name_span, def_of: None });
    }
    for s in &p.structs {
        decls.push(Decl { name: s.name.clone(), global: g(&s.name), func: false, span: s.name_span, def_of: None });
    }
    for e in &p.enums {
        decls.push(Decl { name: e.name.clone(), global: g(&e.name), func: false, span: e.name_span, def_of: None });
    }
    for i in &p.interfaces {
        decls.push(Decl { name: i.name.clone(), global: g(&i.name), func: false, span: i.name_span, def_of: None });
    }
    for f in &p.failures {
        decls.push(Decl { name: f.name.clone(), global: g(&f.name), func: false, span: f.name_span, def_of: None });
    }
    for u in &p.units {
        decls.push(Decl { name: u.name.clone(), global: g(&u.name), func: false, span: u.name_span, def_of: None });
    }
    for c in &p.consts {
        decls.push(Decl { name: c.name.clone(), global: g(&c.name), func: false, span: c.name_span, def_of: None });
    }
    for d in &p.defs {
        let of = d.of.as_ref().map_or(d.name.clone(), |o| o.0.clone());
        decls.push(Decl { name: d.name.clone(), global: g(&d.name), func: false, span: d.name_span, def_of: Some(of) });
    }
    let exports = p.exports.iter().map(|e| (e.shown_as().to_string(), e.name.clone())).collect();
    Info { name: name.to_string(), prefix, decls, exports }
}

fn build_scope(infos: &[Info], m: usize, graph: &ModuleGraph, diags: &mut Vec<Diagnostic>) -> (HashMap<String, Bind>, bool, bool) {
    let info = &infos[m];
    let prog = &graph.modules[m].program;
    let mut scope: HashMap<String, Bind> = HashMap::new();
    let mut io = false;
    let mut si = false;
    for d in &info.decls {
        // Two declarations with one name are the checker's E0202.
        scope.entry(d.name.clone()).or_insert(Bind::Item { item: Item { global: d.global.clone(), func: d.func }, import: None });
    }
    let own_span = |name: &str| info.decls.iter().find(|d| d.name == name).map(|d| d.span);
    for (idx, imp) in prog.imports.iter().enumerate() {
        let Some(Some(target)) = graph.modules[m].targets.get(idx) else { continue };
        let j = match target {
            Target::Std(name) => {
                if imp.plain_name().is_some() || imp.binding().is_some() && imp.items.is_empty() && !imp.glob && imp.alias.is_none() {
                    io |= *name == "io";
                    si |= *name == "si";
                } else {
                    diags.push(
                        Diagnostic::at(&codes::E0901, imp.span, format!("this form of `in` is not supported for the standard module `{name}` yet"))
                            .help(format!("import it whole: `in {name};`")),
                    );
                }
                continue;
            }
            Target::File(j) => *j,
        };
        let from = &infos[j];
        // The module's name in this file.
        let bind_module = |scope: &mut HashMap<String, Bind>, name: &str, at: Span, diags: &mut Vec<Diagnostic>| match scope.get(name) {
            Some(Bind::Item { import: None, .. }) => {
                diags.push(
                    Diagnostic::at(&codes::W0241, own_span(name).unwrap_or(at), format!("`{name}` hides the module `{}` imported in this file", imp.path.join(".")))
                        .note("this name wins inside this file")
                        .help(format!("to use the module too, rename it with `as`: `in {} as other{};`", imp.path.join("."), cap(name))),
                );
            }
            Some(Bind::Item { .. }) => diags.push(clash(name, at, &imp.path)),
            Some(Bind::Module(_)) => {}
            None => {
                scope.insert(name.to_string(), Bind::Module(*target));
            }
        };
        if imp.glob {
            if let Some((alias, aspan)) = &imp.alias {
                bind_module(&mut scope, alias, *aspan, diags);
            }
            if imp.alias.is_some() && !imp.also_bare {
                continue;
            }
            for (shown, item) in from.exported() {
                match scope.get(&shown) {
                    None => {
                        scope.insert(shown, Bind::Item { item, import: Some(idx) });
                    }
                    Some(Bind::Item { item: have, .. }) if have.global == item.global => {}
                    Some(existing) => {
                        let hidden_by = match existing {
                            Bind::Item { import: None, .. } => "your own declaration".to_string(),
                            Bind::Item { import: Some(k), .. } => format!("the import on line {}", line_number(graph, prog.imports[*k].span)),
                            Bind::Module(_) => "a module of that name".to_string(),
                        };
                        // The module of this import: the name it is given, or (without `as`) the
                        // last part of its path, which `as` must name first.
                        let module_hint = match &imp.alias {
                            Some((alias, _)) => format!("write `{alias}::{shown}`"),
                            None => {
                                let last = imp.path.last().map(String::as_str).unwrap_or_default();
                                format!("write `{last}::{shown}` after `in * from {} as {last} also bare;`", imp.path.join("."))
                            }
                        };
                        diags.push(
                            Diagnostic::at(&codes::W0240, imp.span, format!("`{shown}` from `{}` is hidden by {hidden_by}", imp.path.join(".")))
                                .note("this import brings a name that is already taken")
                                .note_line(format!("to use both, import by name with `as`: `in {shown} as other{} from {};`", cap(&shown), imp.path.join(".")))
                                .help(format!("did you mean to use the module's `{shown}`? {module_hint}")),
                        );
                    }
                }
            }
            continue;
        }
        if !imp.items.is_empty() {
            for it in &imp.items {
                let bound = it.alias.clone().unwrap_or_else(|| it.name.clone());
                let found = from.exported().into_iter().filter(|(s, _)| *s == it.name).collect::<Vec<_>>();
                if found.is_empty() {
                    diags.push(missing_name(&it.name, it.span, from, &imp.path));
                    continue;
                }
                let new = found[0].1.clone();
                match scope.get(&bound) {
                    Some(Bind::Item { item, .. }) if item.global == new.global => continue,
                    Some(_) => {
                        diags.push(clash(&bound, it.span, &imp.path));
                        continue;
                    }
                    None => {}
                }
                for (shown, item) in found {
                    let key = if shown == it.name { bound.clone() } else { shown };
                    scope.insert(key, Bind::Item { item, import: Some(idx) });
                }
                // The defs of the struct come with it.
                if let Some(Bind::Item { item, .. }) = scope.get(&bound).cloned() {
                    let defs = from.decls.iter().filter(|d| d.def_of.as_deref() == Some(it.name.as_str()) && d.name != it.name && d.global != item.global);
                    for d in defs.collect::<Vec<_>>() {
                        scope.entry(d.name.clone()).or_insert(Bind::Item { item: Item { global: d.global.clone(), func: false }, import: Some(idx) });
                    }
                }
            }
            continue;
        }
        // A module: under its alias or the last part of the path.
        if let Some(name) = imp.binding() {
            let at = imp.alias.as_ref().map_or(imp.path_span, |a| a.1);
            bind_module(&mut scope, name, at, diags);
        }
    }
    (scope, io, si)
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// The line of `span` in its file, counting from 1.
fn line_number(graph: &ModuleGraph, span: Span) -> usize {
    let (file, local) = graph.map.locate(span);
    graph.map.source(file).line_of(local.start) + 1
}

fn clash(name: &str, at: Span, path: &[String]) -> Diagnostic {
    Diagnostic::at(&codes::E0232, at, format!("`{name}` is already taken in this file"))
        .note("a second name with this spelling")
        .help(format!("give one of them a name of its own with `as`: `in {name} as {name}2 from {};`", path.join(".")))
}

fn missing_name(name: &str, at: Span, from: &Info, path: &[String]) -> Diagnostic {
    let module = path.join(".");
    if from.decls.iter().any(|d| d.name == name) {
        Diagnostic::at(&codes::E0236, at, format!("`{name}` is private to the module `{module}`"))
            .note("not exported")
            .help(format!("add `out` before its declaration in `{module}`, or list it: `out {name};`"))
    } else {
        let mut d = Diagnostic::at(&codes::E0237, at, format!("the module `{module}` has no `{name}`")).note("no such name");
        let have: Vec<String> = from.exported().into_iter().map(|(s, _)| s).collect();
        if have.is_empty() {
            d = d.help(format!("`{module}` exports nothing yet: mark what it offers with `out`"));
        } else {
            d = d.help(format!("`{module}` exports: {}", have.join(", ")));
        }
        d
    }
}

struct Walker<'a> {
    cx: &'a Ctx,
    m: usize,
    diags: &'a mut Vec<Diagnostic>,
    locals: Vec<String>,
    tparams: Vec<String>,
    /// Function calls found in the body being walked.
    calls: Vec<(String, Span)>,
    extra_generics: Vec<TypeName>,
    /// `(function, function it calls, where)` for every free function walked.
    edges: Vec<(String, String, Span)>,
    /// The spans of names that are modules where they are used as a call's
    /// receiver (`physics` in `physics::gravity(2.0)`).
    module_spans: Vec<Span>,
    /// SI units this file used without `in si;`, reported once each.
    si_reported: HashSet<String>,
}

impl Walker<'_> {
    fn scope(&self) -> &HashMap<String, Bind> {
        &self.cx.scopes[self.m]
    }

    fn own(&self, name: &str) -> String {
        format!("{}{}", self.cx.infos[self.m].prefix, name)
    }

    // ---- names ----

    /// The checker's name for `module::name`, with the visibility check.
    fn member(&mut self, module: usize, name: &str, at: Span) -> Option<Item> {
        let info = &self.cx.infos[module];
        if let Some(i) = info.find(name) {
            return Some(i);
        }
        let path: Vec<String> = info.name.split('.').map(String::from).collect();
        let d = missing_name(name, at, info, &path);
        self.diags.push(d);
        None
    }

    fn module_of(&self, name: &str) -> Option<usize> {
        match self.scope().get(name) {
            Some(Bind::Module(Target::File(j))) => Some(*j),
            _ => None,
        }
    }

    /// Rewrite a type-position name (`Vector3`, `phys::Vector3`).
    fn type_name(&mut self, name: &mut String, span: Span) {
        if let Some((first, rest)) = name.split_once("::") {
            let (first, rest) = (first.to_string(), rest.to_string());
            if self.locals.contains(&first) {
                return;
            }
            if let Some(j) = self.module_of(&first)
                && !rest.contains("::")
                && let Some(item) = self.member(j, &rest, span)
            {
                *name = item.global;
            }
            return;
        }
        if self.tparams.contains(name) {
            return;
        }
        if let Some(Bind::Item { item, .. }) = self.scope().get(name.as_str()) {
            *name = item.global.clone();
        }
    }

    fn global_of(&self, name: &str) -> Option<Item> {
        match self.scope().get(name) {
            Some(Bind::Item { item, .. }) => Some(item.clone()),
            _ => None,
        }
    }

    fn ty(&mut self, t: &mut TypeName) {
        if !t.name.is_empty() {
            self.type_name(&mut t.name, t.span);
        }
        if let Some(f) = &mut t.func {
            for p in &mut f.params {
                self.ty(p);
            }
            self.ty(&mut f.ret);
        }
        for a in &mut t.args {
            self.ty(a);
        }
        if let Some(u) = &mut t.unit {
            self.unit_spec(u);
        }
        if let Some(u) = &mut t.unit_arg {
            self.unit_spec(u);
        }
        if let Some(v) = &mut t.val {
            self.const_expr(v);
        }
        for d in &mut t.dims {
            match d {
                Dim::Param(name, at) if !self.tparams.iter().any(|p| p == name) => {
                    let mut owned = name.to_string();
                    if owned.contains("::") {
                        self.type_name(&mut owned, *at);
                        *name = Box::leak(owned.into_boxed_str());
                    } else if let Some(i) = self.global_of(name) {
                        *name = Box::leak(i.global.into_boxed_str());
                    }
                }
                Dim::Calc(c, _) => {
                    let mut owned = (*c).clone();
                    self.const_expr(&mut owned);
                    *c = Box::leak(Box::new(owned));
                }
                _ => {}
            }
        }
    }

    fn const_expr(&mut self, c: &mut ConstExpr) {
        match c {
            ConstExpr::Name(n, _) => {
                if !self.tparams.contains(n) && let Some(i) = self.global_of(n) {
                    *n = i.global;
                }
            }
            ConstExpr::Neg(a, _) | ConstExpr::Not(a, _) => self.const_expr(a),
            ConstExpr::Bin(_, a, b, _) => {
                self.const_expr(a);
                self.const_expr(b);
            }
            ConstExpr::Num(..) => {}
        }
    }

    fn unit_spec(&mut self, u: &mut UnitSpec) {
        self.unit_expr(&mut u.expr);
    }

    /// `name` is not a unit of this file: if it is an SI unit and the file has
    /// no `in si;`, that is the error (the SI units are joined into the
    /// program for every file, so the checker alone would accept it).
    fn si_unit(&mut self, name: &str, at: Span) {
        if self.cx.si_imported[self.m] || !self.cx.si_names.contains(name) || !self.si_reported.insert(name.to_string()) {
            return;
        }
        self.diags.push(
            Diagnostic::at(&codes::E0402, at, format!("unknown unit `{name}`"))
                .note("an SI unit, and this file does not import `si`")
                .help("add `in si;` at the top of this file: every file imports what it uses"),
        );
    }

    fn unit_expr(&mut self, e: &mut UnitExpr) {
        match e {
            UnitExpr::Name(n, at) => {
                if !self.tparams.contains(n) {
                    match self.global_of(n) {
                        Some(i) => *n = i.global,
                        None => self.si_unit(n, *at),
                    }
                }
            }
            UnitExpr::Explicit(n, at) => match self.global_of(n) {
                Some(i) => *n = i.global,
                None => self.si_unit(n, *at),
            },
            UnitExpr::Mul(a, b) | UnitExpr::Div(a, b) => {
                self.unit_expr(a);
                self.unit_expr(b);
            }
            UnitExpr::Pow(a, _, _) => self.unit_expr(a),
            UnitExpr::One(_) => {}
        }
    }

    fn tparam_list(&mut self, ps: &mut [TypeParam]) {
        for p in ps.iter() {
            self.tparams.push(p.name.clone());
        }
        for p in ps.iter_mut() {
            for b in &mut p.bounds {
                self.ty(b);
            }
        }
    }

    // ---- declarations ----

    fn program(&mut self, p: &mut Program) {
        for u in &mut p.units {
            u.name = self.own(&u.name);
            if let Some(d) = &mut u.def && let Some(e) = &mut d.expr {
                self.unit_expr(e);
            }
        }
        for c in &mut p.consts {
            c.name = self.own(&c.name);
            self.ty(&mut c.ty);
            if let Some(v) = &mut c.value {
                self.init(v);
            }
        }
        for s in &mut p.structs {
            self.struct_decl(s);
        }
        for e in &mut p.enums {
            e.name = self.own(&e.name);
            let mark = self.tparams.len();
            self.tparam_list(&mut e.tparams);
            for v in &mut e.variants {
                for f in &mut v.fields {
                    self.ty(&mut f.ty);
                }
            }
            self.tparams.truncate(mark);
        }
        for i in &mut p.interfaces {
            i.name = self.own(&i.name);
            let mark = self.tparams.len();
            self.tparam_list(&mut i.tparams);
            for e in &mut i.extends {
                self.ty(e);
            }
            for m in &mut i.methods {
                self.ty(&mut m.ret);
                for q in &mut m.params {
                    self.ty(&mut q.ty);
                }
            }
            for f in &mut i.fields {
                self.ty(&mut f.ty);
            }
            self.tparams.truncate(mark);
        }
        for f in &mut p.failures {
            f.name = self.own(&f.name);
            for v in &mut f.variants {
                for fl in &mut v.fields {
                    self.ty(&mut fl.ty);
                }
                self.locals = v.fields.iter().map(|x| x.name.clone()).collect();
                self.expr(&mut v.message);
                self.locals.clear();
            }
        }
        for d in &mut p.defs {
            d.name = self.own(&d.name);
            if let Some((o, sp)) = &mut d.of {
                self.type_name(o, *sp);
            }
            for a in &mut d.of_args {
                self.ty(a);
            }
            if let Some((f, sp)) = &mut d.from {
                self.type_name(f, *sp);
            }
            for a in &mut d.assigns {
                self.expr(&mut a.value);
            }
        }
        for i in &mut p.impls {
            self.ty(&mut i.ty);
            for w in &mut i.with {
                self.ty(w);
            }
            for m in &mut i.methods {
                self.method(m);
            }
        }
        for f in &mut p.funcs {
            f.name = self.own(&f.name);
            let name = f.name.clone();
            self.calls.clear();
            self.func(f);
            let calls = std::mem::take(&mut self.calls);
            self.edges.extend(calls.into_iter().map(|(to, sp)| (name.clone(), to, sp)));
        }
        for t in p.optionals.iter_mut().chain(p.generics.iter_mut()) {
            self.ty(t);
        }
    }

    fn struct_decl(&mut self, s: &mut Struct) {
        s.name = self.own(&s.name);
        let mark = self.tparams.len();
        self.tparam_list(&mut s.tparams);
        for w in &mut s.with {
            self.ty(w);
        }
        if let Some((d, sp)) = &mut s.defaults {
            self.type_name(d, *sp);
        }
        for f in &mut s.fields {
            self.ty(&mut f.ty);
        }
        for m in &mut s.methods {
            if m.ctor {
                m.func.name = s.name.clone();
            }
            self.method(m);
        }
        self.tparams.truncate(mark);
    }

    fn method(&mut self, m: &mut Method) {
        self.func(&mut m.func);
        // Calls from methods are not part of the cross-file check.
        self.calls.clear();
    }

    fn func(&mut self, f: &mut Func) {
        let mark = self.tparams.len();
        self.tparam_list(&mut f.tparams);
        for p in &mut f.params {
            self.ty(&mut p.ty);
        }
        self.ty(&mut f.ret);
        for (n, sp) in &mut f.fails {
            self.type_name(n, *sp);
        }
        self.locals = f.params.iter().map(|p| p.name.clone()).collect();
        self.block(&mut f.body);
        self.locals.clear();
        self.tparams.truncate(mark);
    }

    // ---- statements ----

    fn block(&mut self, b: &mut Block) {
        let mark = self.locals.len();
        for s in &mut b.stmts {
            self.stmt(s);
        }
        self.locals.truncate(mark);
    }

    fn init(&mut self, i: &mut Init) {
        match i {
            Init::Expr(e) => self.expr(e),
            Init::Fields(vs, _) => {
                for v in vs {
                    self.expr(&mut v.value);
                }
            }
        }
    }

    fn stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Let { ty, name, value, .. } => {
                self.ty(ty);
                if let Some(v) = value {
                    self.init(v);
                }
                self.locals.push(name.clone());
            }
            Stmt::Assign { target, value, .. } => {
                self.expr(target);
                self.expr(value);
            }
            Stmt::If { branches, els, .. } => {
                for (c, b) in branches {
                    self.expr(c);
                    self.block(b);
                }
                if let Some(b) = els {
                    self.block(b);
                }
            }
            Stmt::While { cond, body, .. } => {
                self.expr(cond);
                self.block(body);
            }
            Stmt::For { init, cond, step, body, .. } => {
                let mark = self.locals.len();
                if let Some(i) = init {
                    self.stmt(i);
                }
                if let Some(c) = cond {
                    self.expr(c);
                }
                if let Some(st) = step {
                    self.stmt(st);
                }
                self.block(body);
                self.locals.truncate(mark);
            }
            Stmt::ForEach { ty, name, iter, step, body, .. } => {
                self.ty(ty);
                self.expr(iter);
                if let Some(st) = step {
                    self.expr(st);
                }
                let mark = self.locals.len();
                self.locals.push(name.clone());
                self.block(body);
                self.locals.truncate(mark);
            }
            Stmt::ForEntries { key, value, iter, body, .. } => {
                self.ty(&mut key.ty);
                self.ty(&mut value.ty);
                self.expr(iter);
                let mark = self.locals.len();
                self.locals.push(key.name.clone());
                self.locals.push(value.name.clone());
                self.block(body);
                self.locals.truncate(mark);
            }
            Stmt::Match { value, cases, .. } => {
                self.expr(value);
                for c in cases {
                    let mark = self.locals.len();
                    for p in &mut c.patterns {
                        self.pattern(p);
                    }
                    if let Some(g) = &mut c.guard {
                        self.expr(g);
                    }
                    self.block(&mut c.body);
                    self.locals.truncate(mark);
                }
            }
            Stmt::Break { .. } | Stmt::Skip { .. } => {}
            Stmt::Return { value, .. } | Stmt::Resume { value, .. } => {
                if let Some(v) = value {
                    self.expr(v);
                }
            }
            Stmt::Fail { value, because, .. } => {
                self.expr(value);
                if let Some(b) = because {
                    self.expr(b);
                }
            }
            Stmt::Unsafe { body, .. } => self.block(body),
            Stmt::Destroy { target, .. } => self.expr(target),
            Stmt::Fill { with, body, .. } => {
                if let Some(FillWith::Named(n, sp)) = with {
                    self.type_name(n, *sp);
                }
                self.block(body);
            }
            Stmt::Expr(e) => self.expr(e),
        }
    }

    fn pattern(&mut self, p: &mut Pattern) {
        match p {
            Pattern::Any(_) => {}
            Pattern::Expr(e) | Pattern::Glob(e) => self.expr(e),
            Pattern::Variant { enum_name, args, .. } => {
                if let Some((n, sp)) = enum_name {
                    self.type_name(n, *sp);
                }
                for a in args {
                    self.sub_pattern(a);
                }
            }
            Pattern::Bind { ty, name, .. } => {
                self.ty(ty);
                self.locals.push(name.clone());
            }
        }
    }

    fn sub_pattern(&mut self, p: &mut SubPat) {
        match p {
            SubPat::Any(_) => {}
            SubPat::Bind { ty, name, .. } => {
                self.ty(ty);
                self.locals.push(name.clone());
            }
            SubPat::Value(e) | SubPat::Glob(e) => self.expr(e),
            SubPat::Variant(p) => self.pattern(p),
        }
    }

    // ---- expressions ----

    fn call_target(&mut self, name: &mut String, at: Span) {
        if self.locals.contains(name) {
            return;
        }
        if let Some(i) = self.global_of(name) {
            if i.func {
                self.calls.push((i.global.clone(), at));
            }
            *name = i.global;
        }
    }

    fn expr(&mut self, e: &mut Expr) {
        let span = e.span;
        // A module-qualified name is rewritten to the plain name first.
        match &mut e.kind {
            ExprKind::Method { recv, name, name_span, targs, unit: None, args } => {
                let module = match &recv.kind {
                    ExprKind::Var(v) if !self.locals.contains(v) => self.module_of(v),
                    _ => None,
                };
                if let Some(j) = module {
                    self.module_spans.push(recv.span);
                    let (name_span, targs_taken, args_taken) = (*name_span, std::mem::take(targs), std::mem::take(args));
                    let Some(item) = self.member(j, name, name_span) else {
                        e.kind = ExprKind::NoneValue;
                        return;
                    };
                    if item.func {
                        self.calls.push((item.global.clone(), span));
                    }
                    if !targs_taken.is_empty() {
                        self.extra_generics.push(TypeName { args: targs_taken.clone(), ..TypeName::plain(item.global.clone(), name_span) });
                    }
                    e.kind = ExprKind::Call { name: item.global, targs: targs_taken, args: args_taken };
                }
            }
            ExprKind::Path { owner, owner_span, name, name_span } => {
                let (os, ns) = (*owner_span, *name_span);
                if !owner.contains("::") && !self.locals.contains(owner) && let Some(j) = self.module_of(owner) {
                    // `module::NAME`: a constant (or a type used as a value).
                    let name = name.clone();
                    match self.member(j, &name, ns) {
                        Some(item) => e.kind = ExprKind::Var(item.global),
                        None => e.kind = ExprKind::NoneValue,
                    }
                    return;
                }
                self.type_name(owner, os);
                return;
            }
            _ => {}
        }
        match &mut e.kind {
            ExprKind::Num(n) => {
                if let Some(u) = &mut n.unit {
                    self.unit_spec(u);
                }
            }
            ExprKind::Bool(_) | ExprKind::SelfValue | ExprKind::NoneValue | ExprKind::Path { .. } => {}
            ExprKind::Str(parts) => {
                for p in parts {
                    if let StrPart::Interp(x) = p {
                        self.expr(x);
                    }
                }
            }
            ExprKind::Var(n) => {
                if !self.locals.contains(n) && let Some(i) = self.global_of(n) {
                    *n = i.global;
                }
            }
            ExprKind::TypeArgs { name, targs } => {
                self.type_name(name, span);
                for t in targs {
                    self.ty(t);
                }
            }
            ExprKind::Field { recv, .. } => self.expr(recv),
            ExprKind::Unary(_, a) | ExprKind::Deref(a) | ExprKind::CopyOf(a) | ExprKind::Lend(a) | ExprKind::AddressOf(a) | ExprKind::PassUp(a) | ExprKind::Now(a) => self.expr(a),
            ExprKind::Binary(_, a, b) | ExprKind::Maybe { value: a, fallback: b } => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::IncDec { target, .. } => self.expr(target),
            ExprKind::Borrow { operand, .. } => self.expr(operand),
            ExprKind::Lambda { params, body, .. } => {
                let mark = self.locals.len();
                for p in params.iter_mut() {
                    self.ty(&mut p.ty);
                    self.locals.push(p.name.clone());
                }
                match body {
                    LambdaBody::Expr(x) => self.expr(x),
                    LambdaBody::Block(b) => self.block(b),
                }
                self.locals.truncate(mark);
            }
            ExprKind::Call { name, targs, args } => {
                self.call_target(name, span);
                for t in targs {
                    self.ty(t);
                }
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Method { recv, targs, args, unit, name, .. } => {
                // `io::outln` and friends: the standard module must be imported by this file.
                if let ExprKind::Var(v) = &recv.kind
                    && v == "io"
                    && !self.locals.contains(v)
                    && self.cx.io_imported[self.m]
                {
                    self.module_spans.push(recv.span);
                }
                if let ExprKind::Var(v) = &recv.kind
                    && v == "io"
                    && !self.locals.contains(v)
                    && !self.cx.io_imported[self.m]
                    && self.scope().get("io").is_none()
                {
                    self.diags.push(
                        Diagnostic::at(&codes::E0206, recv.span, "module `io` is used but not imported")
                            .note("not imported in this file")
                            .help("add `in io;` at the top of this file: every file imports what it uses"),
                    );
                }
                let _ = name;
                self.expr(recv);
                for t in targs {
                    self.ty(t);
                }
                if let Some(u) = unit {
                    self.unit_spec(u);
                }
                for a in args {
                    self.expr(a);
                }
            }
            ExprKind::Range(a, b) => {
                self.expr(a);
                self.expr(b);
            }
            ExprKind::Take { place, leaving, .. } => {
                self.expr(place);
                if let Some(l) = leaving {
                    self.expr(l);
                }
            }
            ExprKind::New { name, name_span, targs, values } => {
                self.type_name(name, *name_span);
                for t in targs {
                    self.ty(t);
                }
                for v in values {
                    self.expr(&mut v.value);
                }
            }
            ExprKind::Handled { call, handlers } => {
                self.expr(call);
                for h in handlers {
                    if let HandlerKind::Type(n, sp) = &mut h.kind {
                        self.type_name(n, *sp);
                    }
                    let mark = self.locals.len();
                    if let Some((b, _)) = &h.bind {
                        self.locals.push(b.clone());
                    }
                    self.block(&mut h.body);
                    self.locals.truncate(mark);
                }
            }
            ExprKind::ListLit(items) => {
                for i in items {
                    self.expr(i);
                }
            }
            ExprKind::MapLit(pairs) => {
                for (k, v) in pairs {
                    self.expr(k);
                    self.expr(v);
                }
            }
            ExprKind::Index { recv, index } => {
                self.expr(recv);
                self.expr(index);
            }
            ExprKind::OptionalTest { value, .. } => self.expr(value),
        }
    }
}

/// The chain slips (`ast::ChainSlip`) of a file, decided by its own `in`
/// lines: a name one of them binds is a module, anything else is the E0007.
/// Used for one-file programs and wherever the names are not resolved (a
/// program with errors already); `resolve` decides exactly, with the scopes.
pub fn chain_slips_by_imports(prog: &mut Program, diags: &mut Vec<Diagnostic>) {
    for slip in std::mem::take(&mut prog.chain_slips) {
        if !prog.imports.iter().any(|i| i.binding() == Some(slip.owner.as_str())) {
            diags.push(slip.diag);
        }
    }
}

/// Functions of different files must not call each other in a circle.
fn cross_file_recursion(edges: &[(String, String, Span)], module_of: &HashMap<String, usize>, names: &[String], diags: &mut Vec<Diagnostic>) {
    let mut adj: HashMap<&str, Vec<(&str, Span)>> = HashMap::new();
    for (a, b, sp) in edges {
        if module_of.contains_key(a) && module_of.contains_key(b) {
            adj.entry(a).or_default().push((b, *sp));
        }
    }
    let mut reported: HashSet<String> = HashSet::new();
    let mut order: Vec<&String> = module_of.keys().collect();
    order.sort();
    let shown = |g: &str| -> String {
        let m = names[module_of[g]].as_str();
        let plain = g.rsplit("::").next().unwrap_or(g);
        format!("`{plain}` ({})", if module_of[g] == 0 { "main" } else { m })
    };
    for u in order {
        if reported.contains(u.as_str()) {
            continue;
        }
        let Some(outs) = adj.get(u.as_str()) else { continue };
        for &(v, sp) in outs {
            if module_of[v] == module_of[u.as_str()] {
                continue;
            }
            // Can `v` get back to `u`?
            let mut parent: HashMap<&str, &str> = HashMap::new();
            let mut queue = VecDeque::from([v]);
            parent.insert(v, v);
            while let Some(x) = queue.pop_front() {
                if x == u {
                    break;
                }
                for &(y, _) in adj.get(x).into_iter().flatten() {
                    if !parent.contains_key(y) {
                        parent.insert(y, x);
                        queue.push_back(y);
                    }
                }
            }
            if !parent.contains_key(u.as_str()) {
                continue;
            }
            // u, v, ..., u
            let mut path = vec![u.as_str()];
            let mut cur = u.as_str();
            while cur != v {
                cur = parent[cur];
                path.push(cur);
            }
            path.reverse();
            let mut chain = vec![u.as_str()];
            chain.extend(path);
            for c in &chain {
                reported.insert(c.to_string());
            }
            let line = chain.iter().map(|c| shown(c)).collect::<Vec<_>>().join(" calls ");
            diags.push(
                Diagnostic::at(&codes::E0238, sp, format!("{} and {} call each other across files", shown(u), shown(v)))
                    .note("this call closes a circle between files")
                    .note_line(format!("the chain: {line}"))
                    .help("break the circle: move the shared code into a function of its own, or pass the function as a value"),
            );
            break;
        }
    }
}
