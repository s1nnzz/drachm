//! Region variables, loans and constraint generation (spec 3.5.1 to 3.5.3,
//! rules C1 to C7).

use std::ops::Range;

use super::regions::{param_slots, region_slots, slot_variance, slots_of_place, struct_variance, universals, Universal, Variance};
use super::solve::{Constraint, ConstraintKind, Origin, RegionValue, RegionVid};
use crate::diag::Span;
use crate::hir;
use crate::mir::dataflow::BitSet;
use crate::mir::index::{Idx, IndexVec};
use crate::mir::liveness::Liveness;
use crate::mir::ty::{operand_ty, place_ty};
use crate::mir::*;

/// A dense numbering of the points of a body: one per statement, one per terminator.
#[derive(Debug)]
pub struct Points {
    block_start: IndexVec<BlockId, usize>,
    locs: Vec<Location>,
}

impl Points {
    pub fn new(body: &Body) -> Points {
        let mut block_start = IndexVec::new();
        let mut locs = Vec::new();
        for (b, block) in body.blocks.iter_enumerated() {
            block_start.push(locs.len());
            for i in 0..=block.statements.len() {
                locs.push(Location { block: b, index: i as u32 });
            }
        }
        Points { block_start, locs }
    }

    pub fn len(&self) -> usize {
        self.locs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.locs.is_empty()
    }

    pub fn index(&self, loc: Location) -> usize {
        self.block_start[loc.block] + loc.index as usize
    }

    pub fn loc(&self, i: usize) -> Location {
        self.locs[i]
    }
}

/// One `Rvalue::Ref` (including the reborrow of a `lend` and a method receiver).
#[derive(Debug, Clone)]
pub struct Loan {
    pub place: Place,
    pub kind: BorrowKind,
    pub region: RegionVid,
    pub loc: Location,
    pub span: Span,
    /// The place the borrow is stored into.
    pub dest: Place,
}

/// Everything generated for one body, before solving.
#[derive(Debug)]
pub struct Generated {
    pub points: Points,
    pub universals: Vec<Universal>,
    /// Region variable of slot 0 of each local.
    pub local_start: IndexVec<Local, usize>,
    pub loans: Vec<Loan>,
    pub constraints: Vec<Constraint>,
    pub values: IndexVec<RegionVid, RegionValue>,
}

struct Gen<'a> {
    hir: &'a hir::Program,
    sigs: &'a [LivesSig],
    body: &'a Body,
    out: Generated,
    num_regions: usize,
}

impl Gen<'_> {
    fn fresh(&mut self) -> usize {
        self.num_regions += 1;
        self.num_regions - 1
    }

    fn push(&mut self, sup: usize, sub: usize, loc: Location, kind: ConstraintKind) {
        self.out.constraints.push(Constraint { sup: RegionVid::new(sup), sub: RegionVid::new(sub), origin: Origin { loc, kind } });
    }

    /// The region variables of the slots of `place`.
    fn place_vids(&self, place: &Place) -> Range<usize> {
        let r = slots_of_place(self.hir, self.body, place);
        let base = self.out.local_start[place.local];
        (base + r.start)..(base + r.end)
    }

    fn place_ty(&self, place: &Place) -> Type {
        place_ty(self.body, self.hir, place).expect("validated MIR")
    }

    /// `relate(a, b)` for corresponding slot lists: `a`'s regions must outlive `b`'s
    /// (and the other way round where `ty` makes the slot invariant).
    fn relate(&mut self, a: Range<usize>, ty: Type, b: Range<usize>, loc: Location, kind: ConstraintKind) {
        for (k, (x, y)) in a.zip(b).enumerate() {
            self.push(x, y, loc, kind);
            if slot_variance(self.hir, ty, k) == Variance::Invariant {
                self.push(y, x, loc, kind);
            }
        }
    }

    fn operand_vids(&self, op: &Operand) -> Option<(Range<usize>, Type)> {
        match op {
            Operand::Copy(p) | Operand::Move(p) => Some((self.place_vids(p), self.place_ty(p))),
            Operand::Const(_) => None,
        }
    }

    fn statement(&mut self, loc: Location, s: &Statement) {
        let StatementKind::Assign(dest, rv) = &s.kind else { return };
        let kind = if dest.local == Local(0) && dest.proj.is_empty() { ConstraintKind::Return } else { ConstraintKind::Assign };
        let dv = self.place_vids(dest);
        match rv {
            Rvalue::Use(op) => {
                if let Some((sv, ty)) = self.operand_vids(op) {
                    self.relate(sv, ty, dv, loc, kind);
                }
            }
            Rvalue::DeepCopy(q) => {
                let (sv, ty) = (self.place_vids(q), self.place_ty(q));
                self.relate(sv, ty, dv, loc, kind);
            }
            Rvalue::Ref(bk, place) => self.borrow(loc, s.span, dest, *bk, place),
            Rvalue::Aggregate(sid, ops) => {
                let def = &self.hir.structs[*sid];
                let variance = struct_variance(self.hir, *sid);
                for (fi, op) in ops.iter().enumerate() {
                    let Some(k) = def.lives.iter().position(|&x| x == fi) else { continue };
                    let Some((sv, _)) = self.operand_vids(op) else { continue };
                    let (Some(x), Some(y)) = (sv.clone().next(), dv.clone().nth(k)) else { continue };
                    self.push(x, y, loc, kind);
                    if variance[k] == Variance::Invariant {
                        self.push(y, x, loc, kind);
                    }
                }
            }
            Rvalue::RawRef(_) | Rvalue::Neg(_) | Rvalue::Not(_) | Rvalue::BitNot(_) | Rvalue::Binary(..) | Rvalue::Shift { .. } | Rvalue::IntPow(..) | Rvalue::Convert(..) => {}
        }
    }

    /// C2 and C3.
    fn borrow(&mut self, loc: Location, span: Span, dest: &Place, bk: BorrowKind, place: &Place) {
        let region = self.fresh();
        self.out.loans.push(Loan { place: place.clone(), kind: bk, region: RegionVid::new(region), loc, span, dest: dest.clone() });
        let dv = self.place_vids(dest);
        if let Some(slot0) = dv.clone().next() {
            // The loan lasts as long as anything the borrow flows into.
            self.push(region, slot0, loc, ConstraintKind::Borrow);
            let ty = self.place_ty(place);
            let inner = self.place_vids(place);
            let rest = (slot0 + 1)..dv.end;
            let mutable = matches!(bk, BorrowKind::Mut { .. });
            for (k, (x, y)) in inner.zip(rest).enumerate() {
                self.push(x, y, loc, ConstraintKind::Borrow);
                if mutable || slot_variance(self.hir, ty, k) == Variance::Invariant {
                    self.push(y, x, loc, ConstraintKind::Borrow);
                }
            }
        }
        // C3: walk the supporting prefixes from the end towards the local.
        for i in (0..place.proj.len()).rev() {
            if place.proj[i] != Proj::Deref {
                continue;
            }
            let prefix = Place { local: place.local, proj: place.proj[..i].to_vec() };
            let pty = self.place_ty(&prefix);
            match pty {
                Type::Ref { mutable, .. } => {
                    if let Some(r) = self.place_vids(&prefix).next() {
                        self.push(r, region, loc, ConstraintKind::Reborrow);
                    }
                    if !mutable {
                        break;
                    }
                }
                _ => break,
            }
        }
    }

    /// C5.
    fn call(&mut self, loc: Location, callee: &Callee, args: &[Arg], dest: &Option<Place>) {
        let Callee::Func(f) = *callee else { return };
        let func = &self.hir.funcs[f];
        // Fresh variables for every universal of the callee's signature.
        let mut params: Vec<Range<usize>> = Vec::new();
        for i in 0..func.params {
            let n = param_slots(self.hir, f, i);
            let start = self.num_regions;
            for _ in 0..n {
                self.fresh();
            }
            params.push(start..start + n);
        }
        let nret = region_slots(self.hir, func.ret);
        let ret_start = self.num_regions;
        for _ in 0..nret {
            self.fresh();
        }
        let ret = ret_start..ret_start + nret;
        for (i, a) in args.iter().enumerate() {
            if let (Some((sv, ty)), Some(p)) = (self.operand_vids(&a.op), params.get(i)) {
                self.relate(sv, ty, p.clone(), loc, ConstraintKind::Call);
            }
        }
        let listed: &[usize] = match &self.sigs[f] {
            LivesSig::Declared(v) | LivesSig::Inferred(v) => v,
            LivesSig::NoBorrowResult => &[],
        };
        let listed = listed.to_vec();
        for x in listed {
            let Some(p) = params.get(x) else { continue };
            for s in p.clone() {
                for r in ret.clone() {
                    self.push(s, r, loc, ConstraintKind::Call);
                }
            }
        }
        if let Some(d) = dest {
            let dv = self.place_vids(d);
            let ty = func.ret;
            self.relate(ret, ty, dv, loc, ConstraintKind::Call);
        }
    }
}

/// Generate the regions, loans and constraints of `body` and seed the values.
pub fn generate(hir: &hir::Program, sigs: &[LivesSig], body: &Body, live: &Liveness) -> Generated {
    let points = Points::new(body);
    let univ = universals(hir, body);
    let nu = univ.len();
    let mut local_start: IndexVec<Local, usize> = IndexVec::new();
    // Parameters, then the return value, take the universals in order.
    let mut u = 0;
    let mut param_start = vec![0usize; body.arg_count];
    for (i, start) in param_start.iter_mut().enumerate() {
        *start = u;
        u += region_slots(hir, body.locals[Local(i as u32 + 1)].ty);
    }
    let ret_start = u;
    let mut next = nu;
    for (l, d) in body.locals.iter_enumerated() {
        let n = region_slots(hir, d.ty);
        let start = if l == Local(0) {
            ret_start
        } else if (1..=body.arg_count).contains(&l.index()) {
            param_start[l.index() - 1]
        } else {
            let s = next;
            next += n;
            s
        };
        local_start.push(start);
    }
    let mut g = Gen {
        hir,
        sigs,
        body,
        out: Generated { points, universals: univ, local_start, loans: Vec::new(), constraints: Vec::new(), values: IndexVec::new() },
        num_regions: next,
    };
    for (b, block) in body.blocks.iter_enumerated() {
        for (i, s) in block.statements.iter().enumerate() {
            g.statement(Location { block: b, index: i as u32 }, s);
        }
        let loc = Location { block: b, index: block.statements.len() as u32 };
        if let TerminatorKind::Call { callee, args, dest, .. } = &block.terminator.kind {
            g.call(loc, callee, args, dest);
        }
    }
    // C6: a borrow cannot outlive what its target borrows.
    let start = Location { block: BlockId(0), index: 0 };
    for (l, d) in body.locals.iter_enumerated() {
        if let Type::Ref { to, .. } = d.ty {
            let inner = region_slots(hir, *to);
            let base = g.out.local_start[l];
            for k in 0..inner {
                g.push(base + 1 + k, base, start, ConstraintKind::Liveness);
            }
        }
    }
    let nreg = g.num_regions;
    let mut out = g.out;
    // Seeds: universals hold every point and their own end; locals hold the
    // points where they are live (C1).
    let np = out.points.len();
    let mut values: IndexVec<RegionVid, RegionValue> =
        IndexVec::from_vec((0..nreg).map(|_| RegionValue { points: BitSet::new(np), ends: BitSet::new(nu) }).collect());
    for i in 0..nu {
        let v = &mut values[RegionVid::new(i)];
        for p in 0..np {
            v.points.insert(p);
        }
        v.ends.insert(i);
    }
    live.for_each_live_before(|loc, live_set| {
        let p = out.points.index(loc);
        for l in live_set.iter() {
            let l = Local(l as u32);
            let base = out.local_start[l];
            for k in 0..region_slots(hir, body.locals[l].ty) {
                values[RegionVid::new(base + k)].points.insert(p);
            }
        }
    });
    out.values = values;
    out
}

/// The type of an operand, for callers outside this module.
pub fn operand_type(body: &Body, hir: &hir::Program, op: &Operand) -> Type {
    operand_ty(body, hir, op).expect("validated MIR")
}
