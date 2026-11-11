//! The checked program: every expression has a concrete type, every
//! conversion is explicit, and names are resolved to ids. Both backends
//! (Cranelift and C) are generated from the MIR built from this
//! (`mir::build`), which makes the moves, copies, temporaries and drops.
//!
//! Ownership. A type is *owning* if its values own heap memory: `string`,
//! and every struct with a `string` field (directly or in a struct field;
//! `Program::owns`). An expression of an owning type is one of two kinds:
//! - a *place*: `Local`, `Field` of a place, `Deref`, and for strings `Str`;
//!   the MIR builder borrows it where it is used, or moves it where a value
//!   is wanted;
//! - an *owned value*, freshly made: every other one (a string literal is
//!   both: its bytes are not owned, so it never needs dropping). When one is
//!   used as a place (the receiver of `::len()`, a `&string` argument) the
//!   builder puts it in a temporary that dies at the end of the statement.
//!
//! The HIR itself says nothing about copies or drops: `DeepCopy` is the
//! program's own `copy x`, and `Destroy` its own `destroy x;`.
//!
//! Structs that own no strings are plain values: any expression of such a
//! type may be used anywhere, and using it copies its bits.

use crate::diag::Span;
use crate::types::Type;

pub type FuncId = usize;
pub type LocalId = usize;
pub type StructId = usize;

pub struct Program {
    pub funcs: Vec<Func>,
    pub structs: Vec<StructDef>,
    pub main: FuncId,
    /// Uses a borrow, a pointer or another step 3 node: the type checker
    /// accepts it, but the backends cannot build it yet.
    pub uses_borrows: bool,
}

/// A struct's layout, C-style: fields in declaration order, each at the next
/// multiple of its alignment; the size is rounded up to the largest alignment.
pub struct StructDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub size: u32,
    pub align: u32,
    /// Holds a `string` somewhere inside, so its values are owned.
    pub owns: bool,
    /// Copies automatically: not `stays`, no `drop()`, every field Copy, and
    /// at most 32 bytes.
    pub copy: bool,
    /// Declared `stays`: never copied automatically.
    pub stays: bool,
    /// The `drop()` method, if there is one.
    pub has_drop: Option<FuncId>,
    /// Indexes of the borrow fields named by `lives`.
    pub lives: Vec<usize>,
}

pub struct FieldDef {
    pub name: String,
    pub ty: Type,
    pub offset: u32,
    /// Declared `mut`.
    pub mutable: bool,
}

impl Program {
    /// Size and alignment of a value of type `t` in memory, in bytes.
    pub fn size_align(&self, t: Type) -> (u32, u32) {
        if t == Type::Str {
            return (STR_SIZE, 8);
        }
        scalar_size(t).map_or_else(
            || {
                let s = &self.structs[t.struct_id().expect("a value type")];
                (s.size, s.align)
            },
            |n| (n, n),
        )
    }

    /// Is `t` an owning type (see the module comment)?
    pub fn owns(&self, t: Type) -> bool {
        match t {
            Type::Str => true,
            Type::Struct { id, .. } => self.structs[id].owns,
            _ => false,
        }
    }

    pub fn struct_def(&self, t: Type) -> &StructDef {
        &self.structs[t.struct_id().expect("a struct type")]
    }
}

/// Size of a `string` value (`drachm_str`: pointer, length, capacity).
pub const STR_SIZE: u32 = 24;

/// Size (which is also the alignment) of a number or bool; `None` for the rest.
pub fn scalar_size(t: Type) -> Option<u32> {
    // A unit is erased before any backend, so only the number's size matters.
    match t.bare() {
        Type::Int { bits, .. } | Type::Float(bits) => Some(bits as u32 / 8),
        Type::Bool => Some(1),
        Type::Ref { .. } | Type::Ptr { .. } => Some(8),
        _ => None,
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
    pub name_span: Span,
    /// From the return type to the closing `)`.
    pub span: Span,
    /// The closing `}` of the body.
    pub body_end: Span,
    pub is_unsafe: bool,
    /// Declared `lives`: indexes of the parameters named.
    pub lives: Option<Vec<usize>>,
}

pub struct Local {
    pub name: String,
    pub ty: Type,
    pub span: Span,
    pub mutability: Mutability,
    pub is_param: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutability {
    Fixed,
    Mut,
    Open,
}

/// Names an `If` or a `Loop` for the jumps that target it. Unique within a function.
pub type Label = usize;

/// Something that can be assigned to: a local, then a path of fields and
/// dereferences inside it, outermost first.
#[derive(Clone)]
pub struct Place {
    pub local: LocalId,
    pub proj: Vec<HirProj>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HirProj {
    Field(usize),
    Deref,
}

impl Place {
    /// The whole local.
    pub fn local(local: LocalId) -> Place {
        Place { local, proj: Vec::new(), span: Span::new(0, 0) }
    }

    /// The field indexes along the path; the backends (which see no `Deref`) use this.
    pub fn fields(&self) -> impl Iterator<Item = usize> + '_ {
        self.proj.iter().map(|p| match p {
            HirProj::Field(i) => *i,
            HirProj::Deref => unreachable!("dereferences cannot be built yet"),
        })
    }
}

pub type Block = Vec<Stmt>;

pub enum Stmt {
    /// Store a value in a place; the value the place held, if any, is dropped
    /// by the MIR builder.
    Assign(Place, Expr),
    /// Tests the branches in order and runs the body of the first whose
    /// test holds (a branch with no test always does), then carries on
    /// after the whole statement. `if` chains and `match` both become this.
    If { label: Label, branches: Vec<Branch> },
    /// Each time round: if there is a `cond` and it is false, the loop ends;
    /// otherwise `body` runs, then `step`. `continue` jumps to `step`.
    Loop { label: Label, cond: Option<Expr>, body: Block, step: Block, end: Span },
    Jump(Jump),
    Return(Option<Expr>),
    Expr(Expr),
    /// A user declaration of a local, before its initializer's `Assign`.
    /// The backends ignore it.
    Decl(LocalId),
    /// `unsafe { }`: runs its block.
    Unsafe(Block),
    /// `destroy x;`: runs `drop()` now and ends the local.
    Destroy(Place),
}

pub struct Branch {
    pub test: Option<Expr>,
    pub body: Block,
    /// The closing `}` of the body.
    pub end: Span,
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
    pub span: Span,
}

impl Expr {
    /// An expression with a dummy span.
    pub fn new(kind: ExprKind, ty: Type) -> Expr {
        Expr { kind, ty, span: Span::new(0, 0) }
    }

    pub fn at(mut self, span: Span) -> Expr {
        self.span = span;
        self
    }

    /// A local, a dereference, or a field of one: it can be referred to
    /// where it is (for `self` arguments and fields) instead of copying the
    /// value.
    pub fn is_place(&self) -> bool {
        match &self.kind {
            ExprKind::Local(_) | ExprKind::Deref(_) => true,
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
    /// A string literal: a not-owned view of read-only bytes.
    Str(String),
    /// A local. For an owning type, a place (borrowed where it is used).
    Local(LocalId),
    /// Field `n` of a struct value. For an owning struct the inside is a
    /// place, and so is the field.
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
    /// Arguments in order; an owning one is handed to the callee. For a
    /// function with `has_self`, the first argument is the receiver, which the
    /// callee works on in place.
    Call(FuncId, Vec<Expr>),
    /// `++`/`--` on a number. The value is the old one for postfix, the new
    /// one for prefix.
    IncDec { place: Place, inc: bool, prefix: bool },
    /// `value::text()` of a number or bool: an owned `string`.
    Text(Box<Expr>),
    /// An integer as text in base 2, 8 or 16 (`io::outf`'s `%x` and friends):
    /// an owned `string`.
    Radix { value: Box<Expr>, base: u32, upper: bool },
    /// A string runtime function. Arguments have the types `StrFn::params`
    /// gives; `string` ones are places.
    StrCall(StrFn, Vec<Expr>),
    /// The texts of several `string` places one after another: an owned value
    /// (interpolation and `io::outf`).
    Join(Vec<Expr>),
    /// `io::out` / `io::outln` of a `string` place.
    Out { newline: bool, arg: Box<Expr> },
    /// `io::inln()`: the next line of standard input, an owned `string`.
    InLine,
    /// `io::ended()`: true when standard input has nothing left.
    InEnded,
    /// `text::to<T>()`: the number the text reads as (a number type `to`).
    ParseText { text: Box<Expr>, to: Type },
    /// `&place` / `&mut place` (the place may also be a temporary value).
    Borrow { mutable: bool, place: Box<Expr> },
    /// Reads through a borrow or pointer; also a place.
    Deref(Box<Expr>),
    /// User `copy e`: a deep copy, an owned value.
    DeepCopy(Box<Expr>),
    /// `take place [leaving v]`.
    Take { place: Place, leaving: Option<Box<Expr>> },
    /// `lend r`: passes a `&mut` on and keeps it.
    Lend(Box<Expr>),
    /// `address of e`
    AddressOf(Box<Expr>),
    /// `none` as a pointer.
    NullPtr,
}

/// Runtime string functions with a fixed signature (`runtime/strings.h`, and
/// the `drachm_text_*` helpers in `runtime/runtime.c`). A `string` result
/// comes back through an out pointer in the C signature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrFn {
    Concat,
    /// Appends the second argument to the first, a `string` local or field.
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
