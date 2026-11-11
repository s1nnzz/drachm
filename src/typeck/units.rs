//! Units of measure in the checker (`docs/impl/step4.5-units.md` 2.3, 2.7):
//! the `unit` declarations of a file, resolution of the units written in
//! angle brackets, and the erasure of unit types before the backends.
//!
//! This is the home of all unit logic in the checker. Arithmetic and
//! conversion with units are step U4; until then the places they will go
//! report "not supported yet" (E0901), so that no unit can slip through.

use std::collections::{HashMap, HashSet};

use num_rational::BigRational;
use num_traits::{One, Zero};

use super::{Checker, Globals, Origin, Val};
use crate::ast;
use crate::consts::Const;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir;
use crate::types::{self, Type};
use crate::units::{Unit, UnitError, UnitTable};

/// The unit table of a program, and the units whose declarations were wrong.
/// Uses of a failed unit are not reported again: the declaration already was.
#[derive(Default)]
pub struct UnitEnv {
    table: UnitTable,
    failed: HashSet<String>,
}

/// Declares the units of a file. Order does not matter: a unit that uses
/// another one evaluates it first. Runs before any type is resolved.
pub(super) fn declare_units(g: &mut Globals, decls: &[ast::UnitDecl], diags: &mut Vec<Diagnostic>) {
    let mut by_name: HashMap<&str, &ast::UnitDecl> = HashMap::new();
    for d in decls {
        match by_name.get(d.name.as_str()) {
            Some(first) => diags.push(
                Diagnostic::at(&codes::E0403, d.name_span, format!("`{}` is declared twice", d.name))
                    .note("declared again here")
                    .label(first.name_span, "first declared here"),
            ),
            None => {
                by_name.insert(&d.name, d);
            }
        }
    }
    let mut ev = Eval { env: g.units.get_mut(), decls: &by_name, stack: Vec::new(), diags, at: Span::new(0, 0) };
    for d in decls {
        ev.name(&d.name, d.name_span);
    }
}

/// Evaluates unit expressions against the table. With `decls` empty it only
/// looks names up, which is what a use of a unit needs.
struct Eval<'a, 'd> {
    env: &'a mut UnitEnv,
    decls: &'d HashMap<&'d str, &'d ast::UnitDecl>,
    /// The declarations being evaluated, outermost first, for cycle messages.
    stack: Vec<&'d str>,
    diags: &'a mut Vec<Diagnostic>,
    /// Where errors about the expression being evaluated point.
    at: Span,
}

impl<'d> Eval<'_, 'd> {
    /// The unit called `name`, declaring it first if the file declares it.
    fn name(&mut self, name: &str, at: Span) -> Option<&'static Unit> {
        if let Some(u) = self.env.table.lookup(name) {
            return Some(u);
        }
        if self.env.failed.contains(name) {
            return None;
        }
        if let Some(&decl) = self.decls.get(name) {
            return self.declare(decl);
        }
        self.diags.push(
            Diagnostic::at(&codes::E0402, at, format!("unknown unit `{name}`"))
                .note("no unit with this name is declared")
                .help(format!("declare it with `unit {name};` (a base unit) or `unit {name} = ...;` (a size of other units)")),
        );
        None
    }

    /// Evaluates one declaration. A declaration that fails is remembered, so
    /// the error is reported once.
    fn declare(&mut self, decl: &'d ast::UnitDecl) -> Option<&'static Unit> {
        if let Some(start) = self.stack.iter().position(|n| *n == decl.name) {
            let mut chain = self.stack[start..].to_vec();
            chain.push(&decl.name);
            self.diags.push(Diagnostic::at(
                &codes::E0403,
                decl.name_span,
                format!("`{}` is defined in terms of itself ({})", decl.name, chain.join(" -> ")),
            ));
            self.env.failed.insert(decl.name.clone());
            return None;
        }
        self.stack.push(&decl.name);
        let outer = std::mem::replace(&mut self.at, decl.span);
        let unit = self.define(decl);
        self.at = outer;
        self.stack.pop();
        if unit.is_none() {
            self.env.failed.insert(decl.name.clone());
        }
        unit
    }

    fn define(&mut self, decl: &'d ast::UnitDecl) -> Option<&'static Unit> {
        let Some(def) = &decl.def else {
            // `unit NAME;`: a new base unit.
            if let Some((_, span)) = &decl.offset {
                self.offset_error(*span);
                return None;
            }
            let r = self.env.table.base(&decl.name);
            return self.check(r, decl.name_span);
        };
        let base = match &def.expr {
            Some(e) => self.expr(e)?,
            None => self.env.table.dimensionless(),
        };
        let factor = match &def.factor {
            Some((c, span)) => {
                let f = rational(c);
                if f <= BigRational::zero() {
                    self.diags.push(
                        Diagnostic::at(&codes::E0403, *span, "a unit's factor must be a number greater than zero")
                            .note("not greater than zero"),
                    );
                    return None;
                }
                f
            }
            None => BigRational::one(),
        };
        let r = self.env.table.scaled(base, &factor);
        let scaled = self.check(r, decl.span)?;
        // `x = e / factor + c` between a reading `x` of the new unit and `e`
        // of the defining expression, so the offsets add up (2.3).
        let mut offset = &base.offset / &factor;
        if let Some((c, span)) = &decl.offset {
            if !matches!(def.expr, Some(ast::UnitExpr::Name(..))) {
                self.offset_error(*span);
                return None;
            }
            offset += rational(c);
        }
        let unit = self.env.table.with_offset(scaled, offset);
        let r = self.env.table.define(&decl.name, unit);
        self.check(r, decl.name_span)?;
        Some(unit)
    }

    fn offset_error(&mut self, span: Span) {
        self.diags.push(
            Diagnostic::at(&codes::E0403, span, "`offset` goes only after a single unit")
                .note("not a single unit")
                .help("for example `unit degC = K offset -273.15;`"),
        );
    }

    /// A unit expression, as written.
    fn expr(&mut self, e: &'d ast::UnitExpr) -> Option<&'static Unit> {
        match e {
            ast::UnitExpr::Name(n, span) => self.name(n, *span),
            ast::UnitExpr::One(_) => Some(self.env.table.dimensionless()),
            ast::UnitExpr::Mul(a, b) => {
                let (a, b) = self.operands(a, b)?;
                let r = self.env.table.mul(a, b);
                self.check(r, self.at)
            }
            ast::UnitExpr::Div(a, b) => {
                let (a, b) = self.operands(a, b)?;
                let r = self.env.table.div(a, b);
                self.check(r, self.at)
            }
            ast::UnitExpr::Pow(b, n, span) => {
                let b = self.expr(b)?;
                self.no_offset(b)?;
                let r = self.env.table.pow(b, *n);
                self.check(r, *span)
            }
        }
    }

    fn operands(&mut self, a: &'d ast::UnitExpr, b: &'d ast::UnitExpr) -> Option<(&'static Unit, &'static Unit)> {
        let a = self.expr(a)?;
        let b = self.expr(b)?;
        self.no_offset(a)?;
        self.no_offset(b)?;
        Some((a, b))
    }

    /// A point (`degC`) stands alone: its offset would be lost in a product.
    fn no_offset(&mut self, u: &'static Unit) -> Option<()> {
        if u.offset.is_zero() {
            return Some(());
        }
        self.diags.push(
            Diagnostic::at(&codes::E0405, self.at, "a unit with an offset cannot be multiplied, divided or raised to a power")
                .note("this unit has an offset, such as `degC`")
                .help("use the point on its own, or write its difference as `delta` (`delta degC`)"),
        );
        None
    }

    fn check<T>(&mut self, r: Result<T, UnitError>, at: Span) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.diags.push(unit_error(e, at));
                None
            }
        }
    }
}

/// The diagnostic for a failed unit operation.
fn unit_error(e: UnitError, at: Span) -> Diagnostic {
    match e {
        UnitError::ExponentRange => Diagnostic::at(&codes::E0405, at, "a unit's power is too large")
            .note("the power is out of range")
            .help("a unit's power must stay within 16 bits"),
        UnitError::NotPositive => Diagnostic::at(&codes::E0403, at, "a unit's factor must be a number greater than zero"),
        UnitError::Redeclared(n) => Diagnostic::at(&codes::E0403, at, format!("`{n}` is declared twice")),
        UnitError::Unknown(n) => Diagnostic::at(&codes::E0402, at, format!("unknown unit `{n}`")),
    }
}

/// A plain number as a rational: a whole number or a decimal, exactly.
fn rational(c: &Const) -> BigRational {
    match c {
        Const::Int(i) => BigRational::from_integer(i.clone()),
        Const::Dec(r) => r.clone(),
    }
}

impl Globals {
    /// The unit a `<...>` names. Reports unknown and bad units to `diags`;
    /// `None` after an error.
    pub(super) fn resolve_unit_spec(&self, spec: &ast::UnitSpec, at: Span, diags: &mut Vec<Diagnostic>) -> Option<&'static Unit> {
        let mut env = self.units.borrow_mut();
        let no_decls = HashMap::new();
        let mut ev = Eval { env: &mut env, decls: &no_decls, stack: Vec::new(), diags, at };
        let u = ev.expr(&spec.expr)?;
        if spec.delta {
            return Some(ev.env.table.delta_of(u));
        }
        Some(u)
    }

    /// The number `base` with the unit `unit`, and the name the unit prints as.
    pub(super) fn unit_type(&self, base: Type, unit: &'static Unit) -> Type {
        let name = self.units.borrow().table.display(unit);
        types::name_unit(unit, &name);
        base.with_unit(unit)
    }

    /// A type written with a unit (`float64<m>`, `&mut uint32<kg>`). Errors
    /// go to `diags`; a unit on anything but a number is E0405.
    pub(super) fn resolve_unit_type(&self, t: &ast::TypeName, spec: &ast::UnitSpec, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let Some(base) = self.resolve_base_quiet(t).filter(|b| b.is_numeric()) else {
            diags.push(
                Diagnostic::at(&codes::E0405, t.span, format!("`{}` cannot have a unit", t.name))
                    .note("units go only on number types")
                    .help("for example `float64<m>`"),
            );
            return None;
        };
        let unit = self.resolve_unit_spec(spec, t.span, diags)?;
        let ty = self.unit_type(base, unit);
        Some(t.mods.iter().rev().fold(ty, |ty, (m, _)| match m {
            ast::TypeMod::Ref { mutable } => Type::borrow(*mutable, ty),
            ast::TypeMod::Ptr => Type::ptr(ty),
        }))
    }
}

/// A value has a unit type (so it may not be used by the U3 code paths).
pub(super) fn has_unit(v: &Val) -> bool {
    matches!(v, Val::Expr(e) if e.ty.mentions_unit())
}

fn not_yet(span: Span, what: &str) -> Diagnostic {
    Diagnostic::at(&codes::E0901, span, format!("{what} is not supported yet"))
}

impl Checker<'_> {
    /// A number literal with a unit (`9.6<s>`). The unit is checked; building
    /// the value is step U4.
    pub(super) fn unit_literal(&mut self, spec: &ast::UnitSpec, span: Span) -> Option<Val> {
        self.g.resolve_unit_spec(spec, span, self.diags);
        self.diags.push(not_yet(span, "a number with a unit"));
        None
    }

    /// A value of a unit type taking part in arithmetic or a comparison. U4.
    pub(super) fn unit_arithmetic_not_yet(&mut self, span: Span) {
        self.diags.push(not_yet(span, "arithmetic and comparisons with units"));
    }

    /// `::in<unit>()` or `::to` involving units. U4.
    pub(super) fn unit_method_not_yet(&mut self, span: Span, what: &str) {
        self.diags.push(not_yet(span, what));
    }

    /// A conversion where a unit is on either side. Only a value that already
    /// has exactly the target type passes; U4 builds the rest.
    pub(super) fn unit_into(&mut self, v: Val, to: Type, from: Origin) -> Option<hir::Expr> {
        match v {
            Val::Expr(e) if e.ty == to => Some(e),
            _ => {
                self.diags.push(not_yet(from.span, "converting to or from a unit"));
                None
            }
        }
    }
}

/// Rewrites every type of the program to its bare number type. Run once,
/// after all diagnostics: no unit reaches the passes after the checker.
pub fn erase(p: &mut hir::Program) {
    visit_types(p, &mut |t| *t = t.bare());
}

/// Panics if a unit type is still in the program. A debug check that
/// `erase` ran; `check` calls it in debug builds.
pub fn assert_erased(p: &mut hir::Program) {
    visit_types(p, &mut |t| assert!(!t.mentions_unit(), "unit type `{t}` survived erasure"));
}

/// Calls `f` on every type in the program: struct fields, signatures, locals
/// and every expression.
fn visit_types(p: &mut hir::Program, f: &mut dyn FnMut(&mut Type)) {
    for s in &mut p.structs {
        for field in &mut s.fields {
            f(&mut field.ty);
        }
    }
    for func in &mut p.funcs {
        f(&mut func.ret);
        for local in &mut func.locals {
            f(&mut local.ty);
        }
        visit_block(&mut func.body, f);
    }
}

fn visit_block(block: &mut hir::Block, f: &mut dyn FnMut(&mut Type)) {
    for s in block {
        visit_stmt(s, f);
    }
}

fn visit_stmt(s: &mut hir::Stmt, f: &mut dyn FnMut(&mut Type)) {
    match s {
        hir::Stmt::Assign(_, e) | hir::Stmt::Expr(e) => visit_expr(e, f),
        hir::Stmt::If { branches, .. } => {
            for b in branches {
                if let Some(test) = &mut b.test {
                    visit_expr(test, f);
                }
                visit_block(&mut b.body, f);
            }
        }
        hir::Stmt::Loop { cond, body, step, .. } => {
            if let Some(c) = cond {
                visit_expr(c, f);
            }
            visit_block(body, f);
            visit_block(step, f);
        }
        hir::Stmt::Return(value) => {
            if let Some(e) = value {
                visit_expr(e, f);
            }
        }
        hir::Stmt::Unsafe(body) => visit_block(body, f),
        hir::Stmt::Jump(_) | hir::Stmt::Decl(_) | hir::Stmt::Destroy(_) | hir::Stmt::Drop(_) => {}
    }
}

fn visit_expr(e: &mut hir::Expr, f: &mut dyn FnMut(&mut Type)) {
    f(&mut e.ty);
    for child in children_mut(e) {
        visit_expr(child, f);
    }
}

/// The direct sub-expressions of `e`. Every variant is named, so a new one
/// is a compile error here until it is handled.
fn children_mut(e: &mut hir::Expr) -> Vec<&mut hir::Expr> {
    use hir::ExprKind as K;
    match &mut e.kind {
        K::Int(_) | K::Float(_) | K::Bool(_) | K::Str(_) | K::Local(_) | K::Move(_) | K::NullPtr => Vec::new(),
        K::IncDec { .. } => Vec::new(),
        K::Field(x, _)
        | K::Neg(x)
        | K::Not(x)
        | K::BitNot(x)
        | K::Convert(x)
        | K::Text(x)
        | K::Copy(x)
        | K::DeepCopy(x)
        | K::Deref(x)
        | K::Lend(x)
        | K::AddressOf(x)
        | K::Temp(_, x)
        | K::Scoped(x, _) => vec![&mut **x],
        K::Radix { value, .. } => vec![&mut **value],
        K::Out { arg, .. } => vec![&mut **arg],
        K::Borrow { place, .. } => vec![&mut **place],
        K::Binary(_, a, b) | K::IntPow(a, b) => vec![&mut **a, &mut **b],
        K::Logic { lhs, rhs, .. } => vec![&mut **lhs, &mut **rhs],
        K::Shift { value, amount, .. } => vec![&mut **value, &mut **amount],
        K::Struct(fields) => fields.iter_mut().map(|(_, v)| v).collect(),
        K::Call(_, args) | K::StrCall(_, args) => args.iter_mut().collect(),
        K::Join(parts) => parts.iter_mut().collect(),
        K::Take { leaving, .. } => leaving.as_deref_mut().into_iter().collect(),
    }
}
