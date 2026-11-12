//! `fill` blocks and `def`s (S5B.2). A value declared without one (`Player p;`)
//! is given its fields one at a time, in `fill p { ... }`. Inside the block,
//! only `p.field = value;` assignments and statements that do not touch `p`
//! are allowed; a field already set is read with `now`. The completeness
//! checks (E0505, E0506) are on the MIR, where the per-field initialisation
//! analysis lives (`mir::check`).
//!
//! A `def` is a named set of field values for one struct, applied by
//! `fill p with NAME { ... }`; `defaults` names the struct's default one.

use std::collections::HashMap;

use super::*;

/// A `def`: the field values it sets, already converted to the field types.
pub struct DefInfo {
    pub name: String,
    pub struct_id: StructId,
    pub assigns: Vec<(usize, hir::Expr)>,
}

/// Checks every `def` (values are constant expressions: they cannot name a
/// field, since defs have no locals) and sets each struct's default.
pub(super) fn declare_defs(g: &mut Globals, src: &str, prog: &ast::Program, diags: &mut Vec<Diagnostic>) {
    let mut owners: Vec<Option<StructId>> = Vec::new();
    for d in &prog.defs {
        let (target, at) = match &d.of {
            Some((n, sp)) => (n.clone(), *sp),
            None => (d.name.clone(), d.name_span),
        };
        let owner = g.structs_by_name.get(&target).copied();
        if owner.is_none() {
            diags.push(Diagnostic::at(&codes::E0201, at, format!("unknown struct `{target}`")).help("a def is for a struct declared in this file"));
        }
        owners.push(owner);
    }
    // A def name may be given once.
    let mut kept: Vec<usize> = Vec::new();
    for (i, d) in prog.defs.iter().enumerate() {
        let Some(_) = owners[i] else { continue };
        if let Some(&j) = kept.iter().find(|&&j| prog.defs[j].name == d.name) {
            diags.push(
                Diagnostic::at(&codes::E0509, d.name_span, format!("two defs are named `{}`", d.name))
                    .note("defined again here")
                    .label(prog.defs[j].name_span, "first defined here")
                    .help("give one of them another name"),
            );
            continue;
        }
        kept.push(i);
    }
    let own: Vec<Vec<(usize, hir::Expr)>> = {
        let mut cx = Checker {
            src,
            g: &mut *g,
            diags: &mut *diags,
            ret: None,
            method: None,
            scopes: vec![HashMap::new()],
            locals: Vec::new(),
            targets: Vec::new(),
            labels: 0,
            filling: Vec::new(),
            now_depth: 0,
            ctor_self: None,
            fails: Vec::new(),
            outcome: None,
            suffix_ok: None,
            handlers: Vec::new(),
        };
        kept.iter().map(|&i| cx.def_values(&prog.defs[i], owners[i].unwrap())).collect()
    };
    // `def X of S from Y`: Y's values, then X's own, which win.
    let base_of: Vec<Option<usize>> = kept
        .iter()
        .map(|&i| {
            let (base, span) = prog.defs[i].from.as_ref()?;
            let found = kept.iter().position(|&j| prog.defs[j].name == *base);
            if found.is_none() {
                diags.push(
                    Diagnostic::at(&codes::E0508, *span, format!("there is no def named `{base}` to extend"))
                        .help("the defs in this file are declared with `def NAME of STRUCT { ... }`"),
                );
            }
            found
        })
        .collect();
    let mut done: Vec<Option<Vec<(usize, hir::Expr)>>> = vec![None; kept.len()];
    let mut busy = vec![false; kept.len()];
    for k in 0..kept.len() {
        flatten(k, &base_of, &own, &mut done, &mut busy);
    }
    for (n, &i) in kept.iter().enumerate() {
        let d = &prog.defs[i];
        let assigns = done[n].clone().unwrap_or_default();
        g.defs.push(DefInfo { name: d.name.clone(), struct_id: owners[i].unwrap(), assigns });
    }
    for s in &prog.structs {
        let (Some(&sid), Some((name, span))) = (g.structs_by_name.get(&s.name), &s.defaults) else { continue };
        match g.defs.iter().position(|d| d.name == *name && d.struct_id == sid) {
            Some(idx) => g.structs[sid].default_def = Some(idx),
            None => diags.push(
                Diagnostic::at(&codes::E0508, *span, format!("`{}` has no default def named `{name}`", s.name))
                    .help(format!("write `def {name} of {} {{ ... }}`", s.name)),
            ),
        }
    }
}

/// The values of def `k` with its base's values underneath. A cycle of
/// `from` contributes nothing past the repeat.
fn flatten(
    k: usize,
    base_of: &[Option<usize>],
    own: &[Vec<(usize, hir::Expr)>],
    done: &mut [Option<Vec<(usize, hir::Expr)>>],
    busy: &mut [bool],
) -> Vec<(usize, hir::Expr)> {
    if let Some(v) = &done[k] {
        return v.clone();
    }
    if busy[k] {
        return Vec::new();
    }
    busy[k] = true;
    let mut v = match base_of[k] {
        Some(b) => flatten(b, base_of, own, done, busy),
        None => Vec::new(),
    };
    v.retain(|(f, _)| !own[k].iter().any(|(g, _)| g == f));
    v.extend(own[k].iter().cloned());
    busy[k] = false;
    done[k] = Some(v.clone());
    v
}

impl Checker<'_> {
    /// The field values of one def for struct `s`.
    fn def_values(&mut self, d: &ast::DefDecl, s: StructId) -> Vec<(usize, hir::Expr)> {
        let sname = self.g.structs[s].name.clone();
        let fields: Vec<(String, Option<Type>)> = self.g.structs[s].fields.iter().map(|f| (f.name.clone(), f.ty)).collect();
        let mut out = Vec::new();
        let mut seen: Vec<(usize, Span)> = Vec::new();
        for a in &d.assigns {
            let Some(i) = fields.iter().position(|f| f.0 == a.name) else {
                let names: Vec<&str> = fields.iter().map(|f| f.0.as_str()).collect();
                let mut diag = Diagnostic::at(&codes::E0201, a.name_span, format!("`{sname}` has no field `{}`", a.name));
                diag = match similar(&a.name, &names) {
                    Some(near) => diag.help(format!("did you mean `{near}`?")),
                    None => diag.help(format!("its fields are {}", list(&names))),
                };
                self.diags.push(diag);
                continue;
            };
            if let Some(&(_, first)) = seen.iter().find(|(j, _)| *j == i) {
                self.diags.push(
                    Diagnostic::at(&codes::E0509, a.name_span, format!("`{}` is given twice in `{}`", a.name, d.name))
                        .note("given again here")
                        .label(first, "first given here"),
                );
                continue;
            }
            seen.push((i, a.name_span));
            let Some(fty) = fields[i].1 else { continue };
            if let Some(v) = self.expr(&a.value)
                && let Some(e) = self.into(v, fty, origin(&a.value))
            {
                out.push((i, e));
            }
        }
        out
    }

    /// E0507: a value that is still being filled is read.
    pub(super) fn incomplete_use(&mut self, id: LocalId, at: Span) {
        let name = if self.locals[id].is_self { "self".to_string() } else { self.locals[id].name.clone() };
        let used = self.text(at).to_string();
        self.diags.push(
            Diagnostic::at(&codes::E0507, at, format!("`{name}` cannot be used until it is complete"))
                .note("used here, before every field has a value")
                .help(format!("a field that is already set can be read with `now`: `now {used}.field`")),
        );
    }

    /// `fill [partial] x [with ...] { ... }`.
    pub(super) fn fill(&mut self, partial: bool, target: &(String, Span), with: Option<&ast::FillWith>, body: &ast::Block, span: Span, out: &mut hir::Block) {
        let (name, name_span) = (&target.0, target.1);
        let Some(id) = self.lookup(name) else {
            self.diags.push(Diagnostic::at(&codes::E0201, name_span, format!("unknown binding `{name}`")));
            return;
        };
        let (blocked, ty) = (self.locals[id].is_param || self.locals[id].is_self || self.locals[id].open, self.locals[id].ty);
        if blocked {
            self.diags.push(
                Diagnostic::at(&codes::E0901, span, format!("`fill` of `{name}` is not supported yet"))
                    .note("only a local binding can be filled")
                    .help("fill a local declared without a value: `Player p;`"),
            );
            return;
        }
        let Some(ty) = ty else { return };
        let Some(s) = ty.struct_id() else {
            self.diags.push(
                Diagnostic::at(&codes::E0101, name_span, format!("`fill` gives values to the fields of a struct, and `{name}` is `{ty}`"))
                    .note(format!("this is `{ty}`")),
            );
            return;
        };
        if self.filling.contains(&id) {
            self.diags.push(Diagnostic::at(&codes::E0901, span, format!("a `fill` of `{name}` inside another one is not supported yet")));
            return;
        }
        let def = match with {
            None => None,
            Some(ast::FillWith::Defaults(at)) => match self.g.structs[s].default_def {
                Some(d) => Some(d),
                None => {
                    let sname = self.g.structs[s].name.clone();
                    self.diags.push(
                        Diagnostic::at(&codes::E0510, *at, format!("`{sname}` has no default def, so `with defaults` has nothing to use"))
                            .note("no `defaults` in its head")
                            .help(format!("write `struct {sname} defaults {sname} {{ ... }}` and `def {sname} of {sname} {{ ... }}`")),
                    );
                    return;
                }
            },
            Some(ast::FillWith::Named(n, at)) => match self.g.defs.iter().position(|d| d.name == *n && d.struct_id == s) {
                Some(d) => Some(d),
                None => {
                    let sname = self.g.structs[s].name.clone();
                    self.diags.push(
                        Diagnostic::at(&codes::E0508, *at, format!("`{sname}` has no def named `{n}`"))
                            .help(format!("write `def {n} of {sname} {{ ... }}`")),
                    );
                    return;
                }
            },
        };
        out.push(hir::Stmt::Check { local: id, kind: hir::CheckKind::FillBegin, span });
        self.filling.push(id);
        if let Some(d) = def {
            for (f, e) in self.g.defs[d].assigns.clone() {
                out.push(hir::Stmt::InitField(hir::Place { local: id, proj: vec![hir::HirProj::Field(f)], span: name_span }, e));
            }
        }
        self.scopes.push(HashMap::new());
        for st in body {
            self.stmt(st, out);
        }
        self.scopes.pop();
        self.filling.pop();
        if !partial {
            out.push(hir::Stmt::Check { local: id, kind: hir::CheckKind::FillEnd, span: body.end });
        }
    }
}
