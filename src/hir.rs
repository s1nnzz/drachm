//! The checked program: every expression has a concrete type, every
//! conversion is explicit, and names are resolved to ids. Both backends
//! (Cranelift and C) are generated from this.

use crate::types::Type;

pub type FuncId = usize;
pub type LocalId = usize;
pub type StructId = usize;

pub struct Program {
    pub funcs: Vec<Func>,
    pub structs: Vec<StructDef>,
    pub main: FuncId,
}

/// A struct's layout, C-style: fields in declaration order, each at the next
/// multiple of its alignment; the size is rounded up to the largest alignment.
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub size: u32,
    pub align: u32,
}

pub struct FieldDef {
    pub name: String,
    pub ty: Type,
    pub offset: u32,
}

impl Program {
    /// Size and alignment of a value of type `t` in memory, in bytes.
    pub fn size_align(&self, t: Type) -> (u32, u32) {
        scalar_size(t).map_or_else(
            || {
                let s = &self.structs[t.struct_id().expect("a value type")];
                (s.size, s.align)
            },
            |n| (n, n),
        )
    }

    pub fn struct_def(&self, t: Type) -> &StructDef {
        &self.structs[t.struct_id().expect("a struct type")]
    }
}

/// Size (which is also the alignment) of a number or bool; `None` for the rest.
pub fn scalar_size(t: Type) -> Option<u32> {
    match t {
        Type::Int { bits, .. } | Type::Float(bits) => Some(bits as u32 / 8),
        Type::Bool => Some(1),
        Type::None | Type::Str | Type::Struct { .. } => None,
    }
}

/// Lay out fields of the given sizes and alignments: their offsets, and the
/// struct's size and alignment.
pub fn layout(fields: &[(u32, u32)]) -> (Vec<u32>, u32, u32) {
    let mut offsets = Vec::new();
    let mut end = 0u32;
    let mut align = 1;
    for &(size, a) in fields {
        let at = end.next_multiple_of(a);
        offsets.push(at);
        end = at + size;
        align = align.max(a);
    }
    (offsets, end.next_multiple_of(align), align)
}

pub struct Func {
    /// The symbol name in the object file or C source.
    pub symbol: String,
    /// The first `params` locals are the parameters, in order.
    pub params: usize,
    /// A method's receiver: parameter 0 is `self` and refers to the caller's
    /// value (changes through it are seen by the caller). Every other struct
    /// parameter is a copy.
    pub has_self: bool,
    pub locals: Vec<Local>,
    pub ret: Type,
    pub body: Block,
}

pub struct Local {
    pub name: String,
    pub ty: Type,
}

/// Something that can be assigned to: a local, or a field inside one
/// (`fields` are field indexes, outermost first).
pub struct Place {
    pub local: LocalId,
    pub fields: Vec<usize>,
}

pub type Block = Vec<Stmt>;

pub enum Stmt {
    Assign(Place, Expr),
    If(Expr, Block, Block),
    While(Expr, Block),
    Return(Option<Expr>),
    Expr(Expr),
}

pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
}

impl Expr {
    /// A local or a field of one: the backends can refer to its storage
    /// directly (for `self` arguments) instead of copying the value.
    pub fn is_place(&self) -> bool {
        match &self.kind {
            ExprKind::Local(_) => true,
            ExprKind::Field(inner, _) => inner.is_place(),
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    /// Float power only; integer power is `ExprKind::IntPow`.
    Pow,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl BinOp {
    pub fn is_comparison(self) -> bool {
        matches!(self, BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge)
    }
}

pub enum ExprKind {
    /// The value's bits, zero-extended; only the low `bits` of the type are set.
    Int(u128),
    Float(f64),
    Bool(bool),
    /// A string literal. Only ever the argument of `Out`.
    Str(String),
    /// A local's current value (for a struct, a copy of it).
    Local(LocalId),
    /// Field `n` of a struct value.
    Field(Box<Expr>, usize),
    /// A struct value built from field values, given as (field index, value)
    /// in evaluation order. Every field appears exactly once.
    Struct(Vec<(usize, Expr)>),
    /// Wrapping negation of a number.
    Neg(Box<Expr>),
    /// Logical not of a bool.
    Not(Box<Expr>),
    /// Bitwise not of an integer.
    BitNot(Box<Expr>),
    /// Both operands have the same number or bool type: the result type, or
    /// for comparisons the type compared in.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Integer shift; the amount is any integer type and wraps around the width.
    Shift { left: bool, value: Box<Expr>, amount: Box<Expr> },
    /// Integer power; the exponent is any integer type.
    IntPow(Box<Expr>, Box<Expr>),
    /// Short-circuit `&&` (`and: true`) or `||`.
    Logic { and: bool, lhs: Box<Expr>, rhs: Box<Expr> },
    /// Numeric conversion from the inner expression's type to `ty`.
    Convert(Box<Expr>),
    /// Arguments in order. For a function with `has_self`, the first argument
    /// is the receiver: when it `is_place`, the callee works on that storage.
    Call(FuncId, Vec<Expr>),
    /// `++`/`--` on a number. The value is the old one for postfix, the new
    /// one for prefix.
    IncDec { place: Place, inc: bool, prefix: bool },
    /// `value::text()`: type `Str`, only ever the argument of `Out`.
    Text(Box<Expr>),
    /// `io::out` / `io::outln`. The argument is `Str` or `Text`.
    Out { newline: bool, arg: Box<Expr> },
}
