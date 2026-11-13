//! Units of measure in the checker (`docs/impl/step4.5-units.md` 2.3, 2.7):
//! the `unit` declarations of a file, resolution of the units written in
//! angle brackets, and the erasure of unit types before the backends.
//!
//! This is the home of all unit logic in the checker. Arithmetic and
//! conversion with units are step U4; until then the places they will go
//! report "not supported yet" (E0901), so that no unit can slip through.

use std::collections::{HashMap, HashSet};

use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

use super::optionals;
use super::{Checker, Globals, Origin, Val};
use crate::ast::{BinOp, OpClass, UnOp};
use crate::ast;
use crate::consts::{Const, Folded};
use crate::diag::{codes, Diagnostic, Span};
use crate::hir;
use crate::types::{self, Type};
use crate::units::{scale_pair, Unit, UnitError, UnitTable};

/// The unit table of a program, and the units whose declarations were wrong.
/// Uses of a failed unit are not reported again: the declaration already was.
#[derive(Default)]
pub struct UnitEnv {
    pub(super) table: UnitTable,
    failed: HashSet<String>,
}

/// The parameters of the instance being resolved: a unit parameter stands for
/// the unit it was given (`Type::unit_arg`), looked up before the table.
type Params<'a> = &'a HashMap<String, Type>;

/// The SI units of `in si;` (`src/prelude/si.drh`): ordinary declarations.
pub const SI_PRELUDE: &str = include_str!("../prelude/si.drh");

/// Is `name` a declared unit?
pub(super) fn is_unit_name(g: &Globals, name: &str) -> bool {
    g.units.borrow_mut().table.lookup(name).is_some()
}

/// Declares the units of a file. Order does not matter: a unit that uses
/// another one evaluates it first. Runs before any type is resolved. Units
/// already in the table (the SI prelude) cannot be declared again.
pub(super) fn declare_units(g: &mut Globals, decls: &[ast::UnitDecl], diags: &mut Vec<Diagnostic>) {
    let mut by_name: HashMap<&str, &ast::UnitDecl> = HashMap::new();
    for d in decls {
        if g.units.get_mut().table.lookup(&d.name).is_some() {
            diags.push(
                Diagnostic::at(&codes::E0403, d.name_span, format!("`{}` is declared twice", d.name))
                    .note("already declared by `in si;`"),
            );
            continue;
        }
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
    let none = HashMap::new();
    let mut ev = Eval { env: g.units.get_mut(), decls: &by_name, params: &none, stack: Vec::new(), diags, at: Span::new(0, 0) };
    for d in decls {
        ev.name(&d.name, d.name_span, false);
    }
}

/// Evaluates unit expressions against the table. With `decls` empty it only
/// looks names up, which is what a use of a unit needs.
struct Eval<'a, 'd> {
    env: &'a mut UnitEnv,
    decls: &'d HashMap<&'d str, &'d ast::UnitDecl>,
    params: Params<'a>,
    /// The declarations being evaluated, outermost first, for cycle messages.
    stack: Vec<&'d str>,
    diags: &'a mut Vec<Diagnostic>,
    /// Where errors about the expression being evaluated point.
    at: Span,
}

impl<'d> Eval<'_, 'd> {
    /// The unit called `name`, declaring it first if the file declares it.
    /// `explicit` is `name as unit`: it always means the unit, never a parameter.
    fn name(&mut self, name: &str, at: Span, explicit: bool) -> Option<&'static Unit> {
        if !explicit && let Some(t) = self.params.get(name) {
            if let Some(u) = t.as_unit_arg() {
                return Some(u);
            }
            // A type parameter with a unit's name is the type inside its scope.
            self.diags.push(
                Diagnostic::at(&codes::E0402, at, format!("`{name}` is a type parameter here, so it is not a unit"))
                    .note("inside its scope a type parameter's name is the type")
                    .help(format!("write `{name} as unit` for the unit `{name}`")),
            );
            return None;
        }
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
                .help(format!("add `in si;` for the SI units, or declare it with `unit {name};` (a base unit) or `unit {name} = ...;` (a size of other units)")),
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
            if !matches!(def.expr, Some(ast::UnitExpr::Name(..) | ast::UnitExpr::Explicit(..))) {
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
            ast::UnitExpr::Name(n, span) => self.name(n, *span, false),
            ast::UnitExpr::Explicit(n, span) => self.name(n, *span, true),
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
        Const::NegZero => BigRational::zero(),
    }
}

impl Globals {
    /// The unit a `<...>` names. Reports unknown and bad units to `diags`;
    /// `None` after an error.
    pub(super) fn resolve_unit_spec(&self, spec: &ast::UnitSpec, at: Span, diags: &mut Vec<Diagnostic>) -> Option<&'static Unit> {
        self.resolve_unit_spec_in(spec, &self.tenv, at, diags)
    }

    /// `resolve_unit_spec` with the unit parameters given by `params`.
    pub(super) fn resolve_unit_spec_in(&self, spec: &ast::UnitSpec, params: &HashMap<String, Type>, at: Span, diags: &mut Vec<Diagnostic>) -> Option<&'static Unit> {
        let mut env = self.units.borrow_mut();
        let no_decls = HashMap::new();
        let mut ev = Eval { env: &mut env, decls: &no_decls, params, stack: Vec::new(), diags, at };
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

    /// A unit written as a type argument (`unit m/s`), as its stand-in.
    pub(super) fn resolve_unit_arg(&self, spec: &ast::UnitSpec, at: Span, diags: &mut Vec<Diagnostic>) -> Option<Type> {
        let unit = self.resolve_unit_spec(spec, at, diags)?;
        Some(self.unit_arg(unit))
    }

    /// The stand-in for `unit` in the generic tables (`Type::unit_arg`), with
    /// its display name recorded: instance names and symbols are made from it.
    pub(super) fn unit_arg(&self, unit: &'static Unit) -> Type {
        let name = self.units.borrow().table.display(unit);
        types::name_unit(unit, &name);
        Type::unit_arg(unit)
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
        let ty = optionals::wrap(self, t, self.unit_type(base, unit))?;
        Some(t.mods.iter().rev().fold(ty, |ty, (m, _)| match m {
            ast::TypeMod::Ref { mutable } => Type::borrow(*mutable, ty),
            ast::TypeMod::Ptr => Type::ptr(ty),
            ast::TypeMod::FixedPtr => Type::fixed_ptr(ty),
        }))
    }
}

// ---------------------------------------------------------------------
// U4: values with units (docs/impl/step4.5-units.md 2.5, 2.6, 2.8)
// ---------------------------------------------------------------------

type U = &'static Unit;

/// A value has a unit (or is a constant with one).
pub(super) fn has_unit(v: &Val) -> bool {
    match v {
        Val::UConst(..) => true,
        Val::Expr(e) => e.ty.mentions_unit(),
        Val::Const(_) | Val::NoneLit(_) => false,
    }
}

/// Does a two-sided operator need the unit-aware path? Only when a unit is
/// involved and both sides are numbers; anything else (a string beside a
/// measure, say) takes the ordinary path and its ordinary error.
pub(super) fn involved(l: &Val, r: &Val) -> bool {
    let number = |v: &Val| match v {
        Val::Expr(e) => e.ty.is_numeric(),
        _ => true,
    };
    (has_unit(l) || has_unit(r)) && number(l) && number(r)
}

fn val_unit(v: &Val) -> Option<U> {
    match v {
        Val::UConst(_, u) => Some(u),
        Val::Expr(e) => e.ty.unit(),
        Val::Const(_) | Val::NoneLit(_) => None,
    }
}

fn is_point(u: Option<U>) -> bool {
    u.is_some_and(|u| !u.offset.is_zero())
}

fn dim_of(u: Option<U>) -> &'static [(&'static str, i32)] {
    u.map_or(&[], |u| &u.dim[..])
}

fn scale_of(u: Option<U>) -> BigRational {
    u.map_or_else(BigRational::one, |u| u.scale.clone())
}

fn offset_of(u: Option<U>) -> BigRational {
    u.map_or_else(BigRational::zero, |u| u.offset.clone())
}

/// `x_to = x_from * r + c` between two units of one dimension (2.8): the
/// reading `x` of a unit is the base reading `(x - offset) * scale`.
fn ratio_of(from: Option<U>, to: Option<U>) -> (BigRational, BigRational) {
    let r = scale_of(from) / scale_of(to);
    let c = offset_of(to) - offset_of(from) * &r;
    (r, c)
}

fn is_zero_const(c: &Const) -> bool {
    match c {
        Const::Int(i) => i.is_zero(),
        Const::Dec(r) => r.is_zero(),
        Const::NegZero => true,
    }
}

/// `c` as a constant again: whole values of a whole constant stay whole.
fn rat_const(r: BigRational, whole_ok: bool) -> Const {
    if whole_ok && r.is_integer() { Const::Int(r.to_integer()) } else { Const::Dec(r) }
}

fn convert_const(c: &Const, from: Option<U>, to: Option<U>) -> Const {
    let (r, off) = ratio_of(from, to);
    if r.is_one() && off.is_zero() {
        return c.clone();
    }
    rat_const(rational(c) * r + off, matches!(c, Const::Int(_)))
}

/// An amount (not a reading) in `from` as an amount in `to`: scale only.
fn convert_amount_const(c: &Const, from: U, to: U) -> Const {
    rat_const(rational(c) * (&from.scale / &to.scale), matches!(c, Const::Int(_)))
}

fn retag(mut e: hir::Expr, ty: Type) -> hir::Expr {
    e.ty = ty;
    e
}

fn strip(e: hir::Expr) -> hir::Expr {
    let ty = e.ty.bare();
    retag(e, ty)
}

/// A number of type `ty` holding `r` (a whole number for integer types).
fn lit_rat(r: &BigRational, ty: Type) -> hir::Expr {
    match ty {
        Type::Int { .. } => super::int_lit(Const::Int(r.to_integer()).wrap_to_int(ty), ty),
        _ => hir::Expr::new(hir::ExprKind::Float(Const::Dec(r.clone()).to_float(ty)), ty),
    }
}

fn lit_int(i: &BigInt, ty: Type) -> hir::Expr {
    lit_rat(&BigRational::from_integer(i.clone()), ty)
}

fn bin(op: hir::BinOp, a: hir::Expr, b: hir::Expr, ty: Type) -> hir::Expr {
    hir::Expr::new(hir::ExprKind::Binary(op, Box::new(a), Box::new(b)), ty)
}

fn conv(e: hir::Expr, to: Type) -> hir::Expr {
    if e.ty == to { e } else { hir::Expr::new(hir::ExprKind::Convert(Box::new(e)), to) }
}

fn hir_op(op: BinOp) -> hir::BinOp {
    match op {
        BinOp::Add => hir::BinOp::Add,
        BinOp::Sub => hir::BinOp::Sub,
        BinOp::Mul => hir::BinOp::Mul,
        BinOp::Div => hir::BinOp::Div,
        BinOp::Rem => hir::BinOp::Rem,
        BinOp::Pow => hir::BinOp::Pow,
        BinOp::Eq => hir::BinOp::Eq,
        BinOp::Ne => hir::BinOp::Ne,
        BinOp::Lt => hir::BinOp::Lt,
        BinOp::Le => hir::BinOp::Le,
        BinOp::Gt => hir::BinOp::Gt,
        BinOp::Ge => hir::BinOp::Ge,
        _ => unreachable!("not an operator with units"),
    }
}

/// The 64-bit (or wider) integer type that scaling is done in, so that
/// `uint8<km> + uint8<m>` does not overflow while it is computed.
fn work_type(t: Type, signed: bool) -> Type {
    match t {
        Type::Int { bits, signed: s } => Type::Int { bits: bits.max(64), signed: s || signed },
        t => t,
    }
}

/// What scaling a whole number can do to it.
#[derive(Default)]
struct ScaleInfo {
    /// A division drops a remainder.
    lossy: bool,
    /// The result can leave the range of the target.
    overflow: bool,
}

/// A bare number (its unit is tracked by the caller).
struct Q {
    e: hir::Expr,
}

impl Checker<'_> {
    fn unit_name_of(&self, u: Option<U>) -> String {
        match u {
            Some(u) => self.g.units.borrow().table.display(u),
            None => "1".to_string(),
        }
    }

    /// A type for messages: `float64<m>`, or `a plain number`.
    fn describe(&self, v: &Val) -> String {
        match v {
            Val::Const(_) => "a plain number".to_string(),
            Val::UConst(c, u) => format!("`{}`", self.g.unit_type(c.default_type().unwrap_or(Type::F64), u)),
            Val::Expr(e) => format!("`{}`", e.ty),
            Val::NoneLit(_) => "`none`".to_string(),
        }
    }

    // --- diagnostics ----------------------------------------------------

    fn units_differ(&mut self, lo: Origin, ld: &str, ro: Origin, rd: &str, note: &str, help: &str) {
        self.diags.push(
            Diagnostic::at(&codes::E0401, ro.span, format!("units do not match: {ld} and {rd}"))
                .note(format!("this is {rd}"))
                .label(lo.span, format!("this is {ld}"))
                .note_line(note)
                .help(help),
        );
    }

    /// A plain number where a measure is needed (E0408).
    fn plain_for_measure(&mut self, o: Origin, is_const: bool, unit: Option<U>) {
        let name = self.unit_name_of(unit);
        let text = self.text(o.span).to_string();
        let help = if is_const && !o.compound && text.len() <= 32 {
            if is_point(unit) {
                format!("a plain zero is not a point; write `0<{name}>`")
            } else {
                format!("give it a unit: `{text}<{name}>`")
            }
        } else {
            format!("a plain number can scale a measure with `*` and `/`; to make one a measure write `{} * 1<{name}>`", if o.compound { format!("({text})") } else { text })
        };
        self.diags.push(
            Diagnostic::at(&codes::E0408, o.span, "a plain number cannot stand for a measure")
                .note("this number has no unit")
                .help(help),
        );
    }

    fn point_error(&mut self, at: Span, message: &str, help: &str) {
        self.diags.push(Diagnostic::at(&codes::E0409, at, message).note("this is a point, like `degC`").help(help));
    }

    fn bit_error(&mut self, at: Span, sym: &str, found: &str) {
        self.diags.push(
            Diagnostic::at(&codes::E0406, at, format!("`{sym}` cannot be used on a value with a unit"))
                .note(format!("this is {found}"))
                .note_line("shifting and the bit operators change a pattern of bits, not a measure")
                .help("to scale a measure use `*`: `len * 2`"),
        );
    }

    /// The scaling of a whole-number conversion can lose or overflow.
    /// `sum` says the conversion happens inside an operator, so the fix is
    /// not to write `::in` on the whole expression.
    fn warn_scale(&mut self, from: Type, to: Type, info: &ScaleInfo, at: Origin, sum: bool) {
        if !(info.lossy || info.overflow) {
            return;
        }
        let (fu, tu) = (self.unit_name_of(from.unit()), self.unit_name_of(to.unit()));
        let strip = to.unit().is_none();
        let what = match (info.lossy, strip) {
            (true, true) => format!("a whole number of {fu} loses its remainder in the base unit"),
            (true, false) => format!("a whole number of {fu} is divided into {tu}, and the remainder is dropped"),
            (false, true) => format!("the amount in the base unit of {fu} can be larger than `{to}` holds ({}); larger values wrap around", to.describe_range()),
            (false, false) => format!("{fu} is scaled up into {tu}, and `{to}` holds {}; larger values wrap around", to.describe_range()),
        };
        let text = self.text(at.span).to_string();
        let help = if sum {
            format!("put the smaller unit on the left to keep the remainder, or convert one side first with `::in<{tu}>()`")
        } else if strip {
            "use a wider number type".to_string()
        } else if text.len() > 40 || text.contains('\n') {
            format!("add `::in<{tu}>()` to say the conversion is intended")
        } else if at.compound || text.contains("::") || text.contains("->") {
            format!("write `({text})::in<{tu}>()` to say the conversion is intended")
        } else {
            format!("write `{text}::in<{tu}>()` to say the conversion is intended")
        };
        self.diags.push(
            Diagnostic::at(
                &codes::W0102,
                at.span,
                format!("converting `{from}` to `{to}` can {}", if info.lossy { "lose information" } else { "overflow" }),
            )
            .note(format!("this is `{from}`"))
            .note_line(what)
            .help(help),
        );
    }

    // --- scaling --------------------------------------------------------

    /// `e * r + c` in the number type `out` (`e` is a bare number). Floats
    /// scale in a float type; whole numbers in a wide integer type with the
    /// multiplications first and one truncating division at the end (Q5).
    fn scale_expr(&mut self, e: hir::Expr, out: Type, r: &BigRational, c: &BigRational) -> (hir::Expr, ScaleInfo) {
        let inn = e.ty;
        let mut info = ScaleInfo::default();
        if r.is_one() && c.is_zero() {
            return (conv(e, out), info);
        }
        if inn.is_float() || out.is_float() {
            let ft = if inn.is_float() { inn } else { out };
            let mut x = conv(e, ft);
            let limit = BigInt::one() << 53;
            let (p, q) = (r.numer(), r.denom());
            if *p <= limit && *q <= limit {
                if !p.is_one() {
                    x = bin(hir::BinOp::Mul, x, lit_int(p, ft), ft);
                }
                if !q.is_one() {
                    x = bin(hir::BinOp::Div, x, lit_int(q, ft), ft);
                }
            } else {
                x = bin(hir::BinOp::Mul, x, lit_rat(r, ft), ft);
            }
            if !c.is_zero() {
                let op = if c.is_negative() { hir::BinOp::Sub } else { hir::BinOp::Add };
                x = bin(op, x, lit_rat(&c.abs(), ft), ft);
            }
            return (conv(x, out), info);
        }
        // Whole numbers: x * p / q + a / b = (x * (p * b) + a * q) / (q * b).
        let (p, q, a, b) = (r.numer(), r.denom(), c.numer(), c.denom());
        let (m, d, add) = (p * b, q * b, a * q);
        let (Type::Int { bits: ib, signed: is }, Type::Int { bits: ob, signed: os }) = (inn, out) else { unreachable!() };
        let w = Type::Int { bits: ib.max(ob).max(64), signed: is || os || add.is_negative() };
        let mut x = conv(e, w);
        if !m.is_one() {
            x = bin(hir::BinOp::Mul, x, lit_int(&m, w), w);
        }
        if !add.is_zero() {
            let op = if add.is_negative() { hir::BinOp::Sub } else { hir::BinOp::Add };
            x = bin(op, x, lit_int(&add.abs(), w), w);
        }
        if !d.is_one() {
            x = bin(hir::BinOp::Div, x, lit_int(&d, w), w);
            info.lossy = true;
        }
        if let (Some((ilo, ihi)), Some((lo, hi))) = (inn.int_range(), out.int_range()) {
            let top = (&ihi * &m + &add).div_floor(&d);
            let bottom = (&ilo * &m + &add).div_floor(&d);
            info.overflow = top > hi || bottom < lo;
        }
        (conv(x, out), info)
    }

    /// The number `e` (with or without a unit) as a value of type `to`
    /// (a number type, with or without a unit of the same dimension).
    fn convert_value(&mut self, e: hir::Expr, to: Type, silent: bool, at: Origin) -> hir::Expr {
        let (fu, tu) = (e.ty.unit(), to.unit());
        let (ft, tt) = (e.ty.bare(), to.bare());
        let from_ty = e.ty;
        let e = strip(e);
        let out = if fu == tu {
            if ft != tt && !silent && !ft.converts_losslessly_to(tt) {
                self.warn_lossy(ft, tt, at);
            }
            conv(e, tt)
        } else {
            let (r, c) = ratio_of(fu, tu);
            if ft != tt && !silent && !ft.converts_losslessly_to(tt) {
                self.warn_lossy(ft, tt, at);
            }
            let (x, info) = self.scale_expr(e, tt, &r, &c);
            if !silent && ft.is_int() && tt.is_int() {
                self.warn_scale(from_ty, to, &info, at, false);
            }
            x
        };
        retag(out, to)
    }

    // --- literals and constants -----------------------------------------

    /// A number literal with a unit (`9.6<s>`).
    pub(super) fn unit_literal(&mut self, n: &ast::NumLit, span: Span) -> Option<Val> {
        let spec = n.unit.as_ref()?;
        let u = self.g.resolve_unit_spec(spec, span, self.diags)?;
        let plain = u.dim.is_empty() && u.scale.is_one() && u.offset.is_zero();
        match n.suffix {
            Some(ty) => {
                let e = self.fit_const(&n.value, ty, span, false);
                let ty = if plain { ty } else { self.g.unit_type(ty, u) };
                Some(Val::Expr(retag(e, ty)))
            }
            None if plain => Some(Val::Const(n.value.clone())),
            None => Some(Val::UConst(n.value.clone(), u)),
        }
    }

    /// A constant with a unit given its default number type.
    pub(super) fn unit_materialise(&mut self, c: &Const, u: U, span: Span) -> Option<hir::Expr> {
        let Some(ty) = c.default_type() else {
            self.diags.push(Diagnostic::at(&codes::E0002, span, "this constant is too large for any number type").note("does not fit in 128 bits"));
            return None;
        };
        let e = self.fit_const(c, ty, span, true);
        Some(retag(e, self.g.unit_type(ty, u)))
    }

    // --- conversion to a typed slot -------------------------------------

    /// A conversion where a unit is on either side: the target's unit leads,
    /// a unit into a plain number strips to the base amount (Q1), and a
    /// plain number into a unit is not decided yet.
    pub(super) fn unit_into(&mut self, v: Val, to: Type, from: Origin) -> Option<hir::Expr> {
        if let Val::Expr(e) = &v
            && e.ty.is_borrow()
            && !to.is_borrow()
        {
            let v = self.read(v);
            return self.unit_into(v, to, from);
        }
        let non_number = matches!(&v, Val::Expr(e) if !e.ty.is_numeric());
        if !to.is_numeric() || non_number {
            return match v {
                Val::Expr(e) if e.ty == to => Some(e),
                v => {
                    let found = self.describe(&v);
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, from.span, format!("expected `{to}`, found {found}")).note(format!("this is {found}")),
                    );
                    None
                }
            };
        }
        let tu = to.unit();
        let fu = val_unit(&v);
        let plain_value = fu.is_none();
        if plain_value && tu.is_some() {
            // Q12b: a plain value into a slot with a unit attaches that unit:
            // `float64<m> d = 5;` is 5 m. The number is the reading, unscaled.
            return match v {
                Val::Const(c) => {
                    let e = self.fit_const(&c, to.bare(), from.span, false);
                    Some(retag(e, to))
                }
                Val::Expr(e) => Some(retag(self.convert_value(e, to.bare(), false, from), to)),
                Val::UConst(..) => unreachable!("a unit constant has a unit"),
                Val::NoneLit(s) => {
                    self.none_without_type(s);
                    None
                }
            };
        }
        // A plain target takes the base amount of any unit (Q1).
        if tu.is_some() && dim_of(fu) != dim_of(tu) {
            let found = self.describe(&v);
            self.diags.push(
                Diagnostic::at(&codes::E0401, from.span, format!("units do not match: expected `{to}`, found {found}"))
                    .note(format!("this is {found}"))
                    .note_line("a value fits a variable only when it measures the same kind of thing")
                    .help("to get a value of another kind, multiply or divide: `distance / time`"),
            );
            return None;
        }
        match v {
            Val::Const(_) => unreachable!(),
            // `none` has no unit, so it never reaches the unit path.
            Val::NoneLit(_) => unreachable!("`none` has no unit"),
            Val::UConst(c, u) => {
                let value = convert_const(&c, Some(u), tu);
                let e = self.fit_const(&value, to.bare(), from.span, false);
                Some(retag(e, to))
            }
            Val::Expr(e) => Some(self.convert_value(e, to, false, from)),
        }
    }

    // --- arithmetic -----------------------------------------------------

    /// A two-sided operator where a unit is involved (2.5).
    pub(super) fn unit_binary(&mut self, op: BinOp, lv: Val, lo: Origin, rv: Val, ro: Origin, span: Span) -> Option<Val> {
        match op.class() {
            OpClass::Shift | OpClass::Bitwise => {
                let (side, o) = if has_unit(&lv) { (&lv, lo) } else { (&rv, ro) };
                let found = self.describe(side);
                self.bit_error(o.span, op.symbol(), &found);
                None
            }
            OpClass::Logic => unreachable!(),
            _ => match op {
                BinOp::Pow => self.unit_pow(lv, lo, rv, ro, span),
                BinOp::Mul | BinOp::Div => self.unit_muldiv(op, lv, lo, rv, ro, span),
                _ => self.unit_additive(op, lv, lo, rv, ro, span),
            },
        }
    }

    /// The two sides as bare numbers of one type (the left side leads).
    fn unit_pair(&mut self, lv: Val, lo: Origin, rv: Val, ro: Origin) -> Option<(Q, Q, Type)> {
        match (lv, rv) {
            (Val::Expr(a), rv) => {
                let lead = a.ty.bare();
                let q = Q { e: strip(a) };
                let b = match rv {
                    Val::Expr(b) => {
                        let e = strip(b);
                        let e = if e.ty == lead { e } else { self.into(Val::Expr(e), lead, ro)? };
                        Q { e }
                    }
                    Val::Const(c) => Q { e: self.fit_const(&c, lead, ro.span, false) },
                    Val::UConst(c, _) => Q { e: self.fit_const(&c, lead, ro.span, false) },
                    Val::NoneLit(_) => unreachable!("`none` has no unit"),
                };
                Some((q, b, lead))
            }
            (lv, Val::Expr(b)) => {
                let other = b.ty.bare();
                let c = match lv {
                    Val::Const(c) | Val::UConst(c, _) => c,
                    Val::Expr(_) | Val::NoneLit(_) => unreachable!("a unit operand is a number"),
                };
                let ty = if c.fits(other) { other } else { c.default_type()? };
                let a = Q { e: self.fit_const(&c, ty, lo.span, true) };
                let lead = ty;
                let e = strip(b);
                let e = if e.ty == lead { e } else { self.into(Val::Expr(e), lead, ro)? };
                Some((a, Q { e }, lead))
            }
            _ => unreachable!("constants are folded first"),
        }
    }

    fn plain_beside_measure(&mut self, v: Val, o: Origin, u: U) -> Option<Val> {
        if let Val::Const(c) = &v
            && is_zero_const(c)
            && u.offset.is_zero()
        {
            return Some(Val::UConst(c.clone(), u));
        }
        let is_const = matches!(v, Val::Const(_));
        self.plain_for_measure(o, is_const, Some(u));
        None
    }

    /// `+ - %` and the comparisons.
    fn unit_additive(&mut self, op: BinOp, lv: Val, lo: Origin, rv: Val, ro: Origin, span: Span) -> Option<Val> {
        let cmp = op.class() == OpClass::Comparison;
        let (mut lv, mut rv) = (lv, rv);
        match (val_unit(&lv), val_unit(&rv)) {
            (None, Some(u)) => lv = self.plain_beside_measure(lv, lo, u)?,
            (Some(u), None) => rv = self.plain_beside_measure(rv, ro, u)?,
            _ => {}
        }
        let (lu, ru) = (val_unit(&lv)?, val_unit(&rv)?);
        let (ld, rd) = (self.describe(&lv), self.describe(&rv));
        if lu.dim != ru.dim {
            let what = if cmp { "compared" } else { "added or subtracted" };
            self.units_differ(
                lo,
                &ld,
                ro,
                &rd,
                &format!("values can only be {what} when they are the same kind of unit"),
                "to relate different kinds, multiply or divide them: `distance / time`",
            );
            return None;
        }
        // The unit both sides are brought to, and the unit of the result.
        let (pl, pr) = (!lu.offset.is_zero(), !ru.offset.is_zero());
        let mut common = false;
        let (work, res) = match (pl, pr) {
            _ if (pl || pr) && op == BinOp::Rem => {
                self.point_error(span, "`%` cannot be used on a point", "subtract two points to get a delta, then use `%` on that");
                return None;
            }
            (false, false) => {
                common = lu != ru;
                (lu, lu)
            }
            (true, true) => match op {
                BinOp::Add => {
                    self.point_error(span, "two points cannot be added", "add a `delta` to a point, or subtract two points to get a delta");
                    return None;
                }
                BinOp::Sub => (lu, self.table(|t| t.delta_of(lu))),
                _ => (lu, lu),
            },
            (true, false) => {
                if cmp {
                    self.units_differ(lo, &ld, ro, &rd, "a point is a position on a scale and a delta is an amount; they cannot be compared", "convert the point first: `t::in<K>()`");
                    return None;
                }
                (lu, lu)
            }
            (false, true) => {
                if cmp {
                    self.units_differ(lo, &ld, ro, &rd, "a point is a position on a scale and a delta is an amount; they cannot be compared", "convert the point first: `t::in<K>()`");
                    return None;
                }
                if op == BinOp::Sub {
                    self.point_error(span, "a point cannot be subtracted from a delta", "write `point - delta`, or subtract two points to get a delta");
                    return None;
                }
                (ru, ru)
            }
        };
        // Constants fold exactly.
        if let (Val::UConst(ca, _), Val::UConst(cb, _)) = (&lv, &rv) {
            let (a, b) = if pl != pr {
                // An amount added to a point converts by scale alone.
                (convert_amount_const(ca, lu, work), convert_amount_const(cb, ru, work))
            } else {
                (convert_const(ca, Some(lu), Some(work)), convert_const(cb, Some(ru), Some(work)))
            };
            return match Const::binary(op, &a, &b) {
                Ok(Folded::Value(c)) => Some(Val::UConst(c, res)),
                Ok(Folded::Bool(b)) => Some(Val::Expr(hir::Expr::new(hir::ExprKind::Bool(b), Type::Bool))),
                Err(err) => {
                    self.fold_error(err, op, span, ro);
                    None
                }
            };
        }
        let (a, b, t) = self.unit_pair(lv, lo, rv, ro)?;
        let hop = hir_op(op);
        let at = Origin { span, compound: true };
        let result = if common {
            // Through the common base (the owner's prefix note):
            // a + b = (a * ka + b * kb) / ka, a < b = a * ka < b * kb.
            let (ka, kb) = scale_pair(&lu.scale, &ru.scale);
            let w = work_type(t, false);
            let (mut aw, mut bw) = (conv(a.e, w), conv(b.e, w));
            if !ka.is_one() {
                aw = bin(hir::BinOp::Mul, aw, lit_int(&ka, w), w);
            }
            if !kb.is_one() {
                bw = bin(hir::BinOp::Mul, bw, lit_int(&kb, w), w);
            }
            if cmp {
                bin(hop, aw, bw, Type::Bool)
            } else {
                let mut x = bin(hop, aw, bw, w);
                if !ka.is_one() {
                    x = bin(hir::BinOp::Div, x, lit_int(&ka, w), w);
                    if t.is_int() {
                        let info = ScaleInfo { lossy: true, overflow: false };
                        self.warn_scale(self.g.unit_type(t, ru), self.g.unit_type(t, lu), &info, at, true);
                    }
                }
                retag(conv(x, t), self.g.unit_type(t, res))
            }
        } else {
            let (ra, mut ca) = ratio_of(Some(lu), Some(work));
            let (rb, mut cb) = ratio_of(Some(ru), Some(work));
            if pl != pr {
                // An amount added to a point converts by scale alone.
                ca = BigRational::zero();
                cb = BigRational::zero();
            }
            let (ae, ia) = self.scale_expr(a.e, t, &ra, &ca);
            let (be, ib) = self.scale_expr(b.e, t, &rb, &cb);
            if t.is_int() {
                let (wt, rt) = (self.g.unit_type(t, work), self.g.unit_type(t, ru));
                self.warn_scale(rt, wt, &ia, ro, true);
                self.warn_scale(rt, wt, &ib, ro, true);
            }
            if cmp {
                bin(hop, ae, be, Type::Bool)
            } else {
                bin(hop, ae, be, self.g.unit_type(t, res))
            }
        };
        Some(Val::Expr(result))
    }

    fn table<R>(&self, f: impl FnOnce(&mut UnitTable) -> R) -> R {
        f(&mut self.g.units.borrow_mut().table)
    }

    /// `*` and `/`.
    fn unit_muldiv(&mut self, op: BinOp, lv: Val, lo: Origin, rv: Val, ro: Origin, span: Span) -> Option<Val> {
        let (lu, ru) = (val_unit(&lv), val_unit(&rv));
        for (u, o) in [(lu, lo), (ru, ro)] {
            if is_point(u) {
                self.point_error(o.span, "a point cannot be multiplied or divided", "subtract two points to get a delta, and scale that");
                return None;
            }
        }
        // The unit of the result, and whether every dimension cancels (Q3).
        let table_op = |t: &mut UnitTable, a: U, b: U| if op == BinOp::Mul { t.mul(a, b) } else { t.div(a, b) };
        let (res, collapse) = match (lu, ru) {
            (Some(a), Some(b)) => {
                let r = self.table(|t| table_op(t, a, b));
                let u = self.check_unit(r, span)?;
                (u, u.dim.is_empty())
            }
            (Some(a), None) => (a, false),
            (None, Some(b)) if op == BinOp::Mul => (b, false),
            (None, Some(b)) => {
                let r = self.table(|t| {
                    let one = t.dimensionless();
                    t.div(one, b)
                });
                let u = self.check_unit(r, span)?;
                (u, u.dim.is_empty())
            }
            (None, None) => unreachable!(),
        };
        if let (Val::Const(_) | Val::UConst(..), Val::Const(_) | Val::UConst(..)) = (&lv, &rv) {
            let (ca, cb) = match (&lv, &rv) {
                (Val::Const(a) | Val::UConst(a, _), Val::Const(b) | Val::UConst(b, _)) => (a, b),
                _ => unreachable!(),
            };
            return match Const::binary(op, ca, cb) {
                Ok(Folded::Value(c)) if collapse => {
                    let whole = matches!(c, Const::Int(_));
                    Some(Val::Const(rat_const(rational(&c) * &res.scale, whole)))
                }
                Ok(Folded::Value(c)) => Some(Val::UConst(c, res)),
                Ok(Folded::Bool(_)) => unreachable!(),
                Err(err) => {
                    self.fold_error(err, op, span, ro);
                    None
                }
            };
        }
        let (a, b, t) = self.unit_pair(lv, lo, rv, ro)?;
        let hop = hir_op(op);
        let e = if collapse {
            self.collapse(op, a.e, b.e, &res.scale, t)
        } else {
            bin(hop, a.e, b.e, self.g.unit_type(t, res))
        };
        Some(Val::Expr(e))
    }

    fn check_unit<T>(&mut self, r: Result<T, UnitError>, at: Span) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(e) => {
                self.diags.push(unit_error(e, at));
                None
            }
        }
    }

    /// `a op b` where every dimension cancels: a plain number with the
    /// scale folded in. For whole numbers the numerator scale multiplies
    /// first and the denominator scale goes into the divisor, so `4<km> /
    /// 3<m>` is `(4 * 1000) / 3`, not `(4 / 3) * 1000`.
    fn collapse(&mut self, op: BinOp, a: hir::Expr, b: hir::Expr, scale: &BigRational, t: Type) -> hir::Expr {
        if op == BinOp::Mul || t.is_float() {
            let x = bin(hir_op(op), a, b, t);
            return self.scale_expr(x, t, scale, &BigRational::zero()).0;
        }
        let (p, q) = (scale.numer(), scale.denom());
        let w = work_type(t, false);
        let (mut aw, mut bw) = (conv(a, w), conv(b, w));
        if !p.is_one() {
            aw = bin(hir::BinOp::Mul, aw, lit_int(p, w), w);
        }
        if !q.is_one() {
            bw = bin(hir::BinOp::Mul, bw, lit_int(q, w), w);
        }
        conv(bin(hir::BinOp::Div, aw, bw, w), t)
    }

    /// `a ** n` with a constant whole exponent.
    fn unit_pow(&mut self, lv: Val, lo: Origin, rv: Val, ro: Origin, span: Span) -> Option<Val> {
        let n = match &rv {
            Val::Const(Const::Int(n)) if val_unit(&lv).is_some() => n.clone(),
            _ => {
                let found = self.describe(&rv);
                self.diags.push(
                    Diagnostic::at(&codes::E0404, ro.span, "a unit can only be raised to a constant whole power")
                        .note(format!("this is {found}, not a constant whole number"))
                        .note_line("the unit of the result depends on the exponent, so it has to be known while compiling")
                        .help("write the exponent as a constant (`side ** 2`), or multiply: `side * side`"),
                );
                return None;
            }
        };
        let Some(a) = val_unit(&lv) else { unreachable!() };
        if !a.offset.is_zero() {
            self.point_error(lo.span, "a point cannot be raised to a power", "subtract two points to get a delta, and use that");
            return None;
        }
        let Some(n32) = n.to_i32() else {
            self.diags.push(unit_error(UnitError::ExponentRange, ro.span));
            return None;
        };
        let r = self.table(|t| t.pow(a, n32));
        let res = self.check_unit(r, span)?;
        let collapse = res.dim.is_empty();
        match lv {
            Val::UConst(c, _) => match Const::binary(BinOp::Pow, &c, &Const::Int(n)) {
                Ok(Folded::Value(v)) if collapse => {
                    let whole = matches!(v, Const::Int(_));
                    Some(Val::Const(rat_const(rational(&v) * &res.scale, whole)))
                }
                Ok(Folded::Value(v)) => Some(Val::UConst(v, res)),
                Ok(Folded::Bool(_)) => unreachable!(),
                Err(err) => {
                    self.fold_error(err, BinOp::Pow, span, ro);
                    None
                }
            },
            Val::NoneLit(_) => unreachable!("`none` has no unit"),
            Val::Expr(e) => {
                let lead = e.ty.bare();
                let Val::Expr(x) = self.binary_vals(BinOp::Pow, Val::Expr(strip(e)), lo, rv, ro, span)? else { return None };
                if collapse {
                    Some(Val::Expr(self.scale_expr(x, lead, &res.scale, &BigRational::zero()).0))
                } else {
                    let ty = self.g.unit_type(lead, res);
                    Some(Val::Expr(retag(x, ty)))
                }
            }
            Val::Const(_) => unreachable!(),
        }
    }

    /// `-x` and `~x` on a value that may have a unit. `Ok` hands the value
    /// on to the ordinary code; `Err` is the finished answer.
    pub(super) fn unit_unary(&mut self, op: UnOp, v: Val, span: Span) -> Result<Val, Option<Val>> {
        let Some(u) = val_unit(&v) else { return Ok(v) };
        if op == UnOp::BitNot {
            let found = self.describe(&v);
            self.bit_error(span, "~", &found);
            return Err(None);
        }
        if !u.offset.is_zero() {
            self.point_error(span, "a point cannot be negated", "negate a delta instead: subtract two points first");
            return Err(None);
        }
        match v {
            Val::UConst(c, u) => Err(Some(Val::UConst(c.neg(), u))),
            v => Ok(v),
        }
    }

    // --- methods --------------------------------------------------------

    /// `x::in<unit>()`: the unit changes, the number type stays (Q11).
    pub(super) fn unit_in(&mut self, recv: &ast::Expr, spec: &ast::UnitSpec, args: &[ast::Expr], name_span: Span, span: Span) -> Option<Val> {
        if let Some(a) = args.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, a.span, "`in` takes no arguments"));
        }
        let v = self.expr(recv);
        let target = self.g.resolve_unit_spec(spec, span, self.diags);
        let (v, target) = (v?, target?);
        let o = super::origin(recv);
        let Some(fu) = val_unit(&v) else {
            match &v {
                Val::Expr(e) if !e.ty.is_numeric() => {
                    self.diags.push(
                        Diagnostic::at(&codes::E0201, name_span, format!("`{}` has no method `in`", e.ty)).label(recv.span, format!("this is `{}`", e.ty)),
                    );
                }
                _ => {
                    // `5::in<m>()` is a conversion of a measure, not a slot (Q12b covers
                    // declaring and assigning only), so a plain number still needs its unit.
                    self.plain_for_measure(o, matches!(v, Val::Const(_)), Some(target));
                }
            }
            return None;
        };
        let (from, to) = (self.describe(&v), self.unit_name_of(Some(target)));
        if fu.dim != target.dim {
            self.diags.push(
                Diagnostic::at(&codes::E0407, name_span, format!("cannot convert {from} to `{to}`"))
                    .note(format!("`{to}` measures a different kind of thing"))
                    .label(recv.span, format!("this is {from}"))
                    .help("`::in<unit>()` converts to a unit of the same kind"),
            );
            return None;
        }
        if spec.delta && !fu.offset.is_zero() {
            self.diags.push(
                Diagnostic::at(&codes::E0407, name_span, format!("cannot convert {from} to `{to}`"))
                    .note("a point is not a delta")
                    .label(recv.span, format!("this is {from}"))
                    .help("subtract two points to get a delta"),
            );
            return None;
        }
        match v {
            Val::UConst(c, _) => Some(Val::UConst(convert_const(&c, Some(fu), Some(target)), target)),
            Val::Expr(e) => {
                let to = self.g.unit_type(e.ty.bare(), target);
                Some(Val::Expr(self.convert_value(e, to, true, o)))
            }
            Val::Const(_) | Val::NoneLit(_) => unreachable!("a unit value is a number"),
        }
    }

    /// `x::to<T>()` where a unit is involved: the number type changes and the
    /// unit stays (Q11).
    pub(super) fn unit_to(&mut self, value: Val, to: Type, recv: Span, targ: Span) -> Option<Val> {
        if to.unit().is_some() {
            self.diags.push(
                Diagnostic::at(&codes::E0407, targ, format!("`to` changes the number type, but `{to}` has a unit"))
                    .note("a unit does not go here")
                    .help("to change the unit write `x::in<km>()`"),
            );
            return None;
        }
        if !to.is_numeric() {
            self.diags.push(Diagnostic::at(&codes::E0101, targ, format!("`to` converts between number types, not to `{to}`")));
            return None;
        }
        match value {
            Val::UConst(c, u) => {
                let e = self.fit_const(&c, to, recv, true);
                Some(Val::Expr(retag(e, self.g.unit_type(to, u))))
            }
            Val::Expr(e) if e.ty.is_numeric() => {
                let u = e.ty.unit();
                let ty = match u {
                    Some(u) => self.g.unit_type(to, u),
                    None => to,
                };
                Some(Val::Expr(retag(conv(strip(e), to), ty)))
            }
            Val::Expr(e) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0101, recv, format!("`to` converts numbers, found `{}`", e.ty)).note(format!("this is `{}`", e.ty)),
                );
                None
            }
            Val::Const(_) | Val::NoneLit(_) => unreachable!("a unit value is a number"),
        }
    }

    /// `x::unitText()`: the unit's name as a `string` constant (Q2).
    pub(super) fn unit_text(&mut self, value: Option<Val>, recv: Span, name_span: Span) -> Option<Val> {
        let value = value?;
        let Some(u) = val_unit(&value) else {
            let found = self.describe(&value);
            self.diags.push(
                Diagnostic::at(&codes::E0101, name_span, format!("`unitText` needs a number with a unit, found {found}"))
                    .label(recv, format!("this is {found}"))
                    .help("a number gets a unit from a literal such as `3<m>` or a typed variable such as `float64<m>`"),
            );
            return None;
        };
        let name = self.unit_name_of(Some(u));
        Some(Val::Expr(super::str_lit(name)))
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
    for e in &mut p.enums {
        for v in &mut e.variants {
            for field in &mut v.fields {
                f(&mut field.ty);
            }
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
        hir::Stmt::Assign(_, e) | hir::Stmt::Expr(e) | hir::Stmt::InitField(_, e) => visit_expr(e, f),
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
        hir::Stmt::Resume(_, value) => {
            if let Some(e) = value {
                visit_expr(e, f);
            }
        }
        hir::Stmt::Reraise(_) => {}
        hir::Stmt::Jump(_) | hir::Stmt::Decl(_) | hir::Stmt::Destroy(_) | hir::Stmt::Forget(_) | hir::Stmt::Check { .. } => {}
    }
}

fn visit_expr(e: &mut hir::Expr, f: &mut dyn FnMut(&mut Type)) {
    f(&mut e.ty);
    for child in children_mut(e) {
        visit_expr(child, f);
    }
    if let hir::ExprKind::Handled { arms, .. } = &mut e.kind {
        for a in arms {
            visit_block(&mut a.body, f);
        }
    }
}

/// The direct sub-expressions of `e`. Every variant is named, so a new one
/// is a compile error here until it is handled.
fn children_mut(e: &mut hir::Expr) -> Vec<&mut hir::Expr> {
    use hir::ExprKind as K;
    match &mut e.kind {
        K::Int(_) | K::Float(_) | K::Bool(_) | K::Str(_) | K::Local(_) | K::FuncRef(_) | K::NullPtr | K::InLine | K::InEnded => Vec::new(),
        K::IncDec { .. } => Vec::new(),
        K::Field(x, _)
        | K::Neg(x)
        | K::Not(x)
        | K::BitNot(x)
        | K::Convert(x)
        | K::PtrConv(x)
        | K::Unsize(x)
        | K::Text(x)
        | K::DeepCopy(x)
        | K::Deref(x)
        | K::Lend(x)
        | K::Discriminant(x)
        | K::Payload { value: x, .. }
        | K::AddressOf(x) => vec![&mut **x],
        K::Variant { args, .. } => args.iter_mut().collect(),
        K::Len(x) => vec![&mut **x],
        K::Index(a, b) | K::ListPush { list: a, value: b } => vec![&mut **a, &mut **b],
        K::ListCall { args, .. } => args.iter_mut().collect(),
        K::List(items) | K::Array(items) => items.iter_mut().collect(),
        K::Map(pairs) => pairs.iter_mut().flat_map(|(k, v)| [k, v]).collect(),
        K::MapCall { args, .. } => args.iter_mut().collect(),
        K::Radix { value, .. } => vec![&mut **value],
        K::Out { arg, .. } => vec![&mut **arg],
        K::TryParse { text, .. } => vec![&mut **text],
        K::FailMaybe { call, fallback } => vec![&mut **call, &mut **fallback],
        K::PassUp { call } | K::Handled { call, .. } => vec![&mut **call],
        K::CauseWrap { msg, slot } => vec![&mut **msg, &mut **slot],
        K::Report { msg, slot, exit } => vec![&mut **msg, &mut **slot].into_iter().chain(exit.as_deref_mut()).collect(),
        K::Borrow { place, .. } => vec![&mut **place],
        K::Binary(_, a, b) | K::IntPow(a, b) => vec![&mut **a, &mut **b],
        K::Logic { lhs, rhs, .. } | K::Maybe { value: lhs, fallback: rhs } => vec![&mut **lhs, &mut **rhs],
        K::Shift { value, amount, .. } => vec![&mut **value, &mut **amount],
        K::Struct(fields) => fields.iter_mut().map(|(_, v)| v).collect(),
        K::Call(_, args) | K::StrCall(_, args) | K::DynCall(_, _, args) | K::FnCall(args) | K::Closure(_, args) => args.iter_mut().collect(),
        K::Join(parts) => parts.iter_mut().collect(),
        K::Take { leaving, .. } => leaving.as_deref_mut().into_iter().collect(),
    }
}
