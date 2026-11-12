//! Mutability checks on the MIR (spec 3.8): what may not change, and what may
//! not be lent `&mut`. One pass per body, reporting
//!
//! - E0205: a write to an immutable binding, parameter or field, including the
//!   late initialisation rules of spec 3.1.6 and reassigning an `open` binding;
//! - E0311: a change, a `&mut` borrow, or a method that changes `self`, through
//!   a `&` borrow (a `mut` field does not help there, owner Q5);
//! - E0312: a `&mut` borrow, or a method that changes `self`, of something that
//!   is neither `mut` nor `open` (or of a temporary value).
//!
//! The method case has one rule of its own: a method that changes only `mut`
//! fields may be called on an owned value that is not `mut` (owner Q-MF); see
//! `ownership::writes_only_mut_fields`.

use super::check::source_name;
use super::dataflow::{BitSet, Results};
use super::moves::{EverInit, MoveData};
use super::*;
use crate::diag::{Span, codes};
use crate::hir::StrFn;
use crate::ownership::{MutKind, MutReason, SelfMode, reason_chain, writes_only_mut_fields};
use std::collections::HashMap;

const THIS_VALUE: &str = "this value";

/// The user's spelling of a place: dereferences vanish (`(*p).name` is `p.name`).
pub fn place_text(hir: &hir::Program, body: &Body, p: &Place) -> String {
    let Some(name) = local_name(body, p.local) else { return THIS_VALUE.to_string() };
    let mut text = name.to_string();
    let mut t = body.locals[p.local].ty;
    for pr in &p.proj {
        match pr {
            Proj::Deref => t = t.target().unwrap_or(t),
            Proj::Downcast(_) => {}
            Proj::Index(_) => {
                text.push_str("[..]");
                t = t.elem().unwrap_or(t);
            }
            Proj::Field(f) => {
                if t.enum_id().is_some() {
                    return text;
                }
                let def = hir.struct_def(t);
                text.push('.');
                text.push_str(&def.fields[*f].name);
                t = def.fields[*f].ty;
            }
        }
    }
    text
}

/// The declared type of a binding as the user wrote it (units included), for
/// help text; the type itself when no text was kept.
fn spelled(decl: &LocalDecl) -> String {
    match &decl.kind {
        LocalKind::Param { written: Some(w), .. } | LocalKind::User { written: Some(w), .. } => w.clone(),
        _ => decl.ty.to_string(),
    }
}

fn local_name(body: &Body, l: Local) -> Option<&str> {
    match &body.locals[l].kind {
        LocalKind::User { name, .. } | LocalKind::Param { name, .. } => Some(name),
        _ => None,
    }
}

/// The type of each prefix of `p`: `types[k]` is the type after `k` projections.
fn proj_types(hir: &hir::Program, body: &Body, p: &Place) -> Vec<Type> {
    let mut t = body.locals[p.local].ty;
    let mut types = vec![t];
    let mut variant = None;
    for pr in &p.proj {
        t = match pr {
            Proj::Deref => t.target().unwrap_or(t),
            Proj::Index(_) => t.elem().unwrap_or(t),
            Proj::Downcast(v) => {
                variant = Some(*v);
                t
            }
            Proj::Field(f) => match variant.take() {
                Some(v) => hir.enum_def(t).variants[v].fields[*f].ty,
                None => hir.struct_def(t).fields[*f].ty,
            },
        };
        types.push(t);
    }
    types
}

/// Why a place cannot be changed (or lent `&mut`).
#[derive(Clone)]
enum Blocked {
    /// A binding, or a part of one, that is neither `mut` nor `open` (the root).
    Binding(Local),
    /// Reached through a `&` borrow, which is the place given.
    Shared(Place),
    /// A temporary value: it has no name to make `mut`.
    Temp,
}

/// Can `p` be changed in place, going by the table of spec 3.8. Walks the
/// projections from the end: a `mut` field of an owned value is changeable
/// whatever its binding, a `&mut` or raw pointer is changeable, a `&` is not.
fn changeable(hir: &hir::Program, body: &Body, p: &Place) -> Result<(), Blocked> {
    let types = proj_types(hir, body, p);
    for i in (0..p.proj.len()).rev() {
        match p.proj[i] {
            Proj::Field(f) => {
                let owner = types[i];
                let mut_field =
                    owner.struct_id().is_some() && hir.struct_def(owner).fields[f].mutable && !p.proj[..i].contains(&Proj::Deref);
                if mut_field {
                    return Ok(());
                }
            }
            Proj::Downcast(_) | Proj::Index(_) => {}
            Proj::Deref => {
                return match types[i] {
                    Type::Ref { mutable: false, .. } => Err(Blocked::Shared(Place { local: p.local, proj: p.proj[..i].to_vec() })),
                    _ => Ok(()),
                };
            }
        }
    }
    match body.locals[p.local].kind {
        LocalKind::Return | LocalKind::DropFlag => Ok(()),
        LocalKind::Temp => Err(Blocked::Temp),
        LocalKind::User { mutability, .. } | LocalKind::Param { mutability, .. } => match mutability {
            Mutability::Fixed => Err(Blocked::Binding(p.local)),
            Mutability::Mut | Mutability::Open => Ok(()),
        },
    }
}

/// A method that changes only `mut` fields may be called on an owned value,
/// whatever its binding (Q-MF). Through a borrow it may not, which the caller
/// reports before asking.
fn mut_field_method(hir: &hir::Program, method: FuncId, p: &Place) -> bool {
    !p.proj.contains(&Proj::Deref) && writes_only_mut_fields(hir, method)
}

/// A `&mut` borrow the builder makes for a method call.
#[derive(Clone, Copy)]
enum Role {
    /// The receiver of method `method`; `span` is the receiver as written.
    Receiver { method: FuncId, span: Span },
    /// The `&mut` of `s::append` (`s += t`).
    Append,
}

/// The statement that makes each `&mut` borrow passed as a receiver or to
/// `append`, keyed by its location.
fn roles(hir: &hir::Program, body: &Body) -> HashMap<Location, Role> {
    let mut made: HashMap<Local, Location> = HashMap::new();
    for (b, block) in body.blocks.iter_enumerated() {
        for (i, s) in block.statements.iter().enumerate() {
            if let StatementKind::Assign(d, Rvalue::Ref(BorrowKind::Mut { .. }, _)) = &s.kind
                && d.proj.is_empty()
            {
                made.insert(d.local, Location { block: b, index: i as u32 });
            }
        }
    }
    let mut out = HashMap::new();
    for block in body.blocks.iter() {
        let TerminatorKind::Call { callee, args, .. } = &block.terminator.kind else { continue };
        let Some(Arg { op: Operand::Move(p), span }) = args.first() else { continue };
        if !p.proj.is_empty() {
            continue;
        }
        let Some(&at) = made.get(&p.local) else { continue };
        let role = match callee {
            Callee::Func(f) if hir.funcs[*f].has_self => Role::Receiver { method: *f, span: *span },
            Callee::Str(StrFn::Append) | Callee::ListPush(_) => Role::Append,
            Callee::ListOp(op, _) if op.changes() => Role::Append,
            _ => continue,
        };
        out.insert(at, role);
    }
    out
}

/// Does `rv` read local `l` as a whole or in part?
fn reads_local(rv: &Rvalue, l: Local) -> bool {
    let ops: Vec<&Operand> = match rv {
        Rvalue::Use(o) | Rvalue::Neg(o) | Rvalue::Not(o) | Rvalue::BitNot(o) | Rvalue::Convert(o, _) => vec![o],
        Rvalue::Binary(_, a, b) | Rvalue::IntPow(a, b) => vec![a, b],
        Rvalue::Shift { value, amount, .. } => vec![value, amount],
        Rvalue::Aggregate(_, ops) | Rvalue::Variant(_, _, ops) | Rvalue::Array(_, ops) => ops.iter().collect(),
        Rvalue::Ref(..) | Rvalue::RawRef(_) | Rvalue::DeepCopy(_) | Rvalue::Discriminant(_) | Rvalue::Len(_) | Rvalue::ListNew(..) => Vec::new(),
    };
    ops.iter().any(|o| matches!(o, Operand::Copy(p) | Operand::Move(p) if p.local == l))
}

/// The span of the place a write changes, as the user wrote it. The builder
/// gives a statement a span inside the value it writes (often the value's
/// first operand), so the place is found again in the HIR: the assignment (or
/// `++`) with the same place whose value holds that span. Where none is found,
/// the statement's own span is used.
fn target_span(hir: &hir::Program, body: &Body, at: Span, dest: &Place) -> Span {
    find_target(&hir.funcs[body.func].body, at, dest).unwrap_or(at)
}

fn find_target(block: &hir::Block, at: Span, dest: &Place) -> Option<Span> {
    fn same_place(hp: &hir::Place, dest: &Place) -> bool {
        hp.local + 1 == dest.local.index()
            && hp.proj.len() == dest.proj.len()
            && hp.proj.iter().zip(&dest.proj).all(|(h, m)| match (h, m) {
                (hir::HirProj::Field(a), Proj::Field(b)) => a == b,
                (hir::HirProj::Deref, Proj::Deref) => true,
                (hir::HirProj::Index(_), Proj::Index(_)) => true,
                _ => false,
            })
    }
    let holds = |e: &hir::Expr| e.span.start <= at.start && at.end <= e.span.end;
    block.iter().find_map(|s| match s {
        hir::Stmt::Assign(hp, e) if holds(e) && same_place(hp, dest) => Some(hp.span),
        // `take p.f leaving v` writes `p.f` with the value `v`.
        hir::Stmt::Assign(_, hir::Expr { kind: hir::ExprKind::Take { place, leaving: Some(v) }, .. })
            if holds(v) && same_place(place, dest) =>
        {
            Some(place.span)
        }
        hir::Stmt::Expr(e @ hir::Expr { kind: hir::ExprKind::IncDec { place, .. }, .. }) if holds(e) && same_place(place, dest) => {
            Some(place.span)
        }
        hir::Stmt::If { branches, .. } => branches.iter().find_map(|b| find_target(&b.body, at, dest)),
        hir::Stmt::Loop { body, step, .. } => find_target(body, at, dest).or_else(|| find_target(step, at, dest)),
        hir::Stmt::Unsafe(b) => find_target(b, at, dest),
        _ => None,
    })
}

/// The live component of the block that starts each local's storage (spec 3.1.6).
fn storage_components(body: &Body, cycles: &IndexVec<BlockId, Option<u32>>) -> IndexVec<Local, Option<Option<u32>>> {
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
    live_scc
}

struct Cx<'a> {
    hir: &'a hir::Program,
    body: &'a Body,
    md: &'a MoveData,
    modes: &'a [Option<SelfMode>],
    cycles: &'a IndexVec<BlockId, Option<u32>>,
    declared: IndexVec<Local, Option<Option<u32>>>,
    roles: HashMap<Location, Role>,
}

pub fn check(
    hir: &hir::Program,
    body: &Body,
    md: &MoveData,
    ever: &Results<EverInit>,
    cycles: &IndexVec<BlockId, Option<u32>>,
    modes: &[Option<SelfMode>],
    out: &mut Vec<Diagnostic>,
) {
    let cx = Cx { hir, body, md, modes, cycles, declared: storage_components(body, cycles), roles: roles(hir, body) };
    ever.visit(body, |loc, state, _| {
        let block = &body.blocks[loc.block];
        let Some(stmt) = block.statements.get(loc.index as usize) else { return };
        let StatementKind::Assign(dest, rv) = &stmt.kind else { return };
        if dest.proj.is_empty() {
            cx.whole_local(out, loc, state, dest.local, rv, stmt.span);
        } else {
            cx.field_write(out, state, stmt.span, dest);
        }
        if let Rvalue::Ref(BorrowKind::Mut { .. }, p) = rv {
            cx.mut_borrow(out, state, loc, stmt.span, p);
        }
    });
}

impl Cx<'_> {
    fn name(&self, l: Local) -> &str {
        local_name(self.body, l).unwrap_or("")
    }

    fn text(&self, p: &Place) -> String {
        place_text(self.hir, self.body, p)
    }

    /// Has the root of a binding got a value by now? Where it has none, the
    /// move checks already say so (E0207, E0332), and a change is not added.
    fn assigned(&self, state: &BitSet, root: Local) -> bool {
        matches!(self.body.locals[root].kind, LocalKind::Param { .. }) || state.contains(self.md.local_path[root].index())
    }

    /// A whole binding assigned: its first assignment is fine where it has no
    /// value yet and no loop repeats it (spec 3.1.6); reassigning an `open` binding
    /// is its own variant.
    fn whole_local(&self, out: &mut Vec<Diagnostic>, loc: Location, state: &BitSet, l: Local, rv: &Rvalue, at: Span) {
        let decl = &self.body.locals[l];
        match &decl.kind {
            LocalKind::Param { name, mutability: Mutability::Fixed, .. } => {
                let ty = spelled(decl);
                out.push(
                    Diagnostic::at(&codes::E0205, at, format!("cannot change parameter `{name}`: it is not `mut`"))
                        .note("changed here")
                        .label(decl.span, "declared here without `mut`")
                        .help(format!("declare it as `mut {ty} {name}` to allow changes")),
                );
            }
            LocalKind::Param { name, mutability: Mutability::Open, .. } => {
                let ty = spelled(decl);
                out.push(
                    Diagnostic::at(&codes::E0205, at, format!("cannot reassign parameter `{name}`: it is `open`, not `mut`"))
                        .note("reassigned here")
                        .label(decl.span, "declared `open`: its contents may change, but it is never reassigned")
                        .help(format!("declare it `mut {ty} {name}` to allow reassigning")),
                );
            }
            LocalKind::User { name, mutability: mutability @ (Mutability::Fixed | Mutability::Open), .. } => {
                let Some(declared_in) = self.declared[l] else { return };
                let in_loop = self.cycles[loc.block].is_some() && self.cycles[loc.block] != declared_in;
                let assigned = state.contains(self.md.local_path[l].index());
                if !assigned && !in_loop {
                    return;
                }
                let ty = spelled(decl);
                let d = if in_loop {
                    // Its first assignment, but a loop would repeat it.
                    Diagnostic::at(&codes::E0205, at, format!("cannot assign `{name}` inside a loop: it is not `mut`"))
                        .note("the loop would assign it again each time round")
                        .label(decl.span, "declared here, outside the loop, without `mut`")
                        .help(format!("declare it as `mut {ty} {name}`, or move the declaration into the loop"))
                } else if *mutability == Mutability::Open {
                    Diagnostic::at(&codes::E0205, at, format!("cannot reassign `{name}`: it is `open`, not `mut`"))
                        .note("reassigned here")
                        .label(decl.span, "declared `open`: its contents may change, but it is never reassigned")
                        .help(format!("declare it `mut {ty} {name}` to allow reassigning"))
                } else {
                    // `x++` and `x += v` read the place they change; a plain `x = v` does not.
                    let note = if reads_local(rv, l) { "changed here" } else { "assigned a second time here" };
                    Diagnostic::at(&codes::E0205, at, format!("cannot change `{name}`: it is not `mut`"))
                        .note(note)
                        .label(decl.span, "declared here without `mut`")
                        .help(format!("declare it as `mut {ty} {name}` to allow changes"))
                };
                out.push(d);
            }
            _ => {}
        }
    }

    /// `p.f = v` (a write with projections).
    fn field_write(&self, out: &mut Vec<Diagnostic>, state: &BitSet, stmt_span: Span, dest: &Place) {
        let at = target_span(self.hir, self.body, stmt_span, dest);
        match changeable(self.hir, self.body, dest) {
            Ok(()) | Err(Blocked::Temp) => {}
            Err(Blocked::Shared(borrow)) => out.push(self.change_through_shared(at, dest, &borrow)),
            Err(Blocked::Binding(root)) => {
                if self.assigned(state, root) {
                    out.push(self.change_binding(at, dest));
                }
            }
        }
    }

    /// A `&mut` borrow of `p`: a receiver, the `&mut` of `append`, or a `&mut`
    /// the user wrote (or `lend` made of a `&mut` the user holds, which is never blocked).
    fn mut_borrow(&self, out: &mut Vec<Diagnostic>, state: &BitSet, loc: Location, at: Span, p: &Place) {
        let Err(blocked) = changeable(self.hir, self.body, p) else { return };
        let role = self.roles.get(&loc).copied();
        match (role, blocked) {
            (_, Blocked::Shared(borrow)) => match role {
                Some(Role::Receiver { method, span }) => out.push(self.method_through_shared(span, method, &borrow)),
                Some(Role::Append) => out.push(self.change_through_shared(at, p, &borrow)),
                None => out.push(self.lend_through_shared(at, p, &borrow)),
            },
            (Some(Role::Append), Blocked::Binding(root)) => {
                if self.assigned(state, root) {
                    out.push(self.change_binding(at, p));
                }
            }
            (Some(Role::Append), Blocked::Temp) => {}
            (Some(Role::Receiver { method, span }), blocked) => {
                if let Blocked::Binding(root) = blocked
                    && !self.assigned(state, root)
                {
                    return;
                }
                if mut_field_method(self.hir, method, p) {
                    return;
                }
                let d = match blocked {
                    Blocked::Binding(root) => self.method_on_binding(span, method, p, root),
                    _ => self.method_on_temp(span, method),
                };
                out.push(d);
            }
            (None, blocked) => {
                if let Blocked::Binding(root) = blocked
                    && !self.assigned(state, root)
                {
                    return;
                }
                let d = match blocked {
                    Blocked::Binding(root) => self.lend_binding(at, p, root),
                    _ => self.lend_temp(at),
                };
                out.push(d);
            }
        }
    }

    /// E0205 for a write to a field (or for `s += t` of a whole binding) of a
    /// binding that is not `mut` and whose field is not `mut`.
    fn change_binding(&self, at: Span, place: &Place) -> Diagnostic {
        let root = place.local;
        let decl = &self.body.locals[root];
        let name = self.name(root);
        let lty = spelled(decl);
        let param = matches!(decl.kind, LocalKind::Param { .. });
        if place.proj.is_empty() {
            let head = if param {
                format!("cannot change parameter `{name}`: it is not `mut`")
            } else {
                format!("cannot change `{name}`: it is not `mut`")
            };
            return Diagnostic::at(&codes::E0205, at, head)
                .note("changed here")
                .label(decl.span, "declared here without `mut`")
                .help(format!("declare it as `mut {lty} {name}` to allow changes"));
        }
        let whose = if param { "parameter " } else { "" };
        let types = proj_types(self.hir, self.body, place);
        let last = place.proj.len() - 1;
        let Proj::Field(f) = place.proj[last] else {
            // An item of a list or array in a binding that is not `mut`.
            return Diagnostic::at(
                &codes::E0205,
                at,
                format!("cannot change `{}`: {whose}`{name}` is not `mut`", self.text(place)),
            )
            .note("changed here")
            .label(decl.span, "declared here without `mut`")
            .help(format!("declare it as `mut {lty} {name}` to allow changes"));
        };
        let def = self.hir.struct_def(types[last]);
        let field = &def.fields[f];
        Diagnostic::at(
            &codes::E0205,
            at,
            format!("cannot change `{}`: {whose}`{name}` is not `mut`, and neither is the field", self.text(place)),
        )
        .note("changed here")
        .label(decl.span, "declared here without `mut`")
        .help(format!("declare it as `mut {lty} {name}`, or the field as `mut {} {};` in `{}`", field.ty, field.name, def.name))
    }

    /// Adds the note about the borrow `&` a change went through.
    fn borrow_help(&self, mut d: Diagnostic, borrow: &Place) -> Diagnostic {
        let types = proj_types(self.hir, self.body, borrow);
        let target = types.last().and_then(|t| t.target()).map_or(String::new(), |t| t.to_string());
        if borrow.proj.is_empty() {
            let name = self.name(borrow.local);
            d = d.label(self.body.locals[borrow.local].span, format!("`{name}` is a `&` borrow, which only reads"));
            d.help(format!("take a `&mut` borrow: declare it `&mut {target} {name}`"))
        } else {
            d.help("take a `&mut` borrow of the value instead".to_string())
        }
    }

    /// E0311: a change of `place` through the `&` borrow `borrow`.
    fn change_through_shared(&self, at: Span, place: &Place, borrow: &Place) -> Diagnostic {
        let d = Diagnostic::at(&codes::E0311, at, format!("cannot change `{}` through a `&` borrow", self.text(place)))
            .note("change attempted here");
        self.borrow_help(d, borrow)
    }

    fn lend_through_shared(&self, at: Span, place: &Place, borrow: &Place) -> Diagnostic {
        let d = Diagnostic::at(&codes::E0311, at, format!("cannot lend `{}` as `&mut` through a `&` borrow", self.text(place)))
            .note("`&mut` borrow here");
        self.borrow_help(d, borrow)
    }

    /// E0311, the method variant: a method that changes `self` on a value
    /// reached through a `&` borrow.
    fn method_through_shared(&self, at: Span, method: FuncId, borrow: &Place) -> Diagnostic {
        let m = self.method_name(method);
        let d = Diagnostic::at(&codes::E0311, at, format!("cannot call `{m}` through a `&` borrow: `{m}` changes `self`"))
            .note("call attempted here");
        let d = self.borrow_help(d, borrow);
        self.self_reasons(d, method)
    }

    /// E0312, the method variant on a binding that is neither `mut` nor `open`.
    fn method_on_binding(&self, at: Span, method: FuncId, place: &Place, root: Local) -> Diagnostic {
        let m = self.method_name(method);
        let decl = &self.body.locals[root];
        let name = self.name(root);
        let ty = spelled(decl);
        let d = Diagnostic::at(
            &codes::E0312,
            at,
            format!("cannot call `{m}` on `{}`: `{m}` changes `self`, and `{name}` is neither `mut` nor `open`", self.text(place)),
        )
        .note("call attempted here")
        .label(decl.span, "declared here without `mut` or `open`")
        .help(format!("declare it `open {ty} {name}` (its contents may change) or `mut {ty} {name}` (it may also be reassigned)"));
        self.self_reasons(d, method)
    }

    fn method_on_temp(&self, at: Span, method: FuncId) -> Diagnostic {
        let m = self.method_name(method);
        let d = Diagnostic::at(&codes::E0312, at, format!("cannot call `{m}` on a temporary value: `{m}` changes `self`"))
            .note("call attempted here")
            .help(format!("store the value in a `mut` binding first, then call `{m}` on that"));
        self.self_reasons(d, method)
    }

    /// E0312 for a `&mut` the user writes on a binding that is neither `mut` nor `open`.
    fn lend_binding(&self, at: Span, place: &Place, root: Local) -> Diagnostic {
        let decl = &self.body.locals[root];
        let name = self.name(root);
        let ty = spelled(decl);
        Diagnostic::at(&codes::E0312, at, format!("cannot lend `{}` as `&mut`: it is neither `mut` nor `open`", self.text(place)))
            .note("`&mut` borrow here")
            .label(decl.span, "declared here without `mut` or `open`")
            .help(format!("declare it `open {ty} {name}` (its contents may change) or `mut {ty} {name}` (it may also be reassigned)"))
    }

    fn lend_temp(&self, at: Span) -> Diagnostic {
        Diagnostic::at(&codes::E0312, at, "cannot lend a temporary value as `&mut`")
            .note("`&mut` borrow here")
            .help("store the value in a `mut` binding first, then lend that".to_string())
    }

    fn method_name(&self, method: FuncId) -> String {
        source_name(&self.hir.funcs[method].symbol).1
    }

    /// The labels that say why `method` changes `self`: its own reason, and the
    /// reason at the end of a chain of calls.
    fn self_reasons(&self, mut d: Diagnostic, method: FuncId) -> Diagnostic {
        let chain = reason_chain(self.modes, method);
        let Some((first_fn, first)) = chain.first() else { return d };
        let m = self.method_name(*first_fn);
        d = d.label(first.span, format!("`{m}` changes `self` here: it {}", self.why(first)));
        if let Some((last_fn, last)) = chain.last().filter(|_| chain.len() > 1) {
            let n = self.method_name(*last_fn);
            d = d.label(last.span, format!("`{n}` changes `self` here: it {}", self.why(last)));
        }
        d
    }

    fn why(&self, r: &MutReason) -> String {
        match &r.kind {
            MutKind::Writes(text) => format!("writes `{text}`"),
            MutKind::LendsMut(text) => format!("lends `{text}` as `&mut`"),
            MutKind::Calls(callee) => format!("calls `{}`, which changes `self`", self.method_name(*callee)),
            MutKind::IsDrop => "is `drop`".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `f`'s body of a program whose locals are: `_0`, `p: &Player`, `q: &mut Player`,
    /// `r: Player`, `s: mut Player`, `t: open Player`.
    fn fixture() -> (hir::Program, Program) {
        let src = "struct Player { public: string name; mut uint32 hp; }
none f(&Player p, &mut Player q, Player r, mut Player s, open Player t) { }
none main() { }
";
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mut mir_diags = Vec::new();
        let mir = build_program(&hir, &mut mir_diags);
        assert!(mir_diags.is_empty());
        (hir, mir)
    }

    #[test]
    fn changeable_follows_the_table() {
        let (hir, mir) = fixture();
        let body = mir.bodies.iter().flatten().find(|b| b.name == "f").unwrap();
        let place = |local: usize, proj: Vec<Proj>| Place { local: Local::new(local), proj };
        let check = |p: Place| changeable(&hir, body, &p);
        let name = Proj::Field(0);
        let hp = Proj::Field(1);

        // Bare locals: `mut` and `open` change, a plain binding does not.
        assert!(check(place(4, vec![])).is_ok() && check(place(5, vec![])).is_ok());
        assert!(matches!(check(place(3, vec![])), Err(Blocked::Binding(l)) if l == Local::new(3)));
        // A `mut` field of an owned value changes whatever its binding is; any other field follows the binding.
        assert!(check(place(3, vec![hp])).is_ok());
        assert!(matches!(check(place(3, vec![name])), Err(Blocked::Binding(_))));
        assert!(check(place(5, vec![name])).is_ok() && check(place(4, vec![name])).is_ok());
        // Behind a `&mut` anything changes; behind a `&` nothing does, not even a `mut` field.
        assert!(check(place(2, vec![Proj::Deref, hp])).is_ok());
        assert!(check(place(2, vec![Proj::Deref, name])).is_ok());
        assert!(matches!(check(place(1, vec![Proj::Deref, hp])), Err(Blocked::Shared(_))));
        assert!(matches!(check(place(1, vec![Proj::Deref, name])), Err(Blocked::Shared(_))));
        assert!(matches!(check(place(1, vec![Proj::Deref])), Err(Blocked::Shared(_))));
        // The borrow named in the error is the place before its dereference.
        match check(place(1, vec![Proj::Deref, name])) {
            Err(Blocked::Shared(borrow)) => assert_eq!(borrow, place(1, vec![])),
            _ => panic!("expected a change through a shared borrow"),
        }
    }
}
