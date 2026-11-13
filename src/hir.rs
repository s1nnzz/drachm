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
pub type EnumId = usize;

pub struct Program {
    pub funcs: Vec<Func>,
    pub structs: Vec<StructDef>,
    pub enums: Vec<EnumDef>,
    pub main: FuncId,
    /// Every list and array type the program uses (elements included), each
    /// once, an element before the types built from it. The backends make the
    /// drop and copy glue and the C array types from this list.
    pub collections: Vec<Type>,
    /// Struct methods that implement an interface method which does not say
    /// `changes self`: they must not change `self` (checked after `self_mode`).
    pub conformances: Vec<Conformance>,
    /// By interface index (the `id` of `Type::Dyn`): what a `&Shape` can call and read.
    pub dyn_ifaces: Vec<DynIface>,
    /// Every pair (implementing type, interface) a borrow is converted at: one
    /// read-only table each (S6E). The backends emit them all.
    /// Environments of lifted functions which capture outer locals.
    pub closures: std::collections::BTreeMap<FuncId, Closure>,
    pub vtables: Vec<VTable>,
    /// Instances of generic functions and methods whose type arguments hold
    /// borrows: per function, groups of region slots that are one region
    /// because they come from the same type parameter. A slot is
    /// (parameter, slot within it), the parameter `None` for the result.
    pub ties: std::collections::BTreeMap<FuncId, Vec<Vec<(Option<usize>, usize)>>>,
}

pub struct Closure {
    pub env: Type,
    /// Indexed by field in the environment struct, in lexical declaration order.
    pub captures: Vec<Capture>,
}

pub struct Capture {
    /// The lifted body's local id for this captured binding.
    pub local: LocalId,
    /// Borrowing captures are shared until mutation inference proves otherwise.
    /// `None` is an owned field in a `move` closure.
    pub borrow: Option<bool>,
}

/// An interface as the backends see it: the slots of a vtable.
#[derive(Debug, Clone)]
pub struct DynIface {
    pub name: String,
    /// The interfaces it extends, all the way up (not itself): a vtable holds
    /// the table of the same type for each, so a `&Shape3D` becomes a `&Shape`.
    pub ancestors: Vec<usize>,
    /// Every non-`static` method, parents first.
    pub methods: Vec<DynMethod>,
    /// Every required field, parents first.
    pub fields: Vec<DynField>,
}

impl DynIface {
    /// A vtable is `[size, align, one table per ancestor, one code address per
    /// method, one offset per field]`, each a word. The slot of ancestor `a`:
    pub fn ancestor_slot(&self, a: usize) -> Option<usize> {
        self.ancestors.iter().position(|&x| x == a).map(|i| 2 + i)
    }

    /// The slot of method `m`.
    pub fn method_slot(&self, m: usize) -> usize {
        2 + self.ancestors.len() + m
    }

    /// The slot of the offset of field `f`.
    pub fn field_slot(&self, f: usize) -> usize {
        2 + self.ancestors.len() + self.methods.len() + f
    }
}

#[derive(Debug, Clone)]
pub struct DynMethod {
    pub name: String,
    /// The parameters after `self`.
    pub params: Vec<Type>,
    pub ret: Type,
    /// Says `changes self`: the receiver is a `&mut`.
    pub changes: bool,
    /// Can be called through a `&Iface`: no `Self` and no borrow in the result.
    pub callable: bool,
}

#[derive(Debug, Clone)]
pub struct DynField {
    pub name: String,
    pub ty: Type,
}

/// The table for `ty` seen as interface `iface` (layout: `DynIface::ancestor_slot`).
#[derive(Debug, Clone)]
pub struct VTable {
    pub ty: Type,
    pub iface: usize,
    /// By `DynIface::methods`; `None` for one that cannot be called through a borrow.
    pub methods: Vec<Option<FuncId>>,
    /// By `DynIface::fields`: the offset of the field in `ty`.
    pub fields: Vec<u32>,
}

impl Program {
    /// `Type::Dyn` for interface `iface`.
    pub fn dyn_type(&self, iface: usize) -> Type {
        Type::Dyn { id: iface, name: crate::types::intern(&self.dyn_ifaces[iface].name) }
    }

    /// The index of the vtable of `ty` for `iface`.
    pub fn vtable_index(&self, ty: Type, iface: usize) -> usize {
        self.vtables.iter().position(|v| v.ty == ty && v.iface == iface).expect("a vtable for every conversion")
    }
}

/// One struct method standing for an interface method that promises not to change `self`.
#[derive(Debug, Clone)]
pub struct Conformance {
    pub func: FuncId,
    /// `Circle::grow`
    pub method: String,
    /// `Shape::grow`
    pub promised: String,
    /// The interface method's line, unless the interface is built in.
    pub promised_at: Option<Span>,
    /// Where `grow` is declared in the interface (for the help text).
    pub signature: String,
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
    /// The `key()` method, if there is one: what a map hashes this struct by.
    pub key_fn: Option<FuncId>,
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

/// An enum: a tag, then the fields of the variant the tag names. The tag is
/// at offset 0; every variant's fields are laid out C-style from the same
/// payload start (`enum_layout`), and each field's `offset` counts from the
/// start of the enum.
pub struct EnumDef {
    pub name: String,
    /// In declaration order; a variant's tag is its index.
    pub variants: Vec<VariantDef>,
    pub kind: EnumKind,
    /// Bytes of the tag: 1, 2 or 4, the smallest that holds every variant.
    pub tag_size: u32,
    pub size: u32,
    pub align: u32,
    /// Some payload owns memory.
    pub owns: bool,
    /// Copies automatically: every payload is Copy and it is at most 32 bytes.
    pub copy: bool,
    pub name_span: Span,
}

pub struct VariantDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
    pub name_span: Span,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EnumKind {
    /// Declared with `enum`.
    User,
    /// `T?` (S5A.2): variants `none` (tag 0, no field) and `Some` (tag 1, one field of type `T`).
    Optional(Type),
    /// Declared with `failure`. Field 0 of every variant is the hidden cause
    /// word (`CAUSE_FIELD`), so it sits at the same offset in all of them.
    Failure,
    /// What a function that `fails` returns: `Ok(value)` (tag 0, no field when
    /// the function returns `none`) or `Fail(failure)` (tag 1).
    Outcome { ok: Type, fail: Type },
    /// Which failure type a function with several ended with: one variant for
    /// each, in the order of the `fails` list.
    FailSet,
}

/// The hidden first field of every variant of a failure: null, or the
/// chain of `because` messages (`drachm_cause_*` in the runtime).
pub const CAUSE_FIELD: &str = "drachm_cause";

impl FieldDef {
    pub fn is_cause(&self) -> bool {
        self.name == CAUSE_FIELD
    }
}

/// Bytes of the tag of an enum with `variants` variants.
pub fn tag_size_for(variants: usize) -> u32 {
    match variants {
        0..=256 => 1,
        257..=65536 => 2,
        _ => 4,
    }
}

/// The layout of an enum whose variants have fields of the given
/// (size, alignment): each variant's field offsets from the start of the
/// enum, then the enum's size and alignment. The tag comes first; the
/// payload starts at the next multiple of the largest field alignment.
pub fn enum_layout(tag_size: u32, variants: &[Vec<(u32, u32)>]) -> (Vec<Vec<u32>>, u32, u32) {
    let payload_align = variants.iter().flatten().map(|&(_, a)| a).max().unwrap_or(1);
    let start = tag_size.next_multiple_of(payload_align);
    let mut offsets = Vec::new();
    let mut payload = 0;
    for fields in variants {
        let (offs, size, _) = layout(fields);
        payload = payload.max(size);
        offsets.push(offs.into_iter().map(|o| start + o).collect());
    }
    let align = tag_size.max(payload_align);
    (offsets, (start + payload).next_multiple_of(align), align)
}

impl Program {
    /// Size and alignment of a value of type `t` in memory, in bytes.
    pub fn size_align(&self, t: Type) -> (u32, u32) {
        if t.is_fat() || t.is_func() {
            return (16, 8);
        }
        if t == Type::Str || t.is_list() {
            return (STR_SIZE, 8);
        }
        if t.is_map() {
            return (MAP_SIZE, 8);
        }
        if let Type::Array { elem, len } = t {
            let (size, align) = self.size_align(*elem);
            return ((size as u64 * len) as u32, align);
        }
        scalar_size(t).map_or_else(
            || match t {
                Type::Enum { id, .. } => (self.enums[id].size, self.enums[id].align),
                _ => {
                    let s = &self.structs[t.struct_id().expect("a value type")];
                    (s.size, s.align)
                }
            },
            |n| (n, n),
        )
    }

    pub fn enum_def(&self, t: Type) -> &EnumDef {
        &self.enums[t.enum_id().expect("an enum type")]
    }

    /// The type of enum `id`.
    pub fn enum_type(&self, id: EnumId) -> Type {
        Type::Enum { id, name: crate::types::intern(&self.enums[id].name) }
    }

    /// Is `t` an owning type (see the module comment)?
    pub fn owns(&self, t: Type) -> bool {
        match t {
            Type::Str | Type::List { .. } | Type::Map { .. } => true,
            Type::Array { elem, .. } => self.owns(*elem),
            Type::Struct { id, .. } => self.structs[id].owns,
            Type::Enum { id, .. } => self.enums[id].owns,
            _ => false,
        }
    }

    pub fn struct_def(&self, t: Type) -> &StructDef {
        &self.structs[t.struct_id().expect("a struct type")]
    }
}

/// Size of a `string` value (`drachm_str`: pointer, length, capacity).
pub const STR_SIZE: u32 = 24;
/// Size of a map in memory: the runtime's `drachm_map` (48 bytes, `runtime/lists.h`).
pub const MAP_SIZE: u32 = 48;

/// Size (which is also the alignment) of a number or bool; `None` for the rest.
pub fn scalar_size(t: Type) -> Option<u32> {
    // A unit is erased before any backend, so only the number's size matters.
    match t.bare() {
        Type::Int { bits, .. } | Type::Float(bits) => Some(bits as u32 / 8),
        Type::Bool => Some(1),
        fat if fat.is_fat() => None,
        Type::Ref { .. } | Type::Ptr { .. } | Type::FixedPtr { .. } => Some(8),
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
    /// The written `lives` clause, from the word to the end of the list.
    pub lives_span: Option<Span>,
}

pub struct Local {
    pub name: String,
    pub ty: Type,
    pub span: Span,
    pub mutability: Mutability,
    pub is_param: bool,
    /// The type as the user wrote it (units included), for help text: units
    /// are erased before MIR, so `ty` alone cannot spell them.
    pub written: Option<String>,
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

#[derive(Clone)]
pub enum HirProj {
    Field(usize),
    Deref,
    /// The payload of variant `n` of an enum (always followed by a `Field`).
    Downcast(usize),
    /// Element `i` of a list or array (`i` is any integer, already checked
    /// for being one; a negative one counts from the end).
    Index(Box<Expr>),
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
            HirProj::Deref | HirProj::Downcast(_) | HirProj::Index(_) => unreachable!("dereferences cannot be built yet"),
        })
    }
}

pub type Block = Vec<Stmt>;

#[derive(Clone)]
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
    /// Ends the value at the place like a move whose result nobody keeps.
    /// Made by `match` after `take` patterns have moved the parts of an enum
    /// out: nothing is dropped for the value any more.
    Forget(Place),
    /// A check of a value that is being filled or built (see `CheckKind`).
    Check { local: LocalId, kind: CheckKind, span: Span },
    /// `local.field = e` inside a `fill` block or a constructor: gives the
    /// field its value (an initialisation, not a change).
    InitField(Place, Expr),
    /// `maybe v;` in a failure handler: the value of the `Handled` expression
    /// with this label (none for a call that returns nothing); leaves the handler.
    Resume(Label, Option<Expr>),
    /// `fail e;` with `e` a failure the function caught: returns it (moved out
    /// of the place) as this function's failure.
    Reraise(Place),
}

/// One arm of a `Handled` call.
#[derive(Clone)]
pub struct HandlerArm {
    /// For a call that can fail with several types: the index of the failure
    /// type this arm takes. `None` takes whatever no arm before it took, with
    /// the whole failure (the failure itself when the call has one type).
    pub member: Option<usize>,
    /// The local the failure is moved into; `body` starts with its `Decl`.
    pub bind: LocalId,
    pub body: Block,
    /// The closing `}` of the body.
    pub end: Span,
}

/// Where a value that is being given its fields is checked (S5B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    /// The start of a `fill`: the value must have no value at all yet
    /// (`E0506`), or be partly filled already.
    FillBegin,
    /// The end of a `fill` block: every field must have a value (`E0505`).
    FillEnd,
    /// The end of a constructor, before `self` is returned: every field must
    /// have a value (`E0504`).
    CtorEnd,
}

#[derive(Clone)]
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

#[derive(Clone)]
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
            ExprKind::Field(inner, _) | ExprKind::Payload { value: inner, .. } | ExprKind::Index(inner, _) => inner.is_place(),
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

#[derive(Clone)]
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
    /// An enum value of the given variant, built from its payload values
    /// (one for each field, in order).
    Variant { enum_id: EnumId, variant: usize, args: Vec<Expr> },
    /// The tag of an enum value (an unsigned integer of the tag's size). The
    /// operand is read where it is, never moved.
    Discriminant(Box<Expr>),
    /// Field `field` of variant `variant` of an enum value that is known to
    /// be that variant (the test for it came first). A place when `value` is.
    Payload { value: Box<Expr>, variant: usize, field: usize },
    /// `call maybe fallback`, `call` being a call of a function that fails
    /// (its type is the outcome): the call's value, or the fallback when it
    /// failed. The fallback runs only on failure. Typed with the value's type.
    FailMaybe { call: Box<Expr>, fallback: Box<Expr> },
    /// `call fails`: the call's value, or a return of its failure from the
    /// caller. Typed with the value's type.
    PassUp { call: Box<Expr> },
    /// `text::to<T>()`: reading the text as a number of type `to`, which can
    /// fail with `fail` (`ConvertError`). Only ever the call of a `Maybe` or
    /// a `PassUp`; its own type is `to`.
    TryParse { text: Box<Expr>, to: Type, fail: Type },
    /// A new cause for a failure: a link carrying a copy of the message `msg`
    /// (a `string` place) in front of the chain whose pointer is in the
    /// `uint64` place `slot`, which is emptied. A `uint64` pointer word.
    CauseWrap { msg: Box<Expr>, slot: Box<Expr> },
    /// Prints the report of an escaping failure to standard error (`error:`,
    /// the message `msg`, then the chain in the `uint64` place `slot`, which
    /// is emptied and freed).
    /// With `exit`, the program ends with that `uint8` as its exit code once
    /// the report is printed.
    Report { msg: Box<Expr>, slot: Box<Expr>, exit: Option<Box<Expr>> },
    /// `p::freeze()` / `p::thaw()`: the same pointer bits under the other
    /// pointer kind (`*T` to `*fixed T` or back). Needs `unsafe`.
    PtrConv(Box<Expr>),
    /// A named function adapted to the function-value ABI.
    FuncRef(FuncId),
    /// A lifted function with its captures as places in the enclosing function.
    Closure(FuncId, Vec<Expr>),
    /// Indirect call with the callable value at index zero.
    FnCall(Vec<Expr>),
    /// A `&S` / `&mut S` made a `&Iface` / `&mut Iface` (the expression's type):
    /// the borrow and the vtable of `S` for the interface.
    Unsize(Box<Expr>),
    /// A method through a borrow of an interface: (interface, method index
    /// in `DynIface::methods`, arguments). The first is the receiver, a
    /// place of type `Type::Dyn`, borrowed `&mut` when the method `changes self`.
    DynCall(usize, usize, Vec<Expr>),
    /// `base[index]`: element `index` of a list or array, a place when `base`
    /// is. `index` is an integer of any width up to 64 bits; a negative one
    /// counts from the end. Out of range stops the program.
    Index(Box<Expr>, Box<Expr>),
    /// The number of items of a list or array place, a `uint64`.
    Len(Box<Expr>),
    /// `[a, b, c]` of list type: a fresh list holding the values.
    List(Vec<Expr>),
    /// `[a, b, c]` of array type: the values side by side.
    Array(Vec<Expr>),
    /// `list::push(value)`: `list` is a changeable list place; `value` is moved in.
    ListPush { list: Box<Expr>, value: Box<Expr> },
    /// `{ k: v, ... }` of map type: a fresh map, the pairs inserted in order.
    Map(Vec<(Expr, Expr)>),
    /// A map method (`MapOp`): `args[0]` is the map, the rest its arguments.
    MapCall { op: MapOp, args: Vec<Expr> },
    /// A list method with a runtime call (`ListOp`): `args[0]` is the list
    /// place, the rest are the method's arguments.
    ListCall { op: ListOp, args: Vec<Expr> },
    /// `value maybe fallback` (S5A.2): the payload of the optional `value`
    /// (of tag `Some`), or `fallback` when it is `none`. `value` is a place
    /// (read where it is; only for Copy optionals) or an owned temporary.
    Maybe { value: Box<Expr>, fallback: Box<Expr> },
    /// `call fails with (...) => { }`: the call's value, or what the arm for
    /// its failure gives with `Resume(label, v)`. `call` is typed with its outcome.
    Handled { call: Box<Expr>, arms: Vec<HandlerArm>, label: Label },
}

impl Expr {
    /// The direct sub-expressions of this one (not the statements of handlers).
    pub fn children(&self) -> Vec<&Expr> {
        use ExprKind as K;
        match &self.kind {
            K::Int(_) | K::Float(_) | K::Bool(_) | K::Str(_) | K::Local(_) | K::FuncRef(_) | K::NullPtr | K::InLine | K::InEnded | K::IncDec { .. } => Vec::new(),
            K::Field(x, _)
            | K::Neg(x)
            | K::Not(x)
            | K::BitNot(x)
            | K::Convert(x)
            | K::PtrConv(x)
            | K::Unsize(x)
            | K::Text(x)
            | K::DeepCopy(x)
            | K::Deref(x)
            | K::Lend(x)
            | K::Discriminant(x)
            | K::Payload { value: x, .. }
            | K::AddressOf(x) => vec![&**x],
            K::Variant { args, .. } => args.iter().collect(),
            K::Len(x) => vec![&**x],
            K::Index(a, b) | K::ListPush { list: a, value: b } => vec![&**a, &**b],
            K::Map(pairs) => pairs.iter().flat_map(|(k, v)| [k, v]).collect(),
            K::MapCall { args, .. } => args.iter().collect(),
            K::ListCall { args, .. } => args.iter().collect(),
            K::List(items) | K::Array(items) => items.iter().collect(),
            K::Radix { value, .. } => vec![&**value],
            K::Out { arg, .. } => vec![&**arg],
            K::TryParse { text, .. } => vec![&**text],
            K::FailMaybe { call, fallback } => vec![&**call, &**fallback],
            K::PassUp { call } | K::Handled { call, .. } => vec![&**call],
            K::CauseWrap { msg, slot } => vec![&**msg, &**slot],
            K::Report { msg, slot, exit } => vec![&**msg, &**slot].into_iter().chain(exit.as_deref()).collect(),
            K::Borrow { place, .. } => vec![&**place],
            K::Binary(_, a, b) | K::IntPow(a, b) => vec![&**a, &**b],
            K::Logic { lhs, rhs, .. } | K::Maybe { value: lhs, fallback: rhs } => vec![&**lhs, &**rhs],
            K::Shift { value, amount, .. } => vec![&**value, &**amount],
            K::Struct(fields) => fields.iter().map(|(_, v)| v).collect(),
            K::Call(_, args) | K::StrCall(_, args) | K::DynCall(_, _, args) | K::FnCall(args) | K::Closure(_, args) => args.iter().collect(),
            K::Join(parts) => parts.iter().collect(),
            K::Take { leaving, .. } => leaving.as_deref().into_iter().collect(),
        }
    }
}

/// The map methods (S6D). `Get` is `m[k]` and `m::get(k)`: a `V?`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapOp {
    /// `map[key]`: the value under `key`, or none (`V?`).
    Get,
    /// `map::len()`: the number of entries, a `uint64`.
    Len,
    /// `map::set(key, value)` and `map[key] = value`: adds or replaces.
    Set,
    /// `map::remove(key)`: the entry goes; its value is dropped.
    Remove,
    /// `map[key] += x` reads the value under `key` (a `V`, not an optional)
    /// and stops the program when there is none.
    Expect,
    /// Walking a map by a cursor (a `uint64`, 0 to start): is there an entry
    /// at or after the cursor (`bool`)?
    More,
    /// The key of that entry (`K`, a copy).
    KeyAt,
    /// The value of that entry (`V`, a copy).
    ValAt,
    /// The cursor just after that entry (`uint64`).
    After,
}

impl MapOp {
    /// Does the call change the map (it needs a changeable map place)?
    pub fn changes(self) -> bool {
        matches!(self, MapOp::Set | MapOp::Remove)
    }
}

/// The list methods that call the runtime (`runtime/lists.h`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListOp {
    /// `list::pop()`: the last item, or none (`T?`).
    Pop,
    /// `list::get(i)`: the item at `i` (negative counts from the end), or none.
    Get,
    /// `list::get(i)` of items that own memory: a view of the item, `&T?`
    /// (the optional holds a borrow of the list).
    View,
    /// `list::insert(i, value)`.
    Insert,
    /// `list::remove(i)`: the item at `i`, moved out; the rest move down.
    Remove,
    /// `take xs[i] reorders`: the item at `i` moved out; the last item fills
    /// its place, and the list is one shorter (the order is not kept).
    SwapRemove,
    /// `take xs[i] leaving v`: the item at `i` moved out, `v` (moved in) takes
    /// its place. The length and the order are kept.
    TakeLeaving,
    /// `list::clear()`: drops every item.
    Clear,
    /// `list::join(sep)` of a `string[?]`: the items with `sep` between them.
    Join,
}

impl ListOp {
    /// Does the call change the list (it needs a changeable list place)?
    pub fn changes(self) -> bool {
        matches!(self, ListOp::Pop | ListOp::Insert | ListOp::Remove | ListOp::SwapRemove | ListOp::TakeLeaving | ListOp::Clear)
    }
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
    /// The byte length of a string (the `for` loop over its characters).
    ByteLen,
    /// The character starting at a byte offset (`for`).
    NextChar,
    /// The same character as a borrowed view into the string (`for (&string c of s)`).
    CharView,
    /// Whether a string matches a pattern with `*` (`match`).
    Glob,
    /// Number text padded with zeros after the sign (`io::outf`'s `%0N`).
    ZeroPad,
    /// The pieces between the occurrences of a separator: a `string[?]`.
    Split,
    /// The lines of a text, without their line ends: a `string[?]`.
    Lines,
    /// The characters of a text, one owned string each: a `string[?]`.
    Chars,
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
            ByteLen => "drachm_str_byte_len",
            NextChar => "drachm_str_next_char",
            CharView => "drachm_str_char_view",
            Glob => "drachm_str_glob",
            Split => "drachm_str_split",
            Lines => "drachm_str_lines",
            Chars => "drachm_str_chars",
        }
    }

    pub fn params(self) -> &'static [Type] {
        use StrFn::*;
        match self {
            Len | IsEmpty | Reverse | Upper | Lower | Trim => &[S],
            Concat | Append | Eq | Cmp | Find | Contains | StartsWith | EndsWith | Count | Glob | Split => &[S, S],
            Lines | Chars => &[S],
            Sub | Remove => &[S, S64, S64],
            Insert => &[S, S64, S],
            Replace => &[S, S, S],
            ReplaceLimit => &[S, S, S, U64],
            Repeat | ZeroPad => &[S, U64],
            PadLeft | PadRight => &[S, U64, S],
            Fixed => &[Type::F64, Type::U32],
            ByteLen => &[S],
            NextChar | CharView => &[S, U64],
        }
    }

    pub fn ret(self) -> Type {
        use StrFn::*;
        match self {
            Append => Type::None,
            Eq | IsEmpty | Contains | StartsWith | EndsWith | Glob => Type::Bool,
            Cmp => Type::S32,
            Len | Count | ByteLen => U64,
            Find => S64,
            Split | Lines | Chars => Type::list(S),
            _ => S,
        }
    }
}
