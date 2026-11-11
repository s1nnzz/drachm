//! The syntax tree, exactly as written. Names and types are resolved later,
//! by the checker, which turns this into `hir`.

use crate::consts::Const;
use crate::diag::Span;
use crate::types::Type;

#[derive(Debug)]
pub struct Program {
    pub imports: Vec<Import>,
    /// `unit` declarations, in the order written (order does not matter to the checker).
    pub units: Vec<UnitDecl>,
    pub funcs: Vec<Func>,
    pub structs: Vec<Struct>,
    /// The file flag `//--!$unsafe: true` is present (set by `lib::check`).
    pub unsafe_allowed: bool,
}

/// `struct Name { fields and methods }`
#[derive(Debug)]
pub struct Struct {
    pub name: String,
    pub name_span: Span,
    /// `lives a, b`: the borrow fields it names.
    pub lives: Vec<(String, Span)>,
    /// The `stays` clause: never copied automatically.
    pub stays: Option<Span>,
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
    /// Borrow and pointer marks written before the name, outermost first:
    /// `&mut string` is `[Ref { mutable: true }]`, `*uint32` is `[Ptr]`.
    pub mods: Vec<(TypeMod, Span)>,
    /// The unit in angle brackets: `float64<m/s>`, `float64<delta degC>`.
    pub unit: Option<UnitSpec>,
}

/// A unit written in angle brackets after a number type or a number literal,
/// or after `::in`. `delta` marks a difference of points (`delta degC`).
#[derive(Debug, Clone, PartialEq)]
pub struct UnitSpec {
    pub delta: bool,
    pub expr: UnitExpr,
}

/// A unit expression as written. `Pow` is only ever written after a name.
#[derive(Debug, Clone, PartialEq)]
pub enum UnitExpr {
    Name(String, Span),
    /// The dimensionless `1`.
    One(Span),
    Mul(Box<UnitExpr>, Box<UnitExpr>),
    Div(Box<UnitExpr>, Box<UnitExpr>),
    Pow(Box<UnitExpr>, i32, Span),
}

/// `unit NAME;`, `unit NAME = def;`, optionally followed by `offset SIGNED`.
#[derive(Debug)]
pub struct UnitDecl {
    pub name: String,
    pub name_span: Span,
    /// `None` for a base unit (`unit m;`).
    pub def: Option<UnitDef>,
    /// `offset -273.15`: the value with its sign.
    pub offset: Option<(Const, Span)>,
    pub span: Span,
}

/// The right side of `unit NAME = ...`: a factor, a unit expression, or both.
/// `unit km = 1000 m;` has both; `unit Hz = 1 / s;` is the factor `1` with
/// `1 / s` (written as `Div(One, s)`); `unit pct = 0.01;` has only a factor.
#[derive(Debug)]
pub struct UnitDef {
    pub factor: Option<(Const, Span)>,
    pub expr: Option<UnitExpr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeMod {
    Ref { mutable: bool },
    Ptr,
}

#[derive(Debug)]
pub struct Param {
    pub mutable: bool,
    pub open: bool,
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
    /// The word `unsafe` before the return type.
    pub is_unsafe: Option<Span>,
    /// `lives a, b`: parameters whose borrows the result may hold.
    pub lives: Vec<(String, Span)>,
    /// From `lives` to the end of its list, if written.
    pub lives_span: Option<Span>,
    /// From the return type to the closing `)`.
    pub span: Span,
}

/// `{ statements }`. `end` is the closing `}`.
#[derive(Debug)]
pub struct Block {
    pub stmts: Vec<Stmt>,
    pub end: Span,
}

impl std::ops::Deref for Block {
    type Target = Vec<Stmt>;
    fn deref(&self) -> &Vec<Stmt> {
        &self.stmts
    }
}

impl<'a> IntoIterator for &'a Block {
    type Item = &'a Stmt;
    type IntoIter = std::slice::Iter<'a, Stmt>;
    fn into_iter(self) -> Self::IntoIter {
        self.stmts.iter()
    }
}

#[derive(Debug)]
pub enum Stmt {
    Let { mutable: bool, open: bool, ty: TypeName, name: String, name_span: Span, value: Option<Init>, span: Span },
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
    /// `unsafe { }`
    Unsafe { body: Block, span: Span },
    /// `destroy x;`: runs `drop()` now and ends `x`.
    Destroy { target: Expr, span: Span },
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
    /// The unit written directly after the number: `9.6<s>`.
    pub unit: Option<UnitSpec>,
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
    /// `unit` is set only by `recv::in<unit>()`, where `targs` is empty.
    Method { recv: Box<Expr>, name: String, name_span: Span, targs: Vec<TypeName>, unit: Option<UnitSpec>, args: Vec<Expr> },
    /// `a..b`, inclusive. Only allowed after `of` in a `for` and as a `case` pattern.
    Range(Box<Expr>, Box<Expr>),
    /// `&e` or `&mut e`.
    Borrow { mutable: bool, operand: Box<Expr> },
    /// `*e`
    Deref(Box<Expr>),
    /// `copy e`
    CopyOf(Box<Expr>),
    /// `take e` or `take e leaving v`.
    Take { place: Box<Expr>, leaving: Option<Box<Expr>> },
    /// `lend e`
    Lend(Box<Expr>),
    /// `address of e`
    AddressOf(Box<Expr>),
    /// `none` as a value.
    NoneValue,
}
