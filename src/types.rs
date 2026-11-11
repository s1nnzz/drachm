//! Drachm's types: the built-in ones, plus structs by id.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::sync::{LazyLock, Mutex};

use num_bigint::BigInt;
use num_rational::BigRational;
use num_traits::One;

use crate::units::Unit;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    Int { bits: u16, signed: bool },
    Float(u16),
    Bool,
    /// The empty type: what a `none` function returns.
    None,
    /// Text: an owned UTF-8 string (`string`, also spelled `str`).
    Str,
    /// A struct: its index in `hir::Program::structs`, and its name for messages.
    Struct { id: usize, name: &'static str },
    /// `&T` / `&mut T`. `to` is never itself a `Ref`.
    Ref { mutable: bool, to: &'static Type },
    /// `*T`: a raw pointer, which may be written through and may be `none`.
    Ptr { to: &'static Type },
    /// `*fixed T`: a pointer that may be read but not written through
    /// (`address of` an immutable binding). A different kind from `Ptr`.
    FixedPtr { to: &'static Type },
    /// A number with a unit: `float64<m/s>`. `of` is always `Int` or `Float`.
    /// Units are checked by `typeck::units`; they are erased before the backends.
    Unit { of: &'static Type, unit: &'static Unit },
}

/// Leaks each distinct type once, so a `Type` can point at another and stay `Copy`.
fn intern_type(t: Type) -> &'static Type {
    static TYPES: LazyLock<Mutex<HashSet<&'static Type>>> = LazyLock::new(|| Mutex::new(HashSet::new()));
    let mut types = TYPES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(&found) = types.get(&t) {
        return found;
    }
    let leaked: &'static Type = Box::leak(Box::new(t));
    types.insert(leaked);
    leaked
}

/// The text of each unit that a `Type` names, by the unit's address. Units
/// are leaked, so an address is never reused. Display names are not part of
/// a unit's identity, so they live here rather than in `Type`.
static UNIT_NAMES: Mutex<BTreeMap<usize, String>> = Mutex::new(BTreeMap::new());

/// Record the name a unit prints as (`km`, `delta degC`). Set when a unit is
/// first used in a type.
pub fn name_unit(unit: &'static Unit, name: &str) {
    let mut names = UNIT_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    names.insert(unit as *const Unit as usize, name.to_string());
}

fn unit_name(unit: &'static Unit) -> String {
    let names = UNIT_NAMES.lock().unwrap_or_else(|e| e.into_inner());
    names.get(&(unit as *const Unit as usize)).cloned().unwrap_or_else(|| unit.to_string())
}

/// A struct's name as a `&'static str`, so that `Type` can stay `Copy` and
/// print itself. Each distinct name is stored once for the whole process.
pub fn intern(name: &str) -> &'static str {
    static NAMES: Mutex<BTreeSet<&'static str>> = Mutex::new(BTreeSet::new());
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(n) = names.get(name) {
        return n;
    }
    let n: &'static str = Box::leak(name.into());
    names.insert(n);
    n
}

/// Why a type name did not resolve.
pub enum TypeError {
    Unknown,
    /// A real Drachm type the compiler cannot handle yet.
    NotYet,
}

impl Type {
    pub const U32: Type = Type::Int { bits: 32, signed: false };
    pub const S32: Type = Type::Int { bits: 32, signed: true };
    pub const F64: Type = Type::Float(64);

    pub fn from_name(name: &str) -> Result<Type, TypeError> {
        let (kind, bits) = match name.find(|c: char| c.is_ascii_digit()) {
            Some(i) => (&name[..i], &name[i..]),
            None => (name, ""),
        };
        let int = |signed| match bits {
            "8" | "16" | "32" | "64" | "128" => Ok(Type::Int { bits: bits.parse().unwrap(), signed }),
            "256" => Err(TypeError::NotYet),
            _ => Err(TypeError::Unknown),
        };
        match kind {
            "uint" => int(false),
            "sint" => int(true),
            "float" => match bits {
                "32" => Ok(Type::Float(32)),
                "64" => Ok(Type::Float(64)),
                "8" | "16" => Err(TypeError::NotYet),
                _ => Err(TypeError::Unknown),
            },
            "bool" if bits.is_empty() => Ok(Type::Bool),
            "string" | "str" if bits.is_empty() => Ok(Type::Str),
            _ => Err(TypeError::Unknown),
        }
    }

    /// The type a literal suffix names: `u8`, `s`, `f32`, ... (`u`/`s` are
    /// 32-bit, `f` is `float64`).
    pub fn from_suffix(suffix: &str) -> Result<Type, TypeError> {
        let (kind, bits) = suffix.split_at(1);
        match (kind, bits) {
            ("u", "") => Ok(Type::U32),
            ("s", "") => Ok(Type::S32),
            ("f", "") => Ok(Type::F64),
            ("u", _) => Type::from_name(&format!("uint{bits}")),
            ("s", _) => Type::from_name(&format!("sint{bits}")),
            ("f", _) => Type::from_name(&format!("float{bits}")),
            _ => Err(TypeError::Unknown),
        }
    }

    /// `&to` or `&mut to`.
    pub fn borrow(mutable: bool, to: Type) -> Type {
        Type::Ref { mutable, to: intern_type(to) }
    }

    /// `*to`
    pub fn ptr(to: Type) -> Type {
        Type::Ptr { to: intern_type(to) }
    }

    /// `*fixed to`
    pub fn fixed_ptr(to: Type) -> Type {
        Type::FixedPtr { to: intern_type(to) }
    }

    /// This number type with `unit`. A dimensionless unit of scale 1 is the
    /// plain number, since it changes nothing.
    pub fn with_unit(self, unit: &'static Unit) -> Type {
        let of = self.bare();
        if unit.dim.is_empty() && unit.scale == BigRational::one() && unit.offset == BigRational::from_integer(0.into()) {
            return of;
        }
        Type::Unit { of: intern_type(of), unit }
    }

    /// The number type without its unit. Borrows and pointers are bared too.
    pub fn bare(self) -> Type {
        match self {
            Type::Unit { of, .. } => *of,
            Type::Ref { mutable, to } => Type::borrow(mutable, to.bare()),
            Type::Ptr { to } => Type::ptr(to.bare()),
            Type::FixedPtr { to } => Type::fixed_ptr(to.bare()),
            t => t,
        }
    }

    /// The unit of a number with one (not of a borrow of one).
    pub fn unit(self) -> Option<&'static Unit> {
        match self {
            Type::Unit { unit, .. } => Some(unit),
            _ => None,
        }
    }

    /// Does this type, or the type it points at, carry a unit?
    pub fn mentions_unit(self) -> bool {
        match self {
            Type::Unit { .. } => true,
            Type::Ref { to, .. } | Type::Ptr { to } | Type::FixedPtr { to } => to.mentions_unit(),
            _ => false,
        }
    }

    pub fn is_borrow(self) -> bool {
        matches!(self, Type::Ref { .. })
    }

    /// A raw pointer of either kind (`*T` or `*fixed T`).
    pub fn is_ptr(self) -> bool {
        matches!(self, Type::Ptr { .. } | Type::FixedPtr { .. })
    }

    pub fn is_fixed_ptr(self) -> bool {
        matches!(self, Type::FixedPtr { .. })
    }

    /// What a borrow or pointer points at.
    pub fn target(self) -> Option<Type> {
        match self {
            Type::Ref { to, .. } | Type::Ptr { to } | Type::FixedPtr { to } => Some(*to),
            _ => None,
        }
    }

    pub fn is_int(self) -> bool {
        matches!(self.bare(), Type::Int { .. })
    }

    pub fn is_float(self) -> bool {
        matches!(self.bare(), Type::Float(_))
    }

    pub fn is_numeric(self) -> bool {
        self.is_int() || self.is_float()
    }

    pub fn struct_id(self) -> Option<usize> {
        match self {
            Type::Struct { id, .. } => Some(id),
            _ => None,
        }
    }

    /// Smallest and largest value of an integer type.
    pub fn int_range(self) -> Option<(BigInt, BigInt)> {
        let Type::Int { bits, signed } = self.bare() else { return None };
        let one = BigInt::from(1);
        Some(if signed {
            (-(&one << (bits - 1)), (&one << (bits - 1)) - 1)
        } else {
            (BigInt::from(0), (&one << bits) - 1)
        })
    }

    /// Whole numbers up to 2^n are exact in this float type.
    pub fn float_exact_bits(self) -> u16 {
        match self.bare() {
            Type::Float(32) => 24,
            _ => 53,
        }
    }

    /// Can every value of `self` be represented exactly in `to`?
    pub fn converts_losslessly_to(self, to: Type) -> bool {
        match (self.bare(), to.bare()) {
            (a, b) if a == b => true,
            (Type::Int { bits: fb, signed: fs }, Type::Int { bits: tb, signed: ts }) => {
                if fs == ts { fb <= tb } else { !fs && fb < tb }
            }
            (Type::Int { bits, signed }, Type::Float(_)) => bits - signed as u16 <= to.float_exact_bits(),
            (Type::Float(a), Type::Float(b)) => a <= b,
            _ => false,
        }
    }

    /// "0 to 255", for messages.
    pub fn describe_range(self) -> String {
        match self.bare() {
            Type::Int { .. } => {
                let (lo, hi) = self.int_range().unwrap();
                format!("{lo} to {hi}")
            }
            Type::Float(_) => format!("whole numbers exactly only up to {}", BigInt::from(1) << self.float_exact_bits()),
            _ => String::new(),
        }
    }
}

impl fmt::Display for Type {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Type::Int { bits, signed: false } => write!(f, "uint{bits}"),
            Type::Int { bits, signed: true } => write!(f, "sint{bits}"),
            Type::Float(bits) => write!(f, "float{bits}"),
            Type::Bool => f.write_str("bool"),
            Type::None => f.write_str("none"),
            Type::Str => f.write_str("string"),
            Type::Struct { name, .. } => f.write_str(name),
            Type::Ref { mutable: false, to } => write!(f, "&{to}"),
            Type::Ref { mutable: true, to } => write!(f, "&mut {to}"),
            Type::Ptr { to } => write!(f, "*{to}"),
            Type::FixedPtr { to } => write!(f, "*fixed {to}"),
            Type::Unit { of, unit } => write!(f, "{of}<{}>", unit_name(unit)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrow_types_intern_and_print() {
        let a = Type::borrow(true, Type::Str);
        assert_eq!(a, Type::borrow(true, Type::Str));
        assert_ne!(a, Type::borrow(false, Type::Str));
        assert_eq!(a.to_string(), "&mut string");
        assert_eq!(Type::borrow(false, Type::U32).to_string(), "&uint32");
        assert_eq!(Type::ptr(Type::U32).to_string(), "*uint32");
        assert_eq!(Type::fixed_ptr(Type::U32).to_string(), "*fixed uint32");
        assert!(Type::fixed_ptr(Type::U32).is_ptr() && Type::fixed_ptr(Type::U32).is_fixed_ptr());
        assert!(!Type::ptr(Type::U32).is_fixed_ptr());
        assert!(a.is_borrow() && !a.is_ptr());
        assert_eq!(Type::ptr(Type::Bool).target(), Some(Type::Bool));
    }

    #[test]
    fn unit_types_print_their_name_and_erase_to_the_number() {
        use crate::units::UnitTable;
        let mut table = UnitTable::new();
        let m = table.base("m").unwrap();
        let s = table.base("s").unwrap();
        let speed = table.div(m, s).unwrap();
        let ty = Type::F64.with_unit(speed);
        assert_eq!(ty.to_string(), "float64<m/s>");
        name_unit(speed, "speed");
        assert_eq!(ty.to_string(), "float64<speed>");
        assert_eq!(ty.bare(), Type::F64);
        assert_eq!(Type::borrow(true, ty).bare(), Type::borrow(true, Type::F64));
        assert!(ty.mentions_unit() && !ty.bare().mentions_unit());
        assert!(ty.is_numeric() && ty.is_float());
        assert_eq!(ty.unit(), Some(speed));
        // A dimensionless scale-1 unit is the plain number.
        assert_eq!(Type::U32.with_unit(table.dimensionless()), Type::U32);
    }
}
