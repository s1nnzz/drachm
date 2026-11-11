//! Region values and the solver (spec 3.5.4).
//!
//! A constraint `outlives(sup, sub)` says `sup` contains everything `sub`
//! contains. Values flow along the constraints until nothing changes. The
//! graph is small (tens of regions) and the values are bitsets over points, so
//! a worklist that unions along edges is all it takes; cycles settle on their
//! own.

use crate::mir::dataflow::BitSet;
use crate::mir::index::{newtype_index, Idx, IndexVec};
use crate::mir::Location;

use super::regions::Universal;

newtype_index!(
    /// A region variable.
    RegionVid
);

/// Why a constraint exists, for the explanations (spec 3.6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    /// C2: the borrow flows into its destination.
    Borrow,
    /// C3: the new borrow cannot outlive the borrow it goes through.
    Reborrow,
    /// C4: an assignment (a copy or move of a value holding borrows).
    Assign,
    /// C5: an argument, or the result of a call.
    Call,
    /// C4 on `_0`: the value is returned.
    Return,
    /// C6, and the seeds of C1: facts about types and liveness.
    Liveness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Origin {
    pub loc: Location,
    pub kind: ConstraintKind,
}

/// `sup` contains everything `sub` contains.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Constraint {
    pub sup: RegionVid,
    pub sub: RegionVid,
    pub origin: Origin,
}

/// Where a region is in force: a set of points, and the universal regions
/// whose end it reaches (`ends`, indexed like `Solution::universals`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionValue {
    pub points: BitSet,
    pub ends: BitSet,
}

/// The result of solving one body.
#[derive(Debug)]
pub struct Solution {
    /// The universal regions: region variable `i` is `universals[i]` for
    /// `i < universals.len()`.
    pub universals: Vec<Universal>,
    pub values: IndexVec<RegionVid, RegionValue>,
    pub constraints: Vec<Constraint>,
}

impl Solution {
    pub fn is_universal(&self, r: RegionVid) -> bool {
        r.index() < self.universals.len()
    }
}

/// Solve: `values` holds the seeds (liveness for local regions, everything for
/// universals); this adds what flows along `constraints`.
pub fn solve(num_universals: usize, values: &mut IndexVec<RegionVid, RegionValue>, constraints: &[Constraint]) {
    let n = values.len();
    // Values flow from `sub` to `sup`.
    let mut flows: Vec<Vec<usize>> = vec![Vec::new(); n];
    for c in constraints {
        if c.sup != c.sub {
            flows[c.sub.index()].push(c.sup.index());
        }
    }
    let _ = num_universals;
    let mut queued = vec![true; n];
    let mut work: std::collections::VecDeque<usize> = (0..n).collect();
    while let Some(v) = work.pop_front() {
        queued[v] = false;
        if flows[v].is_empty() {
            continue;
        }
        let from = values[RegionVid::new(v)].clone();
        for &s in &flows[v] {
            let to = &mut values[RegionVid::new(s)];
            let a = to.points.union(&from.points);
            let b = to.ends.union(&from.ends);
            if (a || b) && !queued[s] {
                queued[s] = true;
                work.push_back(s);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mir::BlockId;

    fn loc() -> Location {
        Location { block: BlockId(0), index: 0 }
    }

    fn value(points: &[usize]) -> RegionValue {
        let mut p = BitSet::new(8);
        points.iter().for_each(|&i| {
            p.insert(i);
        });
        RegionValue { points: p, ends: BitSet::new(2) }
    }

    fn c(sup: u32, sub: u32) -> Constraint {
        Constraint { sup: RegionVid(sup), sub: RegionVid(sub), origin: Origin { loc: loc(), kind: ConstraintKind::Assign } }
    }

    #[test]
    fn values_flow_to_the_containing_region_through_cycles() {
        let mut values = IndexVec::from_vec(vec![value(&[]), value(&[1]), value(&[2]), value(&[3])]);
        // 0 contains 1; 1 and 2 contain each other; 2 contains 3.
        solve(0, &mut values, &[c(0, 1), c(1, 2), c(2, 1), c(2, 3)]);
        let at = |i: u32| values[RegionVid(i)].points.iter().collect::<Vec<_>>();
        assert_eq!(at(0), [1, 2, 3]);
        assert_eq!(at(1), [1, 2, 3]);
        assert_eq!(at(2), [1, 2, 3]);
        assert_eq!(at(3), [3]);
    }
}
