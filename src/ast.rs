//! The syntax tree, exactly as written. Names and types are resolved later,
//! by the checker, which turns this into `hir`.

use crate::consts::Const;
use crate::diag::Span;
use crate::types::Type;

/// Internal name of the function generated for a script's top-level statements.
pub(crate) const SCRIPT_ENTRY: &str = "$script";

#[derive(Debug, Default)]
pub struct Program {
    pub imports: Vec<Import>,
    /// What the file shows to other files: `out A, B as C;` lists and `out`
    /// written before a declaration, in the order written.
    pub exports: Vec<Export>,
    /// `unit` declarations, in the order written (order does not matter to the checker).
    pub units: Vec<UnitDecl>,
    pub funcs: Vec<Func>,
    pub structs: Vec<Struct>,
    pub enums: Vec<Enum>,
    /// `interface Name { ... }` declarations.
    pub interfaces: Vec<Interface>,
    /// `TYPE with Interface { ... }` declarations: a type promising an interface
    /// from outside its own declaration.
    pub impls: Vec<ImplDecl>,
    /// `failure Name { ... }` declarations.
    pub failures: Vec<Failure>,
    /// `def NAME of STRUCT { ... }` declarations, in the order written.
    pub defs: Vec<DefDecl>,
    /// Top-level values: `uint32 SIZE = 8;`, `fixed uint32 MAX = SIZE * 2;`.
    pub consts: Vec<ConstDecl>,
    /// The file flag `//--!$unsafe: true` is present (set by `lib::check`).
    pub unsafe_allowed: bool,
    /// Every `T?` written in the file, with `T` as written (no marks, no `?`).
    /// The checker makes one optional type for each distinct `T` (S5A.2).
    pub optionals: Vec<TypeName>,
    /// Every type written with type arguments (`Stack<uint32>`, also the
    /// ones inside other arguments), in the order the parser finished them.
    /// The checker makes the instances they name before it resolves types.
    pub generics: Vec<TypeName>,
    /// `::` written after the first call on a name (`physics::gravity(2.0)::text()`).
    /// It is an error unless the name is a module, which only the resolver
    /// (or the single-file check) knows: it decides and reports these.
    pub chain_slips: Vec<ChainSlip>,
    /// The C enums a header import brings (step 7.1): name and integer type.
    pub c_enums: Vec<(String, crate::types::CKind)>,
}

/// A `::` where `->` is wanted, after the first call on `owner`: the E0007
/// error, held back until the name is known to be a module or not.
#[derive(Debug)]
pub struct ChainSlip {
    pub owner: String,
    /// The span of the name the first call is made on.
    pub owner_span: Span,
    pub diag: crate::diag::Diagnostic,
}

/// `struct Name { fields and methods }`
#[derive(Debug)]
pub struct Struct {
    pub name: String,
    pub name_span: Span,
    /// `<T, U>` after the name: the struct is a template.
    pub tparams: Vec<TypeParam>,
    /// From `struct` to the closing `}`.
    pub span: Span,
    /// `with A and B`: the interfaces it implements.
    pub with: Vec<TypeName>,
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

/// `interface Name extends A and B { signatures; static signatures; fields; }`
#[derive(Debug, Clone)]
pub struct Interface {
    pub name: String,
    pub name_span: Span,
    /// `<T>` after the name (not supported yet).
    pub tparams: Vec<TypeParam>,
    pub extends: Vec<TypeName>,
    pub methods: Vec<IMethod>,
    /// Required fields.
    pub fields: Vec<IField>,
    /// From `interface` to the closing `}`.
    pub span: Span,
}

/// A method an interface lists: a signature, no body.
#[derive(Debug, Clone)]
pub struct IMethod {
    pub is_static: Option<Span>,
    pub operator: Option<BinOp>,
    pub name: String,
    pub name_span: Span,
    pub ret: TypeName,
    pub params: Vec<Param>,
    /// `changes self` after the parameters.
    pub changes_self: Option<Span>,
    /// From the return type to the end of the signature.
    pub span: Span,
}

/// A field an interface requires.
#[derive(Debug, Clone)]
pub struct IField {
    pub ty: TypeName,
    pub name: String,
    pub name_span: Span,
}

/// `TYPE with A and B { methods }`: the methods that make `TYPE` implement
/// the interfaces, written outside the type's own declaration.
#[derive(Debug)]
pub struct ImplDecl {
    pub ty: TypeName,
    pub with: Vec<TypeName>,
    /// `with trusted I`: the methods may use the type's private members.
    pub trusted: bool,
    pub methods: Vec<Method>,
    pub span: Span,
}

/// `enum Name { Variant, Variant(Type name, ...), }`
#[derive(Debug)]
pub struct Enum {
    pub name: String,
    pub name_span: Span,
    pub tparams: Vec<TypeParam>,
    /// From `enum` to the closing `}`.
    pub span: Span,
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

#[derive(Debug, Clone)]
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
    /// `Cell<uint32>` after `of`: the type arguments of a generic struct.
    pub of_args: Vec<TypeName>,
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
#[derive(Debug, Clone)]
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

/// One `in` item. The forms (owner, step 7 batches A and B):
///
/// ```text
/// in io;                                  module            path ["io"]
/// in game.physics;  in physics inside game;   the same module, dotted or in words
/// in game.physics as phys;                module with a name of its own
/// in Vector3 as V3, gravity from physics; items of a module (each may be renamed)
/// in * from game.audio;                   every exported name
/// ```
#[derive(Debug)]
pub struct Import {
    /// The module's path, outermost folder first: `game.physics` is
    /// `["game", "physics"]`, whichever way it was written.
    pub path: Vec<String>,
    /// The words that name the module (the path, or the part after `from`).
    pub path_span: Span,
    /// Written in words (`physics inside game`) rather than with dots.
    pub word_form: bool,
    /// `in A, B as C from path`: the names taken from the module. Empty for
    /// the module forms and for `*`.
    pub items: Vec<ImportItem>,
    /// `in * from path`.
    pub glob: bool,
    /// `in path as name` (or `in * from path as name`).
    pub alias: Option<(String, Span)>,
    /// `in * from path as name also bare;`: the names come bare too.
    pub also_bare: bool,
    /// The module is named by a file path in a string, relative to the
    /// importing file (`"../runtime/scheduler.drh"`). `path` then holds the
    /// file's stem alone, the module's default name.
    pub file: Option<String>,
    /// `in * from c."SDL2/SDL.h" as sdl;`: the module is a C header, with the
    /// header text as written (step 7.1). `path` holds the alias.
    pub c_header: Option<(String, Span)>,
    pub span: Span,
}

impl Import {
    /// `Some("io")` for a bare `in io;`, the only form the checker takes today.
    pub fn plain_name(&self) -> Option<&str> {
        match self.path.as_slice() {
            [one] if self.items.is_empty() && !self.glob && self.alias.is_none() => Some(one),
            _ => None,
        }
    }

    /// The name this import gives the module in the importing file: the alias,
    /// or the last part of the path. `None` for item and `*` imports.
    pub fn binding(&self) -> Option<&str> {
        if !self.items.is_empty() || self.glob && self.alias.is_none() {
            return None;
        }
        Some(self.alias.as_ref().map(|a| a.0.as_str()).unwrap_or_else(|| self.path.last().unwrap()))
    }
}

/// `Vector3 as V3` in `in Vector3 as V3, gravity from physics;`
#[derive(Debug)]
pub struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
    pub span: Span,
}

/// One exported name: `out Vector3 as Vec3;` or the `out` before a declaration.
#[derive(Debug)]
pub struct Export {
    /// The declared (or imported) name.
    pub name: String,
    /// The name other files see, when `as` renames it.
    pub alias: Option<String>,
    pub span: Span,
    /// Written as `out` before the declaration itself, not in a list.
    pub on_decl: bool,
}

impl Export {
    /// The name other files use.
    pub fn shown_as(&self) -> &str {
        self.alias.as_deref().unwrap_or(&self.name)
    }
}

/// A type as written in the source.
#[derive(Debug, Clone)]
pub struct TypeName {
    pub name: String,
    pub span: Span,
    /// A function type: parameter types and the result type.
    pub func: Option<Box<FuncType>>,
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
    /// A value written where a type argument goes (`4`, `-1`, `2 * 8`):
    /// a constant expression for a value parameter (`uint32 N`). Set with
    /// `is_number`. A bare name there is an ordinary `TypeName`; which of the
    /// two it means depends on the parameter.
    pub val: Option<Box<ConstExpr>>,
    /// A unit written where a type argument goes: `Vec3<float64, unit m/s>`.
    /// Its `name` is empty; the unit is here.
    pub unit_arg: Option<UnitSpec>,
    /// The return type the parser gives a constructor: the struct itself,
    /// whatever type arguments it has in this instance.
    pub self_ty: bool,
}

#[derive(Debug, Clone)]
pub struct FuncType {
    pub params: Vec<TypeName>,
    pub ret: TypeName,
}

impl TypeName {
    /// A bare name with nothing else written.
    pub fn plain(name: impl Into<String>, span: Span) -> TypeName {
        TypeName { name: name.into(), span, func: None, mods: Vec::new(), unit: None, optional: None, args: Vec::new(), dims: Vec::new(), is_number: false, val: None, unit_arg: None, self_ty: false }
    }
}

/// One `[..]` after a type: `[?]` is a growing list, `[4]` an array of 4.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Dim {
    List(Span),
    Array(u64, Span),
    /// `[N]`: the length is a value parameter.
    Param(&'static str, Span),
    /// `[N + 1]` and the like: a length computed from more than one name or number.
    Calc(&'static ConstExpr, Span),
}

/// A top-level value: `[fixed] [mut] Type NAME = value;`.
#[derive(Debug)]
pub struct ConstDecl {
    /// `fixed`: the value must be a compile-time constant.
    pub fixed: bool,
    pub mutable: bool,
    pub ty: TypeName,
    pub name: String,
    pub name_span: Span,
    pub value: Option<Init>,
    pub span: Span,
}

/// A constant expression written as a value argument: whole numbers, value
/// parameter names and arithmetic.
#[derive(Debug, Clone, PartialEq)]
pub enum ConstExpr {
    Num(num_bigint::BigInt, Span),
    Name(String, Span),
    Neg(Box<ConstExpr>, Span),
    Not(Box<ConstExpr>, Span),
    Bin(BinOp, Box<ConstExpr>, Box<ConstExpr>, Span),
}

impl ConstExpr {
    pub fn span(&self) -> Span {
        match self {
            ConstExpr::Num(_, s) | ConstExpr::Name(_, s) | ConstExpr::Neg(_, s) | ConstExpr::Not(_, s) | ConstExpr::Bin(_, _, _, s) => *s,
        }
    }
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
    /// `T as unit`: the unit named `T`, even where a type parameter `T` is in
    /// scope (the parameter does not stand for a unit).
    Explicit(String, Span),
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
    /// `foreign *T`: a pointer the C side owns (step 7.1).
    ForeignPtr,
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
    /// `uint32 N`: a value parameter; the integer type it was written with.
    pub value: Option<(String, Span)>,
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
    /// A C function from a header import: the symbol to call. It has no body.
    pub foreign: Option<String>,
}

/// One variable of a `for` over map entries: `mut string name`.
#[derive(Debug)]
pub struct LoopVar {
    pub mutable: bool,
    pub ty: TypeName,
    pub name: String,
    pub name_span: Span,
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
    /// `for (T name of iter) { }`. `step` is the `by n` after a range
    /// (`for (uint32 i of 0..10 by 3)`).
    ForEach { mutable: bool, ty: TypeName, name: String, name_span: Span, iter: Expr, step: Option<Expr>, body: Block, span: Span },
    /// `for (K key, V value of map) { }`: walking a map's entries (Q16).
    ForEntries { key: LoopVar, value: LoopVar, iter: Expr, body: Block, span: Span },
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
    /// `a same as b`: two borrows (or values holding borrows) at the same address.
    Same,
    /// `a not same as b`: the negative of `Same`.
    NotSame,
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
            Same => "same as",
            NotSame => "not same as",
        }
    }

    pub fn class(self) -> OpClass {
        use BinOp::*;
        match self {
            Add | Sub | Mul | Div | Rem | Pow => OpClass::Arithmetic,
            Shl | Shr => OpClass::Shift,
            BitAnd | BitOr | BitXor => OpClass::Bitwise,
            Eq | Ne | Lt | Le | Gt | Ge | Same | NotSame => OpClass::Comparison,
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
pub enum LambdaBody {
    Expr(Box<Expr>),
    Block(Block),
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
    Lambda { moves: bool, params: Vec<Param>, body: LambdaBody },
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
    /// `take place`, `take place leaving v`, or `take xs[i] reorders` (`reorders`).
    Take { place: Box<Expr>, leaving: Option<Box<Expr>>, reorders: bool },
    /// `lend e`
    Lend(Box<Expr>),
    /// A named constant of a C enum, made by the resolver for `alias::NAME`
    /// (step 7.1): the enum's name in the program, and the value.
    CConst { enum_name: String, value: i128 },
    /// `&s as c string`: a NUL-terminated copy of the string for one C call (step 7.1).
    CString(Box<Expr>),
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
    /// `[a, b, c]`: a list or array, whichever the type wanted says.
    ListLit(Vec<Expr>),
    /// `{ k: v, ... }`: a map (S6D); `{}` is the empty one.
    MapLit(Vec<(Expr, Expr)>),
    /// `recv[index]`
    Index { recv: Box<Expr>, index: Box<Expr> },
    /// `value has value` (`has: true`) or `value is empty` (`has: false`) on an optional.
    OptionalTest { value: Box<Expr>, has: bool },
}
