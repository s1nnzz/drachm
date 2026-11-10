//! The syntax tree, exactly as written. Names and types are resolved later,
//! by the checker, which turns this into `hir`.

use crate::consts::Const;
use crate::diag::Span;
use crate::types::Type;

#[derive(Debug)]
pub struct Program {
    pub imports: Vec<Import>,
    pub funcs: Vec<Func>,
    pub structs: Vec<Struct>,
}

/// `struct Name { fields and methods }`
#[derive(Debug)]
pub struct Struct {
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
}

/// `Type name;` or `mut Type name;` inside a struct.
#[derive(Debug)]
pub struct Field {
    pub public: bool,
    pub mutable: bool,
    pub ty: TypeName,
    pub name: String,
    pub name_span: Span,
}

/// A function inside a struct. Operator methods are named by their symbol
/// (`func.name` is `"+"`) and have `operator` set.
#[derive(Debug)]
pub struct Method {
    pub public: bool,
    /// The `static` keyword, if written.
    pub is_static: Option<Span>,
    pub operator: Option<BinOp>,
    pub func: Func,
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
    Let { mutable: bool, ty: TypeName, name: String, name_span: Span, value: Option<Init>, span: Span },
    /// `x = v;`, or `x += v;` when `op` is set. The parser only lets a name,
    /// `self` or a field access (`p.x`, `self.a.b`) be the target.
    Assign { target: Expr, op: Option<BinOp>, value: Expr, span: Span },
    /// A whole `if (..) { } else if (..) { } else { }` chain: one node,
    /// because `skip` counts a chain as one level. `span` covers `if (cond)`.
    If { branches: Vec<(Expr, Block)>, els: Option<Block>, span: Span },
    /// `span` covers `while (cond)`; likewise for the other loops.
    While { cond: Expr, body: Block, span: Span },
    /// `for (init; cond; step) { }`. Each part may be empty.
    For { init: Option<Box<Stmt>>, cond: Option<Expr>, step: Option<Box<Stmt>>, body: Block, span: Span },
    /// `for (T name of iter) { }`.
    ForEach { mutable: bool, ty: TypeName, name: String, name_span: Span, iter: Expr, body: Block, span: Span },
    Match { value: Expr, cases: Vec<Case>, span: Span },
    /// `break;` / `break N;`, and `continue` when `cont` is set. `count`
    /// is the literal N with its span.
    Break { cont: bool, count: Option<(u64, Span)>, span: Span },
    Skip { target: SkipTarget, span: Span },
    Return { value: Option<Expr>, span: Span },
    Expr(Expr),
}

#[derive(Debug, Clone, Copy)]
pub enum SkipTarget {
    /// `skip;`: the outermost level.
    Outermost,
    /// `skip N;`
    Levels(u64, Span),
    /// `skip to next;`
    Next,
}

/// `case p, q { }`. `span` covers `case` and its patterns.
#[derive(Debug)]
pub struct Case {
    pub patterns: Vec<Pattern>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug)]
pub enum Pattern {
    /// `_`
    Any(Span),
    /// A value or an inclusive range (`ExprKind::Range`).
    Expr(Expr),
}

/// What follows `=` in a declaration.
#[derive(Debug)]
pub enum Init {
    /// A single value: `Type x = v;`.
    Expr(Expr),
    /// The construction shorthand: `Point p = 1, 2;` or `Point p = y: 2, x: 1;`.
    /// Positional and named values are kept as written; mixing them is an
    /// error the checker reports.
    Fields(Vec<FieldValue>, Span),
}

/// One value in the construction shorthand, with its field name if given.
#[derive(Debug)]
pub struct FieldValue {
    pub name: Option<(String, Span)>,
    pub value: Expr,
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

/// A piece of a string literal (see `lexer::StrPiece`).
#[derive(Debug)]
pub enum StrPart {
    /// Text exactly as written in the source at `span`; it contains no escapes.
    Raw(String, Span),
    /// The character an escape stands for; `span` covers the escape.
    Escape(char, Span),
    /// `${expr}`.
    Interp(Expr),
}

#[derive(Debug)]
pub enum ExprKind {
    Num(NumLit),
    Bool(bool),
    /// A string literal of any form; only `f` and backtick strings have `Interp` parts.
    Str(Vec<StrPart>),
    Var(String),
    /// The keyword `self`.
    SelfValue,
    /// `recv.name`
    Field { recv: Box<Expr>, name: String, name_span: Span },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `x++`, `++x`, `x--`, `--x`.
    IncDec { inc: bool, prefix: bool, target: Box<Expr> },
    Call { name: String, args: Vec<Expr> },
    /// `recv::name<targs>(args)` or `recv->name(args)`. `recv` may turn out
    /// to be a module (`io::outln`) or a struct name (`Point::origin()`).
    Method { recv: Box<Expr>, name: String, name_span: Span, targs: Vec<TypeName>, args: Vec<Expr> },
    /// `a..b`, inclusive. Only allowed after `of` in a `for` and as a `case` pattern.
    Range(Box<Expr>, Box<Expr>),
}
