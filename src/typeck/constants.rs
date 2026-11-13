//! Named constants: top-level values whose initializer is known when the
//! program is compiled (`uint32 SIZE = 8;`, `fixed uint32 MAX = SIZE * 2;`).
//! They are usable as values and in types (array lengths, value arguments).
//! A top-level value that is only known while the program runs is E0901 for
//! now (script state arrives with step 7).

use std::collections::HashMap;

use crate::ast::{self, BinOp, ExprKind, UnOp};
use crate::consts::{Const, FoldError, Folded};
use crate::diag::{codes, Diagnostic, Span};
use crate::types::Type;

use super::Globals;

/// A named constant: its type and exact value (already wrapped into the type).
#[derive(Clone)]
pub(super) struct ConstInfo {
    pub ty: Type,
    pub value: Const,
}

/// Why an initializer is not a constant.
enum Fail {
    /// Known only when running: the span and what it is.
    Run(Span, String),
    /// Already reported.
    Reported,
}

struct Ctx<'a> {
    g: &'a mut Globals,
    decls: &'a [ast::ConstDecl],
    index: HashMap<&'a str, usize>,
    /// 0 not started, 1 in progress, 2 done.
    state: Vec<u8>,
    diags: &'a mut Vec<Diagnostic>,
}

pub(super) fn declare(g: &mut Globals, decls: &[ast::ConstDecl], diags: &mut Vec<Diagnostic>) {
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut state = vec![0u8; decls.len()];
    for (i, d) in decls.iter().enumerate() {
        if let Some(&first) = index.get(d.name.as_str()) {
            diags.push(
                Diagnostic::at(&codes::E0638, d.name_span, format!("`{}` is defined twice", d.name)).label(decls[first].name_span, "first defined here"),
            );
            state[i] = 2;
        } else {
            index.insert(d.name.as_str(), i);
        }
    }
    let mut cx = Ctx { g, decls, index, state, diags };
    for i in 0..decls.len() {
        cx.decl(i);
    }
}

/// The type a top-level value is declared with, if it is a plain number type.
fn number_type(t: &ast::TypeName) -> Option<Type> {
    let plain = t.args.is_empty() && t.mods.is_empty() && t.dims.is_empty() && t.optional.is_none() && t.unit.is_none() && t.func.is_none() && !t.is_number && !t.self_ty;
    if !plain {
        return None;
    }
    match Type::from_name(&t.name) {
        Ok(ty @ (Type::Int { .. } | Type::Float(_))) => Some(ty),
        _ => None,
    }
}

impl Ctx<'_> {
    fn decl(&mut self, i: usize) {
        if self.state[i] != 0 {
            return;
        }
        self.state[i] = 1;
        let decls = self.decls;
        self.run(&decls[i]);
        self.state[i] = 2;
    }

    fn run(&mut self, d: &ast::ConstDecl) {
        if d.mutable {
            self.diags.push(
                Diagnostic::at(&codes::E0901, d.name_span, "a top-level `mut` value is not supported yet")
                    .help("state shared by functions arrives with scripts; for now declare it inside `main` or pass it along"),
            );
            return;
        }
        let Some(ty) = number_type(&d.ty) else {
            self.diags.push(
                Diagnostic::at(&codes::E0901, d.ty.span, "a top-level value of this type is not supported yet")
                    .help("so far a top-level value is a number: `uint32 SIZE = 8;`"),
            );
            return;
        };
        let value = match &d.value {
            None => {
                self.diags.push(
                    Diagnostic::at(&codes::E0003, d.name_span, format!("`{}` needs a value", d.name))
                        .help(format!("write it like `{} {} = 8;`", d.ty.name, d.name)),
                );
                return;
            }
            Some(ast::Init::Fields(_, span)) => {
                self.diags.push(Diagnostic::at(&codes::E0901, *span, "the construction shorthand is not supported for a top-level value"));
                return;
            }
            Some(ast::Init::Expr(e)) => e,
        };
        match self.eval(value) {
            Ok(c) => {
                let c = if c.fits(ty) {
                    c
                } else {
                    use crate::config::Overflow;
                    let mode = self.g.settings_at(d.name_span).overflow;
                    let wrapped = match ty {
                        Type::Int { .. } if mode == Overflow::Saturate => Const::from_int_bits(c.saturate_to_int(ty), ty),
                        Type::Int { .. } => Const::from_int_bits(c.wrap_to_int(ty), ty),
                        _ => c.clone(),
                    };
                    let diag = if mode == Overflow::Panic && matches!(ty, Type::Int { .. }) {
                        Diagnostic::at(&codes::E0116, value.span, format!("this constant does not fit `{ty}`"))
                            .note_line(format!("`{ty}` holds {}, and this file has `overflow = panic`", ty.describe_range()))
                            .help("use a value that fits or a bigger type")
                    } else {
                        let how = if mode == Overflow::Saturate { "stops at the nearest end" } else { "wraps around" };
                        let mut diag = Diagnostic::at(&codes::W0102, value.span, format!("this constant does not fit `{ty}`"))
                            .note_line(format!("`{ty}` holds {}; a constant outside that {how}", ty.describe_range()));
                        if let Const::Int(v) = &wrapped {
                            diag = diag.note(format!("becomes {v}"));
                        }
                        diag
                    };
                    self.diags.push(diag);
                    wrapped
                };
                let c = match ty {
                    Type::Int { .. } if !matches!(c, Const::Int(_)) => Const::from_int_bits(c.wrap_to_int(ty), ty),
                    _ => c,
                };
                self.g.consts.insert(d.name.clone(), ConstInfo { ty, value: c });
            }
            Err(Fail::Reported) => {}
            Err(Fail::Run(span, what)) if d.fixed => {
                self.g.run_values.insert(d.name.clone());
                self.diags.push(
                    Diagnostic::at(&codes::E0637, span, format!("`{}` is `fixed`, but {what} is not a constant", d.name))
                        .label(d.name_span, "declared `fixed` here")
                        .note("a `fixed` value must be known when the program is compiled")
                        .help("use numbers, other constants and arithmetic, or drop `fixed` to make it a value of the run"),
                );
            }
            Err(Fail::Run(span, what)) => {
                self.g.run_values.insert(d.name.clone());
                self.diags.push(
                    Diagnostic::at(&codes::E0901, span, format!("{what} is only known while the program runs, and top-level values of the run are not supported yet"))
                        .label(d.name_span, format!("`{}` would be one", d.name))
                        .help("a top-level value with a constant initializer works (and can be used in types); for others, declare the value inside `main`"),
                );
            }
        }
    }

    fn eval(&mut self, e: &ast::Expr) -> Result<Const, Fail> {
        match &e.kind {
            ExprKind::Num(n) => Ok(n.value.clone()),
            ExprKind::Var(n) => {
                if !self.g.consts.contains_key(n.as_str()) {
                    match self.index.get(n.as_str()).copied() {
                        Some(i) if self.state[i] == 1 => {
                            self.diags.push(
                                Diagnostic::at(&codes::E0634, e.span, format!("`{n}` is defined in terms of itself"))
                                    .label(self.decls[i].name_span, "defined here"),
                            );
                            return Err(Fail::Reported);
                        }
                        Some(i) => self.decl(i),
                        None => {}
                    }
                }
                match self.g.consts.get(n.as_str()) {
                    Some(info) => Ok(info.value.clone()),
                    None if self.index.contains_key(n.as_str()) => Err(Fail::Reported),
                    None => Err(Fail::Run(e.span, format!("`{n}`"))),
                }
            }
            ExprKind::Unary(op, inner) => {
                let c = self.eval(inner)?;
                match op {
                    UnOp::Neg => Ok(c.neg()),
                    UnOp::BitNot => c.bit_not().map_err(|_| Fail::Run(e.span, "this calculation".into())),
                    UnOp::Not => Err(Fail::Run(e.span, "this calculation".into())),
                }
            }
            ExprKind::Binary(op, l, r) if !matches!(op, BinOp::And | BinOp::Or | BinOp::Same | BinOp::NotSame) => {
                let (a, b) = (self.eval(l)?, self.eval(r)?);
                match Const::binary(*op, &a, &b) {
                    Ok(Folded::Value(c)) => Ok(c),
                    Ok(Folded::Bool(_)) => Err(Fail::Run(e.span, "a comparison".into())),
                    Err(FoldError::DivisionByZero) => {
                        self.diags.push(Diagnostic::at(&codes::E0107, e.span, "this constant expression divides by zero").label(r.span, "this is zero"));
                        Err(Fail::Reported)
                    }
                    Err(FoldError::TooLarge) => {
                        self.diags.push(Diagnostic::at(&codes::E0002, e.span, "this constant expression is too large to evaluate"));
                        Err(Fail::Reported)
                    }
                    Err(_) => Err(Fail::Run(e.span, "this calculation".into())),
                }
            }
            ExprKind::Call { name, .. } => Err(Fail::Run(e.span, format!("the call to `{name}`"))),
            ExprKind::Method { .. } => Err(Fail::Run(e.span, "this call".into())),
            _ => Err(Fail::Run(e.span, "this value".into())),
        }
    }
}

impl Globals {
    /// An extra note for a name used in a type that is not a constant, when it
    /// is a top-level value of the run.
    pub(super) fn run_value_note(&self, name: &str) -> Option<String> {
        self.run_values.contains(name).then(|| format!("`{name}` gets its value while the program runs, and a type must be known when the program is compiled"))
    }
}
