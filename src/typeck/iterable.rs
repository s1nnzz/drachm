//! `for (T x of thing)` over a value of your own type that is `Iterable<T>`
//! (S6G). `interface Iterable<T> { T? at(uint64 i); }` is built in; the loop
//! calls `thing::at(0)`, `at(1)`, ... until one gives `none`. The value is
//! only looked at, so it can be walked again. Lowered to
//!
//! ```text
//! owned = thing;                  // only when `thing` is not a place
//! hidden = &owned;                // a real shared borrow: the body cannot change it (E0302)
//! index = 0;
//! loop {
//!     item = (*hidden)::at(index);
//!     if (item is empty) break;
//!     name = payload of item;     // a copy, a move, or `&T`: a view of it
//!     body                        // step: index = index + 1, so `continue` is safe
//! }
//! ```
//!
//! A type may be `Iterable` over several item types; the variable's type picks
//! the `at` (the one the conformance record for that `Iterable<T>` found).

use std::collections::HashMap;

use super::interfaces::item_of;
use super::{Checker, optionals};
use crate::ast;
use crate::diag::{Diagnostic, Span, codes};
use crate::hir::{self, ExprKind as K};
use crate::types::Type;

/// How a loop over an `Iterable` binds each item.
#[derive(Clone, Copy)]
enum Bind {
    /// `T x` where `T` is a Copy type: a plain read.
    Read,
    /// `T x` where `T` owns memory: the item moves into `x`.
    Take,
    /// `&T x`: a view of the item, which lives for this round.
    View,
}

impl Checker<'_> {
    pub(super) fn for_iterable(&mut self, s: &ast::Stmt, ty: Option<Type>, thing: hir::Expr, iter_span: Span, out: &mut hir::Block) {
        let ast::Stmt::ForEach { mutable, ty: ty_name, name, name_span, body, span, .. } = s else { unreachable!() };
        let thing_ty = thing.ty;
        let cands = self.g.ifaces.iterable.get(&thing_ty).cloned().unwrap_or_default();
        let items: Vec<Type> = cands.iter().map(|&(k, _)| item_of(self.g, k)).collect();
        let mut pick: Option<(usize, Bind)> = None;
        if let Some(t) = ty {
            if let Some(i) = items.iter().position(|&i| i == t) {
                let owning = !self.copies_anyway(items[i]);
                pick = Some((i, if owning { Bind::Take } else { Bind::Read }));
            } else if let Some(i) = items.iter().position(|&i| Type::borrow(false, i) == t) {
                pick = Some((i, Bind::View));
            } else if let Some(i) = items.iter().position(|&i| Type::borrow(true, i) == t) {
                self.diags.push(
                    Diagnostic::at(&codes::E0901, ty_name.span, format!("a changeable view `{t}` of items from `at` is not supported"))
                        .note(format!("`at` gives each `{}` as a value, so changes would be lost", items[i]))
                        .help(format!("write `{}` for the item itself, or `&{}` to look at it", items[i], items[i])),
                );
            } else if let [only] = &items[..] {
                self.diags.push(
                    Diagnostic::at(&codes::E0630, ty_name.span, format!("the items are `{only}`, but the variable is `{t}`"))
                        .label(iter_span, format!("this is `Iterable<{only}>`"))
                        .note(format!("this is `{t}`"))
                        .help(format!("write `{only}` for each item, or `&{only}` to look at each one")),
                );
            } else {
                let list: Vec<String> = items.iter().map(|i| format!("`{i}`")).collect();
                self.diags.push(
                    Diagnostic::at(&codes::E0631, ty_name.span, format!("`{thing_ty}` cannot be walked as `{t}`: it is `Iterable` over {}", list.join(" and ")))
                        .label(iter_span, format!("this is a `{thing_ty}`"))
                        .note(format!("this is `{t}`"))
                        .help(format!("the variable's type picks the items: write {}", list.join(" or "))),
                );
            }
        }
        self.scopes.push(HashMap::new());
        let var = self.declare(name, ty, *mutable, *name_span, false);
        let label = self.new_label();
        let body_end = body.end;
        let (body, _) = self.looped(label, *span, body, None);
        self.scopes.pop();

        let Some((which, bind)) = pick else { return };
        let (_, func) = cands[which];
        let item = items[which];
        let Some(opt) = self.g.sigs[func].ret else { return };
        let mut thing = thing;
        if !thing.is_place() {
            let owned = self.hidden_named("for value", thing_ty);
            out.push(hir::Stmt::Decl(owned));
            out.push(hir::Stmt::Assign(hir::Place::local(owned), thing));
            thing = hir::Expr::new(K::Local(owned), thing_ty);
        }
        let borrow_ty = Type::borrow(false, thing_ty);
        let hidden = self.hidden_named("for value borrow", borrow_ty);
        let index = self.hidden_named("for index", Type::U64);
        out.push(hir::Stmt::Assign(hir::Place::local(hidden), hir::Expr::new(K::Borrow { mutable: false, place: Box::new(thing) }, borrow_ty)));
        out.push(hir::Stmt::Assign(hir::Place::local(index), hir::Expr::new(K::Int(0), Type::U64)));

        let here = || hir::Expr::new(K::Local(index), Type::U64);
        let whole = hir::Expr::new(K::Deref(Box::new(hir::Expr::new(K::Local(hidden), borrow_ty))), thing_ty);
        let slot = self.hidden_named("for item", opt);
        let en = opt.enum_id().expect("an optional is an enum");
        let slot_expr = || hir::Expr::new(K::Local(slot), opt);
        let payload = || hir::Expr::new(K::Payload { value: Box::new(slot_expr()), variant: optionals::SOME, field: 0 }, item);
        let mut inner = vec![
            hir::Stmt::Decl(slot),
            hir::Stmt::Assign(hir::Place::local(slot), hir::Expr::new(K::Call(func, vec![whole, here()]), opt)),
            hir::Stmt::If {
                label: self.new_label(),
                branches: vec![hir::Branch {
                    test: Some(self.tag_test(en, optionals::NONE, &slot_expr())),
                    body: vec![hir::Stmt::Jump(hir::Jump::Break(label))],
                    end: Span::new(0, 0),
                }],
            },
            hir::Stmt::Decl(var),
        ];
        match bind {
            Bind::Read => inner.push(hir::Stmt::Assign(hir::Place::local(var), payload())),
            Bind::Take => {
                let place = hir::Place { local: slot, proj: vec![hir::HirProj::Downcast(optionals::SOME), hir::HirProj::Field(0)], span: Span::new(0, 0) };
                inner.push(hir::Stmt::Assign(hir::Place::local(var), hir::Expr::new(K::Take { place, leaving: None }, item)));
                inner.push(hir::Stmt::Forget(hir::Place::local(slot)));
            }
            Bind::View => {
                let view = Type::borrow(false, item);
                inner.push(hir::Stmt::Assign(hir::Place::local(var), hir::Expr::new(K::Borrow { mutable: false, place: Box::new(payload()) }, view)));
            }
        }
        inner.extend(body);
        let step = vec![hir::Stmt::Assign(
            hir::Place::local(index),
            hir::Expr::new(K::Binary(hir::BinOp::Add, Box::new(here()), Box::new(hir::Expr::new(K::Int(1), Type::U64))), Type::U64),
        )];
        out.push(hir::Stmt::Loop { label, cond: None, body: inner, step, end: body_end });
    }
}
