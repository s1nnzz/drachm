//! Drachm's built-in types.

use std::fmt;

use num_bigint::BigInt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    Int { bits: u16, signed: bool },
    Float(u16),
    Bool,
    /// The empty type: what a `none` function returns.
    None,
    /// Text. Until step 2 it only exists as the argument of `io::out`/`io::outln`.
    Str,
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
            "string" | "str" if bits.is_empty() => Err(TypeError::NotYet),
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

    pub fn is_int(self) -> bool {
        matches!(self, Type::Int { .. })
    }

    pub fn is_float(self) -> bool {
        matches!(self, Type::Float(_))
    }

    pub fn is_numeric(self) -> bool {
        self.is_int() || self.is_float()
    }

    /// Smallest and largest value of an integer type.
    pub fn int_range(self) -> Option<(BigInt, BigInt)> {
        let Type::Int { bits, signed } = self else { return None };
        let one = BigInt::from(1);
        Some(if signed {
            (-(&one << (bits - 1)), (&one << (bits - 1)) - 1)
        } else {
            (BigInt::from(0), (&one << bits) - 1)
        })
    }

    /// Whole numbers up to 2^n are exact in this float type.
    pub fn float_exact_bits(self) -> u16 {
        match self {
            Type::Float(32) => 24,
            _ => 53,
        }
    }

    /// Can every value of `self` be represented exactly in `to`?
    pub fn converts_losslessly_to(self, to: Type) -> bool {
        match (self, to) {
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
        match self {
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
        }
    }
}
