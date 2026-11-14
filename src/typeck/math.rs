//! `math` (step 7 WP6): the checking of a call of `math::sqrt(x)` or `x::sqrt()`, and the
//! constants `float64::pi` and friends.
//!
//! Every `math` function is an intrinsic of `src/stdlib/math.drh`, so one rule here
//! serves every number type and carries units through (owner, 2026-11-14):
//!
//! - `sqrt`: the powers of the unit halve (`9.0<m^2>` gives `3.0<m>`); an odd power is
//!   E0410.
//! - `abs min max clamp floor ceil round`: the unit stays; the arguments must agree
//!   (E0401), and `min max clamp` bring them to the first one's unit.
//! - `sin cos tan`: a plain number (radians) or an angle unit; `atan2` gives radians.
//! - `pow`: with a unit, the exponent is a constant whole number and the powers multiply;
//!   otherwise plain numbers only (E0404, E0411). `exp log log2 log10` are plain.
//! - `abs min max clamp` work on whole numbers too; the rest need floats (interim: a
//!   whole-number value is E0101 with the conversion suggested; a constant such as `2`
//!   is taken as a float, like everywhere a float is wanted).

use num_bigint::BigInt;
use num_traits::{One, ToPrimitive, Zero};

use super::{Checker, Origin, Val};
use crate::ast;
use crate::consts::Const;
use crate::diag::{codes, Diagnostic, Span};
use crate::hir::{self, MathFn};
use crate::types::Type;
use crate::units::Unit;

type U = &'static Unit;

fn unit_of(v: &Val) -> Option<U> {
    match v {
        Val::UConst(_, u) => Some(u),
        Val::Expr(e) => e.ty.unit(),
        Val::Const(_) | Val::NoneLit(_) => None,
    }
}

/// The type a value has, units included, if it is not an untyped constant.
fn typed(v: &Val) -> Option<Type> {
    match v {
        Val::Expr(e) => Some(e.ty),
        _ => None,
    }
}

fn is_signed(t: Type) -> bool {
    matches!(t.bare(), Type::Int { signed: true, .. } | Type::Float(_))
}

impl Checker<'_> {
    /// A call of `math::NAME`. `recv` is the value of `x` in `x::NAME(...)`.
    pub(super) fn math_call(&mut self, name: &str, targs: &[ast::TypeName], args: &[ast::Expr], span: Span, recv: Option<(Val, Origin)>) -> Option<Val> {
        let Some(f) = MathFn::from_name(name) else {
            for a in args {
                self.expr(a);
            }
            self.diags.push(
                Diagnostic::at(&codes::E0201, span, format!("module `math` has no function `{name}`"))
                    .help("it has sqrt, pow, exp, log, log2, log10, hypot, floor, ceil, round, sin, cos, tan, atan2, abs, min, max and clamp"),
            );
            return None;
        };
        let mut vals: Vec<Option<(Val, Origin)>> = Vec::new();
        if let Some(r) = recv {
            vals.push(Some(r));
        }
        for a in args {
            let o = super::origin(a);
            vals.push(self.expr(a).map(|v| (v, o)));
        }
        let vals: Option<Vec<(Val, Origin)>> = vals.into_iter().collect();
        let vals = vals?;
        if let Some(t) = targs.first() {
            self.diags.push(Diagnostic::at(&codes::E0103, t.span, format!("`math::{name}` takes no type arguments")));
            return None;
        }
        if vals.len() != f.arity() {
            let n = f.arity();
            self.diags.push(
                Diagnostic::at(&codes::E0103, span, format!("`math::{name}` takes {n} argument{}, but {} given", if n == 1 { "" } else { "s" }, vals.len())),
            );
            return None;
        }
        // Borrows are read through.
        let vals: Vec<(Val, Origin)> = vals.into_iter().map(|(v, o)| (self.read(v), o)).collect();
        for (v, o) in &vals {
            if let Val::Expr(e) = v
                && !e.ty.is_numeric()
            {
                self.diags.push(
                    Diagnostic::at(&codes::E0101, o.span, format!("`math::{name}` needs a number, found `{}`", e.ty))
                        .note(format!("this is `{}`", e.ty)),
                );
                return None;
            }
            if let Val::NoneLit(s) = v {
                self.none_without_type(*s);
                return None;
            }
        }
        let mode = self.g.settings_at(span).overflow;
        match f {
            MathFn::Abs | MathFn::Min | MathFn::Max | MathFn::Clamp => self.math_same(f, vals, span, mode),
            _ => self.math_float(f, vals, span, mode),
        }
    }

    /// `abs min max clamp`: any number type, the unit stays, the arguments agree.
    fn math_same(&mut self, f: MathFn, vals: Vec<(Val, Origin)>, span: Span, mode: crate::config::Overflow) -> Option<Val> {
        // The first argument leads, as the left side of `+` does: its unit is the result's.
        // A constant takes the number type of the first typed argument, or its own default.
        let typed_ty = vals.iter().find_map(|(v, _)| typed(v));
        let lead = match &vals[0].0 {
            Val::Expr(e) => e.ty,
            Val::UConst(c, u) => {
                let ty = typed_ty.map(Type::bare).or_else(|| c.default_type()).unwrap_or(Type::F64);
                self.g.unit_type(ty, u)
            }
            Val::Const(c) => typed_ty.or_else(|| c.default_type()).unwrap_or(Type::F64),
            Val::NoneLit(_) => unreachable!("checked before"),
        };
        let mut es = Vec::new();
        for (v, o) in vals {
            self.agree(lead, &v, o)?;
            es.push(self.into(v, lead, o)?);
        }
        if f == MathFn::Abs && !is_signed(lead) {
            // A whole number that cannot be negative is its own absolute value.
            return Some(Val::Expr(es.remove(0)));
        }
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Math(f, es, mode), lead).at(span)))
    }

    /// The functions that need a float.
    fn math_float(&mut self, f: MathFn, vals: Vec<(Val, Origin)>, span: Span, mode: crate::config::Overflow) -> Option<Val> {
        let name = f.name();
        // The float type: that of the first typed argument; constants alone are `float64`.
        // A whole number gives the whole-number version where there is one (owner,
        // 2026-11-14): `sqrt`, `log`, `log2`, `log10`, `pow` (the base leads) and the
        // roundings, which leave a whole number alone.
        let lead_ty = if f == MathFn::Pow { typed(&vals[0].0) } else { vals.iter().find_map(|(v, _)| typed(v)) };
        let whole_ok = matches!(f, MathFn::Sqrt | MathFn::Log | MathFn::Log2 | MathFn::Log10 | MathFn::Pow | MathFn::Floor | MathFn::Ceil | MathFn::Round);
        let mut whole = false;
        let float = match lead_ty.or_else(|| vals.iter().find_map(|(v, _)| typed(v))) {
            Some(t) if t.bare().is_float() => t.bare(),
            Some(t) if t.bare().is_int() && whole_ok && lead_ty.is_some() => {
                whole = true;
                t.bare()
            }
            Some(t) => {
                let o = vals.iter().find(|(v, _)| typed(v) == Some(t)).map(|(_, o)| *o)?;
                self.diags.push(
                    Diagnostic::at(&codes::E0101, o.span, format!("`math::{name}` needs a `float64` or `float32`, found `{t}`"))
                        .note(format!("this is `{t}`, a whole number type"))
                        .note_line(format!("`math::{name}` has no whole-number version"))
                        .help(if o.compound {
                            "convert it first: `(value)::to<float64>()`".to_string()
                        } else {
                            "convert it first: `value::to<float64>()`".to_string()
                        }),
                );
                return None;
            }
            None => Type::F64,
        };
        let mut vals = vals;
        if whole {
            return self.math_whole(f, vals, float, span, mode);
        }
        let result: (Vec<hir::Expr>, Type) = match f {
            MathFn::Sqrt => {
                let (v, o) = vals.remove(0);
                let (x, u) = self.measure_arg(v, o, float)?;
                self.math_sqrt(x, u, o, span)?
            }
            MathFn::Floor | MathFn::Ceil | MathFn::Round => {
                let (v, o) = vals.remove(0);
                let (x, u) = self.measure_arg(v, o, float)?;
                let ty = match u {
                    Some(u) => self.g.unit_type(float, u),
                    None => float,
                };
                (vec![x], ty)
            }
            MathFn::Sin | MathFn::Cos | MathFn::Tan => {
                let (v, o) = vals.remove(0);
                (vec![self.angle_arg(name, v, o, float)?], float)
            }
            MathFn::Exp | MathFn::Log | MathFn::Log2 | MathFn::Log10 => {
                let (v, o) = vals.remove(0);
                (vec![self.plain_arg(name, v, o, float)?], float)
            }
            MathFn::Hypot | MathFn::Atan2 => {
                // The arguments agree: the second is brought to the first's unit.
                let (v0, o0) = vals.remove(0);
                let (v1, o1) = vals.remove(0);
                let (x, u) = self.measure_arg(v0, o0, float)?;
                let lead = match u {
                    Some(u) => self.g.unit_type(float, u),
                    None => float,
                };
                self.agree(lead, &v1, o1)?;
                let y = self.into(v1, lead, o1)?;
                let ty = if f == MathFn::Hypot {
                    lead
                } else if self.g.imports_lib(span, "si") && let Some(rad) = self.table(|t| t.lookup("rad")) {
                    // The angle, in a file that has `in si;`.
                    self.g.unit_type(float, rad)
                } else {
                    float
                };
                (vec![x, y], ty)
            }
            MathFn::Pow => {
                let (base, bo) = vals.remove(0);
                let (exp, eo) = vals.remove(0);
                let (b, u) = self.measure_arg(base, bo, float)?;
                match u {
                    None => (vec![b, self.plain_arg(name, exp, eo, float)?], float),
                    Some(u) => {
                        let n = match &exp {
                            Val::Const(Const::Int(n)) => n.to_i32(),
                            Val::Const(Const::Dec(r)) if r.is_integer() => r.to_integer().to_i32(),
                            _ => None,
                        };
                        let Some(n) = n else {
                            self.diags.push(
                                Diagnostic::at(&codes::E0404, eo.span, "a unit can only be raised to a constant whole power")
                                    .note("the unit of the result depends on the exponent, so it has to be known while compiling")
                                    .help("write the exponent as a constant (`math::pow(side, 2)`), or multiply: `side * side`"),
                            );
                            return None;
                        };
                        if !u.offset.is_zero() {
                            self.point_error(bo.span, "a point cannot be raised to a power", "subtract two points to get a delta, and use that");
                            return None;
                        }
                        let r = self.table(|t| t.pow(u, n));
                        let res = self.check_unit(r, span)?;
                        let ty = if res.dim.is_empty() && res.scale.is_one() { float } else { self.g.unit_type(float, res) };
                        let exp = self.fit_const(&Const::Int(BigInt::from(n)), float, eo.span, true);
                        (vec![b, exp], ty)
                    }
                }
            }
            MathFn::Abs | MathFn::Min | MathFn::Max | MathFn::Clamp => unreachable!("handled by math_same"),
        };
        let (es, ty) = result;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Math(f, es, mode), ty).at(span)))
    }

    /// The whole-number versions (`float` is the whole number type of the leading
    /// argument): `sqrt`, `log`, `log2` and `log10` round the true result down, `pow`
    /// is the exact integer power (it follows `overflow`, a negative exponent as
    /// `**` does), and `floor`, `ceil` and `round` give the number back. Every one
    /// but the roundings warns (W0112).
    fn math_whole(&mut self, f: MathFn, mut vals: Vec<(Val, Origin)>, int: Type, span: Span, mode: crate::config::Overflow) -> Option<Val> {
        let name = f.name();
        let mut warn = true;
        let result: (Vec<hir::Expr>, Type) = match f {
            MathFn::Sqrt => {
                let (v, o) = vals.remove(0);
                let (x, u) = self.measure_arg(v, o, int)?;
                self.math_sqrt(x, u, o, span)?
            }
            MathFn::Floor | MathFn::Ceil | MathFn::Round => {
                let (v, o) = vals.remove(0);
                let (x, _) = self.measure_arg(v, o, int)?;
                return Some(Val::Expr(x));
            }
            MathFn::Log | MathFn::Log2 | MathFn::Log10 => {
                let (v, o) = vals.remove(0);
                (vec![self.plain_arg(name, v, o, int)?], int)
            }
            MathFn::Pow => {
                let (base, bo) = vals.remove(0);
                let (exp, eo) = vals.remove(0);
                // A non-negative constant exponent cannot give a fraction.
                warn = !matches!(&exp, Val::Const(c) if !c.is_negative());
                let r = self.binary_vals(crate::ast::BinOp::Pow, base, bo, exp, eo, span)?;
                if warn {
                    self.whole_warning(name, span);
                }
                return Some(r);
            }
            _ => unreachable!("only the functions with a whole-number version come here"),
        };
        if warn {
            self.whole_warning(name, span);
        }
        let (es, ty) = result;
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Math(f, es, mode), ty).at(span)))
    }

    fn whole_warning(&mut self, name: &str, span: Span) {
        self.diags.push(
            Diagnostic::at(&codes::W0112, span, format!("`math::{name}` of a whole number gives a whole number"))
                .note("the result is rounded down")
                .help("for a fractional result convert first: `value::to<float64>()`"),
        );
    }

    /// An argument that may carry a unit: the value as a bare `float` expression, and its unit.
    fn measure_arg(&mut self, v: Val, o: Origin, float: Type) -> Option<(hir::Expr, Option<U>)> {
        match v {
            Val::Const(c) => Some((self.fit_const(&c, float, o.span, true), None)),
            Val::UConst(c, u) => {
                let e = self.fit_const(&c, float, o.span, true);
                Some((e, Some(u)))
            }
            Val::Expr(mut e) => {
                let u = e.ty.unit();
                let want = match u {
                    Some(u) => self.g.unit_type(float, u),
                    None => float,
                };
                if e.ty.bare() != float {
                    self.diags.push(
                        Diagnostic::at(&codes::E0101, o.span, format!("expected `{want}`, found `{}`", e.ty))
                            .note(format!("this is `{}`", e.ty)),
                    );
                    return None;
                }
                e.ty = want;
                Some((e, u))
            }
            Val::NoneLit(_) => unreachable!("checked before"),
        }
    }

    /// An argument that must be a plain number (a unit with nothing left, `pct`, is fine).
    fn plain_arg(&mut self, name: &str, v: Val, o: Origin, float: Type) -> Option<hir::Expr> {
        if let Some(u) = unit_of(&v)
            && !u.dim.is_empty()
        {
            let found = self.describe_arg(&v);
            self.diags.push(
                Diagnostic::at(&codes::E0411, o.span, format!("`math::{name}` takes a plain number, found {found}"))
                    .note(format!("this is {found}"))
                    .help("divide by a unit of the same kind first to get a plain number: `value / 1.0<m>`"),
            );
            return None;
        }
        self.into(v, float, o)
    }

    /// An angle for `sin`, `cos`, `tan`: a plain number (radians) or an angle unit (`rad`, `deg`).
    fn angle_arg(&mut self, name: &str, v: Val, o: Origin, float: Type) -> Option<hir::Expr> {
        if let Some(u) = unit_of(&v)
            && !u.dim.is_empty()
            && !(u.dim.len() == 1 && u.dim[0] == ("rad", 1))
        {
            let found = self.describe_arg(&v);
            self.diags.push(
                Diagnostic::at(&codes::E0411, o.span, format!("`math::{name}` takes a plain number or an angle, found {found}"))
                    .note(format!("this is {found}"))
                    .help("a plain number is radians; an angle has the unit `rad` or `deg` (from `in si;`)"),
            );
            return None;
        }
        // An angle unit goes to its base amount, in radians.
        self.into(v, float, o)
    }

    /// The argument measures the same kind of thing as the leading one: a plain number
    /// beside a measure (zero excepted) or two kinds of unit are errors, as in `+`.
    fn agree(&mut self, lead: Type, v: &Val, o: Origin) -> Option<()> {
        let (lu, vu) = (lead.unit(), unit_of(v));
        let zero = match v {
            Val::Const(Const::Int(i)) => i.is_zero(),
            Val::Const(Const::Dec(r)) => r.is_zero(),
            Val::Const(Const::NegZero) => true,
            _ => false,
        };
        match (lu, vu) {
            (Some(l), Some(r)) if l.dim == r.dim => Some(()),
            (None, None) => Some(()),
            (Some(_), None) if zero => Some(()),
            (None, Some(r)) if r.dim.is_empty() => Some(()),
            (Some(_), None) => {
                self.diags.push(
                    Diagnostic::at(&codes::E0408, o.span, "a plain number cannot stand for a measure")
                        .note("this number has no unit")
                        .help(format!("give it the unit of `{lead}`, for example `5.0<m>`; only zero needs none")),
                );
                None
            }
            (l, r) => {
                let (ld, rd) = (self.table(|t| l.map_or("a plain number".to_string(), |u| t.display(u))), self.table(|t| r.map_or("a plain number".to_string(), |u| t.display(u))));
                self.diags.push(
                    Diagnostic::at(&codes::E0401, o.span, format!("units do not match: `{ld}` and `{rd}`"))
                        .note(format!("this is `{rd}`"))
                        .note_line("the arguments of this function must measure the same kind of thing"),
                );
                None
            }
        }
    }

    fn describe_arg(&self, v: &Val) -> String {
        match v {
            Val::Expr(e) => format!("`{}`", e.ty),
            Val::UConst(_, u) => format!("a number in `{}`", self.table(|t| t.display(u))),
            _ => "a number".to_string(),
        }
    }

    /// `sqrt` of `x` in unit `u`: the unit's powers halve.
    fn math_sqrt(&mut self, x: hir::Expr, u: Option<U>, o: Origin, span: Span) -> Option<(Vec<hir::Expr>, Type)> {
        let float = x.ty.bare();
        let Some(u) = u else { return Some((vec![x], float)) };
        if !u.offset.is_zero() {
            self.point_error(o.span, "a point has no square root", "subtract two points to get a delta, and use that");
            return None;
        }
        if let Some(odd) = u.dim.iter().find(|(_, e)| e % 2 != 0) {
            let shown = self.table(|t| t.display(u));
            self.diags.push(
                Diagnostic::at(&codes::E0410, o.span, format!("`{shown}` has no square root: the power of `{}` is {}", odd.0, odd.1))
                    .note("every power in the unit must be even")
                    .help("a root halves the powers: `m^2` gives `m`; for an odd power take the root of a product or square first: `math::sqrt(a * b)`"),
            );
            return None;
        }
        if let Some(root) = self.table(|t| t.root(u, 2)) {
            return Some((vec![x], self.g.unit_type(float, root)));
        }
        if !float.is_float() {
            self.diags.push(Diagnostic::at(&codes::E0901, o.span, "the whole-number square root of a unit with a scale is not supported yet"));
            return None;
        }
        // The scale has no exact square root (`2 m^2`): take the root of the amount in the
        // unit with scale one, which does.
        let one = self.table(|t| {
            let recip = u.scale.recip();
            t.scaled(u, &recip)
        });
        let one = self.check_unit(one, span)?;
        let root = self.table(|t| t.root(one, 2))?;
        let factor = u.scale.to_f64().unwrap_or(f64::NAN);
        let ty = x.ty;
        let scaled = hir::Expr::new(
            hir::ExprKind::Binary(hir::BinOp::Mul, Box::new(x), Box::new(hir::Expr::new(hir::ExprKind::Float(if float == Type::Float(32) { factor as f32 as f64 } else { factor }), float))),
            ty,
        );
        Some((vec![scaled], self.g.unit_type(float, root)))
    }

    /// `float64::pi`, `float32::e`, `float64::tau`: the constant at the type's precision.
    pub(super) fn float_constant(&mut self, ty: Type, name: &str, name_span: Span, span: Span) -> Option<Val> {
        let Type::Float(bits) = ty else { return None };
        let v = match name {
            "pi" => std::f64::consts::PI,
            "e" => std::f64::consts::E,
            "tau" => std::f64::consts::TAU,
            _ => {
                self.diags.push(
                    Diagnostic::at(&codes::E0201, name_span, format!("`{ty}` has no constant `{name}`"))
                        .help("the constants are `pi`, `e` and `tau`"),
                );
                return None;
            }
        };
        let v = if bits == 32 { v as f32 as f64 } else { v };
        Some(Val::Expr(hir::Expr::new(hir::ExprKind::Float(v), ty).at(span)))
    }

    /// Is `name` a `math` function the file may call as a method on a number?
    /// The method spelling needs no import (owner, 2026-11-14).
    pub(super) fn math_method_applies(&self, name: &str) -> bool {
        MathFn::from_name(name).is_some()
    }
}
