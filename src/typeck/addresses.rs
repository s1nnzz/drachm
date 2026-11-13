//! Address compares: `a same as b` and `xs::containsRef(&p)`.
//!
//! Two borrows are the same when they point at the same place. A list, an
//! array or an optional that holds borrows is the same as another when they
//! hold the same number of items and each pair is the same (the items in
//! order). Owned values have no address to compare, so `same as` on them is
//! E0633: borrow both sides with `&` to compare where they are.
//!
//! A list or optional is compared by a function the checker synthesises, as
//! for list `==` (`lists.rs`); a borrow itself is a plain pointer compare.

use super::optionals;
use super::{Checker, FuncId, Globals, Origin, Sig, Val};
use crate::ast;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, ExprKind as K};
use crate::types::Type;

/// The synthesised functions of this module, keyed by kind and type.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum AddrFunc {
    /// `(&T a, &T b) -> bool`: `T` is a list, an array or an optional of borrows.
    Same,
    /// `(&L a, E item) -> bool`: some item of the list `L` is at the address
    /// `item`, where `E` is the item type (a borrow, or an optional of one).
    ContainsRef,
}

/// Why a value cannot be compared by address.
pub(super) enum Refusal {
    /// No borrow inside: the value is owned, so it has no address to compare.
    Owned,
    /// A map, or a struct or enum, that holds borrows: it has no single address
    /// (the borrows inside are compared one at a time). Names what it is.
    Holds(String),
    /// Holds borrows in a form that is not built yet.
    Later(&'static str),
}

/// Does a value of `ty` hold a borrow, at any depth? (Structs and enums are
/// looked into, up to a few levels, so a recursive type ends.)
fn holds_borrow(g: &Globals, ty: Type, depth: usize) -> bool {
    if depth > 6 {
        return false;
    }
    match ty {
        Type::Ref { .. } => true,
        Type::List { elem } | Type::Array { elem, .. } => holds_borrow(g, *elem, depth + 1),
        Type::Map { key, val, .. } => holds_borrow(g, *key, depth + 1) || holds_borrow(g, *val, depth + 1),
        Type::Struct { id, .. } => g.structs[id].fields.iter().any(|f| f.ty.is_some_and(|t| holds_borrow(g, t, depth + 1))),
        Type::Enum { id, .. } => g.enums[id].variants.iter().any(|v| v.fields.iter().any(|f| f.ty.is_some_and(|t| holds_borrow(g, t, depth + 1)))),
        _ => false,
    }
}

/// Can values of `ty` be compared by address? `None` when they can.
pub(super) fn refusal(g: &Globals, ty: Type) -> Option<Refusal> {
    match ty {
        // A borrow of an interface is two words (data and vtable): not built yet.
        Type::Ref { to: Type::Dyn { .. }, .. } => Some(Refusal::Later("a borrow of an interface")),
        Type::Ref { .. } => None,
        Type::List { elem } | Type::Array { elem, .. } => refusal(g, *elem),
        Type::Map { .. } if holds_borrow(g, ty, 0) => Some(Refusal::Holds("a map of borrows".to_string())),
        Type::Struct { .. } | Type::Enum { .. } if holds_borrow(g, ty, 0) && optionals::inner_of(g, ty).is_none() => {
            Some(Refusal::Holds(format!("`{ty}`")))
        }
        _ => match optionals::inner_of(g, ty) {
            Some(inner) => refusal(g, inner),
            None => Some(Refusal::Owned),
        },
    }
}

/// The function of `kind` for `ty`, made now (its body is built later).
fn addr_func(g: &mut Globals, kind: AddrFunc, ty: Type) -> FuncId {
    if let Some(&f) = g.addr_funcs.get(&(kind, ty)) {
        return f;
    }
    let borrowed = Type::borrow(false, ty);
    let (name, second) = match kind {
        AddrFunc::Same => (format!("{ty} same as"), borrowed),
        AddrFunc::ContainsRef => (format!("{ty} containsRef"), ty.elem().unwrap_or(ty)),
    };
    let sig = Sig { name, params: vec![Some(borrowed), Some(second)], ret: Some(Type::Bool), span: Span::new(0, 0), fails: Vec::new(), outcome: None };
    let f = g.add_sig(sig);
    g.addr_funcs.insert((kind, ty), f);
    g.addr_pending.push((kind, ty, f));
    f
}

/// `a` and `b` are the same address, as a `bool`. Both have type `ty`, which
/// must not be refused (`refusal`). The operands are places or values: a list
/// or optional is lent to its function, so nothing is moved.
pub(super) fn same_address(g: &mut Globals, a: hir::Expr, b: hir::Expr, ty: Type) -> hir::Expr {
    if ty.is_borrow() {
        return hir::Expr::new(K::Binary(hir::BinOp::Eq, Box::new(a), Box::new(b)), Type::Bool);
    }
    let f = addr_func(g, AddrFunc::Same, ty);
    let borrow = |e: hir::Expr| hir::Expr::new(K::Borrow { mutable: false, place: Box::new(e) }, Type::borrow(false, ty));
    hir::Expr::new(K::Call(f, vec![borrow(a), borrow(b)]), Type::Bool)
}

/// The body of an address function (see `AddrFunc`), built once every other
/// function is checked. Hidden locals: `i`, the index of a list.
pub(super) fn addr_func_body(g: &mut Globals, kind: AddrFunc, ty: Type, f: FuncId) -> hir::Func {
    let none = Span::new(0, 0);
    let borrowed = Type::borrow(false, ty);
    let param = |name: &str, ty: Type| hir::Local { name: name.into(), ty, span: none, mutability: hir::Mutability::Fixed, is_param: true, written: None };
    let ret = |v: bool| hir::Stmt::Return(Some(hir::Expr::new(K::Bool(v), Type::Bool)));
    let whole = |l: usize| hir::Expr::new(K::Deref(Box::new(hir::Expr::new(K::Local(l), borrowed))), ty);
    let body = match (kind, ty.elem(), optionals::inner_of(g, ty)) {
        (AddrFunc::Same, Some(elem), _) => {
            // Same length, then each item the same address.
            let i = 2;
            let index = |e: hir::Expr| hir::Expr::new(K::Index(Box::new(e), Box::new(hir::Expr::new(K::Local(i), Type::U64))), elem);
            let len = |e: hir::Expr| hir::Expr::new(K::Len(Box::new(e)), Type::U64);
            let bin = |op, a, b, ty| hir::Expr::new(K::Binary(op, Box::new(a), Box::new(b)), ty);
            let lengths_differ = bin(hir::BinOp::Ne, len(whole(0)), len(whole(1)), Type::Bool);
            let differs = hir::Expr::new(K::Not(Box::new(same_address(g, index(whole(0)), index(whole(1)), elem))), Type::Bool);
            let step = vec![hir::Stmt::Assign(hir::Place::local(i), bin(hir::BinOp::Add, hir::Expr::new(K::Local(i), Type::U64), hir::Expr::new(K::Int(1), Type::U64), Type::U64))];
            let cond = bin(hir::BinOp::Lt, hir::Expr::new(K::Local(i), Type::U64), len(whole(0)), Type::Bool);
            vec![
                hir::Stmt::If { label: 0, branches: vec![hir::Branch { test: Some(lengths_differ), body: vec![ret(false)], end: none }] },
                hir::Stmt::Decl(i),
                hir::Stmt::Assign(hir::Place::local(i), hir::Expr::new(K::Int(0), Type::U64)),
                hir::Stmt::Loop {
                    label: 1,
                    cond: Some(cond),
                    body: vec![hir::Stmt::If { label: 2, branches: vec![hir::Branch { test: Some(differs), body: vec![ret(false)], end: none }] }],
                    step,
                    end: none,
                },
                ret(true),
            ]
        }
        (AddrFunc::Same, None, Some(inner)) => {
            // Both empty, or both holding the same address.
            let tag_ty = crate::mir::ty::tag_type(hir::tag_size_for(2));
            let tag = |e: hir::Expr| hir::Expr::new(K::Discriminant(Box::new(e)), tag_ty);
            let none_tag = || hir::Expr::new(K::Int(optionals::NONE as u128), tag_ty);
            let payload = |e: hir::Expr| hir::Expr::new(K::Payload { value: Box::new(e), variant: optionals::SOME, field: 0 }, inner);
            let (a, b) = (whole(0), whole(1));
            let tags_differ = hir::Expr::new(K::Binary(hir::BinOp::Ne, Box::new(tag(a.clone())), Box::new(tag(b.clone()))), Type::Bool);
            let both_none = hir::Expr::new(K::Binary(hir::BinOp::Eq, Box::new(tag(a.clone())), Box::new(none_tag())), Type::Bool);
            let same_payload = same_address(g, payload(a), payload(b), inner);
            vec![
                hir::Stmt::If { label: 0, branches: vec![hir::Branch { test: Some(tags_differ), body: vec![ret(false)], end: none }] },
                hir::Stmt::If { label: 1, branches: vec![hir::Branch { test: Some(both_none), body: vec![ret(true)], end: none }] },
                hir::Stmt::Return(Some(same_payload)),
            ]
        }
        (AddrFunc::ContainsRef, Some(elem), _) => {
            // Some item is at the address of the item argument.
            let i = 2;
            let index = |e: hir::Expr| hir::Expr::new(K::Index(Box::new(e), Box::new(hir::Expr::new(K::Local(i), Type::U64))), elem);
            let len = |e: hir::Expr| hir::Expr::new(K::Len(Box::new(e)), Type::U64);
            let bin = |op, a, b, ty| hir::Expr::new(K::Binary(op, Box::new(a), Box::new(b)), ty);
            let found = same_address(g, index(whole(0)), hir::Expr::new(K::Local(1), elem), elem);
            let cond = bin(hir::BinOp::Lt, hir::Expr::new(K::Local(i), Type::U64), len(whole(0)), Type::Bool);
            let step = vec![hir::Stmt::Assign(hir::Place::local(i), bin(hir::BinOp::Add, hir::Expr::new(K::Local(i), Type::U64), hir::Expr::new(K::Int(1), Type::U64), Type::U64))];
            vec![
                hir::Stmt::Decl(i),
                hir::Stmt::Assign(hir::Place::local(i), hir::Expr::new(K::Int(0), Type::U64)),
                hir::Stmt::Loop { label: 0, cond: Some(cond), body: vec![hir::Stmt::If { label: 1, branches: vec![hir::Branch { test: Some(found), body: vec![ret(true)], end: none }] }], step, end: none },
                ret(false),
            ]
        }
        _ => unreachable!("an address function is made for a list, an array or an optional"),
    };
    let mut locals = vec![param("a", borrowed), match kind {
        AddrFunc::Same => param("b", borrowed),
        AddrFunc::ContainsRef => param("item", ty.elem().unwrap_or(ty)),
    }];
    locals.push(hir::Local { name: "i".into(), ty: Type::U64, span: none, mutability: hir::Mutability::Mut, is_param: false, written: None });
    hir::Func {
        symbol: format!("drachm_addr_{f}"),
        params: 2,
        has_self: false,
        locals,
        ret: Type::Bool,
        body,
        name_span: none,
        span: none,
        body_end: none,
        is_unsafe: false,
        lives: None,
        lives_span: None,
    }
}

impl Checker<'_> {
    /// `xs::containsRef(&p)`: some item of the list `xs` is at the address `&p`.
    /// The items must be borrows (or optionals of them); the argument is one.
    pub(super) fn list_contains_ref(&mut self, v: hir::Expr, name_span: Span, args: &[ast::Expr]) -> Option<Val> {
        let ty = v.ty;
        let elem = ty.elem()?;
        let [arg] = args else {
            self.diags.push(
                Diagnostic::at(&codes::E0103, name_span, format!("`containsRef` takes 1 argument, but {} given", args.len()))
                    .note("takes 1 argument"),
            );
            for a in args {
                self.expr(a);
            }
            return None;
        };
        if let Some(bad) = self.addr_refusal(elem, name_span, "`containsRef`") {
            for a in args {
                self.expr(a);
            }
            return bad;
        }
        let item = self.coerce(arg, elem)?;
        let f = addr_func(self.g, AddrFunc::ContainsRef, ty);
        let borrow = hir::Expr::new(K::Borrow { mutable: false, place: Box::new(v) }, Type::borrow(false, ty));
        Some(Val::Expr(hir::Expr::new(K::Call(f, vec![borrow, item]), Type::Bool)))
    }

    /// `a same as b` (and `a not same as b`): the two sides must be borrows, or hold borrows, and of one type.
    /// `shown` is the operator as written (`same as` or `not same as`), for the messages.
    pub(super) fn same_as(&mut self, l: &ast::Expr, r: &ast::Expr, span: Span, shown: &str) -> Option<Val> {
        // The borrows themselves, not what they point at (`expr` reads through them).
        let lv = self.expr_raw(l)?;
        let rv = self.expr_raw(r)?;
        let lo = Origin { span: l.span, compound: false };
        let ro = Origin { span: r.span, compound: false };
        let lhs = self.materialise(lv, l.span)?;
        if let Some(bad) = self.addr_refusal(lhs.ty, lo.span.to(span), &format!("`{shown}`")) {
            return bad;
        }
        if let Val::Expr(x) = &rv
            && let Some(bad) = self.addr_refusal(x.ty, ro.span, &format!("`{shown}`"))
        {
            return bad;
        }
        let rhs = self.into(rv, lhs.ty, ro)?;
        let ty = lhs.ty;
        Some(Val::Expr(same_address(self.g, lhs, rhs, ty)))
    }

    /// The error when `ty` cannot be compared by address: E0633 for an owned
    /// value, E0901 for a form not built yet. `None` when it can be compared;
    /// `Some(None)` after an error (the check ends there). `shown` names the
    /// operation in the message.
    fn addr_refusal(&mut self, ty: Type, span: Span, shown: &str) -> Option<Option<Val>> {
        let why = refusal(self.g, ty)?;
        match why {
            Refusal::Owned => {
                let help = if shown.ends_with("same as`") {
                    "borrow both sides with `&` to compare where they are: `&a same as &b`; `==` compares the values".to_string()
                } else {
                    "`contains` looks for the value itself".to_string()
                };
                self.diags.push(
                    Diagnostic::at(&codes::E0633, span, format!("{shown} compares addresses, and `{ty}` is not a borrow"))
                        .note(format!("`{ty}` is an owned value, so it has no address to compare"))
                        .help(help),
                );
            }
            Refusal::Holds(what) => {
                let help = if shown.ends_with("same as`") {
                    "compare the borrows inside it one at a time, with `same as` on each".to_string()
                } else {
                    "`containsRef` takes a borrow to look for, not a value that holds borrows".to_string()
                };
                self.diags.push(
                    Diagnostic::at(&codes::E0633, span, format!("{shown} compares addresses, and {what} has no single address"))
                        .note("it is not compared as one address; `==` compares the values")
                        .help(help),
                );
            }
            Refusal::Later(what) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, span, format!("address compare of {what} is not supported yet"))
                        .help("compare the borrows one at a time with `same as`"),
                );
            }
        }
        Some(None)
    }
}
