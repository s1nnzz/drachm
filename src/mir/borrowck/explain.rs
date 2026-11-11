//! Turning a conflict into an error that explains itself (spec 3.5.7).
//!
//! Three things are shown: where the place was borrowed, the access that is
//! not allowed, and where the borrow is still going to be used.

use std::collections::{HashSet, VecDeque};

use super::conflicts::{AccessKind, Conflict, Via};
use super::constraints::{Generated, Loan};
use super::loans::{next_points, LoanScopes};
use super::regions::region_slots;
use super::solve::{Constraint, RegionValue, RegionVid};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir;
use crate::mir::check::source_name;
use crate::mir::index::{Idx, IndexVec};
use crate::mir::liveness::{holds_user_drop, rvalue_locals, terminator_uses};
use crate::mir::mutability::place_text;
use crate::mir::*;

pub struct Cx<'a> {
    pub hir: &'a hir::Program,
    pub body: &'a Body,
    pub cg: &'a Generated,
    pub values: &'a IndexVec<RegionVid, RegionValue>,
    pub constraints: &'a [Constraint],
    pub scopes: &'a LoanScopes,
    /// Reverse post-order rank of each block.
    pub rank: &'a [usize],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum UseKind {
    Plain,
    Return,
    DropUse,
    NextRound,
}

struct Later {
    local: Option<Local>,
    span: Span,
    kind: UseKind,
}

fn quoted(text: &str) -> String {
    if text == "this value" { text.to_string() } else { format!("`{text}`") }
}

impl Cx<'_> {
    fn local_name(&self, l: Local) -> Option<&str> {
        match &self.body.locals[l].kind {
            LocalKind::User { name, .. } | LocalKind::Param { name, .. } => Some(name),
            _ => None,
        }
    }

    fn text(&self, p: &Place) -> String {
        place_text(self.hir, self.body, p)
    }

    /// The region variables the borrow flows into: everything `loan.region` contains.
    fn holders(&self, loan: &Loan) -> HashSet<usize> {
        let mut flows: Vec<Vec<usize>> = vec![Vec::new(); self.values.len()];
        for c in self.constraints {
            flows[c.sup.index()].push(c.sub.index());
        }
        let mut seen = HashSet::new();
        let mut stack = vec![loan.region.index()];
        while let Some(r) = stack.pop() {
            if seen.insert(r) {
                stack.extend(flows[r].iter().copied());
            }
        }
        seen
    }

    fn holds(&self, l: Local, holders: &HashSet<usize>) -> bool {
        let base = self.cg.local_start[l];
        let n = region_slots(self.hir, self.body.locals[l].ty);
        (base..base + n).any(|v| holders.contains(&v))
    }

    /// The locals the point uses, with the span of each use.
    fn uses_at(&self, loc: Location) -> Vec<(Local, Span)> {
        let block = &self.body.blocks[loc.block];
        let i = loc.index as usize;
        let mut out = Vec::new();
        if i < block.statements.len() {
            let s = &block.statements[i];
            match &s.kind {
                StatementKind::Assign(d, rv) => {
                    rvalue_locals(rv, &mut |l| out.push((l, s.span)));
                    if !d.proj.is_empty() {
                        out.push((d.local, s.span));
                    }
                }
                StatementKind::Drop { place, .. } if place_has_user_drop(self, place) => out.push((place.local, s.span)),
                StatementKind::FakeRead(p) => out.push((p.local, s.span)),
                _ => {}
            }
        } else {
            let t = &block.terminator;
            match &t.kind {
                TerminatorKind::Call { args, .. } => {
                    for a in args {
                        if let Operand::Copy(p) | Operand::Move(p) = &a.op {
                            out.push((p.local, a.span));
                        }
                    }
                }
                other => terminator_uses(other, &mut |l| out.push((l, t.span))),
            }
        }
        out
    }

    /// The first place the borrow is used after `from`.
    fn later_use(&self, li: usize, from: Location, include_from: bool) -> Option<Later> {
        let loan = &self.cg.loans[li];
        let holders = self.holders(loan);
        let region = &self.values[loan.region];
        let mut seen = HashSet::new();
        let mut queue: VecDeque<Location> = if include_from { VecDeque::from([from]) } else { next_points(self.body, from).into() };
        let mut first_any: Option<Later> = None;
        let here = (self.rank[from.block.index()], from.index);
        while let Some(q) = queue.pop_front() {
            if !seen.insert(q) || !region.points.contains(self.cg.points.index(q)) {
                continue;
            }
            for (l, span) in self.uses_at(q) {
                if !self.holds(l, &holders) {
                    continue;
                }
                let block = &self.body.blocks[q.block];
                let is_return = q.index as usize == block.statements.len() && matches!(block.terminator.kind, TerminatorKind::Return);
                let is_drop = (q.index as usize) < block.statements.len() && matches!(block.statements[q.index as usize].kind, StatementKind::Drop { .. });
                let kind = if is_return {
                    UseKind::Return
                } else if is_drop {
                    UseKind::DropUse
                } else if q != from && (self.rank[q.block.index()], q.index) <= here {
                    UseKind::NextRound
                } else {
                    UseKind::Plain
                };
                let later = Later { local: Some(l), span, kind };
                if self.local_name(l).is_some() || kind == UseKind::Return {
                    return Some(later);
                }
                first_any.get_or_insert(later);
            }
            queue.extend(next_points(self.body, q));
        }
        if let Some(l) = first_any {
            return Some(l);
        }
        // Escapes through a return value.
        for b in self.body.blocks.indices() {
            let t = &self.body.blocks[b].terminator;
            if matches!(t.kind, TerminatorKind::Return) && self.rank[b.index()] != usize::MAX {
                return Some(Later { local: None, span: t.span, kind: UseKind::Return });
            }
        }
        None
    }

    /// The universals whose end the loan reaches: (is a return slot, name).
    fn escapes(&self, loan: &Loan) -> Vec<(bool, String)> {
        self.values[loan.region]
            .ends
            .iter()
            .map(|u| (self.cg.universals[u].param.is_none(), self.cg.universals[u].name.clone()))
            .collect()
    }

    fn function_name(&self) -> String {
        source_name(&self.body.name).1
    }

    /// `h`, or `h.src` for the slot of a struct that holds the borrow.
    fn holder_text(&self, l: Local, loan: &Loan) -> Option<String> {
        let name = self.local_name(l)?;
        let ty = self.body.locals[l].ty;
        if let Type::Struct { id, .. } = ty {
            let def = &self.hir.structs[id];
            let holders = self.holders(loan);
            let base = self.cg.local_start[l];
            for (k, &f) in def.lives.iter().enumerate() {
                if holders.contains(&(base + k)) {
                    return Some(format!("{name}.{}", def.fields[f].name));
                }
            }
        }
        Some(name.to_string())
    }
}

fn place_has_user_drop(cx: &Cx, place: &Place) -> bool {
    crate::mir::ty::place_ty(cx.body, cx.hir, place).is_ok_and(|t| holds_user_drop(cx.hir, t))
}

/// The `-> bool` of whether a place text names a temporary.
fn is_temp(text: &str) -> bool {
    text == "this value"
}

/// The error for one conflict.
pub fn diagnostic(cx: &Cx, c: &Conflict) -> Diagnostic {
    let loan = &cx.cg.loans[c.loan];
    let a = &c.access;
    let kind = match loan.kind {
        BorrowKind::Shared => None,
        BorrowKind::Mut { .. } => Some(()),
    };
    let reserved_shared = matches!(loan.kind, BorrowKind::Mut { two_phase: true }) && cx.scopes.reserved[c.loan].contains(cx.cg.points.index(a.scope_loc));
    let loan_is_mut = kind.is_some() && !reserved_shared;
    let owner = cx.text(&loan.place);
    let access_text = cx.text(&a.place);
    // The call that activates a receiver also uses the arguments' borrows.
    let later = cx.later_use(c.loan, a.loc, a.via == Via::Receiver);
    let holder = later.as_ref().and_then(|l| l.local).and_then(|l| cx.holder_text(l, loan));
    let holder_name = holder.as_ref().map(|h| h.split('.').next().unwrap_or(h).to_string());
    let finish = match &holder_name {
        Some(h) => format!("finish using `{h}`"),
        None => "finish using the borrow".to_string(),
    };

    // The label that points at the later use.
    let use_label = |l: &Later| -> String {
        match l.kind {
            UseKind::Return => "the borrow is returned here".to_string(),
            UseKind::DropUse => match l.local.and_then(|x| cx.local_name(x)) {
                Some(n) => format!("`{n}` is dropped here, and its `drop()` can still use the borrow"),
                None => "the value is dropped here, and its `drop()` can still use the borrow".to_string(),
            },
            UseKind::NextRound => "the borrow is used again here, on the next time round the loop".to_string(),
            UseKind::Plain if loan_is_mut && a.kind == AccessKind::MutBorrow => "the first borrow is still used here".to_string(),
            UseKind::Plain if loan_is_mut => "the `&mut` borrow is still used here".to_string(),
            UseKind::Plain => "the borrow is still used here".to_string(),
        }
    };
    let with_use = |mut d: Diagnostic| -> Diagnostic {
        if let Some(l) = &later {
            d = d.label(l.span, use_label(l));
        }
        d
    };

    match a.kind {
        AccessKind::Dead | AccessKind::Drop => return scope_end(cx, c, &owner, later.as_ref(), holder.as_deref()),
        _ => {}
    }

    match crate::mir::borrowck::conflicts::conflict_code(if loan_is_mut { BorrowKind::Mut { two_phase: false } } else { BorrowKind::Shared }, a.kind) {
        Some("E0302") => {
            let (primary, help) = if a.kind == AccessKind::MutBorrow && a.via != Via::Receiver {
                (
                    "`&mut` borrow attempted here".to_string(),
                    format!("{finish} before lending {} as `&mut`", quoted(&access_text)),
                )
            } else {
                let help = match later.as_ref().map(|l| l.kind) {
                    Some(UseKind::Return) => format!("change {} before borrowing it, or return a copy", quoted(&access_text)),
                    Some(UseKind::NextRound) => match &holder_name {
                        Some(h) => format!("finish using `{h}` inside the loop, or borrow {} again each time round", quoted(&access_text)),
                        None => format!("finish using the borrow inside the loop, or borrow {} again each time round", quoted(&access_text)),
                    },
                    Some(UseKind::DropUse) => match &holder_name {
                        Some(h) => format!("end `{h}` before changing {}: `destroy {h};`", quoted(&access_text)),
                        None => format!("end the value that holds the borrow before changing {}", quoted(&access_text)),
                    },
                    _ => format!("{finish} before changing {}", quoted(&access_text)),
                };
                ("change attempted here".to_string(), help)
            };
            let d = Diagnostic::at(&codes::E0302, a.span, format!("cannot change {} while it is borrowed", quoted(&access_text)))
                .note(primary)
                .label(loan.span, format!("{} is borrowed here", quoted(&owner)));
            with_use(d).help(help)
        }
        Some("E0303") => {
            let d = Diagnostic::at(&codes::E0303, a.span, format!("{} is already borrowed with `&mut`", quoted(&owner)))
                .note("second `&mut` borrow here")
                .label(loan.span, "first `&mut` borrow here");
            with_use(d).help(format!("only one `&mut` borrow of {} can be in use at a time; {finish} before borrowing {} again", quoted(&owner), quoted(&owner)))
        }
        Some("E0304") => {
            let lent_parent = loan.place.proj.first() == Some(&Proj::Deref) && matches!(cx.body.locals[loan.place.local].ty, Type::Ref { mutable: true, .. }) && cx.local_name(loan.place.local).is_some();
            let (msg, primary, help, loan_label) = if lent_parent {
                (
                    format!("cannot use {} while a part of it is lent", quoted(&cx.text(&Place::local(loan.place.local)))),
                    "used here".to_string(),
                    match &holder_name {
                        Some(h) => format!("finish using `{h}` first; the lend ended after the call that took it"),
                        None => "finish using the lent borrow first; the lend ended after the call that took it".to_string(),
                    },
                    format!("{} is lent here", quoted(&cx.text(&Place::local(loan.place.local)))),
                )
            } else if a.kind == AccessKind::ShareBorrow {
                (
                    format!("cannot borrow {} while it is borrowed with `&mut`", quoted(&access_text)),
                    "borrowed again here".to_string(),
                    format!("{finish} before borrowing {} again", quoted(&access_text)),
                    format!("{} is borrowed with `&mut` here", quoted(&owner)),
                )
            } else {
                let through = match (&holder, a.place.local == loan.place.local) {
                    (Some(h), true) if a.place.proj.len() >= loan.place.proj.len() => {
                        let rest = &a.place.proj[loan.place.proj.len()..];
                        let mut t = h.split('.').next().unwrap_or(h).to_string();
                        let mut ty = crate::mir::ty::place_ty(cx.body, cx.hir, &loan.place).ok();
                        for pr in rest {
                            if let (Proj::Field(f), Some(Type::Struct { id, .. })) = (pr, ty) {
                                t.push('.');
                                t.push_str(&cx.hir.structs[id].fields[*f].name);
                                ty = Some(cx.hir.structs[id].fields[*f].ty);
                            }
                        }
                        Some(t)
                    }
                    _ => None,
                };
                let help = match through {
                    Some(t) if !is_temp(&access_text) => {
                        format!("read {} before borrowing {} with `&mut`, or read it through `{}`: `{t}`", quoted(&access_text), quoted(&owner), holder_name.clone().unwrap_or_default())
                    }
                    _ => format!("read {} before borrowing {} with `&mut`", quoted(&access_text), quoted(&owner)),
                };
                (
                    format!("cannot use {} while {} is borrowed with `&mut`", quoted(&access_text), quoted(&owner)),
                    "used here".to_string(),
                    help,
                    format!("{} is borrowed with `&mut` here", quoted(&owner)),
                )
            };
            let d = Diagnostic::at(&codes::E0304, a.span, msg).note(primary).label(loan.span, loan_label);
            with_use(d).help(help)
        }
        Some("E0305") => {
            let d = Diagnostic::at(&codes::E0305, a.span, format!("cannot move {} while it is borrowed", quoted(&access_text)))
                .note("moved here")
                .label(loan.span, format!("{} is borrowed here", quoted(&owner)));
            with_use(d).help(format!("{finish} before moving {}, or move a copy: `copy {access_text}`", quoted(&access_text)))
        }
        other => unreachable!("conflict code {other:?} for {a:?}"),
    }
}

/// E0306 and E0307: the owner of a borrowed place ends while the borrow lives on.
fn scope_end(cx: &Cx, c: &Conflict, owner: &str, later: Option<&Later>, holder: Option<&str>) -> Diagnostic {
    let loan = &cx.cg.loans[c.loan];
    let a = &c.access;
    let escapes = cx.escapes(loan);
    let root = loan.place.local;
    let root_name = cx.local_name(root).map(str::to_string);
    let f = cx.function_name();
    let returned = escapes.iter().any(|(ret, _)| *ret);
    let owner_text = root_name.clone().unwrap_or_else(|| "this value".to_string());
    let declared = cx.body.locals[root].span;
    let is_param = (1..=cx.body.arg_count).contains(&root.index());

    if returned {
        // E0307: the borrow is returned.
        let ret_span = later.map_or(a.span, |l| l.span);
        let (msg, help) = match &root_name {
            Some(n) => (
                format!("cannot return a borrow of `{n}`, which belongs to this function"),
                if is_param {
                    format!("take it as a borrow: `&{} {n}`, or return `{n}` itself", cx.body.locals[root].ty)
                        .replace("&&", "&")
                } else if cx.body.locals[root].ty == Type::Str {
                    "return the string itself: change the return type to `string` and write `return s;`".replace("`s`", &format!("`{n}`")).replace("return s;", &format!("return {n};"))
                } else {
                    format!("return the value itself: change the return type to `{}` and write `return {n};`", cx.body.locals[root].ty)
                },
            ),
            None => (
                "cannot return a borrow of a temporary value".to_string(),
                "return the value itself instead of a borrow of it".to_string(),
            ),
        };
        let mut d = Diagnostic::at(&codes::E0307, ret_span, msg).note("the borrow is returned here");
        if let Some(n) = &root_name {
            let label = if is_param {
                format!("`{n}` is taken by value, so it is dropped when `{f}` returns")
            } else {
                format!("`{n}` is created here and dropped when `{f}` returns")
            };
            if declared != Span::new(0, 0) {
                d = d.label(declared, label);
            }
        }
        let has_borrow_param = (0..cx.body.arg_count).any(|i| region_slots(cx.hir, cx.body.locals[Local(i as u32 + 1)].ty) > 0);
        if !has_borrow_param {
            d = d.note_line(format!("`{f}` takes no borrows, so a borrow it returns can only point at something it owns"));
        }
        return d.help(help);
    }

    let kept_in_param = escapes.first().map(|(_, n)| n.clone());
    let (primary, borrow_label, help);
    if let Some(p) = &kept_in_param {
        let pname = p.split('.').next().unwrap_or(p).to_string();
        primary = format!("{} is dropped here, but `{pname}` keeps the borrow after `{f}` returns", quoted(&owner_text));
        borrow_label = format!("{} is borrowed here", quoted(owner));
        help = format!("store a copy: `*{pname} = copy {owner_text}`");
    } else {
        let end = if is_param || matches!(a.via, Via::ScopeEnd) && a.loc.block != BlockId(u32::MAX) && matches!(cx.body.blocks[a.loc.block].terminator.kind, TerminatorKind::Return) && a.loc.index as usize == cx.body.blocks[a.loc.block].statements.len() {
            format!("when `{f}` returns")
        } else {
            "at the end of its block".to_string()
        };
        primary = format!("{} is dropped here, {end}", quoted(&owner_text));
        borrow_label = match holder {
            Some(h) if h.contains('.') => format!("{} is borrowed here, and kept in `{h}`", quoted(owner)),
            _ => format!("{} is borrowed here", quoted(owner)),
        };
        help = if is_param || primary.ends_with("returns") {
            "return the value itself, not a borrow of it".to_string()
        } else {
            match holder {
                Some(h) => {
                    let h = h.split('.').next().unwrap_or(h);
                    format!("declare {} outside the block, so it lives as long as `{h}` is used", quoted(&owner_text))
                }
                None => format!("declare {} outside the block, so it lives as long as the borrow is used", quoted(&owner_text)),
            }
        };
    }
    let mut d = Diagnostic::at(&codes::E0306, a.span, format!("{} does not live long enough", quoted(&owner_text)))
        .note(primary)
        .label(loan.span, borrow_label);
    if let Some(l) = later {
        if l.kind != UseKind::Return {
            let text = match l.kind {
                UseKind::DropUse => "the borrow is still used here, when the holder is dropped".to_string(),
                _ => "the borrow is still used here".to_string(),
            };
            d = d.label(l.span, text);
        }
    }
    d.help(help)
}
