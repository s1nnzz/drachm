//! Region ties (Q3): a generic struct or function made with a type argument
//! that holds a borrow (`Stack<&string>`) has region slots wherever the
//! argument appears. The function bodies are checked as if the type had been
//! written out, so `push(&mut self, &string x)` would store a borrow of one
//! parameter inside another (E0320). What the template says is that both are
//! `T`: the slots that come from the same type parameter are one region.
//! This works the groups out from the declaration as written (the template)
//! and the types it came to. `hir::Program::ties` carries them to the borrow
//! checker, which treats each group as one region in the body and at calls.

use std::collections::{BTreeMap, BTreeSet};

use super::{Globals, optionals};
use crate::ast;
use crate::hir::FuncId;
use crate::types::Type;

/// One instance function whose ties are worked out once every type is known.
pub(super) struct Job {
    pub id: FuncId,
    /// The names of the type parameters of the template.
    pub tparams: Vec<String>,
    /// A method with a receiver (`self` is parameter 0).
    pub has_self: bool,
    /// The instance the method belongs to, and the fields as the template wrote them.
    pub owner: Option<(Type, Vec<(String, ast::TypeName)>)>,
    pub params: Vec<ast::TypeName>,
    pub ret: ast::TypeName,
}

type Pos = (Option<usize>, usize);

impl Globals {
    /// The number of region slots of a value of type `ty` (like the borrow
    /// checker's count, over the checker's own tables).
    pub(super) fn slot_count(&self, ty: Type) -> usize {
        self.slots_deep(ty, 0)
    }

    fn slots_deep(&self, ty: Type, depth: usize) -> usize {
        if depth > 12 {
            return 0;
        }
        match ty {
            Type::Func(_) => 1,
            Type::Ref { to, .. } => 1 + self.slots_deep(*to, depth + 1),
            Type::Struct { id, .. } => {
                let s = &self.structs[id];
                s.lives.iter().filter_map(|&f| s.fields[f].ty).map(|t| self.slots_deep(t, depth + 1)).sum()
            }
            Type::List { elem } | Type::Array { elem, .. } => self.slots_deep(*elem, depth + 1),
            Type::Enum { id, .. } => self.enums[id].variants.iter().flat_map(|v| &v.fields).filter_map(|f| f.ty).map(|t| self.slots_deep(t, depth + 1)).sum(),
            _ => 0,
        }
    }

    /// The slots of field `f` of struct `sid`, within the struct's own.
    fn field_slot_range(&self, sid: usize, f: usize) -> std::ops::Range<usize> {
        let s = &self.structs[sid];
        let mut at = 0;
        for &l in &s.lives {
            let n = s.fields[l].ty.map_or(0, |t| self.slots_deep(t, 1));
            if l == f {
                return at..at + n;
            }
            at += n;
        }
        at..at
    }

    /// Work out the ties of every instance function made.
    pub(super) fn compute_ties(&mut self) {
        let jobs = std::mem::take(&mut self.tie_jobs);
        let mut found = Vec::new();
        for job in &jobs {
            let groups = self.ties_of(job);
            if !groups.is_empty() {
                found.push((job.id, groups));
            }
        }
        self.ties.extend(found);
    }

    fn ties_of(&self, job: &Job) -> Vec<Vec<Pos>> {
        let Some(sig) = self.sigs.get(job.id) else { return Vec::new() };
        let mut out: BTreeMap<String, BTreeSet<Pos>> = BTreeMap::new();
        let shift = job.has_self as usize;
        if job.has_self && let Some((owner, fields)) = &job.owner {
            self.walk_owner(job, *owner, fields, Some(0), 1, &mut out);
        }
        for (i, ast) in job.params.iter().enumerate() {
            if let Some(Some(ty)) = sig.params.get(i) {
                self.walk(job, ast, *ty, Some(i + shift), 0, &mut out);
            }
        }
        if sig.outcome.is_none() && let Some(ty) = sig.ret {
            self.walk(job, &job.ret, ty, None, 0, &mut out);
        }
        out.into_values().filter(|g| g.len() >= 2).map(|g| g.into_iter().collect()).collect()
    }

    /// The slots of the instance `owner`, field by field as the template wrote them.
    fn walk_owner(&self, job: &Job, owner: Type, fields: &[(String, ast::TypeName)], param: Option<usize>, base: usize, out: &mut BTreeMap<String, BTreeSet<Pos>>) {
        let Some(sid) = owner.struct_id() else { return };
        for (name, ast) in fields {
            let Some(fi) = self.structs[sid].fields.iter().position(|f| f.name == *name) else { continue };
            let Some(fty) = self.structs[sid].fields[fi].ty else { continue };
            let range = self.field_slot_range(sid, fi);
            if !range.is_empty() {
                self.walk(job, ast, fty, param, base + range.start, out);
            }
        }
    }

    /// Records where the type parameters appear in `ast`, a type of `ty`
    /// whose first slot is `base` of parameter `param`.
    fn walk(&self, job: &Job, ast: &ast::TypeName, ty: Type, param: Option<usize>, base: usize, out: &mut BTreeMap<String, BTreeSet<Pos>>) {
        let mut ty = ty;
        let mut base = base;
        if ast.func.is_some() {
            return;
        }
        for (m, _) in &ast.mods {
            match m {
                ast::TypeMod::Ref { .. } => {
                    base += 1;
                    let Some(to) = ty.target() else { return };
                    ty = to;
                }
                _ => return,
            }
        }
        if ast.optional.is_some() {
            let Some(inner) = optionals::inner_of(self, ty) else { return };
            ty = inner;
        }
        for _ in &ast.dims {
            let Some(e) = ty.elem() else { return };
            ty = e;
        }
        let n = self.slot_count(ty);
        if n == 0 {
            return;
        }
        let mut put = |name: &str| {
            let set = out.entry(name.to_string()).or_default();
            for k in 0..n {
                set.insert((param, base + k));
            }
        };
        if ast.args.is_empty() {
            if job.tparams.iter().any(|t| *t == ast.name) {
                put(&ast.name);
            } else if let Some((owner, fields)) = &job.owner && ty == *owner {
                self.walk_owner(job, *owner, fields, param, base, out);
            }
            return;
        }
        // A generic type written with arguments: all its slots go with every
        // type parameter its arguments mention (coarser than needed, never unsound).
        fn mentions(a: &ast::TypeName, tparams: &[String], names: &mut BTreeSet<String>) {
            if a.args.is_empty() && tparams.contains(&a.name) {
                names.insert(a.name.clone());
            }
            for x in &a.args {
                mentions(x, tparams, names);
            }
            if let Some(f) = &a.func {
                for p in &f.params {
                    mentions(p, tparams, names);
                }
                mentions(&f.ret, tparams, names);
            }
        }
        let mut names = BTreeSet::new();
        mentions(ast, &job.tparams, &mut names);
        for name in names {
            put(&name);
        }
    }
}
