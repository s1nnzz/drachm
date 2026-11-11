//! Walking the control-flow graph.

use super::{BlockId, Body, IndexVec, Terminator, TerminatorKind};

/// The blocks a terminator can go to, in a stable order (a call's `target`
/// is its only successor: calls never unwind).
pub fn successors(t: &Terminator) -> Vec<BlockId> {
    match &t.kind {
        TerminatorKind::Goto(b) => vec![*b],
        TerminatorKind::If { then, els, .. } => vec![*then, *els],
        TerminatorKind::Switch { cases, otherwise, .. } => {
            cases.iter().map(|(_, b)| *b).chain([*otherwise]).collect()
        }
        TerminatorKind::Call { target, .. } => vec![*target],
        TerminatorKind::Return | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => Vec::new(),
    }
}

/// For each block, the blocks that jump to it (once per edge).
pub fn predecessors(body: &Body) -> IndexVec<BlockId, Vec<BlockId>> {
    let mut preds: IndexVec<BlockId, Vec<BlockId>> =
        IndexVec::from_vec(body.blocks.iter().map(|_| Vec::new()).collect());
    for (b, block) in body.blocks.iter_enumerated() {
        for s in successors(&block.terminator) {
            preds[s].push(b);
        }
    }
    preds
}

/// The blocks reachable from `bb0` in reverse post-order: every block comes
/// after at least one of its predecessors, and after all of them unless the
/// edge is a back edge.
pub fn reverse_postorder(body: &Body) -> Vec<BlockId> {
    let n = body.blocks.len();
    let mut seen = vec![false; n];
    let mut post = Vec::with_capacity(n);
    if n == 0 {
        return post;
    }
    let entry = BlockId(0);
    seen[0] = true;
    // Each frame: a block and the successors not yet visited.
    let mut stack = vec![(entry, successors(&body.blocks[entry].terminator))];
    while let Some((b, rest)) = stack.last_mut() {
        match rest.pop() {
            Some(s) if !seen[s.0 as usize] => {
                seen[s.0 as usize] = true;
                let next = successors(&body.blocks[s].terminator);
                stack.push((s, next));
            }
            Some(_) => {}
            None => {
                post.push(*b);
                stack.pop();
            }
        }
    }
    post.reverse();
    post
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::Span;
    use crate::mir::{BasicBlock, Operand, Constant};

    fn term(kind: TerminatorKind) -> BasicBlock {
        BasicBlock { statements: Vec::new(), terminator: Terminator { kind, span: Span::new(0, 0), unsafe_ctx: false } }
    }

    fn body(blocks: Vec<TerminatorKind>) -> Body {
        Body {
            func: 0,
            name: "t".into(),
            span: Span::new(0, 0),
            arg_count: 0,
            locals: IndexVec::new(),
            blocks: IndexVec::from_vec(blocks.into_iter().map(term).collect()),
            scopes: IndexVec::new(),
            unsafe_flag: false,
            unsafe_fn: false,
            fallthrough_end: None,
        }
    }

    fn cond() -> Operand {
        Operand::Const(Constant::Bool(true))
    }

    #[test]
    fn diamond_with_a_loop() {
        // bb0 -> bb1; bb1 -> bb2 | bb3; bb2 -> bb1 (back edge); bb3 return; bb4 unreachable.
        let b = body(vec![
            TerminatorKind::Goto(BlockId(1)),
            TerminatorKind::If { cond: cond(), then: BlockId(2), els: BlockId(3) },
            TerminatorKind::Goto(BlockId(1)),
            TerminatorKind::Return,
            TerminatorKind::Goto(BlockId(3)),
        ]);
        let preds = predecessors(&b);
        assert_eq!(preds[BlockId(1)], vec![BlockId(0), BlockId(2)]);
        assert_eq!(preds[BlockId(3)], vec![BlockId(1), BlockId(4)]);
        assert!(preds[BlockId(0)].is_empty());

        let rpo = reverse_postorder(&b);
        assert_eq!(rpo.len(), 4, "the unreachable block is left out");
        assert_eq!(rpo[0], BlockId(0));
        let pos = |x: u32| rpo.iter().position(|&b| b == BlockId(x)).unwrap();
        assert!(pos(1) < pos(2) && pos(1) < pos(3));
    }
}
