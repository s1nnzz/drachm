//! Cleanup before code generation (spec 2.8).

use super::traversal::successors;
use super::*;

/// Clean every body of `p` (see `cleanup`). Debug builds then run `validate`,
/// which must still hold.
pub fn cleanup_program(hir: &hir::Program, p: &mut Program) {
    for body in p.bodies.iter_mut().flatten() {
        cleanup(body);
    }
    #[cfg(debug_assertions)]
    if let Err(errors) = validate::validate(p, hir) {
        panic!("cleanup produced invalid MIR:\n{}", errors.join("\n"));
    }
    #[cfg(not(debug_assertions))]
    let _ = hir;
}

fn is_noop(k: &StatementKind) -> bool {
    matches!(k, StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::FakeRead(_) | StatementKind::Nop)
}

fn targets_mut(k: &mut TerminatorKind) -> Vec<&mut BlockId> {
    match k {
        TerminatorKind::Goto(b) => vec![b],
        TerminatorKind::If { then, els, .. } => vec![then, els],
        TerminatorKind::Switch { cases, otherwise, .. } => cases.iter_mut().map(|(_, b)| b).chain([otherwise]).collect(),
        TerminatorKind::Call { target, .. } => vec![target],
        TerminatorKind::Return | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => Vec::new(),
    }
}

/// Spec 2.8: drop the statements with no effect, skip empty `goto` blocks,
/// remove unreachable blocks, merge straight-line chains, and renumber the
/// blocks densely (`bb0` stays the entry). Statements are never reordered, no
/// operand changes, and no `Drop` goes away.
pub fn cleanup(body: &mut Body) {
    for block in body.blocks.iter_mut() {
        block.statements.retain(|s| !is_noop(&s.kind));
    }

    // Jumps to a block that holds only `goto C` go to C (following chains; a
    // cycle of empty blocks is left alone, it is an infinite loop).
    let n = body.blocks.len();
    let forward: Vec<BlockId> = (0..n)
        .map(|i| {
            let mut cur = BlockId(i as u32);
            for _ in 0..n {
                let b = &body.blocks[cur];
                match b.terminator.kind {
                    TerminatorKind::Goto(next) if b.statements.is_empty() && next != cur => cur = next,
                    _ => return cur,
                }
            }
            BlockId(i as u32)
        })
        .collect();
    for block in body.blocks.iter_mut() {
        for t in targets_mut(&mut block.terminator.kind) {
            *t = forward[t.index()];
        }
    }

    // Reachable blocks from the entry.
    let mut reach = vec![false; n];
    let mut stack = vec![BlockId(0)];
    reach[0] = true;
    while let Some(b) = stack.pop() {
        for s in successors(&body.blocks[b].terminator) {
            if !reach[s.index()] {
                reach[s.index()] = true;
                stack.push(s);
            }
        }
    }
    // Merge `A -> goto B` when A is B's only predecessor.
    let mut preds = vec![0usize; n];
    for (i, block) in body.blocks.iter().enumerate() {
        if reach[i] {
            for s in successors(&block.terminator) {
                preds[s.index()] += 1;
            }
        }
    }
    let mut merged = vec![false; n];
    for a in 0..n {
        if merged[a] || !reach[a] {
            continue;
        }
        loop {
            let TerminatorKind::Goto(b) = body.blocks[BlockId(a as u32)].terminator.kind else { break };
            let bi = b.index();
            if bi == a || bi == 0 || preds[bi] != 1 || merged[bi] {
                break;
            }
            let span = body.blocks[b].terminator.span;
            let hole = BasicBlock { statements: Vec::new(), terminator: Terminator { kind: TerminatorKind::Unreachable, span, unsafe_ctx: false } };
            let taken = std::mem::replace(&mut body.blocks[b], hole);
            let blk = &mut body.blocks[BlockId(a as u32)];
            blk.statements.extend(taken.statements);
            blk.terminator = taken.terminator;
            merged[bi] = true;
            preds[bi] = 0;
        }
    }

    let alive: Vec<bool> = (0..n).map(|i| reach[i] && !merged[i]).collect();
    let mut new_id = vec![BlockId(u32::MAX); n];
    let mut next = 0u32;
    for i in 0..n {
        if alive[i] {
            new_id[i] = BlockId(next);
            next += 1;
        }
    }
    let old = std::mem::take(&mut body.blocks).into_vec();
    let mut blocks = Vec::with_capacity(next as usize);
    for (i, mut b) in old.into_iter().enumerate() {
        if alive[i] {
            for t in targets_mut(&mut b.terminator.kind) {
                *t = new_id[t.index()];
            }
            blocks.push(b);
        }
    }
    body.blocks = IndexVec::from_vec(blocks);
    body.fallthrough_end = body.fallthrough_end.and_then(|f| alive[f.index()].then(|| new_id[f.index()]));
}

pub(crate) fn operand_place(op: &Operand) -> Option<&Place> {
    match op {
        Operand::Copy(p) | Operand::Move(p) => Some(p),
        Operand::Const(_) => None,
    }
}

pub(crate) fn rvalue_places(rv: &Rvalue) -> Vec<&Place> {
    let ops: Vec<&Operand> = match rv {
        Rvalue::MakeFn(..) => Vec::new(),
        Rvalue::Use(a) | Rvalue::Neg(a) | Rvalue::Not(a) | Rvalue::BitNot(a) | Rvalue::Convert(a, _) | Rvalue::Unsize(a, _) => vec![a],
        Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => vec![a, b],
        Rvalue::Shift { value, amount, .. } => vec![value, amount],
        Rvalue::Aggregate(_, ops) | Rvalue::Variant(_, _, ops) | Rvalue::Array(_, ops) => ops.iter().collect(),
        Rvalue::ListNew(..) => Vec::new(),
        Rvalue::Ref(_, p) | Rvalue::RawRef(p) | Rvalue::DeepCopy(p) | Rvalue::Discriminant(p) | Rvalue::Len(p) => return vec![p],
    };
    ops.into_iter().filter_map(operand_place).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built(src: &str) -> (hir::Program, Program) {
        let (hir, diags) = crate::check(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mir = build_program(&hir, &mut Vec::new());
        (hir, mir)
    }

    fn block(term: TerminatorKind) -> BasicBlock {
        let span = Span::new(0, 0);
        BasicBlock { statements: Vec::new(), terminator: Terminator { kind: term, span, unsafe_ctx: false } }
    }

    fn assign() -> Statement {
        Statement {
            kind: StatementKind::Assign(Place::local(Local(0)), Rvalue::Use(Operand::Const(Constant::Bool(true)))),
            span: Span::new(0, 0),
            unsafe_ctx: false,
            mark: Mark::None,
        }
    }

    fn nop() -> Statement {
        Statement { kind: StatementKind::Nop, span: Span::new(0, 0), unsafe_ctx: false, mark: Mark::None }
    }

    fn body_of(blocks: Vec<BasicBlock>) -> Body {
        let (hir, _) = crate::check("none main() { }");
        let hir = hir.unwrap();
        let mut b = build_program(&hir, &mut Vec::new()).bodies.remove(0).unwrap();
        b.blocks = IndexVec::from_vec(blocks);
        b.fallthrough_end = None;
        b
    }

    #[test]
    fn chains_collapse_and_unreachable_goes() {
        // bb0 -> bb1 -> bb2 -> return; bb3 is unreachable.
        let mut blocks = vec![
            block(TerminatorKind::Goto(BlockId(1))),
            block(TerminatorKind::Goto(BlockId(2))),
            block(TerminatorKind::Return),
            block(TerminatorKind::Goto(BlockId(2))),
        ];
        blocks[0].statements.extend([assign(), nop()]);
        blocks[2].statements.push(assign());
        blocks[3].statements.push(assign());
        let mut b = body_of(blocks);
        cleanup(&mut b);
        assert_eq!(b.blocks.len(), 1, "everything merges into the entry");
        assert_eq!(b.blocks[BlockId(0)].statements.len(), 2, "no-ops go, effective statements stay in order");
        assert!(matches!(b.blocks[BlockId(0)].terminator.kind, TerminatorKind::Return));
    }

    #[test]
    fn a_loop_is_untouched() {
        // bb0 -> bb1; bb1: if c -> bb2, bb3; bb2 -> bb1; bb3: return.
        let cond = Operand::Const(Constant::Bool(true));
        let mut blocks = vec![
            block(TerminatorKind::Goto(BlockId(1))),
            block(TerminatorKind::If { cond, then: BlockId(2), els: BlockId(3) }),
            block(TerminatorKind::Goto(BlockId(1))),
            block(TerminatorKind::Return),
        ];
        blocks[1].statements.push(assign());
        blocks[2].statements.push(assign());
        let mut b = body_of(blocks);
        cleanup(&mut b);
        // bb1 has two predecessors, so it is not merged into bb0, and bb0 (the
        // entry) is not skipped; the back edge keeps its target.
        assert_eq!(b.blocks.len(), 4);
        let TerminatorKind::Goto(back) = b.blocks[BlockId(2)].terminator.kind else { panic!() };
        assert_eq!(back, BlockId(1));
    }

    #[test]
    fn real_programs_stay_valid() {
        let (hir, mut mir) = built(
            "in io;
uint32 steps(uint64 start) {
    mut uint64 n = start;
    mut uint32 count = 0;
    while (n != 1) {
        if ((n & 1) == 0) { n >>= 1; } else { n = 3 * n + 1; }
        count++;
    }
    return count;
}
none main() { io::outln(steps(27)::text()); }
",
        );
        let before: usize = mir.bodies.iter().flatten().map(|b| b.blocks.len()).sum();
        cleanup_program(&hir, &mut mir);
        let after: usize = mir.bodies.iter().flatten().map(|b| b.blocks.len()).sum();
        assert!(after < before, "{after} < {before}");
        assert!(validate(&mir, &hir).is_ok());
    }
}
