//! MIR invariants (spec 2.3). Checked: V1, V2, V4, V5, V6, V7 (use before
//! assignment of the builder's temps), plus reducibility and "every block is
//! reachable" (they stand in for V9's cleanup). V3 waits for WP2; V8 and V9
//! have nothing to check before cleanup and elaboration exist.

use super::traversal::{predecessors, reverse_postorder, successors};
use super::ty::{local_ty, operand_ty, place_ty, rvalue_ty};
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
            if matches!(d.kind, LocalKind::DropFlag) && d.ty != Type::Bool {
                self.err(l, "a drop flag must be bool");
            }
        }
        self.graph();
        self.definite_assignment();
    }

    fn at(loc: Location) -> String {
        format!("{} statement {}", loc.block, loc.index)
    }

    fn place(&mut self, at: &str, pl: &Place) -> Option<Type> {
        match place_ty(self.body, self.hir, pl) {
            Ok(t) => Some(t),
            Err(e) => {
                self.err(at, format!("ill-typed place: {e}"));
                None
            }
        }
    }

    fn operand(&mut self, at: &str, op: &Operand) -> Option<Type> {
        match op {
            Operand::Copy(pl) | Operand::Move(pl) => {
                let t = self.place(at, pl)?;
                // A `&` is Copy and a `&mut` is move-only; the builder picks the operand by `is_copy`.
                match (op, t) {
                    (Operand::Copy(_), Type::Ref { mutable: true, .. }) => {
                        self.err(at, format!("Copy of a `{t}`: a `&mut` borrow is moved, never copied"));
                    }
                    (Operand::Move(_), Type::Ref { mutable: false, .. }) => {
                        self.err(at, format!("Move of a `{t}`: a `&` borrow is copied, never moved"));
                    }
                    _ => {}
                }
                Some(t)
            }
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
                    Rvalue::MakeFn(..) => {}
                    Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) | Rvalue::Unsize(o, _) => operands.push(o),
                    Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => operands.extend([a, b]),
                    Rvalue::Shift { value, amount, .. } => operands.extend([value, amount]),
                    Rvalue::Aggregate(_, ops) | Rvalue::Variant(_, _, ops) | Rvalue::Array(_, ops) => operands.extend(ops),
                    Rvalue::ListNew(..) | Rvalue::MapNew(_) => {}
                    Rvalue::Ref(_, pl) | Rvalue::RawRef(pl) | Rvalue::DeepCopy(pl) | Rvalue::Discriminant(pl) | Rvalue::Len(pl) => {
                        let t = self.place(&at, pl);
                        if matches!(rv, Rvalue::Len(_)) && !t.is_some_and(|t| t.is_list() || t.is_array()) {
                            self.err(&at, "Len of something that is not a list or array");
                        }
                    }
                }
                let tys: Vec<Option<Type>> = operands.iter().map(|o| self.operand(&at, o)).collect();
                match rv {
                    Rvalue::MakeFn(id, Type::Func(sig), env) => {
                        let f = &self.hir.funcs[*id];
                        let params = f.locals.iter().take(f.params).map(|p| p.ty);
                        let env_ty = env.as_ref().and_then(|p| self.place(&at, p));
                        let env_param = env_ty.map_or(Type::borrow(false, Type::U8), |ty| Type::borrow(true, ty));
                        let want = std::iter::once(env_param).chain(sig.params.iter().copied());
                        if f.has_self || f.ret != sig.ret || !params.eq(want) {
                            self.err(&at, "MakeFn target does not have the callable signature");
                        }
                    }
                    Rvalue::MakeFn(..) => self.err(&at, "MakeFn result is not a callable type"),
                    Rvalue::Binary(_, ..) => {
                        // A comparison may mix `*T` and `*fixed T` of one pointee type.
                        let same_ptr = matches!((tys[0], tys[1]), (Some(a), Some(b)) if a.is_ptr() && b.is_ptr() && a.target() == b.target());
                        if tys[0].is_some() && tys[0] != tys[1] && !same_ptr {
                            self.err(&at, "the operands of a binary operation differ in type");
                        }
                    }
                    Rvalue::Aggregate(sid, ops) => {
                        let fields = &self.hir.structs[*sid].fields;
                        if fields.len() != ops.len() {
                            self.err(&at, format!("Aggregate of {} gives {} fields, expected {}", self.hir.structs[*sid].name, ops.len(), fields.len()));
                        } else {
                            for (i, (f, t)) in fields.iter().zip(&tys).enumerate() {
                                if t.is_some() && *t != Some(f.ty) {
                                    self.err(&at, format!("field {i} of Aggregate has the wrong type"));
                                }
                            }
                        }
                    }
                    Rvalue::Variant(eid, v, ops) => {
                        let fields = &self.hir.enums[*eid].variants[*v].fields;
                        if fields.len() != ops.len() {
                            self.err(&at, format!("Variant {} gives {} values, expected {}", self.hir.enums[*eid].variants[*v].name, ops.len(), fields.len()));
                        } else {
                            for (i, (f, t)) in fields.iter().zip(&tys).enumerate() {
                                if t.is_some() && *t != Some(f.ty) {
                                    self.err(&at, format!("value {i} of Variant has the wrong type"));
                                }
                            }
                        }
                    }
                    Rvalue::Neg(_) | Rvalue::Not(_) | Rvalue::BitNot(_) | Rvalue::Convert(..) | Rvalue::Unsize(..) | Rvalue::Use(_) | Rvalue::Shift { .. } | Rvalue::IntPow(..) => {}
                    Rvalue::Array(t, ops) => {
                        if let Type::Array { elem, len } = t {
                            if *len as usize != ops.len() {
                                self.err(&at, "an Array rvalue has the wrong number of items");
                            }
                            for ty in tys.iter().flatten() {
                                if ty != &**elem {
                                    self.err(&at, format!("an item of an Array of `{t}` is a `{ty}`"));
                                }
                            }
                        } else {
                            self.err(&at, "Array of a type that is not an array");
                        }
                    }
                    Rvalue::MapNew(t) => {
                        if !t.is_map() {
                            self.err(&at, "MapNew of a type that is not a map");
                        }
                    }
                    Rvalue::ListNew(t, _) => {
                        if !t.is_list() {
                            self.err(&at, "ListNew of a type that is not a list");
                        }
                    }
                    Rvalue::Ref(..) | Rvalue::RawRef(_) | Rvalue::DeepCopy(_) | Rvalue::Discriminant(_) | Rvalue::Len(_) => {}
                }
                match (dt, rvalue_ty(self.body, self.hir, rv)) {
                    // `p::freeze()` and `p::thaw()` move the same bits under the other pointer kind.
                    (Some(d), Ok(r)) if d != r && !(d.is_ptr() && r.is_ptr()) => self.err(&at, format!("assigns a `{r}` to a `{d}`")),
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
                    if local_ty(self.body, *f).ok() != Some(Type::Bool) {
                        self.err(&at, "a drop flag must be a bool local");
                    } else if !matches!(self.body.locals[*f].kind, LocalKind::DropFlag) {
                        self.err(&at, format!("{f} is named as a drop flag but is not a DropFlag local"));
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
                    if t != Type::Bool {
                        self.err(&at, format!("If on a `{t}`, not a bool"));
                    }
                }
            }
            TerminatorKind::Switch { discr, cases, .. } => {
                if let Some(t) = self.operand(&at, discr) {
                    match t {
                        Type::Int { signed: false, bits } if bits <= 32 => {
                            for (v, _) in cases {
                                if *v >> bits != 0 {
                                    self.err(&at, format!("Switch case {v} does not fit a `{t}`"));
                                }
                            }
                        }
                        _ => self.err(&at, format!("Switch on a `{t}`, not an enum tag")),
                    }
                }
            }
            TerminatorKind::Call { callee, args, dest, .. } => {
                let tys: Vec<Option<Type>> = args.iter().map(|a| self.operand(&at, &a.op)).collect();
                let dt = dest.as_ref().and_then(|d| self.place(&at, d));
                let (want_args, want_ret) = self.signature(*callee);
                match want_args {
                    Some(_) if matches!(callee, Callee::Join) => {
                        for (i, t) in tys.iter().enumerate() {
                            if let Some(t) = t {
                                if *t != Type::borrow(false, Type::Str) {
                                    self.err(&at, format!("part {i} of Join is a `{t}`, expected `&string`"));
                                }
                            }
                        }
                    }
                    Some(want) if want.len() != tys.len() => {
                        self.err(&at, format!("{callee:?} takes {} arguments, given {}", want.len(), tys.len()));
                    }
                    Some(want) => {
                        for (i, (w, t)) in want.iter().zip(&tys).enumerate() {
                            if let Some(t) = t {
                                if !w.accepts(*t) {
                                    self.err(&at, format!("argument {i} of {callee:?} is a `{t}`, expected {w}"));
                                }
                            }
                        }
                    }
                    None => {}
                }
                match (dest, dt) {
                    (Some(_), Some(d)) if d != want_ret => {
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

    /// What each argument of a callee must be, and its result type.
    fn signature(&self, callee: Callee) -> (Option<Vec<Want>>, Type) {
        let shared = |t: Type| Want::Exact(Type::borrow(false, t));
        match callee {
            Callee::Func(f) => {
                let func = &self.hir.funcs[f];
                let args = (0..func.params)
                    .map(|i| {
                        let t = func.locals[i].ty;
                        // The receiver is a borrow of the struct.
                        if i == 0 && func.has_self { Want::RefTo(t) } else { Want::Exact(t) }
                    })
                    .collect();
                (Some(args), func.ret)
            }
            Callee::Fn(ty) => {
                let Type::Func(sig) = ty else { unreachable!("a callable value") };
                let args = std::iter::once(ty).chain(sig.params.iter().copied()).map(Want::Exact);
                (Some(args.collect()), sig.ret)
            }
            Callee::Dyn { iface, method } => {
                let m = &self.hir.dyn_ifaces[iface].methods[method];
                let dynty = self.hir.dyn_type(iface);
                let recv = Want::Exact(Type::borrow(m.changes, dynty));
                let args = std::iter::once(recv).chain(m.params.iter().map(|&t| Want::Exact(t)));
                (Some(args.collect()), m.ret)
            }
            Callee::Str(f) => {
                let args = f.params().iter().enumerate().map(|(i, &t)| {
                    if t == Type::Str {
                        if f == hir::StrFn::Append && i == 0 { Want::Exact(Type::borrow(true, t)) } else { shared(t) }
                    } else {
                        Want::Exact(t)
                    }
                });
                (Some(args.collect()), f.ret())
            }
            Callee::Out { .. } => (Some(vec![shared(Type::Str)]), Type::None),
            Callee::InLine => (Some(Vec::new()), Type::Str),
            Callee::InEnded => (Some(Vec::new()), Type::Bool),
            Callee::Parse(ty) => (Some(vec![shared(Type::Str)]), ty),
            Callee::ParseStatus(_) => (Some(vec![shared(Type::Str)]), Type::U8),
            Callee::CauseWrap => (Some(vec![shared(Type::Str), Want::Exact(Type::borrow(true, Type::U64))]), Type::U64),
            Callee::Report => (Some(vec![shared(Type::Str), Want::Exact(Type::borrow(true, Type::U64))]), Type::None),
            Callee::ReportExit => (Some(vec![shared(Type::Str), Want::Exact(Type::borrow(true, Type::U64)), Want::Exact(Type::U8)]), Type::None),
            Callee::Text | Callee::Radix { .. } => (None, Type::Str),
            Callee::Join => (Some(Vec::new()), Type::Str),
            Callee::ListPush(elem) => (Some(vec![Want::Exact(Type::borrow(true, Type::list(elem))), Want::Exact(elem)]), Type::None),
            Callee::ListHas(elem) => (Some(vec![Want::Exact(Type::borrow(false, Type::list(elem))), Want::Any]), Type::Bool),
            Callee::MapOp(f, map) => {
                let (k, v) = map.map_parts().expect("a map call is on a map");
                let (mut_map, shared_map) = (Want::Exact(Type::borrow(true, map)), Want::Exact(Type::borrow(false, map)));
                let shared_key = Want::Exact(Type::borrow(false, k));
                match f {
                    MapFn::Get(result) => (Some(vec![shared_map, shared_key]), result),
                    MapFn::Set => (Some(vec![mut_map, Want::Exact(k), Want::Exact(v)]), Type::None),
                    MapFn::Remove => (Some(vec![mut_map, shared_key]), Type::None),
                    MapFn::Len => (Some(vec![shared_map]), Type::U64),
                    MapFn::Expect => (Some(vec![shared_map, shared_key]), v),
                    MapFn::More => (Some(vec![shared_map, Want::Exact(Type::U64)]), Type::Bool),
                    MapFn::KeyAt => (Some(vec![shared_map, Want::Exact(Type::U64)]), k),
                    MapFn::ValAt => (Some(vec![shared_map, Want::Exact(Type::U64)]), v),
                    MapFn::After => (Some(vec![shared_map, Want::Exact(Type::U64)]), Type::U64),
                }
            }
            Callee::ListOp(op, elem) => {
                let list = Type::list(elem);
                let (mut_list, shared_list) = (Want::Exact(Type::borrow(true, list)), Want::Exact(Type::borrow(false, list)));
                match op {
                    hir::ListOp::Get | hir::ListOp::View => (Some(vec![shared_list, Want::Any]), elem),
                    hir::ListOp::Remove | hir::ListOp::SwapRemove => (Some(vec![mut_list, Want::Any]), elem),
                    hir::ListOp::TakeLeaving => (Some(vec![mut_list, Want::Any, Want::Exact(elem)]), elem),
                    hir::ListOp::Insert => (Some(vec![mut_list, Want::Any, Want::Exact(elem)]), Type::None),
                    hir::ListOp::Clear => (Some(vec![mut_list]), Type::None),
                    hir::ListOp::Join => (Some(vec![shared_list, Want::Exact(Type::borrow(false, Type::Str))]), Type::Str),
                    hir::ListOp::Pop => unreachable!("`pop` is a test and a `Remove`, not a call"),
                }
            }
            Callee::IndexPanic { .. } => (Some(vec![Want::Any, Want::Exact(Type::U64)]), Type::None),
            Callee::Stop(_) => (Some(Vec::new()), Type::None),
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
    for pr in &pl.proj {
        if let Proj::Index(i) = pr {
            used(*i);
        }
    }
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
                Rvalue::MakeFn(_, _, Some(p)) => place_use(p, used),
                Rvalue::MakeFn(_, _, None) => {}
                Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) | Rvalue::Unsize(o, _) => operand_use(o, used),
                Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => {
                    operand_use(a, used);
                    operand_use(b, used);
                }
                Rvalue::Shift { value, amount, .. } => {
                    operand_use(value, used);
                    operand_use(amount, used);
                }
                Rvalue::Aggregate(_, ops) | Rvalue::Variant(_, _, ops) | Rvalue::Array(_, ops) => ops.iter().for_each(|o| operand_use(o, used)),
                Rvalue::ListNew(..) | Rvalue::MapNew(_) => {}
                Rvalue::Ref(_, pl) | Rvalue::RawRef(pl) | Rvalue::DeepCopy(pl) | Rvalue::Discriminant(pl) | Rvalue::Len(pl) => place_use(pl, used),
            }
            if dest.proj.is_empty() {
                assigned[dest.local.index()] = true;
            } else {
                used(dest.local);
            }
        }
        StatementKind::Drop { place, flag, .. } => {
            // A drop with a flag runs only if the value exists: it is not a
            // read of a possibly unassigned temp.
            match flag {
                Some(f) => used(*f),
                None => place_use(place, used),
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

/// What an argument's type must be.
enum Want {
    Any,
    Exact(Type),
    /// A `&` or `&mut` of this type.
    RefTo(Type),
}

impl Want {
    fn accepts(&self, t: Type) -> bool {
        match *self {
            Want::Any => true,
            Want::Exact(w) => w == t,
            Want::RefTo(w) => matches!(t, Type::Ref { to, .. } if *to == w),
        }
    }
}

impl std::fmt::Display for Want {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            Want::Any => write!(f, "anything"),
            Want::Exact(t) => write!(f, "`{t}`"),
            Want::RefTo(t) => write!(f, "`&{t}` or `&mut {t}`"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A program with `f(&mut uint32 n, &uint32 k)`, as MIR, to break by hand.
    fn base() -> (hir::Program, Program) {
        let src = "none f(&mut uint32 n, &uint32 k) { n = 5; }\nnone main() { mut uint32 a = 1; f(&mut a, &a); }\n";
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mir = build_program(&hir, &mut Vec::new());
        assert!(validate(&mir, &hir).is_ok());
        (hir, mir)
    }

    fn f_body(mir: &mut Program) -> &mut Body {
        mir.bodies.iter_mut().flatten().find(|b| b.name.ends_with("f")).unwrap()
    }

    fn push_stmt(body: &mut Body, dest: Type, rv: Rvalue) {
        let l = body.locals.push(LocalDecl { ty: dest, kind: LocalKind::Temp, span: Span::new(0, 0), scope: ScopeId(0), in_memory: false });
        body.blocks[BlockId(0)].statements.insert(
            0,
            Statement { kind: StatementKind::Assign(Place::local(l), rv), span: Span::new(0, 0), unsafe_ctx: false, mark: Mark::None },
        );
    }

    fn errors(hir: &hir::Program, mir: &Program) -> String {
        validate(mir, hir).expect_err("must be invalid").join("\n")
    }

    #[test]
    fn a_mut_borrow_is_never_copied() {
        let (hir, mut mir) = base();
        push_stmt(f_body(&mut mir), Type::borrow(true, Type::U32), Rvalue::Use(Operand::Copy(Place::local(Local(1)))));
        assert!(errors(&hir, &mir).contains("a `&mut` borrow is moved, never copied"));
    }

    #[test]
    fn a_shared_borrow_is_never_moved() {
        let (hir, mut mir) = base();
        push_stmt(f_body(&mut mir), Type::borrow(false, Type::U32), Rvalue::Use(Operand::Move(Place::local(Local(2)))));
        assert!(errors(&hir, &mir).contains("a `&` borrow is copied, never moved"));
    }

    #[test]
    fn a_borrow_of_a_borrow_is_ill_typed() {
        let (hir, mut mir) = base();
        push_stmt(f_body(&mut mir), Type::borrow(false, Type::borrow(false, Type::U32)), Rvalue::Ref(BorrowKind::Shared, Place::local(Local(2))));
        assert!(errors(&hir, &mir).contains("a reference to a reference"));
    }

    #[test]
    fn a_borrow_assigned_to_the_wrong_kind_is_ill_typed() {
        let (hir, mut mir) = base();
        // `&mut` stored into a `&` slot.
        push_stmt(f_body(&mut mir), Type::borrow(false, Type::U32), Rvalue::Ref(BorrowKind::Mut { two_phase: false }, Place { local: Local(1), proj: vec![Proj::Deref] }));
        assert!(errors(&hir, &mir).contains("assigns a `&mut uint32` to a `&uint32`"));
    }

    #[test]
    fn a_deref_of_a_number_is_ill_typed() {
        let (hir, mut mir) = base();
        let place = Place { local: Local(1), proj: vec![Proj::Deref, Proj::Deref] };
        push_stmt(f_body(&mut mir), Type::U32, Rvalue::Use(Operand::Copy(place)));
        assert!(errors(&hir, &mir).contains("Deref of `uint32`"));
    }
}
