//! Drop elaboration (spec 3.3, WP6).
//!
//! The builder puts a `Drop` where a value dies, whether or not the value
//! still exists there. Elaboration decides, from the move and initialisation
//! analyses, what each one really does:
//!
//! - initialised on every path: an unconditional `Drop`;
//! - uninitialised on every path (never given a value, or moved): gone;
//! - initialised on some paths: `Drop ... if flag`, where the flag is a `bool`
//!   local kept up to date at every initialisation and move of the value;
//! - a struct some of whose fields were moved separately is dropped field by
//!   field. A struct with a `drop()` is never opened: it is whole at every
//!   `Drop` (plain `take` is E0325; `leaving` gives the field a value again in
//!   the same statement).
//!
//! After this pass no `Drop` remains whose place may be uninitialised without
//! a flag. The backends rely on that.

use std::collections::HashMap;

use super::dataflow::Results;
use super::moves::{self, MaybeInit, MaybeUninit, MoveData, MovePathId};
use super::traversal::predecessors;
use super::*;

/// Elaborate the drops of every body of `p`. Debug builds then run `validate`.
pub fn elaborate_program(hir: &hir::Program, p: &mut Program) {
    for body in p.bodies.iter_mut().flatten() {
        elaborate(hir, body);
    }
    #[cfg(debug_assertions)]
    if let Err(errors) = validate::validate(p, hir) {
        panic!("drop elaboration produced invalid MIR:\n{}", errors.join("\n"));
    }
}

/// Does `ty` have a `drop()` of its own?
pub fn has_drop(hir: &hir::Program, ty: Type) -> bool {
    matches!(ty, Type::Struct { id, .. } if hir.structs[id].has_drop.is_some())
}

/// Does a value of `ty` own a string at any depth, so that copying it needs
/// more than copying its bits?
pub fn holds_string(hir: &hir::Program, ty: Type) -> bool {
    match ty {
        Type::Str => true,
        Type::Struct { id, .. } => hir.structs[id].fields.iter().any(|f| holds_string(hir, f.ty)),
        _ => false,
    }
}

/// The structs that get a drop glue function, in id order.
pub fn drop_glue_structs(hir: &hir::Program) -> Vec<StructId> {
    (0..hir.structs.len()).filter(|&s| needs_drop(hir, struct_type(hir, s))).collect()
}

/// The structs that get a copy glue function, in id order.
pub fn copy_glue_structs(hir: &hir::Program) -> Vec<StructId> {
    (0..hir.structs.len()).filter(|&s| holds_string(hir, struct_type(hir, s))).collect()
}

fn struct_type(hir: &hir::Program, s: StructId) -> Type {
    Type::Struct { id: s, name: crate::types::intern(&hir.structs[s].name) }
}

fn has_deref(p: &Place) -> bool {
    p.proj.contains(&Proj::Deref)
}

struct Elab<'a> {
    hir: &'a hir::Program,
    md: &'a MoveData,
    mi: &'a Results<MaybeInit<'a>>,
    mu: &'a Results<MaybeUninit<'a>>,
    body: &'a Body,
    /// The flag of each path that needs one, in creation order.
    flags: Vec<(MovePathId, Local)>,
    next_local: usize,
}

impl Elab<'_> {
    fn flag(&mut self, p: MovePathId) -> Local {
        if let Some((_, f)) = self.flags.iter().find(|(q, _)| *q == p) {
            return *f;
        }
        let f = Local::new(self.next_local);
        self.next_local += 1;
        self.flags.push((p, f));
        f
    }

    /// The statements that replace a `Drop` of `place` at `loc`.
    fn drop_place(&mut self, loc: Location, place: &Place, reason: DropReason) -> Vec<StatementKind> {
        let ty = place_type(self.hir, self.body, place);
        if !needs_drop(self.hir, ty) {
            return Vec::new();
        }
        if has_deref(place) {
            // Through a borrow the value is always there.
            return vec![StatementKind::Drop { place: place.clone(), flag: None, reason }];
        }
        let fields: Vec<usize> = place
            .proj
            .iter()
            .map(|p| match p {
                Proj::Field(f) => *f,
                Proj::Deref => unreachable!(),
            })
            .collect();
        // The nearest path at or above `place`: a place with no path of its
        // own is in the state of that one.
        let mut k = fields.len();
        let (own, state) = loop {
            let prefix = Place { local: place.local, proj: fields[..k].iter().map(|&f| Proj::Field(f)).collect() };
            if let Some(p) = self.md.path_of(&prefix) {
                break (if k == fields.len() { Some(p) } else { None }, p);
            }
            k -= 1;
        };
        let mi = self.mi.state_at(self.body, loc);
        let mu = self.mu.state_at(self.body, loc);
        let mut out = Vec::new();
        self.elab(place.local, &fields, ty, own, state, &mi, &mu, reason, &mut out);
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn elab(
        &mut self,
        local: Local,
        fields: &[usize],
        ty: Type,
        own: Option<MovePathId>,
        state: MovePathId,
        mi: &dataflow::BitSet,
        mu: &dataflow::BitSet,
        reason: DropReason,
        out: &mut Vec<StatementKind>,
    ) {
        if !needs_drop(self.hir, ty) {
            return;
        }
        if let Some(p) = own
            && !self.md.paths[p].children.is_empty()
            && !has_drop(self.hir, ty)
        {
            let def = self.hir.struct_def(ty);
            for (i, f) in def.fields.iter().enumerate() {
                let mut sub = fields.to_vec();
                sub.push(i);
                let child = self.md.paths[p].children.iter().copied().find(|&c| self.md.paths[c].fields == sub);
                self.elab(local, &sub, f.ty, child, child.unwrap_or(state), mi, mu, reason, out);
            }
            return;
        }
        let (init, uninit) = (mi.contains(state.index()), mu.contains(state.index()));
        let place = Place { local, proj: fields.iter().map(|&f| Proj::Field(f)).collect() };
        match (init, uninit) {
            (false, _) => {}
            (true, false) => out.push(StatementKind::Drop { place, flag: None, reason }),
            (true, true) => {
                let f = self.flag(state);
                out.push(StatementKind::Drop { place, flag: Some(f), reason });
            }
        }
    }
}

fn place_type(hir: &hir::Program, body: &Body, p: &Place) -> Type {
    ty::place_ty(body, hir, p).expect("validated MIR")
}

/// Elaborate the drops of one body.
pub fn elaborate(hir: &hir::Program, body: &mut Body) {
    let has_drops = body.blocks.iter().any(|b| b.statements.iter().any(|s| matches!(s.kind, StatementKind::Drop { flag: None, .. })));
    if !has_drops {
        return;
    }
    let md = MoveData::build(body);
    let preds = predecessors(body);
    let mu = Results::compute(MaybeUninit(&md), body, &preds);
    let mi = Results::compute(MaybeInit(&md), body, &preds);

    let mut el = Elab { hir, md: &md, mi: &mi, mu: &mu, body, flags: Vec::new(), next_local: body.locals.len() };
    let mut replaced: HashMap<Location, Vec<StatementKind>> = HashMap::new();
    for (b, block) in body.blocks.iter_enumerated() {
        for (i, s) in block.statements.iter().enumerate() {
            if let StatementKind::Drop { place, flag: None, reason } = &s.kind {
                let loc = Location { block: b, index: i as u32 };
                let new = el.drop_place(loc, place, *reason);
                replaced.insert(loc, new);
            }
        }
    }
    let flags = std::mem::take(&mut el.flags);

    // The flag writes, after the statement (or call) they belong to.
    // `after[loc]` holds (flag, value) in the order they must happen.
    let mut after: HashMap<Location, Vec<(Local, bool)>> = HashMap::new();
    let mut initial: Vec<(Local, bool)> = Vec::new();
    for &(q, f) in &flags {
        let local = md.paths[q].local;
        initial.push((f, (1..=body.arg_count).contains(&local.index())));
        let mut at: HashMap<Location, (bool, bool)> = HashMap::new();
        for m in md.moves.iter() {
            // The end of a variable's storage changes nothing a flag tells.
            if m.cause != moves::MoveCause::StorageDead && md.is_ancestor(m.path, q) {
                at.entry(m.loc).or_default().0 = true;
            }
        }
        for i in md.inits.iter() {
            if md.is_ancestor(i.path, q) {
                at.entry(i.loc).or_default().1 = true;
            }
        }
        let mut locs: Vec<_> = at.into_iter().collect();
        locs.sort_by_key(|(l, _)| *l);
        for (loc, (moved, inited)) in locs {
            let v = after.entry(loc).or_default();
            if moved {
                v.push((f, false));
            }
            if inited {
                v.push((f, true));
            }
        }
    }

    // Rebuild the statements.
    let flag_stmt = |f: Local, v: bool, span: Span, unsafe_ctx: bool| Statement {
        kind: StatementKind::Assign(Place::local(f), Rvalue::Use(Operand::Const(Constant::Bool(v)))),
        span,
        unsafe_ctx,
        mark: Mark::None,
    };
    let base = body.blocks.len();
    let mut extra: Vec<BasicBlock> = Vec::new();
    for (bi, block) in body.blocks.iter_mut().enumerate() {
        let b = BlockId::new(bi);
        let old = std::mem::take(&mut block.statements);
        let n = old.len();
        let mut stmts = Vec::with_capacity(n);
        for (i, s) in old.into_iter().enumerate() {
            let loc = Location { block: b, index: i as u32 };
            let (span, unsafe_ctx) = (s.span, s.unsafe_ctx);
            match replaced.remove(&loc) {
                Some(new) => stmts.extend(new.into_iter().map(|kind| Statement { kind, span, unsafe_ctx, mark: Mark::None })),
                None => stmts.push(s),
            }
            if let Some(w) = after.get(&loc) {
                stmts.extend(w.iter().map(|&(f, v)| flag_stmt(f, v, span, unsafe_ctx)));
            }
        }
        block.statements = stmts;
        // After a call, at the start of its target: a block of its own with
        // one predecessor (cleanup merges it into the target when it can).
        if let Some(w) = after.get(&Location { block: b, index: n as u32 })
            && let TerminatorKind::Call { target, .. } = &mut block.terminator.kind
        {
            let (span, unsafe_ctx) = (block.terminator.span, block.terminator.unsafe_ctx);
            extra.push(BasicBlock {
                statements: w.iter().map(|&(f, v)| flag_stmt(f, v, span, unsafe_ctx)).collect(),
                terminator: Terminator { kind: TerminatorKind::Goto(*target), span, unsafe_ctx },
            });
            *target = BlockId::new(base + extra.len() - 1);
        }
    }
    for nb in extra {
        body.blocks.push(nb);
    }

    // The flags themselves, set before anything else runs.
    let span = body.span;
    for &(_, f) in &flags {
        debug_assert_eq!(f.index(), body.locals.len());
        body.locals.push(LocalDecl { ty: Type::Bool, kind: LocalKind::DropFlag, span, scope: ScopeId(0), in_memory: false });
    }
    if !initial.is_empty() {
        let sets: Vec<Statement> = initial.iter().map(|&(f, v)| flag_stmt(f, v, span, false)).collect();
        let entry_has_preds = body.blocks.iter().any(|b| traversal::successors(&b.terminator).contains(&BlockId(0)));
        if entry_has_preds {
            // Something jumps back to the entry: the flags are set in a new
            // entry block in front of it.
            let moved = BlockId::new(body.blocks.len());
            let entry = BasicBlock { statements: sets, terminator: Terminator { kind: TerminatorKind::Goto(moved), span, unsafe_ctx: false } };
            let old = std::mem::replace(&mut body.blocks[BlockId(0)], entry);
            body.blocks.push(old);
            for b in body.blocks.iter_mut().skip(1) {
                retarget(&mut b.terminator.kind, BlockId(0), moved);
            }
            if body.fallthrough_end == Some(BlockId(0)) {
                body.fallthrough_end = Some(moved);
            }
        } else {
            let entry = &mut body.blocks[BlockId(0)];
            let mut all = sets;
            all.append(&mut entry.statements);
            entry.statements = all;
        }
    }
}

fn retarget(k: &mut TerminatorKind, from: BlockId, to: BlockId) {
    let fix = |b: &mut BlockId| {
        if *b == from {
            *b = to;
        }
    };
    match k {
        TerminatorKind::Goto(b) => fix(b),
        TerminatorKind::If { then, els, .. } => {
            fix(then);
            fix(els);
        }
        TerminatorKind::Switch { cases, otherwise, .. } => {
            cases.iter_mut().for_each(|(_, b)| fix(b));
            fix(otherwise);
        }
        TerminatorKind::Call { target, .. } => fix(target),
        TerminatorKind::Return | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `Drop`s of `main` after elaboration, as the MIR dump prints them.
    fn drops_of_main(src: &str) -> Vec<String> {
        let (hir, diags) = crate::check(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mut mir = build_program(&hir, &mut Vec::new());
        elaborate_program(&hir, &mut mir);
        let text = print::program(&mir, &hir);
        let section = text.split("\n\n").find(|s| s.starts_with("fn main(")).unwrap_or_else(|| panic!("no main in {text}"));
        section.lines().map(str::trim).filter(|l| l.starts_with("Drop(")).map(str::to_string).collect()
    }

    const PRELUDE: &str = "in io;\nstruct P { public: string name; string title; }\nstruct F { public: string path; none drop() { } }\nnone eat(string s) { io::outln(s); }\n";

    #[test]
    fn certainly_initialised_is_static() {
        let d = drops_of_main(&format!("{PRELUDE}none main() {{ string a = \"x\"; io::outln(a); }}"));
        assert!(d.contains(&"Drop(_1)".to_string()), "{d:?}");
        assert!(d.iter().all(|l| !l.contains(" if ")), "{d:?}");
    }

    #[test]
    fn certainly_moved_disappears() {
        let d = drops_of_main(&format!("{PRELUDE}none main() {{ string a = \"x\"; eat(a); }}"));
        assert!(d.iter().all(|l| !l.starts_with("Drop(_1")), "{d:?}");
    }

    #[test]
    fn maybe_moved_gets_a_flag() {
        let d = drops_of_main(&format!("{PRELUDE}none main() {{ string a = \"x\"; if (a::isEmpty()) {{ eat(a); }} }}"));
        assert_eq!(d.iter().filter(|l| l.starts_with("Drop(_1) if _")).count(), 1, "{d:?}");
        assert!(!d.contains(&"Drop(_1)".to_string()), "{d:?}");
    }

    #[test]
    fn a_partly_moved_struct_is_opened() {
        let d = drops_of_main(&format!("{PRELUDE}none main() {{ P p = \"a\", \"b\"; if (p.name::isEmpty()) {{ eat(take p.name); }} }}"));
        let dropped: Vec<&str> = d.iter().map(String::as_str).filter(|l| l.starts_with("Drop(_1.")).collect();
        assert_eq!(dropped.len(), 2, "{d:?}");
        assert!(dropped[0].starts_with("Drop(_1.0) if _"), "{d:?}");
        assert_eq!(dropped[1], "Drop(_1.1)", "the untouched field is dropped with the struct");
        assert!(!d.contains(&"Drop(_1)".to_string()), "{d:?}");
    }

    #[test]
    fn a_struct_with_drop_is_never_opened() {
        let d = drops_of_main(&format!("{PRELUDE}none main() {{ mut F f = \"a\"; string t = take f.path leaving \"b\"; io::outln(t); }}"));
        assert!(d.contains(&"Drop(_1)".to_string()), "whole at its drop: {d:?}");
        assert!(d.iter().all(|l| !l.starts_with("Drop(_1.")), "{d:?}");
    }

    #[test]
    fn assignment_to_an_unset_variable_drops_nothing() {
        let d = drops_of_main(&format!("{PRELUDE}none main() {{ string a; a = \"x\"; io::outln(a); }}"));
        // The one drop is the scope end's; the assignment's is dead.
        assert_eq!(d.iter().filter(|l| l.starts_with("Drop(_1")).count(), 1, "{d:?}");
    }
}
