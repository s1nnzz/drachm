//! The MIR: a control-flow graph of basic blocks per function, built from the
//! HIR. See `docs/impl/step3-ownership.md` section 2 and `docs/impl/wp1.md`.
//!
//! Phase A of WP1: the builder translates the current HIR (including the
//! legacy ownership nodes) faithfully. Nothing but tests and
//! `drachm check --dump-mir` uses the MIR yet.

pub mod build;
pub mod index;
pub mod print;
pub mod traversal;
pub mod ty;
pub mod validate;

use crate::diag::{Diagnostic, Span};
use crate::hir::{self, FuncId, StructId};
use crate::types::Type;

pub use index::{Idx, IndexVec};
use index::newtype_index;

newtype_index!(
    /// `_0` is the return place.
    Local
);
newtype_index!(
    /// `bb0` is the entry block.
    BlockId
);
newtype_index!(
    /// Index into `Body::scopes`.
    ScopeId
);
newtype_index!(
    /// Index into `Program::strings`.
    StrId
);

impl std::fmt::Display for Local {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "_{}", self.0)
    }
}

impl std::fmt::Display for BlockId {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "bb{}", self.0)
    }
}

/// Placeholder until WP3 infers how a method borrows `self`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelfMode {
    Shared,
    Mut,
}

/// Placeholder until WP8 (lifetimes in signatures).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivesSig {
    Unknown,
}

#[derive(Debug)]
pub struct Program {
    /// By `FuncId`.
    pub bodies: Vec<Option<Body>>,
    pub main: FuncId,
    /// Literal pool, shared by both backends.
    pub strings: IndexVec<StrId, String>,
    /// By `FuncId`; `None` for non-methods.
    pub self_modes: Vec<Option<SelfMode>>,
    /// By `FuncId`.
    pub lives: Vec<LivesSig>,
}

#[derive(Debug)]
pub struct Body {
    pub func: FuncId,
    pub name: String,
    /// Signature span, for E0105.
    pub span: Span,
    /// Locals `1..=arg_count` are the parameters, in order.
    pub arg_count: usize,
    pub locals: IndexVec<Local, LocalDecl>,
    pub blocks: IndexVec<BlockId, BasicBlock>,
    /// Lexical scopes, for diagnostics only.
    pub scopes: IndexVec<ScopeId, Scope>,
    /// The file allows `unsafe`.
    pub unsafe_flag: bool,
    /// Declared `unsafe`.
    pub unsafe_fn: bool,
    /// The block the end of the function body flows into, when that is
    /// reachable and the function returns a value (it ends in `Unreachable`;
    /// WP2 reports E0105 from it).
    pub fallthrough_end: Option<BlockId>,
}

#[derive(Debug)]
pub struct LocalDecl {
    pub ty: Type,
    /// PHASE A STAND-IN for `Type::Ref`, which does not exist yet: when
    /// `Some(mutable)` the local is a `&ty` (`&mut ty`), not a `ty`. Phase B
    /// deletes this field and uses the real reference types.
    pub borrow: Option<bool>,
    pub kind: LocalKind,
    /// Declaration (user locals), or the expression (temps).
    pub span: Span,
    pub scope: ScopeId,
    /// Computed after building (WP4): the local must live in memory.
    pub in_memory: bool,
}

#[derive(Debug)]
pub enum LocalKind {
    /// `_0`; type `none` when the function returns nothing.
    Return,
    /// `self` is a `Param` named "self".
    Param { name: String, mutability: Mutability },
    User { name: String, mutability: Mutability },
    Temp,
    /// Bool, added by drop elaboration.
    DropFlag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mutability {
    Fixed,
    Mut,
    Open,
}

#[derive(Debug)]
pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,
}

#[derive(Debug)]
pub struct Statement {
    pub kind: StatementKind,
    pub span: Span,
    pub unsafe_ctx: bool,
}

#[derive(Debug)]
pub enum StatementKind {
    /// `place = rvalue`. Move, copy and borrow are forms of Assign.
    Assign(Place, Rvalue),
    StorageLive(Local),
    StorageDead(Local),
    /// Destroy the value at `place` if it needs it. `flag` is only set by
    /// drop elaboration.
    Drop { place: Place, flag: Option<Local>, reason: DropReason },
    /// Read of a place with no other effect.
    FakeRead(Place),
    Nop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropReason {
    /// Ends a scope (checked like the owner dying).
    ScopeEnd,
    /// Precedes an assignment (checked like a write).
    Assign,
}

#[derive(Debug)]
pub struct Terminator {
    pub kind: TerminatorKind,
    pub span: Span,
    pub unsafe_ctx: bool,
}

#[derive(Debug)]
pub enum TerminatorKind {
    Goto(BlockId),
    If { cond: Operand, then: BlockId, els: BlockId },
    /// Defined, printed and validated, but the builder never emits it.
    Switch { discr: Operand, cases: Vec<(u128, BlockId)>, otherwise: BlockId },
    /// `dest` is `None` for calls returning `none`.
    Call { callee: Callee, args: Vec<Arg>, dest: Option<Place>, target: BlockId },
    /// Return `_0` (nothing for `none`).
    Return,
    Panic(PanicKind),
    Unreachable,
}

#[derive(Debug)]
pub struct Arg {
    pub op: Operand,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Callee {
    /// User function or method (`self` is `args[0]`).
    Func(FuncId),
    /// Runtime string function; string operands are `&string` temps.
    Str(hir::StrFn),
    /// `io::out` / `io::outln` of a `&string`.
    Out { newline: bool },
    /// Number or bool to an owned string.
    Text,
    Radix { base: u32, upper: bool },
    /// Any number of `&string`, owned result.
    Join,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanicKind {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub local: Local,
    pub proj: Projs,
}

pub type Projs = Vec<Proj>;

impl Place {
    pub fn local(local: Local) -> Place {
        Place { local, proj: Vec::new() }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Proj {
    /// On a struct-typed prefix.
    Field(usize),
    /// On a reference (`&T`, `&mut T`) or `*T`.
    Deref,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Copy(Place),
    Move(Place),
    Const(Constant),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Constant {
    /// Bits zero-extended, as `hir::ExprKind::Int`.
    Int(u128, Type),
    Float(f64, Type),
    Bool(bool),
    /// A non-owning `string` value.
    Str(StrId),
}

#[derive(Debug)]
pub enum Rvalue {
    Use(Operand),
    Neg(Operand),
    Not(Operand),
    BitNot(Operand),
    Binary(hir::BinOp, Operand, Operand),
    Shift { left: bool, value: Operand, amount: Operand },
    IntPow(Operand, Operand),
    /// Numeric only.
    Convert(Operand, Type),
    Ref(BorrowKind, Place),
    /// `address of place`: makes a `*T`.
    RawRef(Place),
    /// A struct built from operands in field (declaration) order.
    Aggregate(StructId, Vec<Operand>),
    /// `copy place` on a non-Copy type: a fresh owned deep copy.
    DeepCopy(Place),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BorrowKind {
    Shared,
    /// `two_phase`: receiver borrows only.
    Mut { two_phase: bool },
}

#[derive(Debug)]
pub struct Scope {
    pub parent: Option<ScopeId>,
    pub span: Span,
    pub kind: ScopeKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    Function,
    Block,
    LoopBody,
    IfBranch,
    MatchCase,
    Unsafe,
}

/// A program point: statement `index` of `block`; `index == statements.len()`
/// is the terminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Location {
    pub block: BlockId,
    pub index: u32,
}

/// Build the MIR of every function. Never reports type errors (typeck has
/// already); `diags` is for the ownership errors phase B adds.
pub fn build_program(hir: &hir::Program, diags: &mut Vec<Diagnostic>) -> Program {
    let p = build::program(hir, diags);
    #[cfg(debug_assertions)]
    if let Err(errors) = validate::validate(&p, hir) {
        panic!("the MIR builder produced invalid MIR:\n{}", errors.join("\n"));
    }
    p
}

/// The text format of Appendix B.
pub fn print_program(p: &Program, hir: &hir::Program) -> String {
    print::program(p, hir)
}

/// Check the invariants V1-V9 that apply at this stage. Every message is one
/// violation.
pub fn validate(p: &Program, hir: &hir::Program) -> Result<(), Vec<String>> {
    validate::validate(p, hir)
}

/// Is a value of `ty` copied (as opposed to moved)? Phase A: numbers, bool and
/// `none`; a struct if every field is Copy and its size is at most 32 bytes.
pub fn is_copy(hir: &hir::Program, ty: Type) -> bool {
    match ty {
        Type::Str => false,
        Type::Struct { id, .. } => {
            let s = &hir.structs[id];
            s.size <= 32 && s.fields.iter().all(|f| is_copy(hir, f.ty))
        }
        _ => true,
    }
}

/// Does destroying a value of `ty` do anything? Phase A: it holds a string.
pub fn needs_drop(hir: &hir::Program, ty: Type) -> bool {
    match ty {
        Type::Str => true,
        Type::Struct { id, .. } => hir.structs[id].fields.iter().any(|f| needs_drop(hir, f.ty)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn struct_ty(hir: &hir::Program, name: &str) -> Type {
        let id = hir.structs.iter().position(|s| s.name == name).unwrap();
        Type::Struct { id, name: crate::types::intern(name) }
    }

    #[test]
    fn copy_depends_on_fields_and_size() {
        let src = "
struct Small { public: uint64 a; uint64 b; uint32 c; }
struct Big { public: uint64 a; uint64 b; uint64 c; uint64 d; uint8 e; }
struct Named { public: string s; }
struct Edge { public: sint8 tag; uint128 big; }
none main() { }
";
        let (hir, diags) = crate::check(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let small = struct_ty(&hir, "Small");
        assert_eq!(hir.struct_def(small).size, 24);
        assert!(is_copy(&hir, small));
        let big = struct_ty(&hir, "Big");
        assert_eq!(hir.struct_def(big).size, 40);
        assert!(!is_copy(&hir, big));
        assert!(!is_copy(&hir, struct_ty(&hir, "Named")));
        let edge = struct_ty(&hir, "Edge");
        assert_eq!(hir.struct_def(edge).size, 32);
        assert!(is_copy(&hir, edge), "32 bytes is still Copy");

        assert!(is_copy(&hir, Type::U32) && is_copy(&hir, Type::Bool) && is_copy(&hir, Type::None));
        assert!(!is_copy(&hir, Type::Str));
        assert!(needs_drop(&hir, Type::Str) && needs_drop(&hir, struct_ty(&hir, "Named")));
        assert!(!needs_drop(&hir, big) && !needs_drop(&hir, Type::U32));
    }
}
