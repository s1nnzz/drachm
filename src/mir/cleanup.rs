//! Cleanup before code generation (spec 2.8), and the rule that decides which
//! programs the MIR backends accept in stage 1 (`eligible`).

use super::traversal::{predecessors, successors};
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

// ---- the stage 1 cutover rule ------------------------------------------------

/// Can this program be compiled from MIR in stage 1? Pure function of the MIR
/// (cleaned or not). See `docs/impl/wp4.md`.
pub fn eligible(hir: &hir::Program, p: &Program) -> bool {
    ineligible_reason(hir, p).is_none()
}

/// Why `eligible` is false, for messages and tests.
pub fn ineligible_reason(hir: &hir::Program, p: &Program) -> Option<String> {
    for s in &hir.structs {
        if s.has_drop.is_some() {
            return Some(format!("struct `{}` has a drop()", s.name));
        }
        if s.fields.iter().any(|f| f.ty.is_ptr()) {
            return Some(format!("struct `{}` has a pointer field", s.name));
        }
    }
    for (f, body) in p.bodies.iter().enumerate() {
        let Some(body) = body else { return Some(format!("function #{f} has no body")) };
        if let Err(why) = analyse(hir, body) {
            return Some(format!("fn {}: {why}", body.name));
        }
    }
    None
}

/// For each local, is it a string temporary whose value is moved into another
/// one? (See `analyse`.) The backends skip the `Drop` of such a local: until
/// drop elaboration exists, the builder drops both the source and the
/// destination of the move, and only the destination owns the string.
/// All `false` for a body that is not eligible.
pub fn moved_out_temps(hir: &hir::Program, body: &Body) -> Vec<bool> {
    analyse(hir, body).unwrap_or_else(|_| vec![false; body.locals.len()])
}

#[derive(Clone, Copy, PartialEq)]
enum Def {
    /// A call result; the call's target block.
    Call(BlockId),
    /// `const "literal"`.
    Lit,
    /// `move` of another string temporary.
    MoveFrom,
}

#[derive(Default, Clone)]
struct TempFacts {
    defs: Vec<Def>,
    /// Statements that move out of it: (block, index in the block).
    moves: Vec<(BlockId, usize)>,
}

fn operand_place(op: &Operand) -> Option<&Place> {
    match op {
        Operand::Copy(p) | Operand::Move(p) => Some(p),
        Operand::Const(_) => None,
    }
}

fn rvalue_places(rv: &Rvalue) -> Vec<&Place> {
    let ops: Vec<&Operand> = match rv {
        Rvalue::Use(a) | Rvalue::Neg(a) | Rvalue::Not(a) | Rvalue::BitNot(a) | Rvalue::Convert(a, _) => vec![a],
        Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => vec![a, b],
        Rvalue::Shift { value, amount, .. } => vec![value, amount],
        Rvalue::Aggregate(_, ops) => ops.iter().collect(),
        Rvalue::Ref(_, p) | Rvalue::RawRef(p) | Rvalue::DeepCopy(p) => return vec![p],
    };
    ops.into_iter().filter_map(operand_place).collect()
}

/// Check one body against the rule and find its moved-out string temporaries.
///
/// True when: no `Switch`, `Panic`, `RawRef` or `DeepCopy`; no `Drop` with a
/// flag; no pointer type; no user, parameter or return local that needs
/// dropping; and every local that does need dropping is a `string` temporary
/// with exactly one definition (a `Text`, `Radix`, `Join` or user call result,
/// a string literal, or a `move` of another such temporary) that is only ever
/// borrowed (`Rvalue::Ref`), dropped, or moved out by that one `move`.
///
/// Deviation from the brief: the builder writes every call result to a fresh
/// temporary and then moves it into the temporary that is used (B7), so a
/// moved-from source and its `Drop` exist. They are accepted when the `move`
/// is the first effective statement of the call's target block, which has no
/// other predecessor: then the move runs before any `Drop` of the source can,
/// and that `Drop` is a no-op the backends skip.
fn analyse(hir: &hir::Program, body: &Body) -> Result<Vec<bool>, String> {
    let nloc = body.locals.len();
    let mut is_str = vec![false; nloc];
    for (l, d) in body.locals.iter_enumerated() {
        if matches!(d.ty, Type::Ptr { .. }) {
            return Err(format!("{l} has a pointer type"));
        }
        if needs_drop(hir, d.ty) {
            if !matches!(d.kind, LocalKind::Temp) || d.ty != Type::Str {
                return Err(format!("{l} ({}) needs dropping and is not a string temporary", d.ty));
            }
            is_str[l.index()] = true;
        }
    }
    let mut facts = vec![TempFacts::default(); nloc];
    let strp = |p: &Place| is_str[p.local.index()];
    let bad = |l: Local, what: &str| Err(format!("string temporary {l} {what}"));

    for (b, block) in body.blocks.iter_enumerated() {
        for (i, s) in block.statements.iter().enumerate() {
            match &s.kind {
                StatementKind::Assign(dest, rv) => {
                    if matches!(rv, Rvalue::RawRef(_)) {
                        return Err("takes a raw pointer".into());
                    }
                    if matches!(rv, Rvalue::DeepCopy(_)) {
                        return Err("deep-copies a value".into());
                    }
                    if strp(dest) {
                        let l = dest.local;
                        if !dest.proj.is_empty() {
                            return bad(l, "is projected");
                        }
                        match rv {
                            Rvalue::Use(Operand::Const(Constant::Str(_))) => facts[l.index()].defs.push(Def::Lit),
                            Rvalue::Use(Operand::Move(src)) if src.proj.is_empty() && strp(src) => {
                                facts[l.index()].defs.push(Def::MoveFrom);
                                facts[src.local.index()].moves.push((b, i));
                            }
                            _ => return bad(l, "is assigned something other than a literal or a moved temporary"),
                        }
                        continue;
                    }
                    match rv {
                        // Borrowing a string temporary is its one real use.
                        Rvalue::Ref(_, p) if strp(p) && p.proj.is_empty() => {}
                        _ => {
                            if let Some(p) = rvalue_places(rv).into_iter().find(|p| strp(p)) {
                                return bad(p.local, "is used other than by `&`");
                            }
                        }
                    }
                }
                StatementKind::Drop { place, flag, .. } => {
                    if flag.is_some() {
                        return Err("has a drop flag".into());
                    }
                    if strp(place) && !place.proj.is_empty() {
                        return bad(place.local, "is projected in a drop");
                    }
                }
                StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop => {}
                StatementKind::FakeRead(p) => {
                    if strp(p) {
                        return bad(p.local, "is read");
                    }
                }
            }
        }
        match &block.terminator.kind {
            TerminatorKind::Switch { .. } => return Err("has a switch".into()),
            TerminatorKind::Panic(_) => return Err("has a panic".into()),
            TerminatorKind::If { cond, .. } => {
                if operand_place(cond).is_some_and(strp) {
                    return Err("branches on a string".into());
                }
            }
            TerminatorKind::Call { callee, args, dest, target } => {
                if let Some(p) = args.iter().filter_map(|a| operand_place(&a.op)).find(|p| strp(p)) {
                    return bad(p.local, "is passed to a call");
                }
                if let Some(d) = dest
                    && strp(d)
                {
                    if !d.proj.is_empty() {
                        return bad(d.local, "is projected");
                    }
                    if !matches!(callee, Callee::Text | Callee::Radix { .. } | Callee::Join | Callee::Func(_)) {
                        return bad(d.local, "is the result of a runtime string call");
                    }
                    facts[d.local.index()].defs.push(Def::Call(*target));
                }
            }
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Unreachable => {}
        }
    }

    let preds = predecessors(body);
    let mut moved_out = vec![false; nloc];
    for l in 0..nloc {
        if !is_str[l] {
            continue;
        }
        let f = &facts[l];
        let local = Local(l as u32);
        if f.defs.len() > 1 {
            return bad(local, "is assigned more than once");
        }
        if f.moves.len() > 1 {
            return bad(local, "is moved from more than once");
        }
        if let Some(&(mb, mi)) = f.moves.first() {
            let Some(Def::Call(target)) = f.defs.first().copied() else {
                return bad(local, "is moved from but is not a call result");
            };
            let first = body.blocks[mb].statements.iter().position(|s| !is_noop(&s.kind));
            if mb != target || first != Some(mi) || preds[target].len() != 1 {
                return bad(local, "is not moved right after the call that made it");
            }
            moved_out[l] = true;
        }
    }
    Ok(moved_out)
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
        }
    }

    fn nop() -> Statement {
        Statement { kind: StatementKind::Nop, span: Span::new(0, 0), unsafe_ctx: false }
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
        assert!(eligible(&hir, &mir));
    }

    #[test]
    fn user_strings_are_not_eligible() {
        let (hir, mir) = built("in io;\nnone main() { string s = \"a\"; io::outln(s); }\n");
        assert!(!eligible(&hir, &mir));
        let (hir, mir) = built("in io;\nnone main() { io::outln(\"a\"); io::outln(5::text()); }\n");
        assert!(eligible(&hir, &mir), "{:?}", ineligible_reason(&hir, &mir));
    }
}
