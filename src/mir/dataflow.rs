//! A small dataflow solver over the MIR's control-flow graph (spec 3.0).
//!
//! Every analysis is a powerset lattice joined by union, with the domain an
//! index range (move paths, move-outs, locals). Forward analyses start from
//! `Analysis::boundary` at `bb0`; backward ones from the boundary at every
//! block that leaves the function. Blocks `bb0` cannot reach are never
//! visited and keep the empty state.

use std::collections::VecDeque;

use super::traversal::{reverse_postorder, successors};
use super::index::Idx;
use super::{BasicBlock, BlockId, Body, IndexVec, Location, Statement, Terminator};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BitSet {
    words: Vec<u64>,
    len: usize,
}

impl BitSet {
    /// All clear.
    pub fn new(len: usize) -> BitSet {
        BitSet { words: vec![0; len.div_ceil(64)], len }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.words.iter().all(|&w| w == 0)
    }

    /// True if it was clear.
    pub fn insert(&mut self, i: usize) -> bool {
        debug_assert!(i < self.len, "bit {i} out of {}", self.len);
        let (w, m) = (i / 64, 1u64 << (i % 64));
        let was = self.words[w] & m != 0;
        self.words[w] |= m;
        !was
    }

    /// True if it was set.
    pub fn remove(&mut self, i: usize) -> bool {
        debug_assert!(i < self.len, "bit {i} out of {}", self.len);
        let (w, m) = (i / 64, 1u64 << (i % 64));
        let was = self.words[w] & m != 0;
        self.words[w] &= !m;
        was
    }

    pub fn contains(&self, i: usize) -> bool {
        i < self.len && self.words[i / 64] & (1u64 << (i % 64)) != 0
    }

    /// True if `self` changed.
    pub fn union(&mut self, other: &BitSet) -> bool {
        debug_assert_eq!(self.len, other.len);
        let mut changed = false;
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            let new = *a | *b;
            changed |= new != *a;
            *a = new;
        }
        changed
    }

    pub fn subtract(&mut self, other: &BitSet) {
        debug_assert_eq!(self.len, other.len);
        for (a, b) in self.words.iter_mut().zip(&other.words) {
            *a &= !*b;
        }
    }

    pub fn clear(&mut self) {
        self.words.iter_mut().for_each(|w| *w = 0);
    }

    pub fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.words.iter().enumerate().flat_map(|(wi, &w)| {
            let mut bits = w;
            std::iter::from_fn(move || {
                if bits == 0 {
                    return None;
                }
                let b = bits.trailing_zeros() as usize;
                bits &= bits - 1;
                Some(wi * 64 + b)
            })
        })
    }
}

pub trait Analysis {
    const FORWARD: bool;
    fn domain_size(&self, body: &Body) -> usize;
    /// State at function entry (forward) or at every exit (backward).
    fn boundary(&self, body: &Body, state: &mut BitSet);
    /// Forward: the state before `stmt` becomes the state after it. Backward:
    /// the state after `stmt` becomes the state before it.
    fn statement(&self, state: &mut BitSet, stmt: &Statement, loc: Location);
    fn terminator(&self, state: &mut BitSet, term: &Terminator, loc: Location);
}

pub enum Item<'a> {
    Statement(&'a Statement),
    Terminator(&'a Terminator),
}

pub struct Results<A: Analysis> {
    pub analysis: A,
    /// Forward: the state at the start of each block. Backward: at its end.
    entry: IndexVec<BlockId, BitSet>,
}

fn at(block: BlockId, index: usize) -> Location {
    Location { block, index: index as u32 }
}

fn apply_block<A: Analysis>(a: &A, state: &mut BitSet, b: BlockId, block: &BasicBlock) {
    if A::FORWARD {
        for (i, s) in block.statements.iter().enumerate() {
            a.statement(state, s, at(b, i));
        }
        a.terminator(state, &block.terminator, at(b, block.statements.len()));
    } else {
        a.terminator(state, &block.terminator, at(b, block.statements.len()));
        for (i, s) in block.statements.iter().enumerate().rev() {
            a.statement(state, s, at(b, i));
        }
    }
}

impl<A: Analysis> Results<A> {
    pub fn compute(a: A, body: &Body, preds: &IndexVec<BlockId, Vec<BlockId>>) -> Results<A> {
        let n = body.blocks.len();
        let size = a.domain_size(body);
        let mut entry: IndexVec<BlockId, BitSet> = IndexVec::from_vec((0..n).map(|_| BitSet::new(size)).collect());
        let rpo = reverse_postorder(body);
        let mut reachable = vec![false; n];
        for b in &rpo {
            reachable[b.index()] = true;
        }
        let order: Vec<BlockId> = if A::FORWARD { rpo } else { rpo.into_iter().rev().collect() };
        if n > 0 {
            if A::FORWARD {
                a.boundary(body, &mut entry[BlockId(0)]);
            } else {
                for &b in &order {
                    if successors(&body.blocks[b].terminator).is_empty() {
                        a.boundary(body, &mut entry[b]);
                    }
                }
            }
        }
        let mut queued = vec![false; n];
        let mut work: VecDeque<BlockId> = VecDeque::new();
        for &b in &order {
            queued[b.index()] = true;
            work.push_back(b);
        }
        while let Some(b) = work.pop_front() {
            queued[b.index()] = false;
            let block = &body.blocks[b];
            let mut state = entry[b].clone();
            apply_block(&a, &mut state, b, block);
            let next: Vec<BlockId> = if A::FORWARD {
                successors(&block.terminator)
            } else {
                preds[b].iter().copied().filter(|p| reachable[p.index()]).collect()
            };
            for s in next {
                if entry[s].union(&state) && !queued[s.index()] {
                    queued[s.index()] = true;
                    work.push_back(s);
                }
            }
        }
        Results { analysis: a, entry }
    }

    /// The state at the start of `b` (forward) or the end of `b` (backward).
    pub fn entry(&self, b: BlockId) -> &BitSet {
        &self.entry[b]
    }

    /// Forward: the state just before `loc`. Backward: just after it.
    pub fn state_at(&self, body: &Body, loc: Location) -> BitSet {
        let block = &body.blocks[loc.block];
        let mut state = self.entry[loc.block].clone();
        let i = loc.index as usize;
        if A::FORWARD {
            for (k, s) in block.statements.iter().enumerate().take(i) {
                self.analysis.statement(&mut state, s, at(loc.block, k));
            }
        } else {
            let len = block.statements.len();
            if i < len {
                self.analysis.terminator(&mut state, &block.terminator, at(loc.block, len));
                for k in (i + 1..len).rev() {
                    self.analysis.statement(&mut state, &block.statements[k], at(loc.block, k));
                }
            }
        }
        state
    }

    /// Every location of every reachable block, in reverse post-order
    /// (forward) or post-order (backward), with the state `state_at` gives.
    pub fn visit(&self, body: &Body, mut f: impl FnMut(Location, &BitSet, Item<'_>)) {
        let rpo = reverse_postorder(body);
        let order: Vec<BlockId> = if A::FORWARD { rpo } else { rpo.into_iter().rev().collect() };
        for b in order {
            let block = &body.blocks[b];
            let len = block.statements.len();
            let mut state = self.entry[b].clone();
            if A::FORWARD {
                for (i, s) in block.statements.iter().enumerate() {
                    f(at(b, i), &state, Item::Statement(s));
                    self.analysis.statement(&mut state, s, at(b, i));
                }
                f(at(b, len), &state, Item::Terminator(&block.terminator));
            } else {
                f(at(b, len), &state, Item::Terminator(&block.terminator));
                self.analysis.terminator(&mut state, &block.terminator, at(b, len));
                for (i, s) in block.statements.iter().enumerate().rev() {
                    f(at(b, i), &state, Item::Statement(s));
                    self.analysis.statement(&mut state, s, at(b, i));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::traversal::predecessors;
    use crate::mir::{Local, LocalKind, Operand, Place, Rvalue, StatementKind, TerminatorKind};

    #[test]
    fn bitset_operations() {
        let mut a = BitSet::new(130);
        assert!(a.is_empty() && a.len() == 130);
        assert!(a.insert(0) && a.insert(64) && a.insert(129));
        assert!(!a.insert(64));
        assert!(a.contains(129) && !a.contains(1) && !a.contains(500));
        assert_eq!(a.iter().collect::<Vec<_>>(), [0, 64, 129]);
        let mut b = BitSet::new(130);
        b.insert(64);
        b.insert(5);
        assert!(a.union(&b));
        assert!(!a.union(&b), "second union changes nothing");
        assert_eq!(a.iter().collect::<Vec<_>>(), [0, 5, 64, 129]);
        a.subtract(&b);
        assert_eq!(a.iter().collect::<Vec<_>>(), [0, 129]);
        assert!(a.remove(0) && !a.remove(0));
        a.clear();
        assert!(a.is_empty());
    }

    fn body_of(src: &str, name: &str) -> Body {
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mut d = Vec::new();
        let mir = crate::mir::build_program(&hir, &mut d);
        mir.bodies.into_iter().flatten().find(|b| b.name == name).unwrap()
    }

    /// The local a statement or terminator assigns as a whole.
    fn defs(stmt: Option<&Statement>, term: Option<&Terminator>) -> Option<Local> {
        match (stmt.map(|s| &s.kind), term.map(|t| &t.kind)) {
            (Some(StatementKind::Assign(p, _)), _) if p.proj.is_empty() => Some(p.local),
            (_, Some(TerminatorKind::Call { dest: Some(p), .. })) if p.proj.is_empty() => Some(p.local),
            _ => None,
        }
    }

    fn operand_locals(op: &Operand, out: &mut Vec<Local>) {
        if let Operand::Copy(p) | Operand::Move(p) = op {
            out.push(p.local);
        }
    }

    fn uses(stmt: Option<&Statement>, term: Option<&Terminator>) -> Vec<Local> {
        let mut out = Vec::new();
        match (stmt.map(|s| &s.kind), term.map(|t| &t.kind)) {
            (Some(StatementKind::Assign(_, rv)), _) => match rv {
                Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) => {
                    operand_locals(o, &mut out)
                }
                Rvalue::Binary(_, a, b) => {
                    operand_locals(a, &mut out);
                    operand_locals(b, &mut out);
                }
                Rvalue::Ref(_, p) | Rvalue::DeepCopy(p) => out.push(p.local),
                _ => {}
            },
            (_, Some(TerminatorKind::If { cond, .. })) => operand_locals(cond, &mut out),
            (_, Some(TerminatorKind::Call { args, .. })) => args.iter().for_each(|a| operand_locals(&a.op, &mut out)),
            _ => {}
        }
        out
    }

    /// Toy forward analysis: which locals have been assigned on some path.
    struct MaybeAssigned;
    impl Analysis for MaybeAssigned {
        const FORWARD: bool = true;
        fn domain_size(&self, body: &Body) -> usize {
            body.locals.len()
        }
        fn boundary(&self, body: &Body, state: &mut BitSet) {
            for i in 1..=body.arg_count {
                state.insert(i);
            }
        }
        fn statement(&self, state: &mut BitSet, stmt: &Statement, _: Location) {
            if let Some(l) = defs(Some(stmt), None) {
                state.insert(l.index());
            }
        }
        fn terminator(&self, state: &mut BitSet, term: &Terminator, _: Location) {
            if let Some(l) = defs(None, Some(term)) {
                state.insert(l.index());
            }
        }
    }

    /// Toy backward analysis: liveness of locals.
    struct Live;
    impl Analysis for Live {
        const FORWARD: bool = false;
        fn domain_size(&self, body: &Body) -> usize {
            body.locals.len()
        }
        fn boundary(&self, _: &Body, _: &mut BitSet) {}
        fn statement(&self, state: &mut BitSet, stmt: &Statement, _: Location) {
            if let Some(l) = defs(Some(stmt), None) {
                state.remove(l.index());
            }
            for l in uses(Some(stmt), None) {
                state.insert(l.index());
            }
        }
        fn terminator(&self, state: &mut BitSet, term: &Terminator, _: Location) {
            if let Some(l) = defs(None, Some(term)) {
                state.remove(l.index());
            }
            for l in uses(None, Some(term)) {
                state.insert(l.index());
            }
        }
    }

    fn local_named(body: &Body, name: &str) -> Local {
        body.locals
            .iter_enumerated()
            .find(|(_, d)| matches!(&d.kind, LocalKind::User { name: n, .. } | LocalKind::Param { name: n, .. } if n == name))
            .map(|(l, _)| l)
            .unwrap_or_else(|| panic!("no local {name}"))
    }

    #[test]
    fn forward_reaches_a_fixpoint_around_a_loop() {
        let src = "none main() {
    mut uint32 a;
    mut uint32 i = 0;
    while (i < 3) {
        if (i == 1) { a = 5; }
        i++;
    }
    uint32 b = i;
}
";
        let body = body_of(src, "main");
        let preds = predecessors(&body);
        let res = Results::compute(MaybeAssigned, &body, &preds);
        let a = local_named(&body, "a").index();
        let b = local_named(&body, "b").index();
        let mut head_sees_a = false;
        let mut final_state = None;
        res.visit(&body, |_, st, item| {
            if let Item::Terminator(t) = item {
                if matches!(t.kind, TerminatorKind::If { .. }) && st.contains(a) {
                    head_sees_a = true;
                }
                if matches!(t.kind, TerminatorKind::Return) {
                    final_state = Some(st.clone());
                }
            }
        });
        assert!(head_sees_a, "the assignment in the loop reaches the loop head around the back edge");
        let fin = final_state.unwrap();
        assert!(fin.contains(a) && fin.contains(b));
        res.visit(&body, |loc, st, _| assert_eq!(&res.state_at(&body, loc), st));
    }

    #[test]
    fn backward_liveness() {
        let src = "none main() {
    uint32 x = 1;
    uint32 y = 2;
    uint32 z = x + 1;
    uint32 w = z;
}
";
        let body = body_of(src, "main");
        let preds = predecessors(&body);
        let res = Results::compute(Live, &body, &preds);
        let (x, y) = (local_named(&body, "x"), local_named(&body, "y"));
        let mut y_def = None;
        let mut x_use = None;
        for (b, block) in body.blocks.iter_enumerated() {
            for (i, s) in block.statements.iter().enumerate() {
                if let StatementKind::Assign(Place { local, proj }, _) = &s.kind {
                    if *local == y && proj.is_empty() {
                        y_def = Some(at(b, i));
                    }
                }
                if uses(Some(s), None).contains(&x) && x_use.is_none() {
                    x_use = Some(at(b, i));
                }
            }
        }
        // After `y = 2`, y is dead (never read) and x is still live (read later).
        let after_y = res.state_at(&body, y_def.unwrap());
        assert!(!after_y.contains(y.index()));
        assert!(after_y.contains(x.index()));
        // After the last use of x, x is dead.
        let after_x = res.state_at(&body, x_use.unwrap());
        assert!(!after_x.contains(x.index()));
        res.visit(&body, |loc, st, _| assert_eq!(&res.state_at(&body, loc), st));
    }

    #[test]
    fn unreachable_blocks_are_never_visited() {
        // The builder prunes dead blocks, so add one by hand.
        let src = "none main() { uint32 a = 1; }\n";
        let mut body = body_of(src, "main");
        let dead = body.blocks.push(BasicBlock {
            statements: Vec::new(),
            terminator: Terminator {
                kind: TerminatorKind::Goto(BlockId(0)),
                span: crate::diag::Span::new(0, 0),
                unsafe_ctx: false,
            },
        });
        let preds = predecessors(&body);
        let res = Results::compute(MaybeAssigned, &body, &preds);
        let mut seen_dead = false;
        res.visit(&body, |loc, _, _| seen_dead |= loc.block == dead);
        assert!(!seen_dead);
        assert!(res.entry(dead).is_empty());
        let res = Results::compute(Live, &body, &preds);
        res.visit(&body, |loc, _, _| seen_dead |= loc.block == dead);
        assert!(!seen_dead);
    }
}
