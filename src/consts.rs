//! Untyped constants. Constant expressions are evaluated exactly (big
//! integers and fractions) and only get a type when they meet one.

use num_bigint::{BigInt, Sign};
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Signed, ToPrimitive, Zero};

use crate::ast::BinOp;
use crate::types::Type;

#[derive(Debug, Clone, PartialEq)]
pub enum Const {
    Int(BigInt),
    /// A decimal constant. Stays decimal even when its value is whole (`2.0`).
    Dec(BigRational),
    /// The sign of decimal zero survives conversion to a float.
    NegZero,
}

pub enum Folded {
    Value(Const),
    Bool(bool),
}

pub enum FoldError {
    DivisionByZero,
    /// The result would be absurdly large.
    TooLarge,
    /// The operator does not apply to decimal constants.
    NeedsWhole,
    NegativeShift,
    /// Not expressible exactly or as a float (e.g. a negative number to a fractional power).
    NotANumber,
}

/// Constants may not grow past this many bits; nothing that large fits any type.
const MAX_BITS: u64 = 4096;

impl Const {
    pub fn is_decimal(&self) -> bool {
        matches!(self, Const::Dec(_) | Const::NegZero)
    }

    pub fn is_negative(&self) -> bool {
        match self {
            Const::Int(v) => v.is_negative(),
            Const::Dec(r) => r.is_negative(),
            Const::NegZero => false,
        }
    }

    fn rational(&self) -> BigRational {
        match self {
            Const::Int(v) => BigRational::from_integer(v.clone()),
            Const::Dec(r) => r.clone(),
            Const::NegZero => BigRational::zero(),
        }
    }

    fn checked(self) -> Result<Const, FoldError> {
        let bits = match &self {
            Const::Int(v) => v.bits(),
            Const::Dec(r) => r.numer().bits().max(r.denom().bits()),
            Const::NegZero => 0,
        };
        if bits > MAX_BITS { Err(FoldError::TooLarge) } else { Ok(self) }
    }

    pub fn neg(&self) -> Const {
        match self {
            Const::Int(v) => Const::Int(-v),
            Const::Dec(r) if r.is_zero() => Const::NegZero,
            Const::Dec(r) => Const::Dec(-r),
            Const::NegZero => Const::Dec(BigRational::zero()),
        }
    }

    /// `~x` on an exact integer is `-x - 1`.
    pub fn bit_not(&self) -> Result<Const, FoldError> {
        match self {
            Const::Int(v) => Ok(Const::Int(-v - 1)),
            Const::Dec(_) | Const::NegZero => Err(FoldError::NeedsWhole),
        }
    }

    pub fn binary(op: BinOp, a: &Const, b: &Const) -> Result<Folded, FoldError> {
        use BinOp::*;
        if let (Eq | Ne | Lt | Le | Gt | Ge, _) = (op, ()) {
            let ord = a.rational().cmp(&b.rational());
            return Ok(Folded::Bool(match op {
                Eq => ord.is_eq(),
                Ne => ord.is_ne(),
                Lt => ord.is_lt(),
                Le => ord.is_le(),
                Gt => ord.is_gt(),
                _ => ord.is_ge(),
            }));
        }
        let value = match (a, b) {
            (Const::Int(x), Const::Int(y)) => Const::Int(match op {
                Add => x + y,
                Sub => x - y,
                Mul => x * y,
                // BigInt `/` and `%` truncate toward zero, as Drachm's do.
                Div | Rem if y.is_zero() => return Err(FoldError::DivisionByZero),
                Div => x / y,
                Rem => x % y,
                Pow => int_pow(x, y)?,
                Shl | Shr => {
                    if y.is_negative() {
                        return Err(FoldError::NegativeShift);
                    }
                    let n = y.to_u64().filter(|&n| n <= MAX_BITS).ok_or(FoldError::TooLarge)?;
                    // BigInt `>>` rounds toward negative infinity: an arithmetic shift.
                    if op == Shl { x << n } else { x >> n }
                }
                BitAnd => x & y,
                BitOr => x | y,
                BitXor => x ^ y,
                _ => unreachable!(),
            }),
            _ => {
                let (x, y) = (a.rational(), b.rational());
                Const::Dec(match op {
                    Add => x + y,
                    Sub => x - y,
                    Mul => x * y,
                    Div | Rem if y.is_zero() => return Err(FoldError::DivisionByZero),
                    Div => x / y,
                    // Sign of the left side, like `fmod`.
                    Rem => &x - (&x / &y).trunc() * &y,
                    Pow => dec_pow(&x, b)?,
                    _ => return Err(FoldError::NeedsWhole),
                })
            }
        };
        value.checked().map(Folded::Value)
    }

    /// The type an untyped constant gets when nothing else gives it one:
    /// `uint32`, `sint32` or `float64`, growing to 64 then 128 bits if it must.
    pub fn default_type(&self) -> Option<Type> {
        match self {
            Const::Dec(_) | Const::NegZero => Some(Type::F64),
            Const::Int(v) => [32, 64, 128]
                .into_iter()
                .map(|bits| Type::Int { bits, signed: v.is_negative() })
                .find(|t| self.fits(*t)),
        }
    }

    /// Is this value representable exactly in `ty`?
    pub fn fits(&self, ty: Type) -> bool {
        match (self, ty) {
            (Const::NegZero, Type::Int { .. } | Type::Float(_)) => true,
            (Const::Int(v), Type::Int { .. }) => {
                let (lo, hi) = ty.int_range().unwrap();
                *v >= lo && *v <= hi
            }
            (Const::Dec(r), Type::Int { .. }) => r.is_integer() && Const::Int(r.to_integer()).fits(ty),
            // Decimal constants are written as decimals: rounding to the
            // nearest float is expected. Only running off to infinity is a misfit.
            (Const::Dec(_), Type::Float(_)) => self.to_float(ty).is_finite(),
            (Const::Int(v), Type::Float(_)) => {
                let f = self.to_float(ty);
                f.is_finite() && BigRational::from_float(f).is_some_and(|r| r == BigRational::from_integer(v.clone()))
            }
            _ => false,
        }
    }

    /// The bits of this constant in integer type `ty`, dropping any fraction
    /// and wrapping around if it does not fit.
    pub fn wrap_to_int(&self, ty: Type) -> u128 {
        let Type::Int { bits, .. } = ty else { unreachable!() };
        let whole = match self {
            Const::Int(v) => v.clone(),
            Const::Dec(r) => r.trunc().to_integer(),
            Const::NegZero => BigInt::zero(),
        };
        let modulus = BigInt::one() << bits;
        let wrapped = ((whole % &modulus) + &modulus) % &modulus;
        wrapped.to_u128().unwrap()
    }

    /// The bits of this constant in integer type `ty`, dropping any fraction
    /// and clamping to the type's nearest end if it does not fit.
    pub fn saturate_to_int(&self, ty: Type) -> u128 {
        let whole = match self {
            Const::Int(v) => v.clone(),
            Const::Dec(r) => r.trunc().to_integer(),
            Const::NegZero => BigInt::zero(),
        };
        let (lo, hi) = ty.int_range().unwrap();
        let clamped = whole.max(lo).min(hi);
        let Type::Int { bits, .. } = ty.bare() else { unreachable!() };
        let modulus = BigInt::one() << bits;
        (((clamped % &modulus) + &modulus) % &modulus).to_u128().unwrap()
    }

    /// The nearest value in float type `ty`.
    pub fn to_float(&self, ty: Type) -> f64 {
        let f = if matches!(self, Const::NegZero) { -0.0 } else { self.rational().to_f64().unwrap_or(f64::NAN) };
        if ty == Type::Float(32) { f as f32 as f64 } else { f }
    }

    /// Read back the value of an integer-typed constant from its bits.
    pub fn from_int_bits(bits: u128, ty: Type) -> Const {
        let Type::Int { bits: width, signed } = ty else { unreachable!() };
        let v = BigInt::from(bits);
        if signed && bits >> (width - 1) & 1 == 1 {
            Const::Int(v - (BigInt::one() << width))
        } else {
            Const::Int(v)
        }
    }
}

fn int_pow(base: &BigInt, exp: &BigInt) -> Result<BigInt, FoldError> {
    if exp.is_negative() {
        // Whole-number division of 1 by the power.
        return Ok(if base.is_zero() {
            return Err(FoldError::DivisionByZero);
        } else if base.is_one() {
            BigInt::one()
        } else if *base == BigInt::from(-1) {
            if exp.is_even() { BigInt::one() } else { BigInt::from(-1) }
        } else {
            BigInt::zero()
        });
    }
    if base.is_zero() || base.is_one() {
        return Ok(if exp.is_zero() { BigInt::one() } else { base.clone() });
    }
    if *base == BigInt::from(-1) {
        return Ok(if exp.is_even() { BigInt::one() } else { BigInt::from(-1) });
    }
    let e = exp.to_u64().ok_or(FoldError::TooLarge)?;
    if base.bits().saturating_mul(e) > MAX_BITS {
        return Err(FoldError::TooLarge);
    }
    Ok(num_traits::pow(base.clone(), e as usize))
}

fn dec_pow(base: &BigRational, exp: &Const) -> Result<BigRational, FoldError> {
    // A whole exponent keeps the result exact.
    let whole = match exp {
        Const::Int(e) => Some(e.clone()),
        Const::Dec(r) if r.is_integer() => Some(r.to_integer()),
        Const::Dec(_) => None,
        Const::NegZero => Some(BigInt::zero()),
    };
    if let Some(e) = whole {
        let n = e.magnitude().to_u64().ok_or(FoldError::TooLarge)?;
        if base.numer().bits().max(base.denom().bits()).saturating_mul(n) > MAX_BITS {
            return Err(FoldError::TooLarge);
        }
        let p = num_traits::pow(base.clone(), n as usize);
        return if e.sign() == Sign::Minus {
            if p.is_zero() { Err(FoldError::DivisionByZero) } else { Ok(p.recip()) }
        } else {
            Ok(p)
        };
    }
    // A fractional exponent has no exact answer in general: fall back to floats.
    let f = base.to_f64().unwrap_or(f64::NAN).powf(exp.rational().to_f64().unwrap_or(f64::NAN));
    BigRational::from_float(f).ok_or(FoldError::NotANumber)
}
