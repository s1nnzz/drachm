//! The MIR: a control-flow graph of basic blocks per function, built from the
//! HIR. See `docs/impl/step3-ownership.md` section 2 and `docs/impl/wp1.md`.
//!
//! The builder lowers the HIR, the analyses in `check` look for ownership
//! errors, `drops` decides what each `Drop` really does, and the two code
//! generators read the result.

pub mod analysis;
pub mod cleanup;
pub mod build;
pub mod drops;
pub mod index;
pub mod print;
pub mod traversal;
pub mod ty;
pub mod validate;
pub mod borrowck;
pub mod check;
pub mod dataflow;
pub mod liveness;
pub mod moves;
pub mod mutability;
pub mod unsafety;

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

pub use crate::ownership::{MutKind, MutReason, SelfMode};

/// What a call site may assume about the borrow a function returns (spec 3.6.1):
/// every return region is outlived by every region of every listed parameter.
/// A parameter index counts from 0; `self` is 0 in a method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LivesSig {
    /// The return type has no region slots.
    NoBorrowResult,
    /// Written `lives a, b`.
    Declared(Vec<usize>),
    /// Computed from the body.
    Inferred(Vec<usize>),
    /// The body had errors: callers assume nothing and report nothing.
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
    /// `destroy x;`: runs `drop()` now. The local is moved afterwards, so
    /// nothing runs at scope end.
    Destroy,
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
    /// `io::inln()`: the next line of input, an owned string.
    InLine,
    /// `io::ended()`.
    InEnded,
    /// A `&string` read as a number of this type (panics on bad text).
    Parse(Type),
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

/// Is a value of `ty` copied (as opposed to moved)? Numbers, bool, `none` and
/// `&T` are; `string`, `&mut T` and `*T` are not; a struct is when
/// `StructDef.copy` says so.
pub fn is_copy(hir: &hir::Program, ty: Type) -> bool {
    match ty {
        Type::Str => false,
        Type::Struct { id, .. } => hir.structs[id].copy,
        Type::Ref { mutable, .. } => !mutable,
        Type::Ptr { .. } | Type::FixedPtr { .. } => false,
        _ => true,
    }
}

/// Does destroying a value of `ty` do anything? A string, a struct with a
/// `drop()`, or a struct holding either. Borrows and raw pointers never.
pub fn needs_drop(hir: &hir::Program, ty: Type) -> bool {
    match ty {
        Type::Str => true,
        Type::Struct { id, .. } => {
            let s = &hir.structs[id];
            s.has_drop.is_some() || s.fields.iter().any(|f| needs_drop(hir, f.ty))
        }
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
        let (hir, diags) = crate::front(src);
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

    fn built(src: &str) -> (hir::Program, Program, Vec<Diagnostic>) {
        let (hir, diags) = crate::front(src);
        let hir = hir.unwrap_or_else(|| panic!("does not check: {diags:?}"));
        let mut mir_diags = Vec::new();
        let mir = build_program(&hir, &mut mir_diags);
        (hir, mir, mir_diags)
    }

    #[test]
    fn unsafe_destroy_and_self() {
        let src = "//--!$unsafe: true
struct G { public: string s; none drop() { } }
struct P { public: uint32 n; uint32 get() { return self.n; } }
none main() {
    mut uint32 n = 1;
    *uint32 p = address of n;
    unsafe { *p = 2; }
    G g = \"a\";
    destroy g;
}
";
        let (hir, mir, diags) = built(src);
        assert!(diags.is_empty());
        let find = |name: &str| mir.bodies.iter().flatten().find(|b| b.name == name).unwrap();
        let main = find("main");
        let all: Vec<&Statement> = main.blocks.iter().flat_map(|b| &b.statements).collect();
        let writes: Vec<bool> = all
            .iter()
            .filter(|s| matches!(&s.kind, StatementKind::Assign(p, _) if p.proj == [Proj::Deref]))
            .map(|s| s.unsafe_ctx)
            .collect();
        assert_eq!(writes, [true], "only the write inside `unsafe` is in an unsafe context");
        assert!(all.iter().any(|s| matches!(s.kind, StatementKind::Drop { reason: DropReason::Destroy, .. })));
        let get = mir.bodies.iter().flatten().find(|b| b.name.ends_with("get")).unwrap();
        let self_ty = get.locals[Local(1)].ty;
        assert!(matches!(self_ty, Type::Ref { mutable: false, .. }), "a method that only reads self borrows it shared: {self_ty}");
        assert!(validate(&mir, &hir).is_ok());
    }

    #[test]
    fn field_moves_and_takes_report() {
        let src = "struct P { public: string name; }
struct F { public: string path; none drop() { } }
none main() {
    P p = \"a\";
    string a = take p.name;
    P q = \"b\";
    string b = q.name;
    F f = \"c\";
    string c = take f.path;
    string d = take f.path leaving \"x\";
}
";
        let (_, _, diags) = built(src);
        let ids: Vec<&str> = diags.iter().map(|d| d.code.id).collect();
        assert_eq!(ids, ["E0324", "E0325"]);
    }
}
