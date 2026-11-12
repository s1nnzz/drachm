//! Move paths and the initialisation analyses (spec 3.1.1 and 3.1.2).
//!
//! A move path is a local followed by field projections, with no `Deref`:
//! the things that can be initialised or moved out. `events` lists, in
//! order, what each statement or terminator does to them; `MoveData` records
//! the moves and initialisations, and the four analyses read it.

use std::collections::HashMap;

use super::dataflow::{Analysis, BitSet};
use super::index::{newtype_index, Idx};
use super::*;

newtype_index!(MovePathId);
newtype_index!(MoveOutId);
newtype_index!(InitId);

#[derive(Debug)]
pub struct MovePath {
    pub local: Local,
    /// Field projections only; empty is the local itself.
    pub fields: Vec<usize>,
    pub parent: Option<MovePathId>,
    /// In creation order.
    pub children: Vec<MovePathId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MoveCause {
    /// Passed whole as argument `index` of a call to a user function.
    CallArg { callee: FuncId, index: usize },
    /// `x = move y` or any other plain move into a place or a struct.
    Assign,
    /// `_0 = move y` (a `return`).
    Return,
    /// From a `take` (the temp `_t = move p.f`).
    Take,
    /// `destroy x;`: the value goes nowhere.
    Destroy,
    /// A `Drop` statement (scope end or before an assignment): the place is
    /// uninitialised afterwards. Never a user-visible "moved here".
    Drop,
    StorageDead,
}

impl MoveCause {
    /// Does the program text show this move?
    pub fn visible(self) -> bool {
        !matches!(self, MoveCause::Drop | MoveCause::StorageDead)
    }
}

#[derive(Debug)]
pub struct MoveOut {
    pub path: MovePathId,
    pub loc: Location,
    pub span: Span,
    pub cause: MoveCause,
}

#[derive(Debug)]
pub struct Init {
    pub path: MovePathId,
    pub loc: Location,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UseKind {
    Read,
    /// `q.f = ...`: `q.f`'s parents must have values.
    FieldWriteBase,
    Assign,
}

#[derive(Debug, Clone)]
pub struct Use {
    pub place: Place,
    pub span: Span,
    pub kind: UseKind,
}

/// What a statement or terminator does, in order: its uses are checked
/// against the state before it (plus the moves earlier in the same list),
/// then the moves and initialisations take effect.
#[derive(Debug, Clone)]
pub enum Event {
    Use(Use),
    /// `place` has no `Deref`.
    Move { place: Place, span: Span, cause: MoveCause },
    /// `place` has no `Deref`.
    Init(Place),
    /// `StorageLive(l)`: every path of `l` starts uninitialised.
    StorageLive(Local),
}

fn has_deref(p: &Place) -> bool {
    p.proj.contains(&Proj::Deref)
}

/// The part of `p` before its first `Deref`.
pub fn prefix_place(p: &Place) -> Place {
    let n = p.proj.iter().position(|x| *x == Proj::Deref).unwrap_or(p.proj.len());
    Place { local: p.local, proj: p.proj[..n].to_vec() }
}

fn operand_events(op: &Operand, span: Span, cause: MoveCause, out: &mut Vec<Event>) {
    match op {
        Operand::Copy(p) => out.push(Event::Use(Use { place: p.clone(), span, kind: UseKind::Read })),
        Operand::Move(p) => {
            out.push(Event::Use(Use { place: p.clone(), span, kind: UseKind::Read }));
            if !has_deref(p) {
                out.push(Event::Move { place: p.clone(), span, cause });
            }
        }
        Operand::Const(_) => {}
    }
}

fn read(p: &Place, span: Span, out: &mut Vec<Event>) {
    out.push(Event::Use(Use { place: p.clone(), span, kind: UseKind::Read }));
}

/// The uses and then the initialisation of an assignment's destination.
fn dest_events(dest: &Place, span: Span, out: &mut Vec<Event>) {
    if has_deref(dest) {
        // Writing through a borrow reads the borrow.
        read(&prefix_place(dest), span, out);
    } else if !dest.proj.is_empty() {
        out.push(Event::Use(Use { place: dest.clone(), span, kind: UseKind::FieldWriteBase }));
    }
}

/// The events at `loc`, in order.
pub fn events(body: &Body, loc: Location) -> Vec<Event> {
    let block = &body.blocks[loc.block];
    let i = loc.index as usize;
    let mut out = Vec::new();
    if i < block.statements.len() {
        let s = &block.statements[i];
        match &s.kind {
            StatementKind::Assign(dest, rv) => {
                let take = !has_deref(dest)
                    && matches!(body.locals[dest.local].kind, LocalKind::Temp)
                    && dest.proj.is_empty()
                    && matches!(rv, Rvalue::Use(Operand::Move(p)) if !has_deref(p) && p.proj.iter().any(|x| matches!(x, Proj::Field(_))));
                let destroy = i > 0
                    && matches!(&block.statements[i - 1].kind,
                        StatementKind::Drop { place, reason: DropReason::Destroy, .. }
                            if matches!(rv, Rvalue::Use(Operand::Move(p)) if p == place));
                let cause = if dest.local == Local(0) && dest.proj.is_empty() {
                    MoveCause::Return
                } else if destroy {
                    MoveCause::Destroy
                } else if take {
                    MoveCause::Take
                } else {
                    MoveCause::Assign
                };
                match rv {
                    Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) => {
                        operand_events(o, s.span, cause, &mut out)
                    }
                    Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => {
                        operand_events(a, s.span, cause, &mut out);
                        operand_events(b, s.span, cause, &mut out);
                    }
                    Rvalue::Shift { value, amount, .. } => {
                        operand_events(value, s.span, cause, &mut out);
                        operand_events(amount, s.span, cause, &mut out);
                    }
                    Rvalue::Ref(_, p) | Rvalue::RawRef(p) | Rvalue::DeepCopy(p) | Rvalue::Discriminant(p) => read(p, s.span, &mut out),
                    Rvalue::Aggregate(_, ops) | Rvalue::Variant(_, _, ops) => ops.iter().for_each(|o| operand_events(o, s.span, cause, &mut out)),
                }
                dest_events(dest, s.span, &mut out);
                if !has_deref(dest) {
                    out.push(Event::Init(dest.clone()));
                }
            }
            StatementKind::StorageLive(l) => out.push(Event::StorageLive(*l)),
            StatementKind::StorageDead(l) => {
                out.push(Event::Move { place: Place::local(*l), span: s.span, cause: MoveCause::StorageDead })
            }
            // `destroy x;` is a `Drop` and then a move out of `x`: the move
            // ends `x`, so the drop itself changes nothing.
            StatementKind::Drop { place, reason, .. } => {
                if !has_deref(place) && *reason != DropReason::Destroy {
                    out.push(Event::Move { place: place.clone(), span: s.span, cause: MoveCause::Drop });
                }
            }
            StatementKind::FakeRead(p) => read(p, s.span, &mut out),
            StatementKind::Nop => {}
        }
    } else {
        let t = &block.terminator;
        match &t.kind {
            TerminatorKind::If { cond, .. } => operand_events(cond, t.span, MoveCause::Assign, &mut out),
            TerminatorKind::Switch { discr, .. } => operand_events(discr, t.span, MoveCause::Assign, &mut out),
            TerminatorKind::Call { callee, args, dest, .. } => {
                for (index, a) in args.iter().enumerate() {
                    let cause = match callee {
                        Callee::Func(f) => MoveCause::CallArg { callee: *f, index },
                        _ => MoveCause::Assign,
                    };
                    operand_events(&a.op, a.span, cause, &mut out);
                }
                if let Some(d) = dest {
                    dest_events(d, t.span, &mut out);
                    if !has_deref(d) {
                        out.push(Event::Init(d.clone()));
                    }
                }
            }
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => {}
        }
    }
    out
}

pub struct MoveData {
    pub paths: IndexVec<MovePathId, MovePath>,
    /// The root path of every local.
    pub local_path: IndexVec<Local, MovePathId>,
    by_place: HashMap<(Local, Vec<usize>), MovePathId>,
    pub moves: IndexVec<MoveOutId, MoveOut>,
    pub inits: IndexVec<InitId, Init>,
    pub moves_at: HashMap<Location, Vec<MoveOutId>>,
    pub inits_at: HashMap<Location, Vec<InitId>>,
    /// Each path and everything below it.
    desc: IndexVec<MovePathId, Vec<MovePathId>>,
    moves_of: IndexVec<MovePathId, Vec<MoveOutId>>,
}

impl MoveData {
    pub fn build(body: &Body) -> MoveData {
        let mut md = MoveData {
            paths: IndexVec::new(),
            local_path: IndexVec::new(),
            by_place: HashMap::new(),
            moves: IndexVec::new(),
            inits: IndexVec::new(),
            moves_at: HashMap::new(),
            inits_at: HashMap::new(),
            desc: IndexVec::new(),
            moves_of: IndexVec::new(),
        };
        for (l, _) in body.locals.iter_enumerated() {
            let id = md.paths.push(MovePath { local: l, fields: Vec::new(), parent: None, children: Vec::new() });
            md.by_place.insert((l, Vec::new()), id);
            md.local_path.push(id);
        }
        for (b, block) in body.blocks.iter_enumerated() {
            for index in 0..=block.statements.len() {
                let loc = Location { block: b, index: index as u32 };
                for e in events(body, loc) {
                    match e {
                        Event::Use(u) => {
                            md.ensure(&prefix_place(&u.place));
                            // The written field itself needs a path too.
                            if u.kind == UseKind::FieldWriteBase {
                                md.ensure(&u.place);
                            }
                        }
                        Event::Move { place, span, cause } => {
                            let path = md.ensure(&place);
                            let id = md.moves.push(MoveOut { path, loc, span, cause });
                            md.moves_at.entry(loc).or_default().push(id);
                        }
                        Event::Init(place) => {
                            let path = md.ensure(&place);
                            let id = md.inits.push(Init { path, loc });
                            md.inits_at.entry(loc).or_default().push(id);
                        }
                        Event::StorageLive(_) => {}
                    }
                }
            }
        }
        md.desc = IndexVec::from_vec(md.paths.indices().map(|p| vec![p]).collect());
        for p in md.paths.indices() {
            let mut a = md.paths[p].parent;
            while let Some(anc) = a {
                md.desc[anc].push(p);
                a = md.paths[anc].parent;
            }
        }
        md.moves_of = IndexVec::from_vec(md.paths.indices().map(|_| Vec::new()).collect());
        for (id, m) in md.moves.iter_enumerated() {
            md.moves_of[m.path].push(id);
        }
        md
    }

    /// The path of `place` (its field prefix), creating it and its parents.
    fn ensure(&mut self, place: &Place) -> MovePathId {
        let fields: Vec<usize> = place
            .proj
            .iter()
            .take_while(|x| !matches!(x, Proj::Deref | Proj::Downcast(_)))
            .map(|x| match x {
                Proj::Field(f) => *f,
                Proj::Deref | Proj::Downcast(_) => unreachable!(),
            })
            .collect();
        self.ensure_fields(place.local, &fields)
    }

    fn ensure_fields(&mut self, local: Local, fields: &[usize]) -> MovePathId {
        if let Some(&id) = self.by_place.get(&(local, fields.to_vec())) {
            return id;
        }
        let parent = self.ensure_fields(local, &fields[..fields.len() - 1]);
        let id = self.paths.push(MovePath { local, fields: fields.to_vec(), parent: Some(parent), children: Vec::new() });
        self.paths[parent].children.push(id);
        self.by_place.insert((local, fields.to_vec()), id);
        id
    }

    /// The path of `place` if it has one (no `Deref` in it).
    pub fn path_of(&self, place: &Place) -> Option<MovePathId> {
        if has_deref(place) {
            return None;
        }
        self.lookup(place)
    }

    /// Longest prefix of `place` before its first `Deref`; always exists for
    /// places the body mentions.
    pub fn prefix_path(&self, place: &Place) -> MovePathId {
        self.lookup(&prefix_place(place)).unwrap_or(self.local_path[place.local])
    }

    fn lookup(&self, place: &Place) -> Option<MovePathId> {
        let fields: Vec<usize> = place
            .proj
            .iter()
            // An enum is one path: a payload place is in the state of the whole enum.
            .take_while(|x| !matches!(x, Proj::Downcast(_)))
            .map(|x| match x {
                Proj::Field(f) => *f,
                Proj::Deref | Proj::Downcast(_) => unreachable!(),
            })
            .collect();
        self.by_place.get(&(place.local, fields)).copied()
    }

    /// Is `a` the path `of`, or above it?
    pub fn is_ancestor(&self, a: MovePathId, of: MovePathId) -> bool {
        let mut cur = Some(of);
        while let Some(c) = cur {
            if c == a {
                return true;
            }
            cur = self.paths[c].parent;
        }
        false
    }

    /// `p` and everything below it.
    pub fn descendants(&self, p: MovePathId) -> Vec<MovePathId> {
        self.desc[p].clone()
    }

    fn below(&self, p: MovePathId) -> &[MovePathId] {
        &self.desc[p]
    }

    /// Every path of `l`.
    fn of_local(&self, l: Local) -> &[MovePathId] {
        self.below(self.local_path[l])
    }

    /// The type of the value at `p`.
    pub fn path_ty(&self, body: &Body, hir: &hir::Program, p: MovePathId) -> Type {
        let path = &self.paths[p];
        let mut t = body.locals[path.local].ty;
        for &f in &path.fields {
            t = hir.struct_def(t).fields[f].ty;
        }
        t
    }
}

/// Does a value of type `ty` hold a `&mut` borrow, in a field at any depth?
pub fn holds_mut_borrow(hir: &hir::Program, ty: Type) -> bool {
    find_mut_borrow(hir, ty).is_some()
}

/// The first field (struct name, field name, field type) of `ty` that is a
/// `&mut`, looking through struct fields; `ty` itself being one gives `None`
/// for the names.
pub fn find_mut_borrow(hir: &hir::Program, ty: Type) -> Option<(String, String, Type)> {
    find_mut_inner(hir, ty, 0)
}

fn find_mut_inner(hir: &hir::Program, ty: Type, depth: usize) -> Option<(String, String, Type)> {
    if depth > 32 {
        return None;
    }
    if let Type::Struct { id, .. } = ty {
        let def = &hir.structs[id];
        for f in &def.fields {
            if matches!(f.ty, Type::Ref { mutable: true, .. }) {
                return Some((def.name.clone(), f.name.clone(), f.ty));
            }
            if let Some(found) = find_mut_inner(hir, f.ty, depth + 1) {
                return Some(found);
            }
        }
    }
    None
}

fn paths_of_state(md: &MoveData) -> usize {
    md.paths.len()
}

/// Apply the moves (`uninit`) then the initialisations of `loc` to a state
/// over paths. `init_means_set` is true for "maybe initialised" (bit = init).
fn path_transfer(md: &MoveData, state: &mut BitSet, stmt: Option<&Statement>, loc: Location, init_means_set: bool) {
    if let Some(StatementKind::StorageLive(l)) = stmt.map(|s| &s.kind) {
        for &p in md.of_local(*l) {
            if init_means_set {
                state.remove(p.index());
            } else {
                state.insert(p.index());
            }
        }
    }
    if let Some(ms) = md.moves_at.get(&loc) {
        for &m in ms {
            for &p in md.below(md.moves[m].path) {
                if init_means_set {
                    state.remove(p.index());
                } else {
                    state.insert(p.index());
                }
            }
        }
    }
    if let Some(is) = md.inits_at.get(&loc) {
        for &i in is {
            for &p in md.below(md.inits[i].path) {
                if init_means_set {
                    state.insert(p.index());
                } else {
                    state.remove(p.index());
                }
            }
        }
    }
}

fn param_paths(md: &MoveData, body: &Body, state: &mut BitSet) {
    for i in 1..=body.arg_count {
        for &p in md.of_local(Local::new(i)) {
            state.insert(p.index());
        }
    }
}

/// Paths that may have a value on some path to here.
pub struct MaybeInit<'a>(pub &'a MoveData);
/// Paths that may have no value on some path to here.
pub struct MaybeUninit<'a>(pub &'a MoveData);
/// Paths that were given a value on some path to here, even if moved since.
/// Forgotten at `StorageDead`/`StorageLive`.
pub struct EverInit<'a>(pub &'a MoveData);
/// The move-outs that may have happened on some path to here.
pub struct ReachingMoves<'a>(pub &'a MoveData);

impl Analysis for MaybeInit<'_> {
    const FORWARD: bool = true;
    fn domain_size(&self, _: &Body) -> usize {
        paths_of_state(self.0)
    }
    fn boundary(&self, body: &Body, state: &mut BitSet) {
        param_paths(self.0, body, state);
    }
    fn statement(&self, state: &mut BitSet, stmt: &Statement, loc: Location) {
        path_transfer(self.0, state, Some(stmt), loc, true);
    }
    fn terminator(&self, state: &mut BitSet, _: &Terminator, loc: Location) {
        path_transfer(self.0, state, None, loc, true);
    }
}

impl Analysis for MaybeUninit<'_> {
    const FORWARD: bool = true;
    fn domain_size(&self, _: &Body) -> usize {
        paths_of_state(self.0)
    }
    fn boundary(&self, body: &Body, state: &mut BitSet) {
        for (l, _) in body.locals.iter_enumerated() {
            if l.index() == 0 || l.index() > body.arg_count {
                for &p in self.0.of_local(l) {
                    state.insert(p.index());
                }
            }
        }
    }
    fn statement(&self, state: &mut BitSet, stmt: &Statement, loc: Location) {
        path_transfer(self.0, state, Some(stmt), loc, false);
    }
    fn terminator(&self, state: &mut BitSet, _: &Terminator, loc: Location) {
        path_transfer(self.0, state, None, loc, false);
    }
}

impl EverInit<'_> {
    fn transfer(&self, state: &mut BitSet, stmt: Option<&Statement>, loc: Location) {
        let md = self.0;
        if let Some(StatementKind::StorageLive(l)) = stmt.map(|s| &s.kind) {
            for &p in md.of_local(*l) {
                state.remove(p.index());
            }
        }
        if let Some(ms) = md.moves_at.get(&loc) {
            for &m in ms {
                if md.moves[m].cause == MoveCause::StorageDead {
                    for &p in md.below(md.moves[m].path) {
                        state.remove(p.index());
                    }
                }
            }
        }
        if let Some(is) = md.inits_at.get(&loc) {
            for &i in is {
                for &p in md.below(md.inits[i].path) {
                    state.insert(p.index());
                }
            }
        }
    }
}

impl Analysis for EverInit<'_> {
    const FORWARD: bool = true;
    fn domain_size(&self, _: &Body) -> usize {
        paths_of_state(self.0)
    }
    fn boundary(&self, body: &Body, state: &mut BitSet) {
        param_paths(self.0, body, state);
    }
    fn statement(&self, state: &mut BitSet, stmt: &Statement, loc: Location) {
        self.transfer(state, Some(stmt), loc);
    }
    fn terminator(&self, state: &mut BitSet, _: &Terminator, loc: Location) {
        self.transfer(state, None, loc);
    }
}

impl ReachingMoves<'_> {
    fn transfer(&self, state: &mut BitSet, stmt: Option<&Statement>, loc: Location) {
        let md = self.0;
        if let Some(StatementKind::StorageLive(l)) = stmt.map(|s| &s.kind) {
            for &p in md.of_local(*l) {
                for &m in &md.moves_of[p] {
                    state.remove(m.index());
                }
            }
        }
        if let Some(ms) = md.moves_at.get(&loc) {
            for &m in ms {
                state.insert(m.index());
            }
        }
        if let Some(is) = md.inits_at.get(&loc) {
            for &i in is {
                let p = md.inits[i].path;
                // An init of `p` ends the moves of `p`, its ancestors and what is below it.
                let mut a = Some(p);
                while let Some(c) = a {
                    for &m in &md.moves_of[c] {
                        state.remove(m.index());
                    }
                    a = md.paths[c].parent;
                }
                for &d in md.below(p) {
                    for &m in &md.moves_of[d] {
                        state.remove(m.index());
                    }
                }
            }
        }
    }
}

impl Analysis for ReachingMoves<'_> {
    const FORWARD: bool = true;
    fn domain_size(&self, _: &Body) -> usize {
        self.0.moves.len()
    }
    fn boundary(&self, _: &Body, _: &mut BitSet) {}
    fn statement(&self, state: &mut BitSet, stmt: &Statement, loc: Location) {
        self.transfer(state, Some(stmt), loc);
    }
    fn terminator(&self, state: &mut BitSet, _: &Terminator, loc: Location) {
        self.transfer(state, None, loc);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::dataflow::{Item, Results};
    use crate::mir::traversal::predecessors;

    fn body_of(src: &str, name: &str) -> (hir::Program, Body) {
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mut d = Vec::new();
        let mir = crate::mir::build_program(&hir, &mut d);
        let body = mir.bodies.into_iter().flatten().find(|b| b.name == name).unwrap();
        (hir, body)
    }

    fn local_named(body: &Body, name: &str) -> Local {
        body.locals
            .iter_enumerated()
            .find(|(_, d)| matches!(&d.kind, LocalKind::User { name: n, .. } | LocalKind::Param { name: n, .. } if n == name))
            .map(|(l, _)| l)
            .unwrap()
    }

    #[test]
    fn paths_of_a_take() {
        let src = "struct Player { public: string name; string title; }
none main() {
    Player p = \"a\", \"b\";
    string n = take p.name;
}
";
        let (_, body) = body_of(src, "main");
        let md = MoveData::build(&body);
        let p = local_named(&body, "p");
        let root = md.local_path[p];
        let name = md.path_of(&Place { local: p, proj: vec![Proj::Field(0)] }).expect("p.name has a path");
        assert_eq!(md.paths[name].parent, Some(root));
        assert_eq!(md.paths[root].children, [name]);
        assert!(md.path_of(&Place { local: p, proj: vec![Proj::Field(1)] }).is_none(), "p.title is never named");
        assert!(md.is_ancestor(root, name) && !md.is_ancestor(name, root));
        assert_eq!(md.descendants(root), [root, name]);
        assert!(md.moves.iter().any(|m| m.path == name && m.cause == MoveCause::Take));
        let through = Place { local: p, proj: vec![Proj::Deref, Proj::Field(0)] };
        assert!(md.path_of(&through).is_none());
        assert_eq!(md.prefix_path(&through), root);
    }

    #[test]
    fn a_conditional_move_is_maybe_uninit() {
        let src = "none consume(string s) { }
bool flip() { return true; }
none main() {
    string s = \"x\";
    if (flip()) { consume(s); }
    consume(s);
}
";
        let (_, body) = body_of(src, "main");
        let md = MoveData::build(&body);
        let preds = predecessors(&body);
        let mu = Results::compute(MaybeUninit(&md), &body, &preds);
        let mi = Results::compute(MaybeInit(&md), &body, &preds);
        let rm = Results::compute(ReachingMoves(&md), &body, &preds);
        let s = md.local_path[local_named(&body, "s")];
        // Look at the second consume: the one later in reverse post-order.
        let mut calls = Vec::new();
        mu.visit(&body, |loc, st, item| {
            if let Item::Terminator(t) = item {
                if let TerminatorKind::Call { args, .. } = &t.kind {
                    if matches!(args.first().map(|a| &a.op), Some(Operand::Move(p)) if p.local == local_named(&body, "s")) {
                        calls.push((loc, st.contains(s.index())));
                    }
                }
            }
        });
        assert_eq!(calls.len(), 2);
        assert!(!calls[0].1, "certainly has a value at the first consume");
        let (second, uninit) = calls[1];
        assert!(uninit, "maybe uninitialised at the second");
        assert!(mi.state_at(&body, second).contains(s.index()), "and maybe initialised");
        let reaching = rm.state_at(&body, second);
        assert_eq!(reaching.iter().filter(|&m| md.moves[MoveOutId::new(m)].cause.visible()).count(), 1);
    }

    #[test]
    fn ever_init_forgets_at_storage_dead() {
        let src = "none main() {
    uint32 x = 1;
    uint32 y = x;
}
";
        let (_, body) = body_of(src, "main");
        let md = MoveData::build(&body);
        let preds = predecessors(&body);
        let ever = Results::compute(EverInit(&md), &body, &preds);
        let x = md.local_path[local_named(&body, "x")];
        let mut seen_set = false;
        let mut at_return = None;
        ever.visit(&body, |_, st, item| match item {
            Item::Statement(s) if matches!(s.kind, StatementKind::StorageDead(_)) => {
                seen_set |= st.contains(x.index());
            }
            Item::Terminator(t) if matches!(t.kind, TerminatorKind::Return) => at_return = Some(st.contains(x.index())),
            _ => {}
        });
        assert!(seen_set, "x was initialised before it died");
        assert_eq!(at_return, Some(false), "and is forgotten after StorageDead");
    }

    #[test]
    fn mut_borrow_fields_are_found() {
        let src = "struct Writer lives sink { &mut string sink; }
struct Wrap { public: uint32 n; }
none main() { }
";
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("{diags:?}"));
        let ty = |name: &str| {
            let id = hir.structs.iter().position(|s| s.name == name).unwrap();
            Type::Struct { id, name: crate::types::intern(name) }
        };
        assert!(holds_mut_borrow(&hir, ty("Writer")));
        assert!(!holds_mut_borrow(&hir, ty("Wrap")));
        let (s, f, t) = find_mut_borrow(&hir, ty("Writer")).unwrap();
        assert_eq!((s.as_str(), f.as_str(), t.to_string().as_str()), ("Writer", "sink", "&mut string"));
    }
}
