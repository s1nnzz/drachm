//! The borrow checker (spec 3.5): liveness of borrows, region constraints,
//! loans in scope, conflicting accesses, and errors that explain themselves.
//!
//! One body at a time: `analyse_body` numbers the points, generates the
//! constraints (C1 to C7), solves them, finds where every loan is in scope and
//! reports the accesses that conflict with a loan.

pub mod conflicts;
pub mod constraints;
pub mod explain;
pub mod loans;
pub mod regions;
pub mod solve;

use crate::diag::{Diagnostic, Span};
use crate::hir;
use crate::mir::index::Idx;
use crate::mir::liveness::Liveness;
use crate::mir::traversal::reverse_postorder;
use crate::mir::*;

pub struct BodyResult {
    /// Empty when `report` is false.
    pub diags: Vec<Diagnostic>,
    /// The region values after solving, for the explanations WP8 gives.
    pub solution: solve::Solution,
}

/// Everything `analyse_body` works out, kept for the debug dump.
pub struct Analysis {
    pub cg: constraints::Generated,
    pub scopes: loans::LoanScopes,
}

fn analyse(hir: &hir::Program, sigs: &[LivesSig], body: &Body) -> (Analysis, solve::Solution) {
    let live = Liveness::compute(hir, body);
    let mut cg = constraints::generate(hir, sigs, body, &live);
    let mut values = std::mem::take(&mut cg.values);
    solve::solve(cg.universals.len(), &mut values, &cg.constraints);
    let scopes = loans::compute(body, &cg, &values);
    let solution = solve::Solution { universals: cg.universals.clone(), values, constraints: cg.constraints.clone() };
    (Analysis { cg, scopes }, solution)
}

/// Check one body against the given signatures.
pub fn analyse_body(hir: &hir::Program, sigs: &[LivesSig], body: &Body, report: bool) -> BodyResult {
    let (a, solution) = analyse(hir, sigs, body);
    let mut diags = Vec::new();
    if report {
        let found = conflicts::find(hir, body, &a.cg, &a.scopes, );
        let rpo = reverse_postorder(body);
        let mut rank = vec![usize::MAX; body.blocks.len()];
        for (i, b) in rpo.iter().enumerate() {
            rank[b.index()] = i;
        }
        let cx = explain::Cx {
            hir,
            body,
            cg: &a.cg,
            values: &solution.values,
            constraints: &solution.constraints,
            scopes: &a.scopes,
            rank: &rank,
        };
        diags = found.iter().map(|c| explain::diagnostic(&cx, c)).collect();
        let key = |d: &Diagnostic| (d.labels.first().map_or(0, |l| l.0.start), d.code.id);
        diags.sort_by(|x, y| key(x).cmp(&key(y)));
    }
    BodyResult { diags, solution }
}

/// WP7's driver: every body in `FuncId` order, with `sigs::provisional`.
/// The compiler's own functions (span 0, 0) are not checked.
pub fn check_program(hir: &hir::Program, mir: &Program, diags: &mut Vec<Diagnostic>) {
    for body in mir.bodies.iter().flatten() {
        if hir.funcs[body.func].span == Span::new(0, 0) {
            continue;
        }
        diags.extend(analyse_body(hir, &mir.lives, body, true).diags);
    }
}

/// The text of `check --dump-borrowck`: per function the points, the regions
/// with their values, and the loans with the points where they are in scope.
pub fn dump(hir: &hir::Program, mir: &Program) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for body in mir.bodies.iter().flatten() {
        if hir.funcs[body.func].span == Span::new(0, 0) {
            continue;
        }
        let (a, sol) = analyse(hir, &mir.lives, body);
        let _ = writeln!(out, "fn {}", body.name);
        for p in 0..a.cg.points.len() {
            let loc = a.cg.points.loc(p);
            let _ = writeln!(out, "  P{p}  {}", crate::mir::print::point_text(mir, hir, body, loc));
        }
        let nu = sol.universals.len();
        for (i, v) in sol.values.iter().enumerate() {
            let pts: Vec<String> = v.points.iter().map(|p| format!("P{p}")).collect();
            let ends: Vec<String> = v.ends.iter().map(|u| format!("end({})", sol.universals[u].name)).collect();
            let name = if i < nu { format!(" ({})", sol.universals[i].name) } else { String::new() };
            let inside = if i < nu { "all points".to_string() } else { pts.join(",") };
            let tail = if ends.is_empty() { String::new() } else { format!(" + {}", ends.join(",")) };
            let _ = writeln!(out, "  region R{i}{name}: {{{inside}}}{tail}");
        }
        for (i, loan) in a.cg.loans.iter().enumerate() {
            let scope: Vec<String> = a.scopes.in_scope[i].iter().map(|p| format!("P{p}")).collect();
            let kind = match loan.kind {
                BorrowKind::Shared => "&",
                BorrowKind::Mut { two_phase: false } => "&mut ",
                BorrowKind::Mut { two_phase: true } => "&mut two_phase ",
            };
            let place = crate::mir::print::place_text(&loan.place);
            let act = match a.scopes.activation[i] {
                Some(l) => format!(" activated at P{}", a.cg.points.index(l)),
                None => String::new(),
            };
            let _ = writeln!(out, "  loan L{i}: {kind}{place} at P{} in {{{}}}{act} region R{}", a.cg.points.index(loan.loc), scope.join(","), loan.region.index());
        }
    }
    out
}

#[cfg(test)]
mod tests;
