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
            self.diags.push(Diagnostic::at(&codes::E0650, span, format!("`{name}` takes {} argument{}, but {} given", sig.params.len(), if sig.params.len() == 1 { "" } else { "s" }, args.len())));
            return None;
        }
        let mut values = vec![f];
        for (a, &p) in args.iter().zip(&sig.params) {
            values.push(self.coerce(a, p)?);
        }
        Some(Val::Expr(hir::Expr::new(K::FnCall(values), sig.ret).at(span)))
    }
}

pub(super) struct Frame {
    enclosing: std::collections::HashSet<String>,
    returns: Option<(Type, Span)>,
}

impl Checker<'_> {
    pub(super) fn unbuilt_capture(&mut self, name: &str, at: Span) -> bool {
        if self.lambda.as_ref().is_some_and(|f| f.enclosing.contains(name)) {
            self.diags.push(Diagnostic::at(&codes::E0901, at, format!("capturing `{name}` in a lambda is not supported yet")));
            return true;
        }
        false
    }

    pub(super) fn lambda_return(&mut self, value: Option<&ast::Expr>, at: Span) -> Option<Option<hir::Expr>> {
        let expr = match value {
            Some(e) => {
                let v = if matches!(e.kind, ast::ExprKind::Borrow { .. } | ast::ExprKind::Lend(_)) {
                    self.expr_raw(e)?
                } else {
                    self.expr(e)?
                };
                Some(self.materialise(v, e.span)?)
            }
            None => None,
        };
        let ty = expr.as_ref().map_or(Type::None, |e| e.ty);
        if ty.is_func() {
            self.diags.push(Diagnostic::at(&codes::E0652, at, "a function value cannot be returned"));
            return None;
        }
        if self.g.holds_borrow(ty) {
            self.diags.push(Diagnostic::at(&codes::E0901, at, "a lambda returning a borrow is not supported yet"));
            return None;
        }
        let frame = self.lambda.as_mut().expect("checking a lambda");
        if let Some((first, span)) = frame.returns {
            if first != ty {
                self.diags.push(
                    Diagnostic::at(&codes::E0651, at, format!("this lambda gives `{ty}` here"))
                        .label(span, format!("another return gives `{first}`")),
                );
                return None;
            }
        } else {
            frame.returns = Some((ty, at));
        }
        Some(expr)
    }

    pub(super) fn lambda_expr(&mut self, _moves: bool, params: &[ast::Param], body: &ast::LambdaBody, at: Span) -> Option<hir::Expr> {
        let types: Vec<_> = params.iter().map(|p| self.g.resolve_binding_type(&p.ty, self.diags)).collect::<Option<_>>()?;
        let id = self.g.add_sig(Sig {
            name: "lambda".into(),
            params: Vec::new(),
            ret: None,
            span: at,
            fails: Vec::new(),
            outcome: None,
        });
        let symbol = format!("drachm_l_{}_{id}", self.symbol.trim_start_matches("drachm_"));
        let mut enclosing = self.lambda.as_ref().map(|f| f.enclosing.clone()).unwrap_or_default();
        enclosing.extend(self.scopes.iter().flat_map(|s| s.keys().cloned()));
        let mut child = Checker::new(self.g, self.src, self.diags, &symbol, None, self.method);
        child.lambda = Some(Frame { enclosing, returns: None });
        child.declare("_env", Some(Type::borrow(false, Type::U8)), false, Span::new(0, 0), true);
        for (p, &ty) in params.iter().zip(&types) {
            let local = child.declare(&p.name, Some(ty), p.mutable, p.span, true);
            child.locals[local].open = p.open;
        }
        let (body, end) = match body {
            ast::LambdaBody::Expr(e) => {
                let value = child.lambda_return(Some(e), e.span)?;
                let stmt = match value {
                    Some(v) if v.ty == Type::None => hir::Stmt::Expr(v),
                    v => hir::Stmt::Return(v),
                };
                (vec![stmt], e.span)
            }
            ast::LambdaBody::Block(b) => (child.block(b), b.end),
        };
        let ret = child.lambda.as_ref().and_then(|f| f.returns).map_or(Type::None, |(ty, _)| ty);
        let locals = child.lower_locals();
        child.g.sigs[id].ret = Some(ret);
        child.g.lambdas.bodies.push((id, hir::Func {
            symbol: symbol.clone(),
            params: 1 + params.len(),
            has_self: false,
            locals,
            ret,
            body,
            name_span: at,
            span: at,
            body_end: end,
            is_unsafe: false,
            lives: None,
            lives_span: None,
        }));
        Some(hir::Expr::new(K::FuncRef(id), Type::func(types, ret)).at(at))
    }
}
