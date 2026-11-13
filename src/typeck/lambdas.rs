//! Function types and callable values (S6F).

use std::cell::Cell;
use std::collections::{BTreeMap, HashMap};

use super::{Checker, FieldInfo, Globals, Sig, StructInfo, Val};
use crate::ast;
use crate::diag::{Diagnostic, Span, codes};
use crate::hir::{self, ExprKind as K};
use crate::types::{FuncSig, Type};

#[derive(Default)]
pub(super) struct State {
    pub bodies: Vec<(hir::FuncId, hir::Func)>,
    pub closures: BTreeMap<hir::FuncId, hir::Closure>,
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
        let v = self.func_value(e)?;
        if let Val::Expr(x) = &v
            && let Type::Func(have) = x.ty
        {
            self.match_func(&have.params, have.ret, want, e.span)?;
        }
        self.into(v, to, super::origin(e))
    }

    /// In a callback context a plain function name can be used as a value.
    pub(super) fn func_value(&mut self, e: &ast::Expr) -> Option<Val> {
        if let ast::ExprKind::Var(name) = &e.kind
            && self.lookup(name).is_none()
            && let Some(&id) = self.g.funcs_by_name.get(name)
        {
            let sig = &self.g.sigs[id];
            if sig.outcome.is_some() || self.g.lambdas.unsafe_named.contains(&id) {
                self.diags.push(Diagnostic::at(&codes::E0901, e.span, "a failing or unsafe function cannot be used as a function value yet"));
                return None;
            }
            let params = sig.params.iter().copied().collect::<Option<_>>()?;
            let ret = sig.ret?;
            if self.g.holds_borrow(ret) {
                self.diags.push(Diagnostic::at(&codes::E0901, e.span, "a function value returning a borrow is not supported yet"));
                return None;
            }
            let ty = Type::func(params, ret);
            let Type::Func(sig) = ty else { unreachable!() };
            let thunk = self.g.named_thunk(id, sig);
            return Some(Val::Expr(hir::Expr::new(K::FuncRef(thunk), ty).at(e.span)));
        }
        self.expr(e)
    }

    pub(super) fn each_callback(&mut self, arg: &ast::Expr, elem: Type) -> Option<hir::Expr> {
        let value = self.func_value(arg)?;
        let callback = self.materialise(value, arg.span)?;
        let shared = Type::borrow(false, elem);
        let bad_shape = || Diagnostic::at(&codes::E0650, arg.span, format!("`each` wants a callback taking `{elem}` or `{shared}`"))
            .note(format!("this has type `{}`", callback.ty));
        let Type::Func(sig) = callback.ty else {
            self.diags.push(bad_shape());
            return None;
        };
        if sig.params == [Type::borrow(true, elem)] {
            self.diags.push(Diagnostic::at(&codes::E0901, arg.span, "a mutable item callback for `each` is not supported yet")
                .help(format!("use `for (&mut {elem} item of items)` for mutable item access")));
            return None;
        }
        if sig.params != [elem] && sig.params != [shared] {
            self.diags.push(bad_shape());
            return None;
        }
        if sig.ret != Type::None {
            self.diags.push(Diagnostic::at(&codes::E0651, arg.span, format!("this function gives `{}`, but `each` wants `none`", sig.ret)));
            return None;
        }
        Some(callback)
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
    /// Local ids of the aliases form a contiguous range after the parameters.
    alias_start: hir::LocalId,
    aliases: Vec<Alias>,
    returns: Option<(Type, Span)>,
}

struct Alias {
    parent: hir::LocalId,
    used: Cell<bool>,
}

impl Checker<'_> {
    fn capture_alias(&self, id: hir::LocalId) -> Option<&Alias> {
        let frame = self.lambda.as_ref()?;
        frame.aliases.get(id.checked_sub(frame.alias_start)?)
    }

    pub(super) fn is_capture(&self, id: hir::LocalId) -> bool {
        self.capture_alias(id).is_some()
    }

    pub(super) fn use_capture(&self, id: hir::LocalId) {
        if let Some(alias) = self.capture_alias(id) {
            alias.used.set(true);
        }
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

    pub(super) fn lambda_expr(&mut self, moves: bool, params: &[ast::Param], body: &ast::LambdaBody, at: Span) -> Option<hir::Expr> {
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
        // Snapshot the names in scope at the literal. Parameters and body
        // bindings can shadow these aliases without recording a capture.
        let visible: HashMap<_, _> = self.scopes.iter().flat_map(|scope| scope.iter()).map(|(name, &id)| (name, id)).collect();
        let mut visible: Vec<_> = visible.into_values().collect();
        visible.sort_unstable();
        let mut child = Checker::new(self.g, self.src, self.diags, &symbol, None, self.method);
        child.declare("$env", Some(Type::borrow(false, Type::U8)), false, Span::new(0, 0), true);
        child.scopes[0].remove("$env");
        for (p, &ty) in params.iter().zip(&types) {
            let local = child.declare(&p.name, Some(ty), p.mutable, p.span, true);
            child.locals[local].open = p.open;
        }
        let alias_start = child.locals.len();
        let mut scope = HashMap::new();
        let mut aliases = Vec::new();
        for parent in visible {
            let outer = &self.locals[parent];
            scope.insert(outer.name.clone(), child.locals.len());
            let mut local = outer.clone();
            local.is_param = false;
            child.locals.push(local);
            aliases.push(Alias { parent, used: Cell::new(false) });
        }
        child.scopes.insert(0, scope);
        child.lambda = Some(Frame { alias_start, aliases, returns: None });
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
        let frame = child.lambda.take().expect("checking a lambda");
        let ret = frame.returns.map_or(Type::None, |(ty, _)| ty);
        let mut locals = child.lower_locals();
        let mut captures = Vec::new();
        let mut values = Vec::new();
        for (i, alias) in frame.aliases.into_iter().enumerate() {
            let local = frame.alias_start + i;
            let ty = locals[local].ty;
            // An alias uses its environment field in MIR, so it needs no slot
            // of its own. An unused alias can be erased entirely from MIR.
            if !alias.used.get() {
                locals[local].ty = Type::None;
                continue;
            }
            if ty.is_func() {
                child.diags.push(Diagnostic::at(&codes::E0901, at, "capturing a function value is not supported yet"));
                return None;
            }
            if self.filling.contains(&alias.parent) {
                child.diags.push(Diagnostic::at(&codes::E0901, at, "capturing a binding while it is being filled is not supported yet"));
                return None;
            }
            let enclosing_self = self.locals[alias.parent].is_self;
            if (moves && enclosing_self) || (child.g.holds_borrow(ty) && !(moves && ty.is_borrow())) {
                child.diags.push(
                    Diagnostic::at(&codes::E0901, at, format!("capturing `{}` this way is not supported yet", locals[local].name))
                        .label(locals[local].span, format!("this binding has type `{ty}`")),
                );
                return None;
            }
            if let Some(f) = &self.lambda
                && let Some(outer_alias) = alias.parent.checked_sub(f.alias_start).and_then(|i| f.aliases.get(i))
            {
                outer_alias.used.set(true);
            }
            captures.push(hir::Capture { local, borrow: (!moves).then_some(false) });
            values.push(hir::Expr::new(K::Local(alias.parent), ty).at(at));
        }
        let expr = if captures.is_empty() {
            K::FuncRef(id)
        } else {
            let env = child.g.closure_env(id, &captures, &locals, at);
            child.g.lambdas.closures.insert(id, hir::Closure { env, captures });
            locals[0].ty = Type::borrow(true, env);
            K::Closure(id, values)
        };
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
        Some(hir::Expr::new(expr, Type::func(types, ret)).at(at))
    }
}


impl Globals {
    fn closure_env(&mut self, code: hir::FuncId, captures: &[hir::Capture], locals: &[hir::Local], at: Span) -> Type {
        let id = self.structs.len();
        let name = format!("#env{code}");
        let ty = Type::Struct { id, name: crate::types::intern(&name) };
        let fields: Vec<_> = captures.iter().enumerate().map(|(i, c)| {
            let local = &locals[c.local];
            let ty = c.borrow.map_or(local.ty, |mutable| Type::borrow(mutable, local.ty));
            FieldInfo { name: format!("capture{i}"), ty: Some(ty), mutable: false, public: false, span: local.span }
        }).collect();
        let lives = fields.iter().enumerate().filter_map(|(i, field)| field.ty.filter(|t| t.is_borrow()).map(|_| i)).collect();
        self.structs.push(StructInfo {
            name,
            name_span: at,
            ty,
            fields,
            methods: Vec::new(),
            stays: true,
            lives,
            has_drop: None,
            ctors: Vec::new(),
            auto_ctor: false,
            default_def: None,
            declared_lives: false,
            placeholder: None,
        });
        ty
    }
}

/// Capturing a place mutably is itself a change in the enclosing body. That
/// can make a method or a nested closure require mutable access in turn.
pub(super) fn infer_captures(program: &mut hir::Program, diags: &mut Vec<Diagnostic>) {
    use crate::ownership::self_mode::{changes_local, infer};
    if program.closures.is_empty() {
        return;
    }
    loop {
        let modes = infer(program);
        let mut changed = Vec::new();
        for (&code, closure) in &program.closures {
            for (i, capture) in closure.captures.iter().enumerate() {
                if capture.borrow == Some(false) && changes_local(program, code, capture.local, &modes) {
                    changed.push((code, i));
                }
            }
        }
        if changed.is_empty() {
            for (&code, closure) in &program.closures {
                let f = &program.funcs[code];
                for capture in &closure.captures {
                    let local = &f.locals[capture.local];
                    if capture.borrow.is_none()
                        && local.mutability == hir::Mutability::Fixed
                        && !matches!(local.ty, Type::Ref { mutable: true, .. })
                        && changes_local(program, code, capture.local, &modes)
                    {
                        diags.push(
                            Diagnostic::at(&codes::E0901, f.name_span, format!("changing the fixed binding `{}` captured by `move` is not supported yet", local.name))
                                .label(local.span, "this binding is fixed"),
                        );
                    }
                }
            }
            break;
        }
        for (code, field) in changed {
            let closure = program.closures.get_mut(&code).unwrap();
            closure.captures[field].borrow = Some(true);
            let env = closure.env.struct_id().unwrap();
            let f = &mut program.structs[env].fields[field];
            f.ty = Type::borrow(true, f.ty.target().unwrap());
        }
    }
}
