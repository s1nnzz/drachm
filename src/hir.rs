//! The checked program: every expression has a concrete type, every
//! conversion is explicit, and names are resolved to ids. Both backends
//! (Cranelift and C) are generated from this.

use crate::types::Type;

pub type FuncId = usize;
pub type LocalId = usize;

pub struct Program {
    pub funcs: Vec<Func>,
    pub main: FuncId,
}

pub struct Func {
    pub name: String,
    /// The first `params` locals are the parameters, in order.
    pub params: usize,
    pub locals: Vec<Local>,
    pub ret: Type,
    pub body: Block,
}

pub struct Local {
    pub name: String,
    pub ty: Type,
}

pub type Block = Vec<Stmt>;

pub enum Stmt {
    Assign(LocalId, Expr),
    If(Expr, Block, Block),
    While(Expr, Block),
    Return(Option<Expr>),
    Expr(Expr),
}

pub struct Expr {
    pub kind: ExprKind,
    pub ty: Type,
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
    Local(LocalId),
    /// Wrapping negation of a number.
    Neg(Box<Expr>),
    /// Logical not of a bool.
    Not(Box<Expr>),
    /// Bitwise not of an integer.
    BitNot(Box<Expr>),
    /// Both operands have the same type: the result type, or for
    /// comparisons the type compared in.
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// Integer shift; the amount is any integer type and wraps around the width.
    Shift { left: bool, value: Box<Expr>, amount: Box<Expr> },
    /// Integer power; the exponent is any integer type.
    IntPow(Box<Expr>, Box<Expr>),
    /// Short-circuit `&&` (`and: true`) or `||`.
    Logic { and: bool, lhs: Box<Expr>, rhs: Box<Expr> },
    /// Numeric conversion from the inner expression's type to `ty`.
    Convert(Box<Expr>),
    Call(FuncId, Vec<Expr>),
    /// `++`/`--` on a numeric local. The value is the old one for postfix,
    /// the new one for prefix.
    IncDec { local: LocalId, inc: bool, prefix: bool },
    /// `value::text()`: type `Str`, only ever the argument of `Out`.
    Text(Box<Expr>),
    /// `io::out` / `io::outln`. The argument is `Str` or `Text`.
    Out { newline: bool, arg: Box<Expr> },
}
