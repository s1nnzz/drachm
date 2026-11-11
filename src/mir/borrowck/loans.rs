//! Loans in scope (spec 3.5.5).
//!
//! A loan is in scope at a point if the point can be reached from the loan's
//! creation through points that all lie inside the loan's region, without the
//! borrow being replaced or its owner dying on the way. That is the forward
//! "borrows in scope" dataflow with the out-of-region kills precomputed, done
//! as one depth-first walk per loan.

use super::constraints::{Generated, Loan, Points};
use super::solve::RegionValue;
use crate::mir::dataflow::BitSet;
use crate::mir::index::IndexVec;
use crate::mir::liveness::{rvalue_locals, terminator_uses};
use crate::mir::traversal::successors;
use crate::mir::*;

/// Where each loan is in scope.
#[derive(Debug)]
pub struct LoanScopes {
    /// Per loan: the points at which it is in scope (just before they run).
    pub in_scope: Vec<BitSet>,
    /// Per loan: points where a two-phase loan is still only reserved.
    pub reserved: Vec<BitSet>,
    /// Per two-phase loan: where it becomes a real `&mut`.
    pub activation: Vec<Option<Location>>,
    /// Per point: the loans in scope there.
    pub by_point: Vec<Vec<usize>>,
}

/// The points that follow `loc` directly.
pub fn next_points(body: &Body, loc: Location) -> Vec<Location> {
    let block = &body.blocks[loc.block];
    if (loc.index as usize) < block.statements.len() {
        vec![Location { block: loc.block, index: loc.index + 1 }]
    } else {
        successors(&block.terminator).into_iter().map(|b| Location { block: b, index: 0 }).collect()
    }
}

/// Does assigning to `p` replace the borrow that `place` goes through?
/// (`place` is `p` followed by a `Deref` and more.)
pub fn overwrites_base(p: &Place, place: &Place) -> bool {
    p.local == place.local && p.proj.len() < place.proj.len() && place.proj[..p.proj.len()] == p.proj[..] && place.proj[p.proj.len()] == Proj::Deref
}

/// Does the statement or terminator at `loc` end `loan` (after its own checks)?
fn kills(body: &Body, loc: Location, loan: &Loan) -> bool {
    let block = &body.blocks[loc.block];
    let i = loc.index as usize;
    if i < block.statements.len() {
        match &block.statements[i].kind {
            StatementKind::Assign(p, _) => overwrites_base(p, &loan.place),
            StatementKind::StorageDead(l) => *l == loan.place.local,
            _ => false,
        }
    } else {
        matches!(&block.terminator.kind, TerminatorKind::Call { dest: Some(d), .. } if overwrites_base(d, &loan.place))
    }
}

/// Does the point at `loc` use local `l`?
fn uses_local(body: &Body, loc: Location, l: Local) -> bool {
    let block = &body.blocks[loc.block];
    let i = loc.index as usize;
    let mut hit = false;
    if i < block.statements.len() {
        match &block.statements[i].kind {
            StatementKind::Assign(d, rv) => {
                hit |= d.local == l && !d.proj.is_empty();
                rvalue_locals(rv, &mut |x| hit |= x == l);
            }
            StatementKind::FakeRead(p) => hit |= p.local == l,
            _ => {}
        }
    } else {
        terminator_uses(&block.terminator.kind, &mut |x| hit |= x == l);
    }
    hit
}

fn walk(body: &Body, points: &Points, loan: &Loan, region: &RegionValue, stop_at: Option<Location>) -> BitSet {
    let mut scope = BitSet::new(points.len());
    let mut stack = next_points(body, loan.loc);
    while let Some(q) = stack.pop() {
        let qi = points.index(q);
        if scope.contains(qi) || !region.points.contains(qi) || Some(q) == stop_at {
            continue;
        }
        scope.insert(qi);
        if !kills(body, q, loan) {
            stack.extend(next_points(body, q));
        }
    }
    scope
}

pub fn compute(body: &Body, cg: &Generated, values: &IndexVec<super::solve::RegionVid, RegionValue>) -> LoanScopes {
    let np = cg.points.len();
    let mut in_scope = Vec::new();
    let mut reserved = Vec::new();
    let mut activation = Vec::new();
    for loan in &cg.loans {
        let region = &values[loan.region];
        let scope = walk(body, &cg.points, loan, region, None);
        let two_phase = matches!(loan.kind, BorrowKind::Mut { two_phase: true });
        let act = if two_phase && loan.dest.proj.is_empty() { find_activation(body, loan) } else { None };
        // Reserved from creation until the first use of the temp.
        let res = if two_phase { walk(body, &cg.points, loan, region, act) } else { BitSet::new(np) };
        in_scope.push(scope);
        reserved.push(res);
        activation.push(act);
    }
    let mut by_point = vec![Vec::new(); np];
    for (l, s) in in_scope.iter().enumerate() {
        for p in s.iter() {
            by_point[p].push(l);
        }
    }
    LoanScopes { in_scope, reserved, activation, by_point }
}

/// The first point after the creation of a two-phase loan that uses the temp
/// holding it (always the call that takes the receiver).
fn find_activation(body: &Body, loan: &Loan) -> Option<Location> {
    let mut seen = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<Location> = next_points(body, loan.loc).into();
    while let Some(q) = queue.pop_front() {
        if !seen.insert(q) {
            continue;
        }
        if uses_local(body, q, loan.dest.local) {
            return Some(q);
        }
        queue.extend(next_points(body, q));
    }
    None
}
