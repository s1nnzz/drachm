//! Late-initialised immutable bindings (spec 3.1.6): a binding that is not
//! `mut` may be given its value once, where it certainly has none yet.
//! Assigning it again, or assigning it inside a loop that began after its
//! declaration, is E0205 (WP5 folds this into `mutability.rs`).

use super::dataflow::Results;
use super::moves::{events, Event, EverInit, MoveData};
use super::*;
use crate::diag::codes;

pub fn check(
    _hir: &hir::Program,
    body: &Body,
    md: &MoveData,
    ever: &Results<EverInit>,
    cycles: &IndexVec<BlockId, Option<u32>>,
    out: &mut Vec<Diagnostic>,
) {
    // The component of the block that starts each local's storage.
    let mut live_scc: IndexVec<Local, Option<Option<u32>>> = IndexVec::from_vec(body.locals.iter().map(|_| None).collect());
    for (b, block) in body.blocks.iter_enumerated() {
        for s in &block.statements {
            if let StatementKind::StorageLive(l) = s.kind {
                if live_scc[l].is_none() {
                    live_scc[l] = Some(cycles[b]);
                }
            }
        }
    }
    ever.visit(body, |loc, state, _| {
        for e in events(body, loc) {
            let Event::Init(place) = e else { continue };
            if !place.proj.is_empty() {
                continue;
            }
            let l = place.local;
            let LocalKind::User { name, mutability: Mutability::Fixed } = &body.locals[l].kind else { continue };
            let Some(declared_in) = live_scc[l] else { continue };
            let in_loop = cycles[loc.block].is_some() && cycles[loc.block] != declared_in;
            if !state.contains(md.local_path[l].index()) && !in_loop {
                continue;
            }
            let block = &body.blocks[loc.block];
            let at = block.statements.get(loc.index as usize).map_or(block.terminator.span, |s| s.span);
            let ty = body.locals[l].ty;
            let d = if in_loop {
                // Its first assignment, but a loop would repeat it.
                Diagnostic::at(&codes::E0205, at, format!("cannot assign `{name}` inside a loop: it is not `mut`"))
                    .note("the loop would assign it again each time round")
                    .label(body.locals[l].span, "declared here, outside the loop, without `mut`")
                    .help(format!("declare it as `mut {ty} {name}`, or move the declaration into the loop"))
            } else {
                Diagnostic::at(&codes::E0205, at, format!("cannot change `{name}`: it is not `mut`"))
                    .note("assigned a second time here")
                    .label(body.locals[l].span, "declared here without `mut`")
                    .help(format!("declare it as `mut {ty} {name}` to allow changes"))
            };
            out.push(d);
        }
    });
}
