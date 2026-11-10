//! The syntax tree, exactly as written. Names and types are resolved later,
//! by the checker, which turns this into `hir`.

use crate::consts::Const;
use crate::diag::Span;
use crate::types::Type;

#[derive(Debug)]
pub struct Program {
    pub imports: Vec<Import>,
    pub funcs: Vec<Func>,
}

/// `in io;`
#[derive(Debug)]
pub struct Import {
    pub module: String,
    pub span: Span,
}

/// A type as written in the source.
#[derive(Debug, Clone)]
pub struct TypeName {
    pub name: String,
    pub span: Span,
}

#[derive(Debug)]
pub struct Param {
    pub mutable: bool,
    pub ty: TypeName,
    pub name: String,
    pub span: Span,
}

#[derive(Debug)]
pub struct Func {
    pub ret: TypeName,
    pub name: String,
    pub name_span: Span,
    pub params: Vec<Param>,
    pub body: Block,
    /// From the return type to the closing `)`.
    pub span: Span,
}

pub type Block = Vec<Stmt>;

#[derive(Debug)]
pub enum Stmt {
    Let { mutable: bool, ty: TypeName, name: String, name_span: Span, value: Option<Expr>, span: Span },
    /// `x = v;`, or `x += v;` when `op` is set.
    Assign { name: String, name_span: Span, op: Option<BinOp>, value: Expr, span: Span },
    If { cond: Expr, then: Block, els: Option<Block> },
    While { cond: Expr, body: Block },
    Return { value: Option<Expr>, span: Span },
    Expr(Expr),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Pow,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpClass {
    Arithmetic,
    Shift,
    Bitwise,
    Comparison,
    Logic,
}

impl BinOp {
    pub fn symbol(self) -> &'static str {
        use BinOp::*;
        match self {
            Add => "+",
            Sub => "-",
            Mul => "*",
            Div => "/",
            Rem => "%",
            Pow => "**",
            Shl => "<<",
            Shr => ">>",
            BitAnd => "&",
            BitOr => "|",
            BitXor => "^",
            Eq => "==",
            Ne => "!=",
            Lt => "<",
            Le => "<=",
            Gt => ">",
            Ge => ">=",
            And => "&&",
            Or => "||",
        }
    }

    pub fn class(self) -> OpClass {
        use BinOp::*;
        match self {
            Add | Sub | Mul | Div | Rem | Pow => OpClass::Arithmetic,
            Shl | Shr => OpClass::Shift,
            BitAnd | BitOr | BitXor => OpClass::Bitwise,
            Eq | Ne | Lt | Le | Gt | Ge => OpClass::Comparison,
            And | Or => OpClass::Logic,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

#[derive(Debug)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
    /// Written inside its own parentheses.
    pub paren: bool,
}

/// A number literal: an exact value plus the type its suffix names, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct NumLit {
    pub value: Const,
    pub suffix: Option<Type>,
}

#[derive(Debug)]
pub enum ExprKind {
    Num(NumLit),
    Bool(bool),
    Str(String),
    Var(String),
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `x++`, `++x`, `x--`, `--x`.
    IncDec { inc: bool, prefix: bool, target: Box<Expr> },
    Call { name: String, args: Vec<Expr> },
    /// `recv::name<targs>(args)` or `recv->name(args)`. `recv` may turn out
    /// to be a module (`io::outln`).
    Method { recv: Box<Expr>, name: String, name_span: Span, targs: Vec<TypeName>, args: Vec<Expr> },
}
