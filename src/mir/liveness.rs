//! Which locals are live at each point (spec 3.4). Backward, domain = locals.
//!
//! A local is *used* by every operand and place that mentions it, and by a
//! write through it (`p.f = ..` and `*r = ..` use `p` and `r`). It is *defined*
//! by assigning the whole local, and by `StorageLive`/`StorageDead`. A `Drop`
//! is not a use, except when the dropped type has a user `drop()` somewhere in
//! it: that `drop()` can read what the value borrows (drop-liveness, Q17).

use super::dataflow::{Analysis, BitSet, Results};
use super::traversal::predecessors;
use super::ty::place_ty;
use super::*;

/// Does destroying a value of `ty` run user code (a `drop()` in it or in a field)?
pub fn holds_user_drop(hir: &hir::Program, ty: Type) -> bool {
    match ty {
        Type::Struct { id, .. } => {
            let s = &hir.structs[id];
            s.has_drop.is_some() || s.fields.iter().any(|f| holds_user_drop(hir, f.ty))
        }
        _ => false,
    }
}

fn operand_local(op: &Operand, f: &mut impl FnMut(Local)) {
    if let Operand::Copy(p) | Operand::Move(p) = op {
        f(p.local);
    }
}

pub fn rvalue_locals(rv: &Rvalue, f: &mut impl FnMut(Local)) {
    match rv {
        Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) => operand_local(o, f),
        Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => {
            operand_local(a, f);
            operand_local(b, f);
        }
        Rvalue::Shift { value, amount, .. } => {
            operand_local(value, f);
            operand_local(amount, f);
        }
        Rvalue::Aggregate(_, ops) => ops.iter().for_each(|o| operand_local(o, f)),
        Rvalue::Ref(_, p) | Rvalue::RawRef(p) | Rvalue::DeepCopy(p) => f(p.local),
    }
}

/// The locals a terminator uses.
pub fn terminator_uses(t: &TerminatorKind, f: &mut impl FnMut(Local)) {
    match t {
        TerminatorKind::If { cond, .. } => operand_local(cond, f),
        TerminatorKind::Switch { discr, .. } => operand_local(discr, f),
        TerminatorKind::Call { args, dest, .. } => {
            args.iter().for_each(|a| operand_local(&a.op, f));
            if let Some(d) = dest {
                if !d.proj.is_empty() {
                    f(d.local);
                }
            }
        }
        TerminatorKind::Return => f(Local(0)),
        TerminatorKind::Goto(_) | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => {}
    }
}

/// Live-variable analysis.
pub struct LiveLocals<'a> {
    pub hir: &'a hir::Program,
    pub body: &'a Body,
}

impl LiveLocals<'_> {
    fn drop_is_use(&self, place: &Place) -> bool {
        place_ty(self.body, self.hir, place).is_ok_and(|t| holds_user_drop(self.hir, t))
    }
}

impl Analysis for LiveLocals<'_> {
    const FORWARD: bool = false;

    fn domain_size(&self, body: &Body) -> usize {
        body.locals.len()
    }

    fn boundary(&self, _: &Body, _: &mut BitSet) {}

    fn statement(&self, state: &mut BitSet, stmt: &Statement, _: Location) {
        match &stmt.kind {
            StatementKind::Assign(dest, rv) => {
                if dest.proj.is_empty() {
                    state.remove(dest.local.index());
                } else {
                    state.insert(dest.local.index());
                }
                rvalue_locals(rv, &mut |l| {
                    state.insert(l.index());
                });
            }
            StatementKind::StorageLive(l) | StatementKind::StorageDead(l) => {
                state.remove(l.index());
            }
            StatementKind::Drop { place, .. } => {
                if self.drop_is_use(place) {
                    state.insert(place.local.index());
                }
            }
            StatementKind::FakeRead(p) => {
                state.insert(p.local.index());
            }
            StatementKind::Nop => {}
        }
    }

    fn terminator(&self, state: &mut BitSet, term: &Terminator, _: Location) {
        if let TerminatorKind::Call { dest: Some(d), .. } = &term.kind {
            if d.proj.is_empty() {
                state.remove(d.local.index());
            }
        }
        terminator_uses(&term.kind, &mut |l| {
            state.insert(l.index());
        });
    }
}

/// Liveness of every local at every point of one body.
pub struct Liveness<'a> {
    results: Results<LiveLocals<'a>>,
    body: &'a Body,
}

impl<'a> Liveness<'a> {
    pub fn compute(hir: &'a hir::Program, body: &'a Body) -> Liveness<'a> {
        let preds = predecessors(body);
        Liveness { results: Results::compute(LiveLocals { hir, body }, body, &preds), body }
    }

    /// The locals live on entry to `loc` (before it runs).
    pub fn live_before(&self, loc: Location) -> BitSet {
        let mut state = self.results.state_at(self.body, loc);
        let block = &self.body.blocks[loc.block];
        let i = loc.index as usize;
        if i < block.statements.len() {
            self.results.analysis.statement(&mut state, &block.statements[i], loc);
        } else {
            self.results.analysis.terminator(&mut state, &block.terminator, loc);
        }
        state
    }

    /// Call `f` with every reachable location and the locals live on entry to it.
    pub fn for_each_live_before(&self, mut f: impl FnMut(Location, &BitSet)) {
        let analysis = &self.results.analysis;
        self.results.visit(self.body, |loc, after, item| {
            let mut before = after.clone();
            match item {
                super::dataflow::Item::Statement(s) => analysis.statement(&mut before, s, loc),
                super::dataflow::Item::Terminator(t) => analysis.terminator(&mut before, t, loc),
            }
            f(loc, &before);
        });
    }

    /// The locals live just after `loc` (on entry to its successors).
    pub fn live_after(&self, loc: Location) -> BitSet {
        self.results.state_at(self.body, loc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(body: &Body, name: &str) -> Local {
        body.locals.iter_enumerated().find(|(_, d)| matches!(&d.kind, LocalKind::User { name: n, .. } | LocalKind::Param { name: n, .. } if n == name)).unwrap().0
    }

    #[test]
    fn a_borrow_is_live_until_its_last_use() {
        let src = "in io;
none main() {
    string name = \"Illy\";
    &string first = &name;
    io::outln(first);
    io::outln(name);
}
";
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("{diags:?}"));
        let mir = build_program(&hir, &mut Vec::new());
        let body = mir.bodies.iter().flatten().find(|b| b.name == "main").unwrap();
        let live = Liveness::compute(&hir, body);
        let first = named(body, "first");
        let mut live_at = Vec::new();
        for (b, block) in body.blocks.iter_enumerated() {
            for i in 0..=block.statements.len() {
                if live.live_before(Location { block: b, index: i as u32 }).contains(first.index()) {
                    live_at.push((b.index(), i));
                }
            }
        }
        // Defined by `first = &name` (not live before it), live up to the temp copy that reads it.
        assert!(!live_at.is_empty());
        let last = *live_at.last().unwrap();
        let (b, i) = last;
        let stmt = &body.blocks[BlockId(b as u32)].statements[i];
        assert!(matches!(&stmt.kind, StatementKind::Assign(_, Rvalue::Ref(..))), "last use is the reborrow `&*first`: {stmt:?}");
        let def = (0..body.blocks[BlockId(0)].statements.len())
            .find(|&i| matches!(&body.blocks[BlockId(0)].statements[i].kind, StatementKind::Assign(p, Rvalue::Ref(..)) if p.local == first))
            .unwrap();
        assert!(!live.live_before(Location { block: BlockId(0), index: def as u32 }).contains(first.index()));
        assert!(live.live_after(Location { block: BlockId(0), index: def as u32 }).contains(first.index()));
    }

    #[test]
    fn a_user_drop_is_a_use() {
        let src = "struct H lives s { public: &string s; none drop() { } }
none main() {
    string a = \"x\";
    H h = &a;
}
";
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("{diags:?}"));
        let mir = build_program(&hir, &mut Vec::new());
        let body = mir.bodies.iter().flatten().find(|b| b.name == "main").unwrap();
        let live = Liveness::compute(&hir, body);
        let h = named(body, "h");
        // `h` is live from its definition to its scope-end drop.
        let drop_at = body
            .blocks
            .iter_enumerated()
            .flat_map(|(b, bl)| bl.statements.iter().enumerate().map(move |(i, s)| (b, i, s)))
            .find(|(_, _, s)| matches!(&s.kind, StatementKind::Drop { place, .. } if place.local == h))
            .map(|(b, i, _)| Location { block: b, index: i as u32 })
            .unwrap();
        assert!(live.live_before(drop_at).contains(h.index()));
    }
}
