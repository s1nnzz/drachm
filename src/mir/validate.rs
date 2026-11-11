//! MIR invariants (spec 2.3). Checked: V1, V2, V4, V5, V6, V7 (use before
//! assignment of the builder's temps), plus reducibility and "every block is
//! reachable" (they stand in for V9's cleanup). V3 waits for WP2; V8 and V9
//! have nothing to check before cleanup and elaboration exist.

use super::traversal::{predecessors, reverse_postorder, successors};
use super::ty::{local_ty, operand_ty, place_ty, rvalue_ty, Ty};
use super::*;

pub fn validate(p: &Program, hir: &hir::Program) -> Result<(), Vec<String>> {
    let mut errors = Vec::new();
    if p.bodies.len() != hir.funcs.len() {
        errors.push(format!("{} bodies for {} functions", p.bodies.len(), hir.funcs.len()));
    }
    for body in p.bodies.iter().flatten() {
        let mut v = Validator { hir, body, errors: Vec::new() };
        v.run();
        errors.extend(v.errors.into_iter().map(|e| format!("fn {}: {e}", body.name)));
    }
    if errors.is_empty() { Ok(()) } else { Err(errors) }
}

struct Validator<'a> {
    hir: &'a hir::Program,
    body: &'a Body,
    errors: Vec<String>,
}

impl Validator<'_> {
    fn err(&mut self, at: impl std::fmt::Display, msg: impl std::fmt::Display) {
        self.errors.push(format!("{at}: {msg}"));
    }

    fn run(&mut self) {
        let body = self.body;
        // V1
        if body.blocks.is_empty() {
            self.err("body", "no blocks");
            return;
        }
        if body.locals.is_empty() {
            self.err("body", "no return place _0");
            return;
        }
        if !matches!(body.locals[Local(0)].kind, LocalKind::Return) {
            self.err("_0", "is not the return place");
        }
        if body.arg_count + 1 > body.locals.len() {
            self.err("body", "more parameters than locals");
            return;
        }
        let mut targets_ok = true;
        for (b, block) in body.blocks.iter_enumerated() {
            for s in successors(&block.terminator) {
                if s.index() >= body.blocks.len() {
                    self.err(b, format!("jumps to {s}, which does not exist"));
                    targets_ok = false;
                }
            }
        }
        if !targets_ok {
            return;
        }
        if let Some(f) = body.fallthrough_end {
            if f.index() >= body.blocks.len() {
                self.err("body", format!("fallthrough_end {f} does not exist"));
            }
        }
        for (b, block) in body.blocks.iter_enumerated() {
            for (i, s) in block.statements.iter().enumerate() {
                self.statement(Location { block: b, index: i as u32 }, &s.kind);
            }
            self.terminator(b, &block.terminator.kind);
        }
        // V6
        for (l, d) in body.locals.iter_enumerated() {
            if matches!(d.kind, LocalKind::DropFlag) && (d.ty != Type::Bool || d.borrow.is_some()) {
                self.err(l, "a drop flag must be bool");
            }
        }
        self.graph();
        self.definite_assignment();
    }

    fn at(loc: Location) -> String {
        format!("{} statement {}", loc.block, loc.index)
    }

    fn place(&mut self, at: &str, pl: &Place) -> Option<Ty> {
        match place_ty(self.body, self.hir, pl) {
            Ok(t) => Some(t),
            Err(e) => {
                self.err(at, format!("ill-typed place: {e}"));
                None
            }
        }
    }

    fn operand(&mut self, at: &str, op: &Operand) -> Option<Ty> {
        match op {
            Operand::Copy(pl) | Operand::Move(pl) => self.place(at, pl),
            Operand::Const(_) => operand_ty(self.body, self.hir, op).ok(),
        }
    }

    // V2 and V4 for one statement.
    fn statement(&mut self, loc: Location, k: &StatementKind) {
        let at = Self::at(loc);
        match k {
            StatementKind::Assign(dest, rv) => {
                let dt = self.place(&at, dest);
                let mut operands: Vec<&Operand> = Vec::new();
                match rv {
                    Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) => operands.push(o),
                    Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => operands.extend([a, b]),
                    Rvalue::Shift { value, amount, .. } => operands.extend([value, amount]),
                    Rvalue::Aggregate(_, ops) => operands.extend(ops),
                    Rvalue::Ref(_, pl) | Rvalue::RawRef(pl) | Rvalue::DeepCopy(pl) => {
                        self.place(&at, pl);
                    }
                }
                let tys: Vec<Option<Ty>> = operands.iter().map(|o| self.operand(&at, o)).collect();
                match rv {
                    Rvalue::Binary(_, ..) => {
                        if tys[0].is_some() && tys[0] != tys[1] {
                            self.err(&at, "the operands of a binary operation differ in type");
                        }
                    }
                    Rvalue::Aggregate(sid, ops) => {
                        let fields = &self.hir.structs[*sid].fields;
                        if fields.len() != ops.len() {
                            self.err(&at, format!("Aggregate of {} gives {} fields, expected {}", self.hir.structs[*sid].name, ops.len(), fields.len()));
                        } else {
                            for (i, (f, t)) in fields.iter().zip(&tys).enumerate() {
                                if t.is_some() && *t != Some(Ty::plain(f.ty)) {
                                    self.err(&at, format!("field {i} of Aggregate has the wrong type"));
                                }
                            }
                        }
                    }
                    Rvalue::Neg(_) | Rvalue::Not(_) | Rvalue::BitNot(_) | Rvalue::Convert(..) | Rvalue::Use(_) | Rvalue::Shift { .. } | Rvalue::IntPow(..) => {}
                    Rvalue::Ref(..) | Rvalue::RawRef(_) | Rvalue::DeepCopy(_) => {}
                }
                match (dt, rvalue_ty(self.body, self.hir, rv)) {
                    (Some(d), Ok(r)) if d != r => self.err(&at, format!("assigns a `{r}` to a `{d}`")),
                    (_, Err(e)) => self.err(&at, format!("ill-typed rvalue: {e}")),
                    _ => {}
                }
            }
            StatementKind::StorageLive(l) | StatementKind::StorageDead(l) => {
                if local_ty(self.body, *l).is_err() {
                    self.err(&at, format!("{l} does not exist"));
                }
            }
            StatementKind::Drop { place, flag, .. } => {
                self.place(&at, place);
                if let Some(f) = flag {
                    if local_ty(self.body, *f).ok() != Some(Ty::plain(Type::Bool)) {
                        self.err(&at, "a drop flag must be a bool local");
                    }
                }
            }
            StatementKind::FakeRead(pl) => {
                self.place(&at, pl);
            }
            StatementKind::Nop => {}
        }
    }

    fn terminator(&mut self, b: BlockId, k: &TerminatorKind) {
        let at = format!("{b} terminator");
        match k {
            TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => {}
            TerminatorKind::If { cond, .. } => {
                if let Some(t) = self.operand(&at, cond) {
                    if t != Ty::plain(Type::Bool) {
                        self.err(&at, format!("If on a `{t}`, not a bool"));
                    }
                }
            }
            TerminatorKind::Switch { discr, .. } => {
                self.operand(&at, discr);
            }
            TerminatorKind::Call { callee, args, dest, .. } => {
                let tys: Vec<Option<Ty>> = args.iter().map(|a| self.operand(&at, &a.op)).collect();
                let dt = dest.as_ref().and_then(|d| self.place(&at, d));
                let (want_args, want_ret) = self.signature(*callee);
                match want_args {
                    Some(want) if want.len() != tys.len() => {
                        self.err(&at, format!("{callee:?} takes {} arguments, given {}", want.len(), tys.len()));
                    }
                    Some(want) => {
                        for (i, (w, t)) in want.iter().zip(&tys).enumerate() {
                            if let Some(t) = t {
                                if t != w {
                                    self.err(&at, format!("argument {i} of {callee:?} is a `{t}`, expected `{w}`"));
                                }
                            }
                        }
                    }
                    None => {}
                }
                match (dest, dt) {
                    (Some(_), Some(d)) if d != Ty::plain(want_ret) => {
                        self.err(&at, format!("the result of {callee:?} is a `{want_ret}`, stored in a `{d}`"));
                    }
                    (None, _) if want_ret != Type::None => {
                        self.err(&at, format!("the result of {callee:?} (a `{want_ret}`) is dropped"));
                    }
                    _ => {}
                }
            }
        }
    }

    /// Argument types (None: not checked, because a reference's referent is
    /// all the phase A `Ty` can tell apart) and result type of a callee.
    fn signature(&self, callee: Callee) -> (Option<Vec<Ty>>, Type) {
        let refd = |t: Type, mutable: bool| Ty { ty: t, borrow: Some(mutable) };
        match callee {
            Callee::Func(f) => {
                let func = &self.hir.funcs[f];
                let args = (0..func.params)
                    .map(|i| {
                        let t = func.locals[i].ty;
                        // The receiver is a reference (mutable until WP3).
                        if i == 0 && func.has_self { refd(t, true) } else { Ty::plain(t) }
                    })
                    .collect();
                (Some(args), func.ret)
            }
            Callee::Str(f) => {
                let args = f.params().iter().enumerate().map(|(i, &t)| {
                    if t == Type::Str { refd(t, f == hir::StrFn::Append && i == 0) } else { Ty::plain(t) }
                });
                (Some(args.collect()), f.ret())
            }
            Callee::Out { .. } => (Some(vec![refd(Type::Str, false)]), Type::None),
            Callee::Text | Callee::Radix { .. } | Callee::Join => (None, Type::Str),
        }
    }

    /// Reachability, reducibility.
    fn graph(&mut self) {
        let body = self.body;
        let rpo = reverse_postorder(body);
        let mut pos = vec![usize::MAX; body.blocks.len()];
        for (i, b) in rpo.iter().enumerate() {
            pos[b.index()] = i;
        }
        for b in body.blocks.indices() {
            if pos[b.index()] == usize::MAX {
                self.err(b, "unreachable from bb0 and not removed");
            }
        }
        #[cfg(debug_assertions)]
        {
            let idom = dominators(body, &rpo, &pos);
            for &u in &rpo {
                for v in successors(&body.blocks[u].terminator) {
                    // In reverse post-order the retreating edges are exactly
                    // those that do not go forward.
                    if pos[v.index()] <= pos[u.index()] && !dominates(&idom, v, u) {
                        self.err(u, format!("the edge to {v} is retreating, but {v} does not dominate {u}: the CFG is not reducible"));
                    }
                }
            }
        }
    }

    /// V5 and V7: forward "definitely assigned" analysis over locals.
    fn definite_assignment(&mut self) {
        let body = self.body;
        let n = body.locals.len();
        let first_temp = self.hir.funcs[body.func].locals.len() + 1;
        let rpo = reverse_postorder(body);
        let preds = predecessors(body);
        let nb = body.blocks.len();

        let mut entry = vec![false; n];
        for i in 1..=body.arg_count {
            entry[i] = true;
        }
        let mut input: Vec<Vec<bool>> = vec![vec![true; n]; nb];
        input[0] = entry;
        let mut output: Vec<Vec<bool>> = vec![vec![true; n]; nb];
        let mut changed = true;
        while changed {
            changed = false;
            for &b in &rpo {
                if b.index() != 0 {
                    let mut st = vec![true; n];
                    for pr in &preds[b] {
                        if rpo.contains(pr) {
                            for (s, o) in st.iter_mut().zip(&output[pr.index()]) {
                                *s &= *o;
                            }
                        }
                    }
                    input[b.index()] = st;
                }
                let mut st = input[b.index()].clone();
                for s in &body.blocks[b].statements {
                    flow_statement(&mut st, &s.kind, &mut |_| {});
                }
                flow_terminator(&mut st, &body.blocks[b].terminator.kind, &mut |_| {});
                if st != output[b.index()] {
                    output[b.index()] = st;
                    changed = true;
                }
            }
        }

        let mut problems = Vec::new();
        for &b in &rpo {
            let mut st = input[b.index()].clone();
            let block = &body.blocks[b];
            for (i, s) in block.statements.iter().enumerate() {
                let mut used = Vec::new();
                let before = st.clone();
                flow_statement(&mut st, &s.kind, &mut |l| used.push(l));
                for l in used {
                    if l.index() >= first_temp && !before[l.index()] {
                        problems.push(format!("{} statement {i}: temp {l} may be used before it is assigned", b));
                    }
                }
            }
            let mut used = Vec::new();
            let before = st.clone();
            flow_terminator(&mut st, &block.terminator.kind, &mut |l| used.push(l));
            for l in used {
                if l.index() >= first_temp && !before[l.index()] {
                    problems.push(format!("{b} terminator: temp {l} may be used before it is assigned"));
                }
            }
            // V5
            if matches!(block.terminator.kind, TerminatorKind::Return)
                && body.locals[Local(0)].ty != Type::None
                && !st[0]
            {
                problems.push(format!("{b}: returns without assigning _0 on every path"));
            }
        }
        self.errors.extend(problems);
    }
}

/// The locals a place reads: its base (a projection through a reference
/// reads the reference).
fn place_use(pl: &Place, used: &mut dyn FnMut(Local)) {
    used(pl.local);
}

fn operand_use(op: &Operand, used: &mut dyn FnMut(Local)) {
    if let Operand::Copy(pl) | Operand::Move(pl) = op {
        place_use(pl, used);
    }
}

/// Update `assigned` for a statement, reporting each local it reads.
fn flow_statement(assigned: &mut [bool], k: &StatementKind, used: &mut dyn FnMut(Local)) {
    match k {
        StatementKind::Assign(dest, rv) => {
            match rv {
                Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) => operand_use(o, used),
                Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => {
                    operand_use(a, used);
                    operand_use(b, used);
                }
                Rvalue::Shift { value, amount, .. } => {
                    operand_use(value, used);
                    operand_use(amount, used);
                }
                Rvalue::Aggregate(_, ops) => ops.iter().for_each(|o| operand_use(o, used)),
                Rvalue::Ref(_, pl) | Rvalue::RawRef(pl) | Rvalue::DeepCopy(pl) => place_use(pl, used),
            }
            if dest.proj.is_empty() {
                assigned[dest.local.index()] = true;
            } else {
                used(dest.local);
            }
        }
        StatementKind::Drop { place, flag, .. } => {
            place_use(place, used);
            if let Some(f) = flag {
                used(*f);
            }
        }
        StatementKind::FakeRead(pl) => place_use(pl, used),
        StatementKind::StorageLive(_) | StatementKind::StorageDead(_) | StatementKind::Nop => {}
    }
}

fn flow_terminator(assigned: &mut [bool], k: &TerminatorKind, used: &mut dyn FnMut(Local)) {
    match k {
        TerminatorKind::If { cond, .. } => operand_use(cond, used),
        TerminatorKind::Switch { discr, .. } => operand_use(discr, used),
        TerminatorKind::Call { args, dest, .. } => {
            for a in args {
                operand_use(&a.op, used);
            }
            if let Some(d) = dest {
                if d.proj.is_empty() {
                    assigned[d.local.index()] = true;
                } else {
                    used(d.local);
                }
            }
        }
        TerminatorKind::Goto(_) | TerminatorKind::Return | TerminatorKind::Panic(_) | TerminatorKind::Unreachable => {}
    }
}

/// Immediate dominators of the reachable blocks (Cooper, Harvey, Kennedy).
#[cfg(debug_assertions)]
fn dominators(body: &Body, rpo: &[BlockId], pos: &[usize]) -> Vec<Option<BlockId>> {
    let preds = predecessors(body);
    let mut idom: Vec<Option<BlockId>> = vec![None; body.blocks.len()];
    idom[0] = Some(BlockId(0));
    let mut changed = true;
    while changed {
        changed = false;
        for &b in rpo.iter().skip(1) {
            let mut new: Option<BlockId> = None;
            for &p in &preds[b] {
                if pos[p.index()] == usize::MAX || idom[p.index()].is_none() {
                    continue;
                }
                new = Some(match new {
                    None => p,
                    Some(cur) => intersect(&idom, pos, cur, p),
                });
            }
            if new != idom[b.index()] {
                idom[b.index()] = new;
                changed = true;
            }
        }
    }
    idom
}

#[cfg(debug_assertions)]
fn intersect(idom: &[Option<BlockId>], pos: &[usize], mut a: BlockId, mut b: BlockId) -> BlockId {
    while a != b {
        while pos[a.index()] > pos[b.index()] {
            a = idom[a.index()].unwrap();
        }
        while pos[b.index()] > pos[a.index()] {
            b = idom[b.index()].unwrap();
        }
    }
    a
}

/// Does `a` dominate `b`?
#[cfg(debug_assertions)]
fn dominates(idom: &[Option<BlockId>], a: BlockId, mut b: BlockId) -> bool {
    loop {
        if a == b {
            return true;
        }
        match idom[b.index()] {
            Some(next) if next != b => b = next,
            _ => return false,
        }
    }
}
