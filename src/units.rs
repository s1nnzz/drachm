//! Unit data for units of measure (step 4.5, `docs/impl/step4.5-units.md`
//! 2.2): canonical units, interning, display names and exact scales. Pure
//! data: no AST and no type checking. The rules that use it live in
//! `typeck/units.rs` (work package U3).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;

use num_bigint::BigInt;
use num_integer::Integer;
use num_rational::BigRational;
use num_traits::{One, Zero};

/// A unit in canonical form: a base-unit exponent vector, an exact scale and
/// an exact offset. Two units are the same type exactly when all three are
/// equal, so `kg*m/s^2` and a unit declared as `N = kg*m/s^2` are one type.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Unit {
    /// Sorted by base name, no zero exponents. `m/s^2` is `[("m", 1), ("s", -2)]`.
    pub dim: Vec<(&'static str, i32)>,
    /// How many of the base units in `dim` one of this unit is.
    pub scale: BigRational,
    /// Zero for every ordinary unit and every delta. Nonzero only for a point
    /// unit such as `degC`: a reading `x` is the base reading `(x - offset) * scale`.
    pub offset: BigRational,
}

impl fmt::Display for Unit {
    /// The canonical expression without a name, e.g. `kg*m/s^2`, `1/s`, `m/(kg*s)`.
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let term = |(n, e): &(&str, i32)| {
            if e.abs() == 1 { n.to_string() } else { format!("{n}^{}", e.abs()) }
        };
        let num: Vec<String> = self.dim.iter().filter(|(_, e)| *e > 0).map(term).collect();
        let den: Vec<String> = self.dim.iter().filter(|(_, e)| *e < 0).map(term).collect();
        let num = if num.is_empty() { "1".to_string() } else { num.join("*") };
        match den.len() {
            0 => write!(f, "{num}"),
            1 => write!(f, "{num}/{}", den[0]),
            _ => write!(f, "{num}/({})", den.join("*")),
        }
    }
}

/// Why a unit operation failed. `typeck/units.rs` maps these to diagnostics.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitError {
    /// E0405: an exponent outside the `i16` range.
    ExponentRange,
    /// E0403: a factor or scale that is not positive.
    NotPositive,
    /// E0403: a unit name declared twice.
    Redeclared(String),
    /// E0402: a unit name that is not declared.
    Unknown(String),
}

/// A unit expression as written, without any resolution: the neutral tree
/// that `parse_unit_text` returns and `UnitTable::eval` resolves.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UnitExprText {
    /// The optional leading `delta` of a unit text. Only at the top.
    Delta(Box<UnitExprText>),
    Name(String),
    /// The dimensionless `1`.
    One,
    Mul(Box<UnitExprText>, Box<UnitExprText>),
    Div(Box<UnitExprText>, Box<UnitExprText>),
    Pow(Box<UnitExprText>, i32),
}

/// Interned, leaked units plus the side tables that are not part of identity:
/// display names and the name -> unit map in declaration order. Every
/// `&'static Unit` lives as long as the program, so units are `Copy`.
#[derive(Default)]
pub struct UnitTable {
    /// Content plus a flag for "is a delta": a delta is a separate pointer
    /// from the ordinary unit with the same content, so it can have its own
    /// display name. Both still compare equal by content.
    interned: HashMap<(Unit, bool), &'static Unit>,
    /// Address of an interned unit -> the name used in messages.
    display: HashMap<usize, String>,
    /// Addresses whose display name came from a `unit NAME` declaration.
    declared: HashSet<usize>,
    names: HashMap<&'static str, &'static Unit>,
    order: Vec<&'static str>,
    name_pool: HashSet<&'static str>,
}

fn paren_if(cond: bool, s: String) -> String {
    if cond { format!("({s})") } else { s }
}

fn key_of(u: &'static Unit) -> usize {
    u as *const Unit as usize
}

impl UnitTable {
    pub fn new() -> Self {
        Self::default()
    }

    /// A `&'static` copy of a name. Names are few, so each is leaked once.
    fn name_str(&mut self, s: &str) -> &'static str {
        if let Some(&n) = self.name_pool.get(s) {
            return n;
        }
        let n: &'static str = Box::leak(s.to_string().into_boxed_str());
        self.name_pool.insert(n);
        n
    }

    /// The interned unit with this content. `text` is the display name to use
    /// if this unit has none yet.
    fn intern(&mut self, unit: Unit, delta: bool, text: String) -> &'static Unit {
        if let Some(&u) = self.interned.get(&(unit.clone(), delta)) {
            self.display.entry(key_of(u)).or_insert(text);
            return u;
        }
        let u: &'static Unit = Box::leak(Box::new(unit.clone()));
        self.interned.insert((unit, delta), u);
        self.display.insert(key_of(u), text);
        u
    }

    /// Declare `name` as `unit`. The first declaration of a unit gives its
    /// display name; an alias (`unit x = 1 kg;`) keeps the first name.
    pub fn define(&mut self, name: &str, unit: &'static Unit) -> Result<(), UnitError> {
        if self.names.contains_key(name) {
            return Err(UnitError::Redeclared(name.to_string()));
        }
        let n = self.name_str(name);
        self.names.insert(n, unit);
        self.order.push(n);
        let key = key_of(unit);
        if self.declared.insert(key) {
            self.display.insert(key, name.to_string());
        }
        Ok(())
    }

    /// Declare the base unit `unit NAME;`.
    pub fn base(&mut self, name: &str) -> Result<&'static Unit, UnitError> {
        let n = self.name_str(name);
        let unit = Unit {
            dim: vec![(n, 1)],
            scale: BigRational::one(),
            offset: BigRational::zero(),
        };
        let u = self.intern(unit, false, name.to_string());
        self.define(name, u)?;
        Ok(u)
    }

    pub fn lookup(&self, name: &str) -> Option<&'static Unit> {
        self.names.get(name).copied()
    }

    /// Declared names in declaration order.
    pub fn declared_names(&self) -> &[&'static str] {
        &self.order
    }

    /// The display name: the declared name, else the expression it was built
    /// from (`km*m`, `m/s`), else the canonical form.
    pub fn display(&self, u: &'static Unit) -> String {
        self.display.get(&key_of(u)).cloned().unwrap_or_else(|| u.to_string())
    }

    /// The dimensionless unit with scale 1 (`1`).
    pub fn dimensionless(&mut self) -> &'static Unit {
        let unit = Unit { dim: Vec::new(), scale: BigRational::one(), offset: BigRational::zero() };
        self.intern(unit, false, "1".to_string())
    }

    /// `a * b`: dimensions add, scales multiply.
    pub fn mul(&mut self, a: &'static Unit, b: &'static Unit) -> Result<&'static Unit, UnitError> {
        let dim = merge(a, b, 1)?;
        let (ta, tb) = (self.display(a), self.display(b));
        let text = format!(
            "{}*{}",
            paren_if(ta.contains('/'), ta),
            paren_if(tb.contains(['*', '/']), tb)
        );
        let unit = Unit { dim, scale: &a.scale * &b.scale, offset: BigRational::zero() };
        Ok(self.intern(unit, false, text))
    }

    /// `a / b`: dimensions subtract, scales divide.
    pub fn div(&mut self, a: &'static Unit, b: &'static Unit) -> Result<&'static Unit, UnitError> {
        let dim = merge(a, b, -1)?;
        let (ta, tb) = (self.display(a), self.display(b));
        let text = format!("{}/{}", ta, paren_if(tb.contains(['*', '/']), tb));
        let unit = Unit { dim, scale: &a.scale / &b.scale, offset: BigRational::zero() };
        Ok(self.intern(unit, false, text))
    }

    /// `a ** n`: exponents multiply, the scale is raised to `n`.
    pub fn pow(&mut self, a: &'static Unit, n: i32) -> Result<&'static Unit, UnitError> {
        if n.unsigned_abs() > i16::MAX as u32 {
            return Err(UnitError::ExponentRange);
        }
        let mut dim = Vec::new();
        for &(name, e) in &a.dim {
            let e = i64::from(e) * i64::from(n);
            if e.abs() > i16::MAX as i64 {
                return Err(UnitError::ExponentRange);
            }
            if e != 0 {
                dim.push((name, e as i32));
            }
        }
        let mut scale = num_traits::pow(a.scale.clone(), n.unsigned_abs() as usize);
        if n < 0 {
            scale = scale.recip();
        }
        let ta = self.display(a);
        let text = format!("{}^{n}", paren_if(ta.contains(['*', '/']), ta));
        Ok(self.intern(Unit { dim, scale, offset: BigRational::zero() }, false, text))
    }

    /// `delta` of a point unit: the same dimension and scale with offset 0.
    /// Equal to the ordinary unit of that dimension and scale by content.
    pub fn delta_of(&mut self, point: &'static Unit) -> &'static Unit {
        let text = format!("delta {}", self.display(point));
        let unit = Unit {
            dim: point.dim.clone(),
            scale: point.scale.clone(),
            offset: BigRational::zero(),
        };
        self.intern(unit, true, text)
    }

    /// `factor` times `u` (`unit kg = 1000 g;` is `scaled(g, 1000)`).
    pub fn scaled(&mut self, u: &'static Unit, factor: &BigRational) -> Result<&'static Unit, UnitError> {
        if !(factor > &BigRational::zero()) {
            return Err(UnitError::NotPositive);
        }
        let text = format!("{}*{}", factor, self.display(u));
        let unit = Unit { dim: u.dim.clone(), scale: &u.scale * factor, offset: u.offset.clone() };
        Ok(self.intern(unit, false, text))
    }

    /// `u` with the offset `c` (`unit degC = K offset -273.15;`): a point unit.
    pub fn with_offset(&mut self, u: &'static Unit, c: BigRational) -> &'static Unit {
        let text = self.display(u);
        let unit = Unit { dim: u.dim.clone(), scale: u.scale.clone(), offset: c };
        self.intern(unit, false, text)
    }

    /// The unit `x` with `x^k` equal to `a`, for a nonzero `k`, when there is
    /// one: every exponent of `a` divides by `k` and its scale has an exact
    /// `k`-th root. `None` otherwise (and for a point unit).
    pub fn root(&mut self, a: &'static Unit, k: i32) -> Option<&'static Unit> {
        if k == 0 || !a.offset.is_zero() || a.dim.iter().any(|(_, e)| e % k != 0) {
            return None;
        }
        let n = k.unsigned_abs();
        let base = if k < 0 { a.scale.recip() } else { a.scale.clone() };
        let scale = exact_root(&base, n)?;
        let dim: Vec<(&'static str, i32)> = a.dim.iter().map(|&(name, e)| (name, e / k)).collect();
        let text = Unit { dim: dim.clone(), scale: scale.clone(), offset: BigRational::zero() }.to_string();
        Some(self.intern(Unit { dim, scale, offset: BigRational::zero() }, false, text))
    }

    /// Resolve a unit expression as written. Names must be declared.
    pub fn eval(&mut self, e: &UnitExprText) -> Result<&'static Unit, UnitError> {
        match e {
            UnitExprText::Delta(inner) => {
                let u = self.eval(inner)?;
                Ok(self.delta_of(u))
            }
            UnitExprText::Name(n) => self.lookup(n).ok_or_else(|| UnitError::Unknown(n.clone())),
            UnitExprText::One => Ok(self.dimensionless()),
            UnitExprText::Mul(l, r) => {
                let (l, r) = (self.eval(l)?, self.eval(r)?);
                self.mul(l, r)
            }
            UnitExprText::Div(l, r) => {
                let (l, r) = (self.eval(l)?, self.eval(r)?);
                self.div(l, r)
            }
            UnitExprText::Pow(b, n) => {
                let b = self.eval(b)?;
                self.pow(b, *n)
            }
        }
    }
}

/// `a` times `b` (sign 1) or `a` over `b` (sign -1), as a sorted exponent vector.
fn merge(a: &Unit, b: &Unit, sign: i64) -> Result<Vec<(&'static str, i32)>, UnitError> {
    let mut sum: BTreeMap<&'static str, i64> = BTreeMap::new();
    for &(n, e) in &a.dim {
        *sum.entry(n).or_insert(0) += i64::from(e);
    }
    for &(n, e) in &b.dim {
        *sum.entry(n).or_insert(0) += sign * i64::from(e);
    }
    let mut dim = Vec::new();
    for (n, e) in sum {
        if e.abs() > i16::MAX as i64 {
            return Err(UnitError::ExponentRange);
        }
        if e != 0 {
            dim.push((n, e as i32));
        }
    }
    Ok(dim)
}

/// The exact `n`-th root of a positive rational, if it has one.
fn exact_root(r: &BigRational, n: u32) -> Option<BigRational> {
    let root = |x: &BigInt| {
        let q = x.nth_root(n);
        (num_traits::pow(q.clone(), n as usize) == *x).then_some(q)
    };
    Some(BigRational::new(root(r.numer())?, root(r.denom())?))
}

/// How many of `to` make one `from`, when both have the same dimension.
pub fn ratio(from: &Unit, to: &Unit) -> Option<BigRational> {
    (from.dim == to.dim).then(|| &from.scale / &to.scale)
}

/// True when no base unit remains (`m/m`, `km/m`).
pub fn is_dimensionless(u: &Unit) -> bool {
    u.dim.is_empty()
}

/// The greatest common divisor of two positive rationals:
/// `gcd(p1/q1, p2/q2) = gcd(p1, p2) / lcm(q1, q2)`.
pub fn gcd_rational(a: &BigRational, b: &BigRational) -> BigRational {
    if a.is_zero() {
        return b.clone();
    }
    if b.is_zero() {
        return a.clone();
    }
    let num = a.numer().gcd(b.numer());
    let den = a.denom().lcm(b.denom());
    BigRational::new(num, den)
}

/// The whole-number factors that bring two positive scales to a common base
/// (2.5): with `g = gcd(sa, sb)`, returns `(sa / g, sb / g)`.
pub fn scale_pair(sa: &BigRational, sb: &BigRational) -> (BigInt, BigInt) {
    let g = gcd_rational(sa, sb);
    if g.is_zero() {
        return (BigInt::one(), BigInt::one());
    }
    ((sa / &g).to_integer(), (sb / &g).to_integer())
}

/// Parse the text of a unit literal or type, as `lexer::number` stores it
/// (2.3 grammar). Names are ASCII. Spaces are allowed only as the single space
/// after a leading `delta`. The error is the byte offset of the first problem.
pub fn parse_unit_text(s: &str) -> Result<UnitExprText, usize> {
    let mut p = Parser { s: s.as_bytes(), i: 0 };
    let e = if s.starts_with("delta ") {
        p.i = "delta ".len();
        UnitExprText::Delta(Box::new(p.expr()?))
    } else {
        p.expr()?
    };
    if p.i != s.len() {
        return Err(p.i);
    }
    Ok(e)
}

pub fn is_valid_unit_text(s: &str) -> bool {
    parse_unit_text(s).is_ok()
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    /// Consume a run of ASCII digits; return how many.
    fn digits(&mut self) -> usize {
        let start = self.i;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.i += 1;
        }
        self.i - start
    }

    fn expr(&mut self) -> Result<UnitExprText, usize> {
        let mut e = self.term()?;
        while let Some(op @ (b'*' | b'/')) = self.peek() {
            self.i += 1;
            let r = Box::new(self.term()?);
            let l = Box::new(e);
            e = if op == b'*' { UnitExprText::Mul(l, r) } else { UnitExprText::Div(l, r) };
        }
        Ok(e)
    }

    fn term(&mut self) -> Result<UnitExprText, usize> {
        let start = self.i;
        match self.peek() {
            Some(b'(') => {
                self.i += 1;
                let e = self.expr()?;
                if self.peek() != Some(b')') {
                    return Err(self.i);
                }
                self.i += 1;
                Ok(e)
            }
            Some(c) if c.is_ascii_digit() => {
                self.digits();
                if &self.s[start..self.i] == b"1" { Ok(UnitExprText::One) } else { Err(start) }
            }
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => {
                self.i += 1;
                while self.peek().is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_') {
                    self.i += 1;
                }
                let name = String::from_utf8_lossy(&self.s[start..self.i]).into_owned();
                if self.peek() != Some(b'^') {
                    return Ok(UnitExprText::Name(name));
                }
                self.i += 1;
                let neg = self.peek() == Some(b'-');
                if neg {
                    self.i += 1;
                }
                let ds = self.i;
                if self.digits() == 0 {
                    return Err(self.i);
                }
                let text = String::from_utf8_lossy(&self.s[ds..self.i]).into_owned();
                let n: i32 = text.parse().map_err(|_| ds)?;
                let n = if neg { -n } else { n };
                Ok(UnitExprText::Pow(Box::new(UnitExprText::Name(name)), n))
            }
            _ => Err(self.i),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(n: i64, d: i64) -> BigRational {
        BigRational::new(BigInt::from(n), BigInt::from(d))
    }

    fn text(s: &str) -> UnitExprText {
        parse_unit_text(s).unwrap_or_else(|at| panic!("{s:?} rejected at {at}"))
    }

    /// A table with the SI-like base units used by most tests.
    fn si() -> UnitTable {
        let mut t = UnitTable::new();
        for b in ["kg", "m", "s", "K"] {
            t.base(b).unwrap();
        }
        t
    }

    #[test]
    fn written_and_declared_derived_units_are_one_type() {
        let mut t = si();
        let n = t.eval(&text("kg*m/s^2")).unwrap();
        t.define("N", n).unwrap();
        let again = t.eval(&text("kg*m/s^2")).unwrap();
        assert!(std::ptr::eq(n, again));
        assert_eq!(t.lookup("N").map(|u| key_of(u)), Some(key_of(n)));
    }

    #[test]
    fn km_over_m_ratio_is_1000() {
        let mut t = si();
        let m = t.lookup("m").unwrap();
        let km = t.scaled(m, &BigRational::from_integer(1000.into())).unwrap();
        t.define("km", km).unwrap();
        assert_eq!(ratio(km, m), Some(BigRational::from_integer(1000.into())));
        assert_eq!(ratio(m, km), Some(r(1, 1000)));
        let s = t.lookup("s").unwrap();
        assert_eq!(ratio(km, s), None);
    }

    #[test]
    fn g_is_a_thousandth_of_kg() {
        let mut t = si();
        let kg = t.lookup("kg").unwrap();
        let g = t.scaled(kg, &r(1, 1000)).unwrap();
        assert_eq!(g.scale, r(1, 1000));
        assert_eq!(g.dim, vec![("kg", 1)]);
        assert_eq!(t.scaled(kg, &r(0, 1)), Err(UnitError::NotPositive));
    }

    #[test]
    fn m_over_m_is_dimensionless_and_km_over_m_folds_the_scale() {
        let mut t = si();
        let m = t.lookup("m").unwrap();
        let q = t.div(m, m).unwrap();
        assert!(is_dimensionless(q));
        assert_eq!(q.scale, BigRational::one());
        let km = t.scaled(m, &BigRational::from_integer(1000.into())).unwrap();
        let q = t.div(km, m).unwrap();
        assert!(is_dimensionless(q));
        assert_eq!(q.scale, BigRational::from_integer(1000.into()));
    }

    #[test]
    fn pow_with_negative_exponents() {
        let mut t = si();
        let s2 = t.eval(&text("s^-2")).unwrap();
        assert_eq!(s2.dim, vec![("s", -2)]);
        assert_eq!(s2.scale, BigRational::one());
        let m = t.lookup("m").unwrap();
        let km = t.scaled(m, &BigRational::from_integer(1000.into())).unwrap();
        t.define("km", km).unwrap();
        let inv = t.eval(&text("km^-1")).unwrap();
        assert_eq!(inv.dim, vec![("m", -1)]);
        assert_eq!(inv.scale, r(1, 1000));
        let sq = t.eval(&text("km^2")).unwrap();
        assert_eq!(sq.scale, BigRational::from_integer(1_000_000.into()));
    }

    #[test]
    fn reciprocal_of_a_second_is_hertz_shaped() {
        let mut t = si();
        let hz = t.eval(&text("1/s")).unwrap();
        assert_eq!(hz.dim, vec![("s", -1)]);
        assert_eq!(hz.to_string(), "1/s");
    }

    #[test]
    fn exponents_past_i16_are_rejected() {
        let mut t = si();
        let m = t.lookup("m").unwrap();
        assert_eq!(t.pow(m, 40000), Err(UnitError::ExponentRange));
        assert_eq!(t.eval(&text("m^40000")), Err(UnitError::ExponentRange));
        assert_eq!(t.eval(&text("m^20000*m^20000")), Err(UnitError::ExponentRange));
        assert!(t.pow(m, 32767).is_ok());
        assert!(t.pow(m, i32::MIN).is_err());
    }

    #[test]
    fn interning_gives_one_pointer_per_content() {
        let mut t = si();
        let a = t.eval(&text("m*s")).unwrap();
        let b = t.eval(&text("s*m")).unwrap();
        assert!(std::ptr::eq(a, b));
        let c = t.eval(&text("m*s^1")).unwrap();
        assert!(std::ptr::eq(a, c));
    }

    #[test]
    fn delta_of_a_point_equals_the_ordinary_unit_and_displays_as_delta() {
        let mut t = si();
        let k = t.lookup("K").unwrap();
        let shifted = t.with_offset(k, r(-27315, 100));
        let degc = t.scaled(shifted, &BigRational::one()).unwrap();
        t.define("degC", degc).unwrap();
        assert_eq!(degc.offset, r(-27315, 100));
        let delta = t.delta_of(degc);
        assert_eq!(*delta, *k);
        assert_eq!(delta.offset, BigRational::zero());
        assert_eq!(t.display(delta), "delta degC");
        assert_eq!(t.display(degc), "degC");
        assert_eq!(t.display(k), "K");
    }

    #[test]
    fn display_of_declared_written_and_product_units() {
        let mut t = si();
        let m = t.lookup("m").unwrap();
        let km = t.scaled(m, &BigRational::from_integer(1000.into())).unwrap();
        t.define("km", km).unwrap();
        let prod = t.eval(&text("km*m")).unwrap();
        assert_eq!(t.display(prod), "km*m");
        let n = t.eval(&text("kg*m/s^2")).unwrap();
        assert_eq!(t.display(n), "kg*m/s^2");
        t.define("N", n).unwrap();
        assert_eq!(t.display(n), "N");
        let q = t.eval(&text("m/(s*kg)")).unwrap();
        assert_eq!(t.display(q), "m/(s*kg)");
        assert_eq!(q.to_string(), "m/(kg*s)");
        assert_eq!(t.display(km), "km");
    }

    #[test]
    fn first_declaration_keeps_the_display_name() {
        let mut t = si();
        let kg = t.lookup("kg").unwrap();
        let alias = t.scaled(kg, &BigRational::one()).unwrap();
        t.define("mass", alias).unwrap();
        assert_eq!(t.display(kg), "kg");
        assert_eq!(t.lookup("mass").map(|u| u.dim.clone()), Some(vec![("kg", 1)]));
    }

    #[test]
    fn redeclaring_a_name_is_an_error() {
        let mut t = si();
        let m = t.lookup("m").unwrap();
        assert_eq!(t.define("m", m), Err(UnitError::Redeclared("m".to_string())));
        assert_eq!(t.base("m"), Err(UnitError::Redeclared("m".to_string())));
        assert_eq!(t.eval(&text("nope")), Err(UnitError::Unknown("nope".to_string())));
    }

    #[test]
    fn canonical_display_orders_and_parenthesises() {
        let mut t = si();
        let u = t.eval(&text("kg*m/s^2")).unwrap();
        assert_eq!(u.to_string(), "kg*m/s^2");
        let u = t.eval(&text("m/s")).unwrap();
        assert_eq!(u.to_string(), "m/s");
    }

    #[test]
    fn scale_pair_brings_scales_to_whole_numbers() {
        assert_eq!(scale_pair(&r(1000, 1), &r(1, 1)), (BigInt::from(1000), BigInt::from(1)));
        assert_eq!(scale_pair(&r(1, 1), &r(1, 1)), (BigInt::from(1), BigInt::from(1)));
        assert_eq!(scale_pair(&r(5280, 1), &r(1, 1)), (BigInt::from(5280), BigInt::from(1)));
        assert_eq!(scale_pair(&r(1, 1000), &r(1, 1)), (BigInt::from(1), BigInt::from(1000)));
        assert_eq!(scale_pair(&r(3, 10), &r(2, 10)), (BigInt::from(3), BigInt::from(2)));
    }

    #[test]
    fn gcd_of_rationals() {
        assert_eq!(gcd_rational(&r(3, 10), &r(2, 10)), r(1, 10));
        assert_eq!(gcd_rational(&r(0, 1), &r(2, 3)), r(2, 3));
        assert_eq!(gcd_rational(&r(4, 1), &r(6, 1)), r(2, 1));
    }

    #[test]
    fn parse_accepts_the_grammar_forms() {
        for s in [
            "m", "kg*m/s^2", "1/s", "km", "m/s", "s^-2", "(kg*m)/s^2", "m/(s*kg)",
            "1", "delta degC", "delta K", "delta", "delta*m", "deltam", "m^0", "_x9",
        ] {
            assert!(is_valid_unit_text(s), "{s:?} should parse");
        }
        assert_eq!(text("delta degC"), UnitExprText::Delta(Box::new(UnitExprText::Name("degC".into()))));
        assert_eq!(text("delta"), UnitExprText::Name("delta".into()));
        assert_eq!(
            text("s^-2"),
            UnitExprText::Pow(Box::new(UnitExprText::Name("s".into())), -2)
        );
    }

    #[test]
    fn parse_rejects_with_the_offset_of_the_problem() {
        assert_eq!(parse_unit_text("3 m"), Err(0));
        assert_eq!(parse_unit_text("10"), Err(0));
        assert_eq!(parse_unit_text("m*"), Err(2));
        assert_eq!(parse_unit_text("m^"), Err(2));
        assert_eq!(parse_unit_text("m^-"), Err(3));
        assert_eq!(parse_unit_text("m  s"), Err(1));
        assert_eq!(parse_unit_text("m s"), Err(1));
        assert_eq!(parse_unit_text("delta  m"), Err(6));
        assert_eq!(parse_unit_text("delta "), Err(6));
        assert_eq!(parse_unit_text("µ"), Err(0));
        assert_eq!(parse_unit_text("mµ"), Err(1));
        assert_eq!(parse_unit_text("m*/s"), Err(2));
        assert_eq!(parse_unit_text("(m"), Err(2));
        assert_eq!(parse_unit_text("()"), Err(1));
        assert_eq!(parse_unit_text(""), Err(0));
        assert_eq!(parse_unit_text("(m)^2"), Err(3));
        assert_eq!(parse_unit_text("m^99999999999"), Err(2));
        assert!(!is_valid_unit_text("1m"));
    }
}
