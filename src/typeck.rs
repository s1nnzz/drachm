use std::collections::HashMap;

use crate::ast::*;
use crate::diag::{codes, Diagnostic, Span};

pub struct Sig {
    pub params: Vec<Type>,
    pub ret: Type,
    pub span: Span,
}

/// Builtins implemented in the C runtime. `print` is overloaded on i64/bool,
/// so it is checked specially rather than listed here.
pub const BUILTINS: &[&str] = &["print"];

/// Checks the whole program, reporting every problem it finds to `diags`.
/// The result is only meaningful for codegen when no errors were reported.
pub fn check(prog: &Program, diags: &mut Vec<Diagnostic>) -> HashMap<String, Sig> {
    let mut sigs: HashMap<String, Sig> = HashMap::new();
    for f in &prog.funcs {
        if BUILTINS.contains(&f.name.as_str()) {
            diags.push(
                Diagnostic::at(&codes::E0203, f.span, format!("`{}` is a builtin and cannot be redefined", f.name))
                    .help("give this function another name"),
            );
            continue;
        }
        if let Some(first) = sigs.get(&f.name) {
            diags.push(
                Diagnostic::at(&codes::E0202, f.span, format!("function `{}` is defined twice", f.name))
                    .note("defined again here")
                    .label(first.span, "first defined here"),
            );
            continue;
        }
        let sig = Sig { params: f.params.iter().map(|p| p.1).collect(), ret: f.ret, span: f.span };
        sigs.insert(f.name.clone(), sig);
    }

    match sigs.get("main") {
        None => diags.push(
            Diagnostic::new(&codes::E0204, "this program has no `main` function")
                .help("add `fn main() { }`; it is where the program starts"),
        ),
        Some(s) if !s.params.is_empty() || !matches!(s.ret, Type::I64 | Type::Unit) => diags.push(
            Diagnostic::at(&codes::E0204, s.span, "`main` has the wrong shape")
                .note("must be `fn main()` or `fn main() -> i64`"),
        ),
        _ => {}
    }

    for f in &prog.funcs {
        let mut cx = Checker { sigs: &sigs, scopes: vec![HashMap::new()], ret: f.ret, diags };
        for (name, ty) in &f.params {
            cx.scopes[0].insert(name.clone(), Binding { ty: *ty, mutable: false, span: f.span });
        }
        cx.block(&f.body);
        if f.ret != Type::Unit && !returns(&f.body) {
            diags.push(
                Diagnostic::at(
                    &codes::E0105,
                    f.span,
                    format!("function `{}` can finish without returning a value", f.name),
                )
                .note(format!("promises to return {}", f.ret))
                .help("add a `return` at the end, or to every branch"),
            );
        }
    }
    sigs
}

/// Does every path through this block hit a `return`?
fn returns(block: &Block) -> bool {
    block.iter().any(|s| match s {
        Stmt::Return { .. } => true,
        Stmt::If { then, els: Some(els), .. } => returns(then) && returns(els),
        _ => false,
    })
}

#[derive(Clone, Copy)]
struct Binding {
    ty: Type,
    mutable: bool,
    span: Span,
}

struct Checker<'a> {
    sigs: &'a HashMap<String, Sig>,
    scopes: Vec<HashMap<String, Binding>>,
    ret: Type,
    diags: &'a mut Vec<Diagnostic>,
}

impl Checker<'_> {
    fn lookup(&self, name: &str) -> Option<Binding> {
        self.scopes.iter().rev().find_map(|s| s.get(name).copied())
    }

    fn block(&mut self, b: &Block) {
        self.scopes.push(HashMap::new());
        for s in b {
            self.stmt(s);
        }
        self.scopes.pop();
    }

    /// Check `e` and report if its type is not `want`. An expression that
    /// already has an error inside is not reported a second time.
    fn expect(&mut self, e: &Expr, want: Type) {
        if let Some(got) = self.expr(e)
            && got != want
        {
            self.diags.push(
                Diagnostic::at(&codes::E0101, e.span, format!("expected {want}, found {got}"))
                    .note(format!("this is {got}")),
            );
        }
    }

    fn stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Let { name, ty, mutable, value, span } => {
                if *ty == Type::Unit {
                    self.diags.push(
                        Diagnostic::at(&codes::E0104, *span, format!("variable `{name}` cannot have type ()"))
                            .note("() holds no value"),
                    );
                    self.expr(value);
                } else {
                    self.expect(value, *ty);
                }
                self.scopes.last_mut().unwrap().insert(name.clone(), Binding { ty: *ty, mutable: *mutable, span: *span });
            }
            Stmt::Assign { name, value, span } => {
                let Some(b) = self.lookup(name) else {
                    self.diags.push(Diagnostic::at(&codes::E0201, *span, format!("unknown variable `{name}`")));
                    self.expr(value);
                    return;
                };
                if !b.mutable {
                    self.diags.push(
                        Diagnostic::at(&codes::E0205, *span, format!("cannot assign to `{name}`: it is immutable"))
                            .note("assigned here")
                            .label(b.span, format!("`{name}` is declared here without `var`"))
                            .help(format!("declare it as `var {name}` to allow assignment")),
                    );
                }
                self.expect(value, b.ty);
            }
            Stmt::If { cond, then, els } => {
                self.expect(cond, Type::Bool);
                self.block(then);
                if let Some(els) = els {
                    self.block(els);
                }
            }
            Stmt::While { cond, body } => {
                self.expect(cond, Type::Bool);
                self.block(body);
            }
            Stmt::Return { value, span } => match value {
                Some(v) => self.expect(v, self.ret),
                None if self.ret != Type::Unit => self.diags.push(
                    Diagnostic::at(&codes::E0105, *span, format!("this function must return {}", self.ret))
                        .note("returns nothing"),
                ),
                None => {}
            },
            Stmt::Expr(e) => {
                self.expr(e);
            }
        }
    }

    /// The type of `e`, or `None` if an error was already reported inside it.
    fn expr(&mut self, e: &Expr) -> Option<Type> {
        Some(match &e.kind {
            ExprKind::Int(_) => Type::I64,
            ExprKind::Bool(_) => Type::Bool,
            ExprKind::Var(name) => match self.lookup(name) {
                Some(b) => b.ty,
                None => {
                    self.diags.push(Diagnostic::at(&codes::E0201, e.span, format!("unknown variable `{name}`")));
                    return None;
                }
            },
            ExprKind::Unary(op, inner) => {
                let ty = match op {
                    UnOp::Neg => Type::I64,
                    UnOp::Not => Type::Bool,
                };
                self.expect(inner, ty);
                ty
            }
            ExprKind::Binary(op, l, r) => {
                use BinOp::*;
                match op {
                    Add | Sub | Mul | Div | Rem => {
                        self.expect(l, Type::I64);
                        self.expect(r, Type::I64);
                        Type::I64
                    }
                    Lt | Le | Gt | Ge => {
                        self.expect(l, Type::I64);
                        self.expect(r, Type::I64);
                        Type::Bool
                    }
                    Eq | Ne => {
                        let lt = self.expr(l);
                        if lt == Some(Type::Unit) {
                            self.diags.push(
                                Diagnostic::at(&codes::E0104, l.span, "cannot compare ()")
                                    .note("this produces no value"),
                            );
                            self.expr(r);
                        } else if let Some(lt) = lt {
                            self.expect(r, lt);
                        } else {
                            self.expr(r);
                        }
                        Type::Bool
                    }
                    And | Or => {
                        self.expect(l, Type::Bool);
                        self.expect(r, Type::Bool);
                        Type::Bool
                    }
                }
            }
            ExprKind::Call(name, args) if name == "print" => {
                if args.len() != 1 {
                    self.diags.push(
                        Diagnostic::at(
                            &codes::E0103,
                            e.span,
                            format!("`print` takes 1 argument, but {} given", args.len()),
                        ),
                    );
                    for a in args {
                        self.expr(a);
                    }
                } else if self.expr(&args[0]) == Some(Type::Unit) {
                    self.diags.push(
                        Diagnostic::at(&codes::E0104, args[0].span, "cannot print ()").note("this produces no value"),
                    );
                }
                Type::Unit
            }
            ExprKind::Call(name, args) => {
                let Some(sig) = self.sigs.get(name) else {
                    self.diags.push(Diagnostic::at(&codes::E0201, e.span, format!("unknown function `{name}`")));
                    for a in args {
                        self.expr(a);
                    }
                    return None;
                };
                if sig.params.len() != args.len() {
                    let n = sig.params.len();
                    self.diags.push(
                        Diagnostic::at(
                            &codes::E0103,
                            e.span,
                            format!("`{name}` takes {n} argument{}, but {} given", if n == 1 { "" } else { "s" }, args.len()),
                        )
                        .label(sig.span, format!("`{name}` is defined here")),
                    );
                    for a in args {
                        self.expr(a);
                    }
                } else {
                    for (a, &p) in args.iter().zip(&sig.params) {
                        self.expect(a, p);
                    }
                }
                sig.ret
            }
        })
    }
}
