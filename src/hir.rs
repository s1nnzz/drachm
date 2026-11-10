//! The checked program: every expression has a concrete type, every
//! conversion is explicit, and names are resolved to ids. Both backends
//! (Cranelift and C) are generated from this.
//!
//! Strings and ownership. Every `string` local, named or hidden, is empty
//! (`{ NULL, 0, 0 }`) when its function starts, and `Drop` leaves it empty
//! again, so dropping a local that holds nothing is harmless. A `string`
//! expression is one of two kinds:
//! - a *place*, borrowed where it is used: `Local`, `Str` and `Temp`;
//! - an *owned value*, freshly made: every other `string` expression (a
//!   literal is both: its bytes are not owned, so it never needs dropping).
//!
//! Places are the operands of `StrCall`, `Join` and `Out`. Owned values go
//! where something takes ownership: the value of an `Assign`, of a `Return`,
//! and the arguments of a `Call` (the callee owns its parameters and drops
//! them). The checker inserts every `Copy`, `Move`, `Temp`, `Scoped` and
//! `Drop`; the backends only carry them out and add no ownership logic of
//! their own. That includes jumps: a `Jump` comes right after the `Drop`s of
//! every `string` local in scope between it and its target.

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

/// Names an `If` or a `Loop` for the jumps that target it. Unique within a function.
pub type Label = usize;

pub type Block = Vec<Stmt>;

pub enum Stmt {
    /// Store a value in a local. For a `string` local the value is owned
    /// and the local is empty beforehand.
    Assign(LocalId, Expr),
    /// Tests the branches in order and runs the body of the first whose
    /// test holds (a branch with no test always does), then carries on
    /// after the whole statement. `if` chains and `match` both become this.
    If { label: Label, branches: Vec<Branch> },
    /// Each time round: if there is a `cond` and it is false, the loop ends;
    /// otherwise `body` runs, then `step`. `continue` jumps to `step`.
    Loop { label: Label, cond: Option<Expr>, body: Block, step: Block },
    Jump(Jump),
    Return(Option<Expr>),
    Expr(Expr),
    /// Free what a `string` local owns, if anything, and leave it empty.
    Drop(LocalId),
}

pub struct Branch {
    pub test: Option<Expr>,
    pub body: Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jump {
    /// Leave the `Loop`.
    Break(Label),
    /// Go to the `Loop`'s step.
    Continue(Label),
    /// Leave the `If`, skipping its remaining branches.
    Leave(Label),
    /// Test branch number `n` of the `If` (and the ones after it, as usual).
    Next(Label, usize),
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
    /// A string literal: a not-owned view of read-only bytes.
    Str(String),
    /// A local; for a `string`, a place.
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
    /// A call. `string` arguments are owned values, handed to the callee.
    Call(FuncId, Vec<Expr>),
    /// `++`/`--` on a numeric local. The value is the old one for postfix,
    /// the new one for prefix.
    IncDec { local: LocalId, inc: bool, prefix: bool },
    /// `value::text()` of a number or bool: an owned `string`.
    Text(Box<Expr>),
    /// An integer as text in base 2, 8 or 16 (`io::outf`'s `%x` and friends):
    /// an owned `string`.
    Radix { value: Box<Expr>, base: u32, upper: bool },
    /// A deep copy of a `string` place: an owned value.
    Copy(Box<Expr>),
    /// The value of a hidden `string` local, taken out of it (it is left
    /// empty): an owned value.
    Move(LocalId),
    /// Evaluate an owned `string` into the hidden local, which is empty
    /// beforehand, and use it as a place. The checker drops the local at the
    /// end of the statement (through `Scoped`).
    Temp(LocalId, Box<Expr>),
    /// Evaluate the inner expression, then drop the locals, in order. The
    /// value is the inner one's. This is how temporaries die at the end of
    /// their statement, and how `return` drops what is in scope.
    Scoped(Box<Expr>, Vec<LocalId>),
    /// A string runtime function. Arguments have the types `StrFn::params`
    /// gives; `string` ones are places.
    StrCall(StrFn, Vec<Expr>),
    /// The texts of several `string` places one after another: an owned value
    /// (interpolation and `io::outf`).
    Join(Vec<Expr>),
    /// `io::out` / `io::outln` of a `string` place.
    Out { newline: bool, arg: Box<Expr> },
}

/// Runtime string functions with a fixed signature (`runtime/strings.h`, and
/// the `drachm_text_*` helpers in `runtime/runtime.c`). A `string` result
/// comes back through an out pointer in the C signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrFn {
    Concat,
    /// Appends the second argument to the first, which is a `string` local.
    Append,
    Eq,
    /// -1, 0 or 1, comparing bytes.
    Cmp,
    Len,
    IsEmpty,
    Reverse,
    Sub,
    Upper,
    Lower,
    Trim,
    Find,
    Contains,
    StartsWith,
    EndsWith,
    Count,
    Replace,
    ReplaceLimit,
    Insert,
    Remove,
    Repeat,
    PadLeft,
    PadRight,
    /// A float with exactly N decimals (`io::outf`'s `%.N`).
    Fixed,
    /// Number text padded with zeros after the sign (`io::outf`'s `%0N`).
    ZeroPad,
}

const S: Type = Type::Str;
const U64: Type = Type::Int { bits: 64, signed: false };
const S64: Type = Type::Int { bits: 64, signed: true };

impl StrFn {
    /// The C function's name.
    pub fn name(self) -> &'static str {
        use StrFn::*;
        match self {
            Concat => "drachm_str_concat",
            Append => "drachm_str_append",
            Eq => "drachm_str_eq",
            Cmp => "drachm_str_cmp",
            Len => "drachm_str_len",
            IsEmpty => "drachm_str_is_empty",
            Reverse => "drachm_str_reverse",
            Sub => "drachm_str_sub",
            Upper => "drachm_str_upper",
            Lower => "drachm_str_lower",
            Trim => "drachm_str_trim",
            Find => "drachm_str_find",
            Contains => "drachm_str_contains",
            StartsWith => "drachm_str_starts_with",
            EndsWith => "drachm_str_ends_with",
            Count => "drachm_str_count",
            Replace => "drachm_str_replace",
            ReplaceLimit => "drachm_str_replace_limit",
            Insert => "drachm_str_insert",
            Remove => "drachm_str_remove",
            Repeat => "drachm_str_repeat",
            PadLeft => "drachm_str_pad_left",
            PadRight => "drachm_str_pad_right",
            Fixed => "drachm_text_fixed",
            ZeroPad => "drachm_text_zero_pad",
        }
    }

    pub fn params(self) -> &'static [Type] {
        use StrFn::*;
        match self {
            Len | IsEmpty | Reverse | Upper | Lower | Trim => &[S],
            Concat | Append | Eq | Cmp | Find | Contains | StartsWith | EndsWith | Count => &[S, S],
            Sub | Remove => &[S, S64, S64],
            Insert => &[S, S64, S],
            Replace => &[S, S, S],
            ReplaceLimit => &[S, S, S, U64],
            Repeat | ZeroPad => &[S, U64],
            PadLeft | PadRight => &[S, U64, S],
            Fixed => &[Type::F64, Type::U32],
        }
    }

    pub fn ret(self) -> Type {
        use StrFn::*;
        match self {
            Append => Type::None,
            Eq | IsEmpty | Contains | StartsWith | EndsWith => Type::Bool,
            Cmp => Type::S32,
            Len | Count => U64,
            Find => S64,
            _ => S,
        }
    }
}
