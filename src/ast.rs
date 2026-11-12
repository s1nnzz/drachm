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
    pub enums: Vec<Enum>,
    /// `failure Name { ... }` declarations.
    pub failures: Vec<Failure>,
    /// `def NAME of STRUCT { ... }` declarations, in the order written.
    pub defs: Vec<DefDecl>,
    /// The file flag `//--!$unsafe: true` is present (set by `lib::check`).
    pub unsafe_allowed: bool,
    /// Every `T?` written in the file, with `T` as written (no marks, no `?`).
    /// The checker makes one optional type for each distinct `T` (S5A.2).
    pub optionals: Vec<TypeName>,
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
    /// The struct's default `def`: `struct Player defaults PlayerStart` or
    /// `struct default Player`.
    pub defaults: Option<(String, Span)>,
    /// `Person(auto);`: keeps the automatic field-order constructor next to
    /// the written ones.
    pub auto_ctor: Option<Span>,
    pub fields: Vec<Field>,
    pub methods: Vec<Method>,
}

/// `enum Name { Variant, Variant(Type name, ...), }`
#[derive(Debug)]
pub struct Enum {
    pub name: String,
    pub name_span: Span,
    pub variants: Vec<Variant>,
}

/// `failure Name { Variant "message" exit N, Variant(Type v) "msg ${v}", }`
#[derive(Debug)]
pub struct Failure {
    pub name: String,
    pub name_span: Span,
    pub variants: Vec<FailVariant>,
}

#[derive(Debug)]
pub struct FailVariant {
    pub name: String,
    pub name_span: Span,
    pub fields: Vec<Field>,
    /// The message, a string literal that interpolates `${value}`.
    pub message: Expr,
    pub exit: Option<(u64, Span)>,
}

#[derive(Debug)]
pub struct Variant {
    pub name: String,
    pub name_span: Span,
    /// The payload fields, in order (`public` and `mutable` are not used).
    pub fields: Vec<Field>,
}

/// `def NAME of STRUCT from BASE { field = value; }`. Without `of`, the struct
/// is the one named like the def (`def Player { }`).
#[derive(Debug)]
pub struct DefDecl {
    pub name: String,
    pub name_span: Span,
    pub of: Option<(String, Span)>,
    pub from: Option<(String, Span)>,
    pub assigns: Vec<DefAssign>,
    pub span: Span,
}

#[derive(Debug)]
pub struct DefAssign {
    pub name: String,
    pub name_span: Span,
    pub value: Expr,
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
    /// A constructor: named after the struct, with no return type.
    pub ctor: bool,
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
    /// The `?` after the name: `uint32?` is an optional `uint32`. Marks written
    /// before the name apply to the optional (`*uint32?` is a pointer to one).
    pub optional: Option<Span>,
    /// Generic arguments: `Map<string, uint32>` has two. An integer literal
    /// argument (`Array<T, 4>`) is a `TypeName` with `is_number` set and the
    /// digits as `name`.
    pub args: Vec<TypeName>,
    /// Sizes after the name, applied left to right: `uint32[?][4]`.
    pub dims: Vec<Dim>,
    /// A number written where a type argument goes.
    pub is_number: bool,
}

impl TypeName {
    /// A bare name with nothing else written.
    pub fn plain(name: impl Into<String>, span: Span) -> TypeName {
        TypeName { name: name.into(), span, mods: Vec::new(), unit: None, optional: None, args: Vec::new(), dims: Vec::new(), is_number: false }
    }
}

/// One `[..]` after a type: `[?]` is a growing list, `[4]` an array of 4.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dim {
    List(Span),
    Array(u64, Span),
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
    /// `*fixed T`
    FixedPtr,
}

#[derive(Debug, Clone)]
pub struct Param {
    pub mutable: bool,
    pub open: bool,
    pub ty: TypeName,
    pub name: String,
    pub span: Span,
}

/// One entry of a type parameter list.
#[derive(Debug, Clone)]
pub struct TypeParam {
    pub name: String,
    pub span: Span,
    /// `unit U`: stands for a unit, not a type.
    pub is_unit: bool,
    /// `T with A and B`.
    pub bounds: Vec<TypeName>,
}

#[derive(Debug)]
pub struct Func {
    pub ret: TypeName,
    pub name: String,
    pub name_span: Span,
    /// `<T, U with A and B, unit V>` after the name.
    pub tparams: Vec<TypeParam>,
    pub params: Vec<Param>,
    pub body: Block,
    /// The word `unsafe` before the return type.
    pub is_unsafe: Option<Span>,
    /// `lives a, b`: parameters whose borrows the result may hold.
    pub lives: Vec<(String, Span)>,
    /// From `lives` to the end of its list, if written.
    pub lives_span: Option<Span>,
    /// Written `lives from a or b`, the form that may name `self`.
    pub lives_from: bool,
    /// `fails A, B`: the failure types the function may end with.
    pub fails: Vec<(String, Span)>,
    /// From `fails` to the end of its list, if written.
    pub fails_span: Option<Span>,
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
    /// `maybe v;` inside a failure handler: the value of the handled call.
    Resume { value: Option<Expr>, span: Span },
    /// `fail X;`, `fail X(args);`, `fail Type::X;`, with `because e` if given.
    Fail { value: Expr, because: Option<Expr>, span: Span },
    /// `unsafe { }`
    Unsafe { body: Block, span: Span },
    /// `destroy x;`: runs `drop()` now and ends `x`.
    Destroy { target: Expr, span: Span },
    /// `fill [partial] x { ... }`, with `with defaults` or `with NAME` after the name.
    Fill { partial: bool, target: (String, Span), with: Option<FillWith>, body: Block, span: Span },
    Expr(Expr),
}

/// One `fails with (...) => { }` or `otherwise (any e) => { }` after a call.
#[derive(Debug)]
pub struct Handler {
    pub kind: HandlerKind,
    /// The name the failure is bound to.
    pub bind: Option<(String, Span)>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug)]
pub enum HandlerKind {
    /// `fails with (Type e)`
    Type(String, Span),
    /// `fails with (any e)`
    Any,
    /// `otherwise (any e)`
    Otherwise,
    /// `fails with none` and `ignore failure`
    Ignore,
}

/// The `with` clause of a `fill`.
#[derive(Debug)]
pub enum FillWith {
    /// `with defaults`: the struct's default `def`.
    Defaults(Span),
    /// `with NAME`: the named `def`.
    Named(String, Span),
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
    /// `case P if cond { }`
    pub guard: Option<Expr>,
    pub body: Block,
    pub span: Span,
}

#[derive(Debug)]
pub enum Pattern {
    /// `_`
    Any(Span),
    /// A value or an inclusive range (`ExprKind::Range`). On an enum, a bare
    /// `Variant` or `Enum::Variant` is a pattern of that variant.
    Expr(Expr),
    /// `globbed "..."`: text in which each `*` is a wildcard (a `string` match only).
    Glob(Expr),
    /// `Variant(Type name, _)` or `Enum::Variant(...)`. `args` is empty
    /// both for `Variant()` and for a bare `Variant` written with no parentheses.
    Variant { enum_name: Option<(String, Span)>, name: String, name_span: Span, args: Vec<SubPat>, parens: bool, span: Span },
    /// `Type name`: the value of an optional, bound to `name`.
    Bind { mode: BindMode, ty: TypeName, name: String, name_span: Span },
}

/// What a binding in a pattern does with a payload that owns memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindMode {
    /// `Type name`
    Plain,
    /// `take Type name`: moves the payload out; the matched value is used up.
    Take,
    /// `copy Type name`: a copy of the payload.
    Copy,
}

/// One payload slot in a variant pattern.
#[derive(Debug)]
pub enum SubPat {
    /// `_`
    Any(Span),
    /// `[take | copy] Type name`
    Bind { mode: BindMode, ty: TypeName, name: String, name_span: Span },
    /// A constant the payload must equal: `2.0`, `true`, `"hi"`.
    Value(Expr),
    /// `globbed "*.png"`: a text payload that matches a glob.
    Glob(Expr),
    /// A pattern for a payload that is itself an enum: `Circle(float64 r)`.
    Variant(Box<Pattern>),
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
    /// `Name::Variant` with no parentheses: a variant of an enum with no payload.
    Path { owner: String, owner_span: Span, name: String, name_span: Span },
    /// The keyword `self`.
    SelfValue,
    /// `Stack<uint32>` before `::`: a type name with its arguments.
    TypeArgs { name: String, targs: Vec<TypeName> },
    /// `recv.name`
    Field { recv: Box<Expr>, name: String, name_span: Span },
    Unary(UnOp, Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    /// `x++`, `++x`, `x--`, `--x`.
    IncDec { inc: bool, prefix: bool, target: Box<Expr> },
    Call { name: String, targs: Vec<TypeName>, args: Vec<Expr> },
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
    /// `new Type of (values)`: a constructor call, with the values as written.
    New { name: String, name_span: Span, targs: Vec<TypeName>, values: Vec<FieldValue> },
    /// `call fails`: pass a failure of the call up to the caller.
    PassUp(Box<Expr>),
    /// `call fails with (T e) => { } ... otherwise (any e) => { }`, or
    /// `call fails with none` / `call ignore failure` (one `Ignore` handler).
    Handled { call: Box<Expr>, handlers: Vec<Handler> },
    /// `now e`: a field already set may be read inside a constructor or `fill`.
    Now(Box<Expr>),
    /// `value maybe fallback`: the value of an optional, or `fallback` when it is `none`.
    Maybe { value: Box<Expr>, fallback: Box<Expr> },
    /// `value has value` (`has: true`) or `value is empty` (`has: false`) on an optional.
    OptionalTest { value: Box<Expr>, has: bool },
}
