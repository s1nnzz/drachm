use std::collections::HashMap;

use crate::ast::*;
use crate::error::{Error, Span};

pub struct Sig {
    pub params: Vec<Type>,
    pub ret: Type,
}

/// Builtins implemented in the C runtime. `print` is overloaded on i64/bool,
/// so it is checked specially rather than listed here.
pub const BUILTINS: &[&str] = &["print"];

pub fn check(prog: &Program) -> Result<HashMap<String, Sig>, Error> {
    let mut sigs = HashMap::new();
    for f in &prog.funcs {
        if BUILTINS.contains(&f.name.as_str()) {
            return Err(Error::new(f.span, format!("'{}' is a builtin and cannot be redefined", f.name)));
        }
        let sig = Sig { params: f.params.iter().map(|p| p.1).collect(), ret: f.ret };
        if sigs.insert(f.name.clone(), sig).is_some() {
            return Err(Error::new(f.span, format!("function '{}' is defined twice", f.name)));
        }
    }

    match sigs.get("main") {
        None => return Err(Error::new(Span::new(0, 0), "no 'main' function")),
        Some(s) if !s.params.is_empty() || !matches!(s.ret, Type::I64 | Type::Unit) => {
            let f = prog.funcs.iter().find(|f| f.name == "main").unwrap();
            return Err(Error::new(f.span, "'main' must be 'fn main()' or 'fn main() -> i64'"));
        }
        _ => {}
    }

    for f in &prog.funcs {
        let mut cx = Checker { sigs: &sigs, scopes: vec![HashMap::new()], ret: f.ret };
        for (name, ty) in &f.params {
            cx.scopes[0].insert(name.clone(), (*ty, false));
        }
        cx.block(&f.body)?;
        if f.ret != Type::Unit && !returns(&f.body) {
            return Err(Error::new(f.span, format!("function '{}' may finish without returning a {}", f.name, f.ret)));
        }
    }
    Ok(sigs)
}

/// Does every path through this block hit a `return`?
fn returns(block: &Block) -> bool {
    block.iter().any(|s| match s {
        Stmt::Return { .. } => true,
        Stmt::If { then, els: Some(els), .. } => returns(then) && returns(els),
        _ => false,
    })
}

struct Checker<'a> {
    sigs: &'a HashMap<String, Sig>,
    /// name -> (type, mutable)
    scopes: Vec<HashMap<String, (Type, bool)>>,
    ret: Type,
}

impl Checker<'_> {
    fn lookup(&self, name: &str) -> Option<(Type, bool)> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn block(&mut self, b: &Block) -> Result<(), Error> {
        self.scopes.push(HashMap::new());
        let r = b.iter().try_for_each(|s| self.stmt(s));
        self.scopes.pop();
        r
    }

    fn expect(&mut self, e: &Expr, want: Type) -> Result<(), Error> {
        let got = self.expr(e)?;
        if got != want {
            return Err(Error::new(e.span, format!("expected {want}, found {got}")));
        }
        Ok(())
    }

    fn stmt(&mut self, s: &Stmt) -> Result<(), Error> {
        match s {
            Stmt::Let { name, ty, mutable, value, span } => {
                if *ty == Type::Unit {
                    return Err(Error::new(*span, "variables cannot have type ()"));
                }
                self.expect(value, *ty)?;
                self.scopes.last_mut().unwrap().insert(name.clone(), (*ty, *mutable));
            }
            Stmt::Assign { name, value, span } => {
                let Some((ty, mutable)) = self.lookup(name) else {
                    return Err(Error::new(*span, format!("unknown variable '{name}'")));
                };
                if !mutable {
                    return Err(Error::new(*span, format!("'{name}' is immutable; declare it with 'var' to allow assignment")));
                }
                self.expect(value, ty)?;
            }
            Stmt::If { cond, then, els } => {
                self.expect(cond, Type::Bool)?;
                self.block(then)?;
                if let Some(els) = els {
                    self.block(els)?;
                }
            }
            Stmt::While { cond, body } => {
                self.expect(cond, Type::Bool)?;
                self.block(body)?;
            }
            Stmt::Return { value, span } => match value {
                Some(v) => self.expect(v, self.ret)?,
                None if self.ret != Type::Unit => {
                    return Err(Error::new(*span, format!("this function must return a {}", self.ret)));
                }
                None => {}
            },
            Stmt::Expr(e) => {
                self.expr(e)?;
            }
        }
        Ok(())
    }

    fn expr(&mut self, e: &Expr) -> Result<Type, Error> {
        Ok(match &e.kind {
            ExprKind::Int(_) => Type::I64,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Var(name) => match self.lookup(name) {
                Some((ty, _)) => ty,
                None => return Err(Error::new(e.span, format!("unknown variable '{name}'"))),
            },
            ExprKind::Unary(op, inner) => {
                let ty = match op {
                    UnOp::Neg => Type::I64,
                    UnOp::Not => Type::Bool,
                };
                self.expect(inner, ty)?;
                ty
            }
            ExprKind::Binary(op, l, r) => {
                use BinOp::*;
                match op {
                    Add | Sub | Mul | Div | Rem => {
                        self.expect(l, Type::I64)?;
                        self.expect(r, Type::I64)?;
                        Type::I64
                    }
                    Lt | Le | Gt | Ge => {
                        self.expect(l, Type::I64)?;
                        self.expect(r, Type::I64)?;
                        Type::Bool
                    }
                    Eq | Ne => {
                        let lt = self.expr(l)?;
                        if lt == Type::Unit {
                            return Err(Error::new(l.span, "cannot compare ()"));
                        }
                        self.expect(r, lt)?;
                        Type::Bool
                    }
                    And | Or => {
                        self.expect(l, Type::Bool)?;
                        self.expect(r, Type::Bool)?;
                        Type::Bool
                    }
                }
            }
            ExprKind::Call(name, args) if name == "print" => {
                if args.len() != 1 {
                    return Err(Error::new(e.span, "print takes exactly one argument"));
                }
                match self.expr(&args[0])? {
                    Type::Unit => return Err(Error::new(args[0].span, "cannot print ()")),
                    _ => Type::Unit,
                }
            }
            ExprKind::Call(name, args) => {
                let Some(sig) = self.sigs.get(name) else {
                    return Err(Error::new(e.span, format!("unknown function '{name}'")));
                };
                if sig.params.len() != args.len() {
                    return Err(Error::new(
                        e.span,
                        format!("'{name}' takes {} argument(s), but {} given", sig.params.len(), args.len()),
                    ));
                }
                for (a, &p) in args.iter().zip(&sig.params) {
                    self.expect(a, p)?;
                }
                sig.ret
            }
        })
    }
}
