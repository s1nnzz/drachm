//! Function types and callable values (S6F).

use super::{Checker, Globals, Sig, Val};
use crate::ast;
use crate::diag::{Diagnostic, Span, codes};
use crate::hir::{self, ExprKind as K};
use crate::types::{FuncSig, Type};

#[derive(Default)]
pub(super) struct State {
    pub bodies: Vec<(hir::FuncId, hir::Func)>,
    thunks: std::collections::HashMap<hir::FuncId, hir::FuncId>,
    pub unsafe_named: std::collections::HashSet<hir::FuncId>,
}

impl Globals {
    pub(super) fn resolve_func(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let f = t.func.as_ref()?;
        if !t.mods.is_empty() {
            diags.push(Diagnostic::at(&codes::E0652, t.span, "a function value cannot be stored behind a borrow or pointer"));
            return None;
        }
        let params = f.params.iter().map(|p| self.resolve_binding_type(p, diags)).collect::<Option<Vec<_>>>()?;
        let ret = self.resolve_return(&f.ret, diags)?;
        if self.holds_borrow(ret) {
            diags.push(Diagnostic::at(&codes::E0901, f.ret.span, "a function value returning a borrow is not supported yet"));
            return None;
        }
        Some(Type::func(params, ret))
    }

    pub(super) fn resolve_return(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let ty = self.resolve(t, diags)?;
        if ty.is_func() {
            diags.push(Diagnostic::at(&codes::E0652, t.span, "a function value cannot be returned"));
            return None;
        }
        Some(ty)
    }

    pub(super) fn resolve_storable(&self, t: &ast::TypeName, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let ty = self.resolve_binding_type(t, diags)?;
        if ty.is_func() {
            diags.push(Diagnostic::at(&codes::E0652, t.span, "a function value cannot be stored in another value"));
            return None;
        }
        Some(ty)
    }

    fn named_thunk(&mut self, id: hir::FuncId, sig: &FuncSig) -> hir::FuncId {
        if let Some(&thunk) = self.lambdas.thunks.get(&id) {
            return thunk;
        }
        let span = self.sigs[id].span;
        let params: Vec<_> = std::iter::once(Type::borrow(false, Type::U8)).chain(sig.params.iter().copied()).collect();
        let thunk = self.add_sig(Sig {
            name: self.sigs[id].name.clone(),
            params: params.iter().copied().map(Some).collect(),
            ret: Some(sig.ret),
            span,
            fails: Vec::new(),
            outcome: None,
        });
        let locals = params.iter().enumerate().map(|(i, &ty)| hir::Local {
            name: format!("_arg{i}"),
            ty,
            span,
            mutability: hir::Mutability::Fixed,
            is_param: true,
            written: None,
        }).collect();
        let args = sig.params.iter().enumerate().map(|(i, &ty)| hir::Expr::new(K::Local(i + 1), ty).at(span)).collect();
        let call = hir::Expr::new(K::Call(id, args), sig.ret).at(span);
        let body = if sig.ret == Type::None { vec![hir::Stmt::Expr(call)] } else { vec![hir::Stmt::Return(Some(call))] };
        self.lambdas.bodies.push((thunk, hir::Func {
            symbol: format!("drachm_fn_thunk_{id}"),
            params: params.len(),
            has_self: false,
            locals,
            ret: sig.ret,
            body,
            name_span: span,
            span,
            body_end: span,
            is_unsafe: false,
            lives: None,
            lives_span: None,
        }));
        self.lambdas.thunks.insert(id, thunk);
        thunk
    }
}

impl Checker<'_> {
    pub(super) fn coerce_func(&mut self, e: &ast::Expr, to: Type, want: &FuncSig) -> Option<hir::Expr> {
        if let ast::ExprKind::Var(name) = &e.kind
            && self.lookup(name).is_none()
            && let Some(&id) = self.g.funcs_by_name.get(name)
        {
            let sig = &self.g.sigs[id];
            if sig.outcome.is_some() || self.g.lambdas.unsafe_named.contains(&id) {
                self.diags.push(Diagnostic::at(&codes::E0901, e.span, "a failing or unsafe function cannot be used as a function value yet"));
                return None;
            }
            let params: Vec<_> = sig.params.iter().copied().collect::<Option<_>>()?;
            self.match_func(&params, sig.ret?, want, e.span)?;
            let thunk = self.g.named_thunk(id, want);
            return Some(hir::Expr::new(K::FuncRef(thunk), to).at(e.span));
        }
        let v = self.expr(e)?;
        if let Val::Expr(x) = &v
            && let Type::Func(have) = x.ty
        {
            self.match_func(&have.params, have.ret, want, e.span)?;
        }
        self.into(v, to, super::origin(e))
    }

    fn match_func(&mut self, params: &[Type], ret: Type, want: &FuncSig, at: Span) -> Option<()> {
        if params != want.params {
            let show = |ts: &[Type]| ts.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ");
            self.diags.push(Diagnostic::at(&codes::E0650, at, format!("this function takes `({})`, but `({})` is wanted", show(params), show(&want.params))));
            return None;
        }
        if ret != want.ret {
            self.diags.push(Diagnostic::at(&codes::E0651, at, format!("this function gives `{ret}`, but `{}` is wanted", want.ret)));
            return None;
        }
        Some(())
    }

    pub(super) fn call_value(&mut self, id: hir::LocalId, name: &str, targs: &[ast::TypeName], args: &[ast::Expr], span: Span) -> Option<Val> {
        let f = self.read_local(id, span)?;
        let Type::Func(sig) = f.ty else {
            self.diags.push(Diagnostic::at(&codes::E0654, span, format!("`{name}` is a `{}` and cannot be called", f.ty)));
            return None;
        };
        self.no_targs(name, targs);
        if args.len() != sig.params.len() {
            self.diags.push(Diagnostic::at(&codes::E0650, span, format!("`{name}` takes {} arguments, but {} given", sig.params.len(), args.len())));
            return None;
        }
        let mut values = vec![f];
        for (a, &p) in args.iter().zip(&sig.params) {
            values.push(self.coerce(a, p)?);
        }
        Some(Val::Expr(hir::Expr::new(K::FnCall(values), sig.ret).at(span)))
    }
}
