# Step 3 implementation spec: ownership and borrow checking

Status: **draft for implementation**, written 2026-11-10 against commit
`ade8668` plus the step 2 and step 4 work described in `docs/PLAN.md`.

Audience: an engineer who has read `CLAUDE.md`, `docs/SPEC.md` and
`docs/PLAN.md`, but not the design discussions. Everything needed to build
step 3 is here; where this document depends on code that was being written in
parallel when it was drafted, it says so in a **LOOK FIRST** box.

Rules that bind this document (from `CLAUDE.md`):

- The language owner decides every user-visible behaviour. Section 5 lists the
  behaviours the SPEC does not settle. Do not pick an answer yourself: ask, then
  record the answer in `docs/SPEC.md` and the design doc. Where a work package
  needs an answer that has not arrived, build the rest and make the undecided
  case report `E0901` ("not supported yet"), which is easy to relax later.
- Every diagnostic message in section 4 is a **draft** for the owner to
  approve. Use the texts as written until they are approved or changed.
- Engineering choices (data structures, algorithms) are made here; the ones
  users can see are flagged with **(visible)**.

Contents

1. Goals, non-goals, rules being implemented
2. The MIR
3. The analyses
4. Diagnostics
5. Open language questions
6. Work packages
7. Performance notes

Appendix A: LOOK FIRST checklist for the parallel branches.
Appendix B: MIR text format.

---

## 0. Code that must be inspected before starting

When this was written, three branches were being merged that this step builds
on. Their code could not be read. The spec is written against the HIR at
`ade8668` plus those features as described in SPEC and PLAN. Before starting
any work package, read Appendix A and fill in the table there (a ten-minute
job); each work package also has its own LOOK FIRST box.

- **(a) strings** (PLAN step 2 items 1-4, 6): real `string` values
  (`{ptr, len, cap}`, `runtime/strings.h`), explicit copy and drop operations
  in the HIR, and **deep copy on every assignment or pass** as a stand-in for
  moves.
- **(b) structs** (PLAN step 2 item 5): fields, `mut` fields, methods with
  implicit `self`, `static` methods, operator methods, the construction
  shorthand, C layout, large structs passed by pointer.
- **(c) control flow** (PLAN step 4): `for` (C-style and `of`), `break N`,
  `continue N`, `skip`/`skip N`/`skip to next`, `match`.

---

## 1. Goals, non-goals, and the rules being implemented

### 1.1 Goals

At the end of step 3:

1. The compiler has a **MIR**: a control-flow graph per function, with explicit
   places, moves, copies, borrows, drops and storage markers. Both backends
   (Cranelift, `src/codegen.rs`; C, `src/cgen.rs`) generate code **from MIR
   only**. The HIR stays the type checker's output and the MIR builder's input.
2. These checks run on MIR, report in the SPEC section 10 style, and never stop
   at the first error:
   - definite assignment (replacing `Flow` in `typeck.rs`), the late-init rules
     for immutable bindings, and missing return;
   - move checking: use after move (E0301), including partial moves;
   - mutability: `mut`, `open`, `mut` fields, `&mut` lending, mutating methods;
   - borrow checking, non-lexical (NLL style): shared XOR mutable (E0302 and
     friends), borrows outliving their owner;
   - lifetime inference for function signatures, and checking of `lives`
     clauses on functions and structs;
   - inference of each method's `self` borrow mode;
   - `unsafe { }` blocks, `*T`, and the `//--!$unsafe: true` file flag.
3. **Drop elaboration**: every owned value is dropped exactly once, on every
   path, including partially moved structs; drop flags only where the path
   decides.
4. Step 2's deep-copy-on-assign workaround is gone; `string b = a;` is a move
   (24-byte copy, no allocation).
5. A leak and double-free test runs over every golden test, on both backends.
6. No benchmark gets slower; string-heavy code gets much faster (the copies are
   gone).

### 1.2 Non-goals (later steps)

- Generics, interfaces, lambdas, closures' captures (step 6). The region
  machinery here is built so that type parameters and closures can be added,
  but nothing is implemented for them.
- Lists and maps (step 6). The SPEC's E0302 example uses `xs[0]` and
  `xs::push`; until lists exist, the tests use strings and struct fields.
- `fails` / `maybe` (step 5). Section 2.6 says how a failing call will appear
  in MIR so step 5 does not need to change the analyses.
- User-defined destructors (not in the SPEC; see open question Q17). Dropping
  is compiler-generated only: strings free their buffer, structs drop their
  fields.
- Threads and data-race checking (later).
- Optimisations on MIR beyond the small cleanup pass in 2.8.

### 1.3 The SPEC rules this step implements

Section numbers refer to `docs/SPEC.md`.

| # | Rule | Source |
|---|---|---|
| S1 | Bindings are immutable by default. `mut` = may be reassigned **and** changed (mutating methods, `&mut` borrows, field writes). `open` = contents may change (mutating methods, `&mut`), never reassigned. `mut open` is redundant (warning). | §2 |
| S2 | `uint32 later;` may be declared without a value; reading before every path assigns it is an error. An immutable binding declared that way may be assigned exactly once, where it is certainly still unassigned (so not inside a loop it was declared outside of). | §2 |
| S3 | Parameters are immutable unless written `mut uint32 n`. | §2 |
| S4 | Strings, lists and most structs **move** on assignment or pass; the source is unusable afterwards (E0301 "used after it moved"). | §6 |
| S5 | Numbers, bools, and structs whose fields are all copyable and whose total size is at most 32 bytes **copy** automatically. | §6 |
| S6 | Explicit copy: `string c = copy b;`. | §6 |
| S7 | Borrows `&T` (shared, many) and `&mut T` (exclusive). The call site writes it: `push(&mut xs, 1)`. Lending `&mut` needs a `mut` or `open` binding. | §6, PLAN step 3 "Decided" |
| S8 | Struct fields are immutable unless marked `mut`. A `mut` field can always be changed, even on an immutable value (like C++ `mutable`). | §6 |
| S9 | Struct fields may hold borrows; the struct names what it borrows from: `struct Parser lives src { &string src; }`. | §6 |
| S10 | Structs holding raw pointers never copy automatically (explicit `copy` still works). | §6 |
| S11 | E0302's exact style (message, labels "is borrowed here" / "change attempted here" / "the borrow is still used here", `help:`). Other borrow errors are written in the same style. | §6 |
| S12 | Lifetimes are inferred. When needed, `lives` names the parameters a returned borrow comes from: `&string longest(&string a, &string b) lives a, b { }`. | §6 |
| S13 | Returning a borrow derived from exactly one borrowed parameter is inferred; ambiguous cases ask for `lives a, b`. `lives` is checked at the definition and used at call sites. | PLAN step 3 item 6 |
| S14 | `self` is never declared; its shared vs mutable borrow mode is **inferred** from the method body. A method that assigns `self.x` or calls a mutating method is `&mut self`, otherwise `&self`. Error messages say which was inferred and why. | §5, PLAN step 3 item 7 |
| S15 | Raw pointers `*T` need BOTH the file flag `//--!$unsafe: true` AND an `unsafe { }` block. `*p = 7;` dereferences. | §6 |
| S16 | File flags are `//--!$key: value` lines, allowed only at the very top of a file. | §1 |
| S17 | No garbage collector. Values are destroyed at the end of their scope, in reverse declaration order. | §6, PLAN step 2 item 6 |
| S18 | Runtime faults panic: message and exit code 101. | §7 |
| S19 | Operands are evaluated left to right. | §2 |
| S20 | Diagnostic codes `03xx` are ownership; E0301 and E0302 are fixed. Report all errors. | §10 |

### 1.4 Readings this spec relies on

These are places where the SPEC's wording or examples settle a behaviour in
the reading given here. They are not open questions, but show them to the
owner together with section 5 so a wrong reading is caught early.

- **R1 (field writes).** A non-`mut` field can be written exactly when the
  value containing it can be changed: through a `mut` binding, or through a
  `&mut` borrow. A `mut` field can be written through any place, including an
  immutable binding (S8). Writing a `mut` field *through a `&` borrow* is open
  (Q5).
- **R2 (builtins borrow).** `io::out`, `io::outln`, `io::outf`, the string
  operators (`+`, comparisons, interpolation `${}`) and every string method
  take their string operands and arguments by shared borrow, implicitly.
  `io::outln(name); io::outln(name);` is valid. ("Methods return a new
  string; the original is never modified", §4.) Q2 is about *user* functions.
- **R3 (receivers).** A method receiver is borrowed implicitly at the call:
  `xs::push(9)` and `p::heal()` write no `&`/`&mut` (SPEC §6's own example).
  The receiver binding still needs to be `mut` or `open` for a mutating method.
- **R4 (auto-deref for `.` and `::`).** Field access and method calls look
  through borrows: `self.r` where `self` is a borrow, `first::text()` where
  `first` is `&uint32` (SPEC examples). Other operations on borrows are open
  (Q1).
- **R5 (drop order).** Locals: reverse declaration order (S17). Struct fields:
  declaration order. Temporaries: at the end of the statement that made them.
  None of this is observable until user destructors exist (Q17).
- **R6 (panics).** A panic exits the process immediately; nothing is dropped.
  So there is no unwinding and no cleanup path in MIR.
- **R7 (`while (true)`).** A loop whose condition is the literal `true` (or a
  C-style `for` with no condition) only finishes through `break`. This is
  today's typeck rule, now falling out of the CFG.
- **R8 (borrow types copy).** `&T` is copyable (8 bytes, any number of shared
  borrows may exist). `&mut T` is not copyable (Q3 is about how one is passed
  on).
- **R9 (`unsafe` is narrow).** An `unsafe { }` block allows raw-pointer
  operations; it does not switch off move or borrow checking of ordinary
  places.
- **R10 (string literals).** A string literal is a value of type `string`
  that does not own its bytes (`cap == 0` in `runtime/strings.h`); dropping it
  does nothing, and a later change (methods return new strings) never writes
  to it. So moving and copying literals is free.

---

## 2. The MIR

### 2.1 Where it sits

```text
source
  │ lexer::lex                       tokens
  │ flags::parse  (new, WP0)         FileFlags (`//--!$key: value` lines)
  │ parser::parse                    ast::Program
  │ typeck::check                    hir::Program   (types, names; NO flow analysis any more)
  │ ownership::self_mode::infer      Vec<SelfMode>  (one per method; HIR level, 3.7)
  │ mir::build::program              mir::Program   (one Body per function without type errors)
  │ mir::check::run                  diagnostics: init/moves, mutability, unsafe, borrows, lives
  │   (stop here if any error)
  │ mir::drops::elaborate            Drop statements made exact, drop flags added
  │ mir::cleanup::run                remove Nop/Storage noise for codegen, merge goto chains
  ├─ codegen::compile(&mir::Program)  Cranelift, plain builds
  └─ cgen::generate(&mir::Program)    C, -O builds
```

`lib.rs` becomes:

```rust
pub struct Checked {
    pub hir: hir::Program,   // kept for the language server (step 8): hover, go-to
    pub mir: mir::Program,   // what the backends use
}

pub fn check(src: &str) -> (Option<Checked>, Vec<Diagnostic>);
pub fn build(c: &Checked, out: &Path, optimise: bool) -> Result<(), String>;
```

`main.rs` changes only in types. Add a hidden flag `drachm check --dump-mir
FILE` that prints the elaborated MIR (Appendix B format) to stdout; tests use
it (6.2).

**Per-function gating.** Today the type checker runs only on a parse-clean
file and its output is used only when there are no errors. Keep that, but
gate MIR per function: `typeck` records `hir::Func::has_errors` (the number
of errors before and after checking the function differ). The MIR builder
skips such functions, and the analyses treat calls *to* them as calls to a
function whose signature is fully known (signatures are checked before
bodies) and whose inferred facts (self mode, `lives` sets) are "unknown":
unknown self mode never produces a mutability error at a call site, unknown
`lives` never produces a borrow error. This keeps one mistake from producing a
cascade, and keeps today's behaviour where E0207 is reported next to type
errors in other functions. **(visible)** E0207, E0205 and E0105 are no longer
reported inside a function that has a type error. Existing fail tests do not
mix them; check after WP2.

### 2.2 What happens to the HIR

The HIR stays a tree. It changes in these ways (WP0, WP1):

1. **Spans.** `hir::Expr` gets `span: Span`; `hir::Stmt` becomes
   `struct Stmt { kind: StmtKind, span: Span }`; `hir::Local` gets
   `span: Span` (the declared name), `mutability: Mutability`, `is_param:
   bool`; `hir::Func` gets `name_span`, `span` (signature), `has_errors`.
   MIR diagnostics need all of these; typeck already has them at hand.
2. **Places.** Assignment targets become place expressions:
   `StmtKind::Assign(PlaceExpr, Expr)` with
   `struct PlaceExpr { local: LocalId, proj: Vec<HirProj>, span: Span }` and
   `enum HirProj { Field(FieldIdx), Deref }`. Field reads stay expressions
   (`ExprKind::Field(Box<Expr>, FieldIdx)`, from branch (b)).
3. **New expressions**: `ExprKind::Borrow { mutable: bool, place: Box<Expr> }`,
   `ExprKind::Deref(Box<Expr>)` (raw pointers, and borrows if Q1 says so),
   `ExprKind::DeepCopy(Box<Expr>)` for the user's `copy x`,
   `ExprKind::Unsafe(Block)` / `StmtKind::Unsafe(Block)`.
4. **Removed**: the `Flow` struct and everything it fed in `typeck.rs`
   (`flow`, `branch`, `Flow::join`, the reachability flag, the late-init
   branch of `check_assignable`, `read_local`'s E0207). Typeck keeps the
   *type* checks of assignments (E0101 etc.) and records mutability; all
   E0205, E0207 and E0105 reporting moves to MIR (WP2, WP5) with **the same
   message texts** so `tests/fail/immutable.err` and `unassigned.err` keep
   passing.
5. **Removed** (WP8): step 2's inserted deep copies and drops. The HIR no
   longer contains drops at all (the MIR builder inserts them from scopes);
   the only copy left is the user's `copy x`.

> **LOOK FIRST (a), (b), (c):** how branch (a) represents its copy and drop
> operations in the HIR (node names, whether user `copy` and inserted copies
> are the same node); how (b) represents field reads, field writes, method
> calls and `self`; how (c) represents `for`, `break N`, `continue N`, `skip`
> and `match`. Keep their node names where they fit the list above; add the
> missing ones.

### 2.3 Data structures

New module `src/mir/mod.rs`. Index types are newtypes over `u32` with a
small `IndexVec<I, T>` (a `Vec<T>` indexed by `I`); write it in
`src/mir/index.rs`, 40 lines, no crate needed.

```rust
use crate::diag::Span;
use crate::types::Type;              // after step 2 this includes Str, Struct, Ref, Ptr
use crate::hir::{FuncId, StructId, FieldIdx};

newtype_index!(Local);               // _0 is the return place
newtype_index!(BlockId);             // bb0 is the entry block
newtype_index!(StrId);               // index into Program::strings

pub struct Program {
    pub bodies: IndexVec<FuncId, Option<Body>>, // None: function had type errors (never when building)
    pub main: FuncId,
    pub strings: IndexVec<StrId, String>,       // literal pool, shared by both backends
    pub structs: IndexVec<StructId, StructInfo>, // see LOOK FIRST (b): may already exist in hir
    pub layouts: Layouts,                        // 2.7: one layout table for both backends
    pub self_modes: Vec<Option<SelfMode>>,       // by FuncId; None for non-methods
    pub lives: IndexVec<FuncId, LivesSig>,       // 3.6: inferred or declared
}

pub struct Body {
    pub func: FuncId,
    pub name: String,
    pub span: Span,                      // signature span, for E0105
    pub arg_count: usize,                // locals 1..=arg_count are the parameters, in order
    pub locals: IndexVec<Local, LocalDecl>,
    pub blocks: IndexVec<BlockId, BasicBlock>,
    pub scopes: IndexVec<ScopeId, Scope>, // lexical scopes, for diagnostics only
    pub unsafe_flag: bool,               // the file allows `unsafe` (copied from FileFlags)
}

pub struct LocalDecl {
    pub ty: Type,
    pub kind: LocalKind,
    pub span: Span,          // declaration (user locals), or the expression (temps)
    pub scope: ScopeId,
    /// Computed by `mir::analysis::address_taken` after building: some place
    /// based on this local is the operand of a `Ref`/`RawRef`, or the type is
    /// an aggregate (string, struct). Backends keep such locals in memory.
    pub in_memory: bool,
}

pub enum LocalKind {
    Return,                                          // _0; type `none` when the fn returns nothing
    Param { name: String, mutability: Mutability },  // `self` is a Param named "self"
    User { name: String, mutability: Mutability },
    Temp,
    DropFlag,                                        // bool, added by drop elaboration (3.3)
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mutability { Fixed, Mut, Open }

pub struct BasicBlock {
    pub statements: Vec<Statement>,
    pub terminator: Terminator,   // during building: Option<Terminator>, unwrapped by `finish`
}

pub struct Statement { pub kind: StatementKind, pub span: Span, pub unsafe_ctx: bool }

pub enum StatementKind {
    /// `place = rvalue`. Move, copy and borrow are rvalue/operand forms of Assign.
    Assign(Place, Rvalue),
    /// The local's storage begins / ends. Every non-param, non-return local
    /// has exactly one StorageLive before any use on every path, and a
    /// StorageDead when its scope ends on every path that leaves the scope.
    StorageLive(Local),
    StorageDead(Local),
    /// Destroy the value at `place` if it needs it. Before elaboration
    /// `flag` is None and the Drop is unconditional-as-written; after
    /// elaboration every Drop is exact: `flag: None` means "certainly
    /// initialised here", `Some(f)` means "only if local f (bool) is true".
    /// `reason` tells the borrow checker whether this drop ends a scope
    /// (checked like the owner dying) or precedes an assignment (checked
    /// like a write, B6); elaboration discards it.
    Drop { place: Place, flag: Option<Local>, reason: DropReason },
    /// Read of a place with no other effect: the scrutinee of `match`,
    /// so that the borrow checker sees the read even if no case reads it.
    FakeRead(Place),
    Nop,
}

pub struct Terminator { pub kind: TerminatorKind, pub span: Span, pub unsafe_ctx: bool }

pub enum TerminatorKind {
    Goto(BlockId),
    If { cond: Operand, then: BlockId, els: BlockId },
    /// Integer/bool switch. `match` on numbers, later on enum tags.
    Switch { discr: Operand, cases: Vec<(u128, BlockId)>, otherwise: BlockId },
    /// A call. `dest` is None for calls returning `none`. Never unwinds (R6).
    Call { callee: Callee, args: Vec<Arg>, dest: Option<Place>, target: BlockId },
    /// Return `_0` (nothing for `none`).
    Return,
    /// A runtime fault found by the compiler's own checks (bounds, later).
    Panic(PanicKind),
    /// Proven unreachable (e.g. after the last statement of a function whose
    /// every path returns; replaces today's `trap(user 1)`).
    Unreachable,
}

pub struct Arg { pub op: Operand, pub span: Span }  // per-argument spans for "moved here" labels

pub enum Callee {
    Func(FuncId),            // user function or method (self is args[0])
    Runtime(RtFn),           // runtime/strings.h and runtime.c entry points
}

/// One variant per runtime entry point the MIR may call. Each has a fixed
/// Drachm-level signature (`RtFn::sig`) used by type and borrow checking:
/// which arguments are borrowed (all string inputs, R2), and whether the
/// result is written to an out-pointer (string results). LOOK FIRST (a):
/// branch (a) has a list like this somewhere; reuse it.
pub enum RtFn { OutStr, OutBool, OutU64, OutS64, OutU128, OutS128, OutF32, OutF64,
                StrConcat, StrLen, StrIsEmpty, StrReverse, StrSub, /* ... every strings.h fn */
                StrCopy, StrDrop, StrEq, StrCmp, /* ... */ }

pub enum PanicKind { /* filled by step 6: Bounds, ... */ }

pub struct Place { pub local: Local, pub proj: Projs }
pub type Projs = SmallVec<[Proj; 2]>;  // or a plain Vec; places are short

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Proj {
    Field(FieldIdx),    // on a struct-typed prefix
    Deref,              // on a `&T`, `&mut T` or `*T` typed prefix
}

pub enum Operand {
    Copy(Place),        // only for Copy types (3.2), including `&T`
    Move(Place),        // only for non-Copy types; the place is uninitialised afterwards
    Const(Constant),
}

pub enum Constant {
    Int(u128, Type),    // bits zero-extended, as hir::ExprKind::Int
    Float(f64, Type),
    Bool(bool),
    Str(StrId),         // a non-owning `string` value (R10)
}

pub enum Rvalue {
    Use(Operand),
    Neg(Operand), Not(Operand), BitNot(Operand),
    /// Same operand types as `hir::ExprKind::Binary`. Integer `/` and `%`
    /// keep their zero check inside the backend, exactly as today.
    Binary(hir::BinOp, Operand, Operand),
    Shift { left: bool, value: Operand, amount: Operand },
    IntPow(Operand, Operand),
    Convert(Operand, Type),             // numeric only
    Ref(BorrowKind, Place),             // `&place` / `&mut place`
    RawRef(bool /* mutable */, Place),  // making a `*T`; syntax pending Q14
    /// A struct value built from its fields in declaration order. The
    /// operands are evaluated left to right in *source* order before this
    /// statement (the builder puts each in a temp), so field order here does
    /// not reorder evaluation.
    Aggregate(StructId, Vec<Operand>),
    /// `copy place` on a non-Copy type: a fresh owned deep copy.
    DeepCopy(Place),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BorrowKind {
    Shared,
    Mut { two_phase: bool },   // two_phase only if Q4 allows it (3.5.6)
}

pub struct Scope { pub parent: Option<ScopeId>, pub span: Span, pub kind: ScopeKind }
pub enum ScopeKind { Function, Block, LoopBody, IfBranch, MatchCase, Unsafe }

/// A program point: statement `index` of `block`; index == statements.len()
/// is the terminator.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Location { pub block: BlockId, pub index: u32 }
```

**Invariants** (checked by `mir::validate`, run after building and after
every pass in debug builds of the compiler, `cfg(debug_assertions)`):

- V1. `blocks[bb0]` is the entry. Every block has a terminator. Every
  `BlockId` mentioned exists.
- V2. Every `Place` is well typed: `Field(i)` only on a struct type with more
  than `i` fields; `Deref` only on `&T`, `&mut T` or `*T`. `place_ty` (a
  function in `mir/ty.rs`) computes the type of every place.
- V3. `Operand::Copy` only of Copy types; `Operand::Move` only of non-Copy
  types and only of places with no `Deref` (moving out through a borrow is
  E0310 and never reaches MIR that passed checking; before checking the
  builder may still emit it, so V3 is checked after `mir::check`).
- V4. `Assign(p, rv)`: `type(rv) == type(p)` with regions erased (types in
  MIR carry no regions; the borrow checker adds them, 3.5).
- V5. A `Return` terminator is reached only after `_0` is assigned on every
  path (functions returning `none` have a `_0` of type `none` that is never
  assigned or read).
- V6. `DropFlag` locals are of type `bool`, are assigned in `bb0` before
  anything else, and are never used except by `Drop { flag }` and flag
  assignments.
- V7. Temps are assigned exactly once on every path before use (the builder
  guarantees it; analyses do not rely on it, backends may).
- V8. After elaboration, no `StorageLive`/`StorageDead`/`FakeRead`/`Nop`
  remain if `cleanup` ran (backends ignore them anyway).
- V9. No critical edges are required by any pass. (Drop elaboration inserts
  flag writes at statements, never on edges.)

### 2.4 Types in MIR

MIR uses `types::Type`. Step 3 adds borrow and pointer types to it:

```rust
pub enum Type {
    Int { bits: u16, signed: bool }, Float(u16), Bool, None,
    Str,                         // from (a)
    Struct(StructId),            // from (b)
    Ref { mutable: bool, to: TyId },
    Ptr { mutable: bool, to: TyId },   // `*T`; mutability of raw pointers pending Q14
}
```

`Type` is `Copy` today. Keep it `Copy` by interning the pointee: a global
`TyId(u32)` into a `TypeTable { types: Vec<Type>, map: HashMap<Type, TyId> }`
owned by `hir::Program` and passed by reference. **LOOK FIRST (b):** if
branch (b) already moved to interned types or made `Type` non-`Copy`, follow
what it did instead.

Region information is **not** stored in `Type`. The borrow checker gives each
local a vector of *region slots*, one per borrow position in its type, in a
fixed pre-order:

```rust
fn region_slots(ty: Type, t: &TypeTable, structs: &StructTable) -> usize {
    match ty {
        Type::Ref { to, .. } => 1 + region_slots(t[to], t, structs),
        Type::Struct(s) => structs[s].region_params.len(),  // `lives` names, 3.6.4
        _ => 0,                                             // Ptr: raw, untracked
    }
}
```

So `&string` has one slot, `&Parser` (with `struct Parser lives src`) has two
(the outer borrow, then `src`), `Parser` has one, `uint32` none.

### 2.5 Building MIR from HIR

`mir/build.rs`. One `Builder` per function:

```rust
struct Builder<'a> {
    hir: &'a hir::Program, func: &'a hir::Func, modes: &'a [Option<SelfMode>],
    body: Body,                         // being built; blocks have Option<Terminator>
    cur: BlockId,                       // the block statements are appended to
    local_map: Vec<Local>,              // hir::LocalId -> mir::Local
    scopes: Vec<ScopeFrame>,            // the scope stack (drops to emit on exit)
    breakables: Vec<Breakable>,         // loops and if-levels, for break/continue/skip
    unsafe_depth: u32,
}

struct ScopeFrame { id: ScopeId, owned: Vec<Local> }   // locals declared here, in order
enum Breakable {
    Loop { break_to: BlockId, continue_to: BlockId, scope_depth: usize },
    /// One level of an if-chain (or one `match` case), for `skip`.
    IfLevel { exit: BlockId, next_branch: Option<BlockId>, scope_depth: usize },
    Wall,   // pushed by a loop body (and later lambdas): skip may not cross it
}
```

**General rules.**

- B1. *Evaluation order.* Every sub-expression that is not already a place or
  a constant is evaluated into a fresh `Temp` local, in left-to-right source
  order (S19). This is what today's C backend does with `const` temporaries;
  it now happens once, in MIR, for both backends.
- B2. *Places vs values.* `as_place(expr)` lowers a place expression (local,
  field chain, deref) to a `Place` without reading it. `as_operand(expr)`
  lowers any expression to an `Operand`: a place becomes `Copy(place)` if its
  type is Copy, else `Move(place)`; anything else is evaluated into a temp
  and then moved/copied out of the temp.
- B3. *Borrowed operands.* Where R2/R3 or a `&` parameter says an operand is
  borrowed, the builder uses `as_place` (materialising a temp if the operand
  is not a place) and emits `_t = &place`; the call gets `Copy(_t)`.
- B4. *Temps with drop glue* are pushed onto the innermost *statement scope*,
  a scope that ends at the end of the enclosing HIR statement (R5). If the
  temp is moved before that (the usual case), drop elaboration removes its
  drop.
- B5. *Scopes.* Entering a HIR block pushes a `ScopeFrame`. A `let` emits
  `StorageLive(l)` and registers `l` in the frame. Leaving a scope normally
  emits, for its locals in **reverse** order: `Drop(l)` (if the type needs
  drop, 2.7) then `StorageDead(l)`. Every non-local exit (`return`,
  `break N`, `continue N`, `skip N`, `skip to next`) emits the same for every
  frame it leaves, innermost first, then jumps. Drops are emitted
  unconditionally here; elaboration (3.3) makes them exact.
- B6. *Assignment to an existing place* `p = e` where `type(p)` needs drop:
  evaluate `e` into an operand first, then `Drop(p)`, then
  `Assign(p, Use(op))`. (`s = s::trim();` reads `s` before dropping it.)
- B7. *Calls write to a fresh temp*, never directly into a user place:
  `_t = call f(..)`, then `p = move _t` (B6). This keeps the borrow checker
  from seeing a write to `p` at the same point as the argument borrows of `p`
  (`x = f(&x)` stays legal). The backends' copy of a 24-byte string or a
  small struct costs nothing in C; for Cranelift see 7.4.
- B8. *Spans.* Each statement gets the span of the HIR expression it came
  from; `Arg` gets the argument's span; `Drop`/`StorageDead` emitted at a
  scope end get the span of the closing `}` (the HIR block needs its end
  span; add `hir::Block { stmts, end: Span }` if not there) or of the
  `return`/`break` that caused them.

**Lowering table.** `⟦e⟧ → op` means "emit statements, produce an operand".

| HIR | MIR |
|---|---|
| `Int`, `Float`, `Bool` | `Const` |
| `Str(s)` (literal) | `Const(Str(id))`, `id` interned in `Program::strings` |
| `Local(id)` | `Copy(_l)` / `Move(_l)` by type (B2) |
| `Field(e, f)` | place `as_place(e)` + `Field(f)`; if `e` is a borrow, insert `Deref` first (R4) |
| `Neg/Not/BitNot(e)` | `_t = Neg(⟦e⟧)` → `Copy(_t)` |
| `Binary(op, l, r)` | `a = ⟦l⟧; b = ⟦r⟧; _t = Binary(op, a, b)` |
| `Binary` on strings (`+`, `==`, `<` …) | `_a = &⟦l⟧place; _b = &⟦r⟧place; _t = call rt(copy _a, copy _b)` (R2) |
| `Shift`, `IntPow`, `Convert` | as `Binary` |
| `Logic { and, l, r }` | `_t = ⟦l⟧; If _t → bbR / bbJ`, `bbR: _t = ⟦r⟧; goto bbJ` (for `||` the branches swap). Short-circuit is control flow. |
| `IncDec { local, inc, prefix }` | `_old = copy _l; _l = Binary(Add/Sub, copy _l, 1)`; value is `_old` (postfix) or `copy _l` read into a temp (prefix) |
| `Call(f, args)` | each arg `⟦a⟧` left to right (borrowed params: B3); `_t = call f(args) -> bbNext` |
| method call `recv::m(args)` (b) | receiver: `as_place(recv)` (temp if not a place); `_s = &place` or `&mut place` by `modes[m]` (if `recv` is already a borrow: `&(*recv)` / `&mut (*recv)`, a reborrow); then args; `call m(copy/move _s, args)`. A `static` method has no receiver. |
| `Borrow { mutable, e }` | `_t = Ref(Shared/Mut, as_place(e))`; on a non-place `e`, materialise a temp first (lifetime of that temp: Q13) |
| `DeepCopy(e)` | Copy type: same as `⟦e⟧` (W0322 if Q11 says so); else `_t = DeepCopy(as_place(e))` |
| `Deref(e)` | place `as_place(e)` + `Deref` |
| struct construction (b) | each field value `⟦v⟧` in *source* order into temps; `_t = Aggregate(S, ops in field order)` |
| `Out { newline, arg }`, `Text` | runtime calls (`RtFn`), arguments borrowed (R2) |
| `f"..${e}.."` (a) | the runtime calls branch (a) chose, all inputs borrowed |
| `StmtKind::Let(l, Some(e))` | `StorageLive(_l)`; `_l = Use(⟦e⟧)` |
| `StmtKind::Let(l, None)` | `StorageLive(_l)` only |
| `StmtKind::Assign(p, e)` | B6 |
| `StmtKind::Expr(e)` | `⟦e⟧`, result discarded: a temp with drop glue is dropped at the end of the statement (B4) |
| `If(c, a, b)` | `_c = ⟦c⟧; If _c → bbA / bbB`; each branch in its own scope; both `goto bbJoin`. Push an `IfLevel` per chain level (below) |
| `While(c, body)` | `goto bbHead; bbHead: _c = ⟦c⟧; If _c → bbBody / bbExit`. Literal `true`: `goto bbBody` instead (R7). Body in a `LoopBody` scope; ends with `goto bbHead`. Push `Wall` and `Loop { break_to: bbExit, continue_to: bbHead }` |
| `Return(e)` | `_0 = Use(⟦e⟧)` (moves the value out, so it is not dropped); drops of all frames (B5); `Return` |
| `Unsafe(block)` | `unsafe_depth += 1` while lowering; every statement/terminator built gets `unsafe_ctx = true` |
| end of function body | if `cur` is reachable: drops of the function frame; `Return` for `none` functions, else `Unreachable` (E0105 is reported by WP2 using CFG reachability, see 3.1.5) |

**Control flow from branch (c)** (port the lowering logic (c) wrote for the
backends; the rules are SPEC §3):

| HIR (c) | MIR |
|---|---|
| C-style `for (init; cond; step) body` | new scope for `init`; `goto bbHead; bbHead: cond → bbBody / bbExit`; body; `goto bbStep; bbStep: step; goto bbHead`. `continue` goes to `bbStep`. No cond: `goto bbBody` (R7). |
| `for (T x of range)` | iteration state in temps (current, end, done flag, inclusive ranges need the done flag to avoid overflow at the maximum); each iteration: `StorageLive(_x); _x = copy _cur;` body; `Drop/StorageDead(_x)` at the end of the body and on `continue`/`break`. |
| `for (string c of s)` | `_it = &s` (a shared borrow, so `s` cannot change during the loop); each iteration the runtime call that (c) used writes a fresh owned one-character string into `_c` (or the loop ends); `_c` is dropped at the end of each iteration unless moved. |
| `break N` / `continue N` | find the N-th `Loop` on the `breakables` stack (counting outward); emit drops for every scope frame above its `scope_depth`; `goto break_to / continue_to`. |
| `skip N` / `skip` / `skip to next` | find the target `IfLevel` (SPEC §3: count outward from the innermost; bare `skip` = outermost level inside the nearest `Wall`; `skip to next` = innermost level's `next_branch`); drops for scopes above its `scope_depth`; `goto exit / next_branch`. The compile errors (N too large, no next branch) are reported where (c) reports them; keep their codes. |
| `match (x)` on numbers/bools | `_d = ⟦x⟧`; `FakeRead`; `Switch { discr: _d, cases, otherwise }` for value cases; ranges and mixed patterns become a chain of compare-and-branch blocks. Each case body is a scope and an `IfLevel` whose `next_branch` is the next case's test block. |
| `match (s)` on strings | `_d = as_place(s)` (not moved); `_b = &_d` once; each case: runtime equality/glob call on `copy _b` → `If`. |

> **LOOK FIRST (c):** how (c) lowered `skip`, `break N` and `match` in each
> backend, and which runtime functions do string iteration and globbing.
> The MIR builder must give exactly the same behaviour; (c)'s golden tests
> are the acceptance test (WP4).

**If-chain levels in detail.** For `if (a) {A} else if (b) {B} else {C}`
the builder creates `bbTestA, bbA, bbTestB, bbB, bbC, bbJoin`. While
lowering `A` it pushes `IfLevel { exit: bbJoin, next_branch: Some(bbTestB) }`;
while lowering `B`: `next_branch: Some(bbC)`; while lowering `C`:
`next_branch: None`. A nested `if` inside `A` pushes its own level on top.

#### Worked example 1

```cpp
in io;

none take(string s) { io::outln(s); }

none main() {
    string a = "hi";
    if (a::isEmpty()) {
        take(a);
    }
    io::outln("done");
}
```

`main` before elaboration (Appendix B format):

```text
fn main() -> none
  _0: none
  _1: string           // a
  _2: &string          // temp
  _3: bool             // temp
bb0:
  StorageLive(_1)
  _1 = const "hi"
  _2 = &_1
  _3 = call StrIsEmpty(copy _2) -> bb1
bb1:
  If copy _3 -> bb2, bb3
bb2:
  call take(move _1) -> bb3
bb3:
  call OutStr(const "done", const true) -> bb4
bb4:
  Drop(_1)
  StorageDead(_1)
  return
```

After drop elaboration (3.3), `_1` is maybe-moved at `bb4`, so it gets a
flag:

```text
  _4: bool             // drop flag of _1
bb0:
  _4 = const false
  StorageLive(_1)
  _1 = const "hi"
  _4 = const true
  ...
bb2:
  _4 = const false
  call take(move _1) -> bb3
...
bb4:
  Drop(_1) if _4
  return
```

### 2.6 Calls that can fail (note for step 5)

Step 5 lowers a failing function to return `{ tag, union(value, error) }`.
In MIR that is `_t = call f(..) -> bbCheck; bbCheck: Switch(copy _t.tag)
→ bbOk / bbFail`. Nothing in step 3's analyses needs to change for it. A
`fails with` handler is ordinary code in `bbFail`; `return` inside it is an
ordinary `Return` with scope drops.

### 2.7 Drop glue, copy glue, layout

```rust
pub fn needs_drop(ty: Type, cx: &Cx) -> bool {
    match ty {
        Type::Str => true,
        Type::Struct(s) => cx.structs[s].fields.iter().any(|f| needs_drop(f.ty, cx)),
        _ => false,       // numbers, bool, none, &T, &mut T, *T
    }
}
```

`is_copy(ty)`: numbers, `bool`, `none`, `&T` → true; `&mut T`, `Str` →
false; `*T` → pending Q15 (until answered: true, matching C pointers, and
note it in the PLAN); `Struct(s)` → every field `is_copy`, no field (at any
depth) is a `*T` (S10), and `layout.size <= 32` (S5; padding question: Q12).
Compute once per struct, memoised; struct fields cannot contain the struct
itself by value, so the recursion ends.

**Layouts.** One `Layouts` table (`src/layout.rs`), computed once with C rules
(size, alignment, field offsets; `uint128`/`sint128` align 16, `bool` 1
byte, `string` 24/8, borrows and pointers 8/8). Both backends read it. The C
backend also emits, after each struct definition,
`_Static_assert(sizeof(S) == N && offsetof(S, f) == K, "layout");` so that a
disagreement between the table and the C compiler fails the build instead of
corrupting memory. **LOOK FIRST (b):** branch (b) has a layout function;
move it here if it is inside one backend.

**Glue functions**, generated once per struct type that needs them, by a
shared module `src/glue.rs` that both backends call to get a description:

- `drop_glue(S)`: for each field in declaration order (R5) that needs drop:
  drop it (`drachm_str_drop(&p->f)` or the field struct's glue).
- `copy_glue(S)`: field by field: Copy fields bitwise, strings with
  `drachm_str_copy`, struct fields with their glue.

Cranelift: a local function `drachm_drop_<S>(ptr)` / `drachm_copy_<S>(out,
ptr)`. C: `static inline` functions of the same names. A string drop is
emitted inline as `if (s.cap != 0) drachm_str_drop(&s);` in both backends
(the check is `drachm_str_drop`'s own first test, hoisted to skip the call
for literals).

### 2.8 Cleanup before codegen

`mir/cleanup.rs`, after elaboration:

1. Remove `StorageLive`, `StorageDead`, `FakeRead`, `Nop`.
2. Remove unreachable blocks (from `bb0`).
3. Merge `A → Goto(B)` when `B` has exactly one predecessor (`A`).
4. Retarget jumps to blocks that contain only `Goto(C)` (no statements).

It must not reorder statements, change operands, or remove `Drop`s. Both
backends see the same cleaned MIR, so this cannot cause a disagreement.

### 2.9 How the backends consume MIR

Both backends emit blocks in **reverse post-order** from `bb0` (compute once
in `mir/traversal.rs`), which puts every block after at least one of its
predecessors and makes "next block" fall-through common.

**Place addresses.** `addr(place)`: start at the local's storage (stack slot
address, or for a by-pointer parameter the incoming pointer); for each
`Field(i)` add `layout.offset(i)`; for each `Deref` load a pointer from the
current address. A place with no `Deref` and no `Field` on a non-memory local
is a register value.

**Cranelift (`codegen.rs`).**

- Locals with `in_memory == false` (scalars never borrowed) become
  `Variable`s exactly as today (`declare_var`/`def_var`/`use_var`); this keeps
  numeric code bit-for-bit the same as before MIR and keeps it SSA.
- Locals with `in_memory == true` get an explicit stack slot
  (`create_sized_stack_slot`, size and alignment from `Layouts`).
- One Cranelift block per MIR block, created up front; emit in RPO; call
  `seal_all_blocks()` once at the end of the function instead of sealing by
  hand (simpler, and safe for arbitrary CFGs; the cost is negligible at
  these sizes).
- `Operand::Copy`/`Move` of a memory place: scalars `load`, aggregates
  `emit_small_memory_copy` (≤ 64 bytes) or a `memcpy` libcall. A `Move`
  never writes to its source.
- `Drop { place, flag: Some(f) }`: `brif f, bbDrop, bbAfter`, glue call in
  `bbDrop`. `flag: None`: glue call inline.
- `Call`: arguments by the ABI from branch (b) (by value, or by pointer for
  large aggregates). **`Move` of a by-pointer aggregate argument passes the
  address of the source place itself, with no copy**: the source is dead
  after the move, so the callee may own and even write that memory. `Copy`
  of a by-pointer aggregate (a Copy struct over the by-pointer threshold)
  copies into a fresh stack slot first, because the callee may change a `mut`
  parameter. String results are written through the out-pointer `addr(dest)`.
- `Unreachable` → `trap(TrapCode::unwrap_user(1))` (as today's dead end).
- `Panic(k)` → call `drachm_panic(msg)`, then `trap`.

**C (`cgen.rs`).** The C backend stops emitting structured C and emits the
CFG:

```c
static uint32_t drachm_f_fib(uint32_t l1) {
    uint32_t l0; uint8_t l2; uint32_t l3; uint32_t l4;   /* every MIR local, declared at the top */
    l2 = l1 < 2u;
    if (!l2) goto bb2;
    l0 = l1;
    return l0;
bb2:;
    l3 = drachm_f_fib((uint32_t)((uint32_t)l1 - (uint32_t)1u));
    ...
}
```

- Every MIR local is a C local declared at the top of the function (drop flags
  `uint8_t`). No `const` temporaries are needed any more: MIR statements are
  already one operation each, with operands that are locals or constants.
  The existing helpers (`work_type`, wrapping casts, the 64-bit division fast
  path, `halves`, float bit constants) are reused per `Rvalue`.
- Blocks in RPO. A label `bbN:;` is printed only for blocks that are jumped
  to by something other than fall-through. `Goto` to the next emitted block
  prints nothing. `If` prints `if (c) goto bbA; goto bbB;` with whichever
  side falls through dropped (negate the condition if needed). `Switch`
  prints a C `switch` with `case K: goto bbX;`. `Unreachable` prints
  `__builtin_unreachable();`.
- **Does goto-style C cost speed?** No, for this MIR. GCC and Clang lower all
  C to a CFG immediately and find loops as natural loops (a back edge to a
  dominating header), not from syntax. Drachm's control flow is reducible
  (`break`, `continue`, `skip` and `skip to next` only jump outward or
  forward; loops have one entry), so every loop optimisation (LICM,
  unrolling, vectorisation, induction variables) sees exactly what it saw in
  structured C. The one thing to avoid is *irreducible* control flow (a jump
  into the middle of a loop); the builder never produces it, and `validate`
  checks reducibility in debug builds (every retreating edge in the DFS is a
  back edge to a dominator). Measure: WP4's acceptance includes running
  `bench/run.sh` before and after.

**Bit-identical behaviour.** The two backends consume the *same* elaborated,
cleaned MIR. Rules:

1. All evaluation order, drop order and drop conditions are fixed in MIR
   (B1, B5, 3.3). A backend may not reorder statements with side effects.
2. All layouts come from `Layouts`; C checks them with `_Static_assert`.
3. All behaviour with room for disagreement stays in the shared runtime, as
   today (float→int, 128-bit division, printing, every string operation).
4. `optimised_output_matches` keeps comparing stdout and exit code for every
   run case built both ways, and now also stderr when `DRACHM_CHECK_LEAKS`
   is set (6.3).

---
## 3. The analyses

`mir::check::run` runs, per body, in this order; later passes skip a body
with errors from an earlier pass only where noted:

1. `moves`: definite assignment, move checking, missing return (3.1).
2. `mutability`: binding and place mutability (3.8).
3. `unsafety`: raw pointer operations and the file flag (3.9).
4. `borrowck`: liveness (3.4), regions and loans (3.5), with `lives`
   inference across the call graph (3.6). Skipped for a body with E0301/E0207
   errors (the move errors would produce noise).

Self modes (3.7) are inferred on the HIR before MIR is built.

### 3.0 Dataflow framework

`mir/dataflow.rs`, one generic solver used by every analysis below.

```rust
pub struct BitSet { words: Vec<u64>, len: usize }   // insert, remove, contains, union, subtract, iter

pub trait Analysis {
    const FORWARD: bool;
    fn domain_size(&self, body: &Body) -> usize;
    /// State at function entry (forward) or at every exit (backward).
    fn boundary(&self, body: &Body, state: &mut BitSet);
    /// All our lattices are powersets joined by union.
    fn statement(&self, state: &mut BitSet, stmt: &Statement, loc: Location);
    fn terminator(&self, state: &mut BitSet, term: &Terminator, loc: Location);
}

pub struct Results<A> { analysis: A, entry: IndexVec<BlockId, BitSet> } // state at block start (forward) / end (backward)

impl<A: Analysis> Results<A> {
    pub fn compute(a: A, body: &Body, preds: &Predecessors) -> Self;
    /// State immediately *before* the statement/terminator at `loc` (forward),
    /// or immediately *after* it (backward). Walks from the block's entry state.
    pub fn state_at(&self, body: &Body, loc: Location) -> BitSet;
    /// Visit every location in order with the state before it, without
    /// recomputing from the block start each time.
    pub fn visit(&self, body: &Body, f: impl FnMut(Location, &BitSet, &Statement | &Terminator));
}
```

Solver: worklist of blocks, initialised in RPO (forward) or post-order
(backward); entry states start empty except the boundary; pop a block, apply
its statements and terminator to a copy of its entry state, join (union) into
each successor's (predecessor's) entry state, push the ones that changed.
Terminates because every transfer function is monotone and the lattice is a
finite powerset. Every lattice in this section is a powerset ordered by ⊆
with join = ∪ (the "must" facts are complements of "may" facts).

### 3.1 Move paths, initialisation and move checking

`mir/moves.rs`.

#### 3.1.1 Move paths

A *move path* is a place made only of a local and `Field` projections, with
no `Deref`: the things that can be (un)initialised.

```rust
newtype_index!(MovePathId);
pub struct MovePath {
    pub place: Place,                    // no Deref in it
    pub parent: Option<MovePathId>,
    pub first_child: Option<MovePathId>, pub next_sibling: Option<MovePathId>,
}
pub struct MoveData {
    pub paths: IndexVec<MovePathId, MovePath>,
    pub local_path: IndexVec<Local, MovePathId>,          // every local has a root path
    pub path_of: HashMap<Place, MovePathId>,
    pub moves: IndexVec<MoveOutId, MoveOut>,             // MoveOut { path, loc, span }
    pub inits: IndexVec<InitId, Init>,                   // Init { path, loc, span }
    pub moves_at: HashMap<Location, SmallVec<[MoveOutId; 2]>>,
    pub inits_at: HashMap<Location, SmallVec<[InitId; 2]>>,
}
```

Build it in one walk over the body:

- every local gets a root path;
- every place that is the source of `Operand::Move`, the destination of an
  `Assign` or a call, or the place of a `Drop`, gets a path, creating parent
  paths as needed (`p.a.b` creates `p.a`); a `Deref` stops the walk: a place
  through a borrow has no move path of its own (moving out of it is E0310,
  writing into it initialises nothing);
- `Move(p)` at `loc` records a `MoveOut`; `Assign(p, _)` and a call's `dest`
  record an `Init`; `Drop(p)` records a `MoveOut` too (after a drop the place
  is uninitialised); `StorageDead(l)` uninitialises the whole local.

Effects on the path sets, for every analysis below: *uninit(p)* applies to
`p` and all its descendants; *init(p)* applies to `p` and all its
descendants.

#### 3.1.2 The analyses

| Analysis | Domain | Gen | Kill | Boundary (entry) |
|---|---|---|---|---|
| `MaybeInit` (forward) | move paths | init(p) | uninit(p) | params and all their descendants |
| `MaybeUninit` (forward) | move paths | uninit(p), and every path of a local at its `StorageLive` | init(p) | every path of every non-param local |
| `EverInit` (forward) | move paths | init(p) | `StorageDead(l)`: every path of `l` | params |
| `ReachingMoves` (forward) | `MoveOutId`s | the MoveOuts at this location | at init(p): every MoveOut whose path is `p`, an ancestor or a descendant of `p`; at `StorageLive(l)`: every MoveOut of `l`'s paths | ∅ |

`p` is *definitely initialised* at `loc` iff `p ∉ MaybeUninit(loc)`.

Order within one statement: an `Assign(p, rv)` first *uses* the operands of
`rv` (checked against the state before the statement), then the moves in
`rv` take effect, then `init(p)`. A `Call` terminator: argument uses, then
argument moves, then `init(dest)`.

#### 3.1.3 Use checks (E0301, E0207)

Every *use* of a place `q` (an operand `Copy(q)`/`Move(q)`, the place of a
`Ref`, `DeepCopy`, `FakeRead`, the base of a field write `q.f = …`, the
receiver of a method) requires the move path `m = longest_move_path_prefix(q)`
(the part before the first `Deref`, if any) to be fully initialised:

```text
fully_init(m) = m ∉ MaybeUninit
             && no ancestor of m ∈ MaybeUninit
             && no descendant of m ∈ MaybeUninit
```

An *assignment* `q = …` to a place `q` with no `Deref`: the parent paths
of `q` (not `q` itself) must be initialised: assigning to `p.name` when `p`
as a whole moved is E0313; assigning to `p.name` after only `p.name` moved
is fine and re-initialises it. Assigning a whole local needs nothing.

When a use fails, find *why*:

1. Collect `ReachingMoves(loc)` whose path is `m`, an ancestor or a
   descendant of `m`. If there are any → **E0301** (labels: every such
   MoveOut's span, sorted by position, at most 3; if one MoveOut's location
   is not before the use in RPO order, i.e. it reaches the use only around a
   loop, its label says "moved here, in the previous time round the loop").
   If the moved path is a descendant (`p.name` moved, `p` used) the message
   names the part (see E0301 drafts).
2. Otherwise the path was never assigned on some path → **E0207** with
   today's texts: "only some paths to here give it one" if `m ∈ MaybeInit`,
   else "nothing has been assigned to it yet".

Report each move path at most once per function (keep a `reported:
BitSet<MovePathId>`), as today ("one report per binding is enough").

#### 3.1.4 Partial moves (decided here, owner may override: Q7)

**Allowed:** moving a field out of a struct held in an owned local or
parameter, at any depth (`string n = p.name;`, `take(p.inner.name);`). After
that:

- `p.name` and anything inside it are unusable (E0301) until reassigned;
- other fields stay usable (`p.hp` is fine);
- `p` as a whole is unusable (E0301, "after `p.name` moved") until `p.name`
  is reassigned or `p` is reassigned;
- `p.name = …` re-initialises the field;
- at the end of the scope only the remaining fields are dropped (3.3).

**Not allowed:**

- moving out through a borrow, including out of `self` (E0310): the owner
  expects the value to be whole when the borrow ends;
- moving out through a raw pointer (E0310, "use `copy`"): nothing tracks it;
- moving out of a struct that has a user destructor, should Q17 add them.

*Justification.* Move paths are per field anyway (the analysis needs them for
`p.name = …` writes), so allowing partial moves costs nothing in the checker
and nothing at runtime: the remaining fields' drops are decided statically or
with one flag per moved field. Forbidding it would force users to `copy`
fields out of structs they own and are about to throw away, which is exactly
the copying step 3 exists to remove. Moving out of borrows must stay an
error, because the lender would later drop or read a hole.

#### 3.1.5 Missing return (E0105)

After building, if the block that ends the function body (the one the builder
gave `Unreachable` because the function returns a value) is reachable from
`bb0` in the CFG, report E0105 with today's text at `Body::span`. `while
(true)` without `break` makes it unreachable (R7). The builder records which
block that is in `Body` (a `fallthrough_end: Option<BlockId>` field).

#### 3.1.6 Late-initialised immutable bindings (S2)

In `mutability` (3.8), for `Assign(_l, …)` to a whole local `l` whose
mutability is `Fixed` or `Open` and which is not a parameter:

- if `l ∉ EverInit(loc)`: allowed (first assignment);
- else → E0205. Choose the variant: if the assigning block is in a
  strongly connected component of the CFG with more than one block (or a
  self-loop) and the `StorageLive(_l)` is *not* in that component → "cannot
  assign `x` inside a loop" (today's text); otherwise "assigned a second
  time here". For an `Open` binding the message is the `open` variant
  (section 4).

Compute SCCs once per body (Tarjan, `mir/traversal.rs`).

### 3.2 Copy and move at the builder

The builder decides `Copy` vs `Move` per operand from `is_copy(type)` (2.7).
There is no separate pass. `copy x` on a non-Copy type becomes
`Rvalue::DeepCopy(place)`, which *uses* `x` (it must be initialised and not
`&mut`-borrowed) but does not move it. `DeepCopy` of a type that contains a
`&mut T` field is E0315 (the copy would be a second `&mut`).

### 3.3 Drop elaboration

`mir/drops.rs`. Runs after all checks passed. Inputs: `MoveData`,
`MaybeInit`, `MaybeUninit` (recomputed on the final MIR).

**Per Drop statement** `Drop { place: p, flag: None }` at `loc` (all
`Drop`s come from B5/B6 and are on move paths), call `elaborate(p, loc)`,
which returns the replacement statements:

```text
elaborate(p, loc):
    if !needs_drop(type(p)):              return []
    if p has child move paths:            // someone moved/assigned a field separately
        // No user destructors (Q17): dropping a struct is dropping its fields.
        return concat(for each field f of type(p) in declaration order:
                        elaborate(child(p, f), loc))
            // child(p, f) is f's move path if it exists, otherwise a virtual
            // path with exactly p's state and p's flag
    let mi = p ∈ MaybeInit(loc), mu = p ∈ MaybeUninit(loc)
    match (mi, mu):
        (false, _)    => []                                   // dead: never initialised, or moved on every path
        (true, false) => [Drop { place: p, flag: None }]      // static: initialised on every path
        (true, true)  => [Drop { place: p, flag: Some(flag(p)) }]  // conditional
```

A virtual path's state is its parent's state, so a field that was never
moved on its own is dropped exactly when its struct would have been.

**Flags.** `flag(p)` creates (once per move path) a `DropFlag` local `f_p`.
Then:

1. In `bb0`, before everything: `f_p = const false`.
2. At every `Init` of `p` *or of an ancestor of `p`*: `f_p = const true`,
   placed immediately after the initialising statement (for a call: at the
   start of the call's `target` block; give that block a single predecessor
   by construction, the builder always creates a fresh target block for a
   call).
3. At every `MoveOut` of `p` *or of an ancestor of `p*`: `f_p = const false`
   immediately after it (a `Drop` counts as a MoveOut).
4. `Drop { place: p, flag: Some(f_p) }` as above.

Inits and moves of *descendants* of `p` do not touch `f_p`: they only matter
for drops of the descendants, which have their own paths and flags (that is
what "has child move paths" handles).

*Correctness argument.* At any `Drop` that got a flag, `f_p` is true iff the
last event on the path from entry that affected `p` (an init or move of `p`
or of an ancestor) was an init. That is exactly "p is initialised here",
given that descendants have separate paths. Flags start false, and before the
first init nothing is initialised.

**After elaboration** no `Drop` remains whose place might be uninitialised
without a flag; that is the invariant the backends rely on, so a bug here is
a double free or a leak. The leak test (6.3) is the safety net.

#### Worked example 2 (partial move)

```cpp
struct Player { string name; string title; mut uint32 hp; }

none main() {
    Player p = "Illy", "Dr", 10;
    if (p.hp > 5) {
        take(p.name);
    }
}
```

At the end of `main`, `p` has a child path `p.name`, so the drop opens:
`p.name` (maybe moved → flag), `p.title` (virtual, state of `p`: definitely
initialised → unconditional), `p.hp` (no drop glue → nothing):

```text
bb_end:
  Drop(_1.0) if _5        // p.name
  Drop(_1.1)              // p.title
  return
```

### 3.4 Liveness

`mir/liveness.rs`. Backward, domain = locals.

- Gen (use) at a location: every local mentioned in an operand, the place of
  a `Ref`/`RawRef`/`DeepCopy`/`FakeRead`, the base local of any place with a
  projection that is *written* (writing `p.f` or `*r` uses `p` / `r`), the
  callee arguments.
- Kill (def): `Assign(_l, …)` and call `dest = _l` to a **whole** local with
  no projection; `StorageDead(_l)`.
- `Drop(p)` is **not** a use for region liveness: dropping a string or a
  struct never reads what any borrow inside it points to (there are no user
  destructors; Q17 would change this into "drop-liveness": a drop of a value
  with a destructor would keep its regions live).

Results are needed per point. Store the block-end sets and recompute inside
a block by walking backwards (cheap; bodies are small). The borrow checker
uses "local `l` is live at `P`" for region liveness constraints.

### 3.5 The borrow checker

`mir/borrowck/`. This is the NLL algorithm (as in the Rust compiler's
2018 borrow checker, without its later Polonius extensions), restricted to
what Drachm has in step 3. Read it once end to end before coding; the parts
depend on each other.

#### 3.5.1 Points and region values

A *point* is a `Location`. Number all points of the body densely:
`point_index(loc) = block_start[loc.block] + loc.index`. A *region* is a set
of points plus a set of *end markers*, one per universal region (below):

```rust
newtype_index!(RegionVid);
struct RegionValue { points: BitSet /* over point indices */, ends: BitSet /* over universal regions */ }
```

Region variables:

- **Universal regions**, first: for each parameter, one per region slot of
  its type (2.4), named for messages by the parameter (`a`, or `p.src` for a
  struct slot). For a method, `self`'s slots come first. Then one per region
  slot of the return type: the *return regions*. Universals start containing
  **every** point of the body and their own end marker `end(U)`.
- **Local regions**: for every other local, one fresh `RegionVid` per region
  slot of its type. `_0`'s slots are the return regions themselves (not
  fresh).
- **Loan regions**: one per `Ref` statement (below).

#### 3.5.2 Loans

Every `Assign(_, Ref(kind, place))` at `loc` creates a loan:

```rust
struct Loan { place: Place, kind: BorrowKind, region: RegionVid, loc: Location, span: Span }
```

`region` is a fresh variable `ρL`. The destination's outer slot must contain
it: constraint `ρL ⊇ … ` see 3.5.3 rule C2.

#### 3.5.3 Constraint generation

Constraints have the form `sup ⊇ sub @ origin` ("sup outlives sub"; origin
= the location and a kind, for explanations). They are location-insensitive:
`sup` must contain every point of `sub` (NLL's model).

- **C1 liveness.** For every point `P` and every local `l` live at `P`:
  every region slot of `l` contains `P`. (Not a constraint between regions;
  it seeds the values.)
- **C2 borrow.** `_d = &'L place` / `&mut place`: `slot0(_d) ⊇ ρL`,
  i.e. `ρL : slot0(_d)` in outlives terms: write it `outlives(ρL, slot0(_d))`.
  Below, `outlives(a, b)` means "`a` contains `b`".
  Then relate the borrowed type into the rest of `_d`'s slots:
  `relate(type(place) slots, slots(_d)[1..], variance = Covariant for &,
  Invariant for &mut)`.
- **C3 reborrow (supporting prefixes).** For the same `Ref`, walk the
  borrowed place's projections from the end towards the local; at each
  `Deref` whose prefix has type `&'r T` or `&'r mut T` (the prefix's outer
  slot `r`): `outlives(r, ρL)`. Stop after the first `Deref` of a *shared*
  borrow (data behind a `&` stays put as long as that `&` lives); continue
  through `&mut`. A `Deref` of `*T` stops the walk with no constraint (raw
  pointers are not tracked; the new borrow is free, R9).
- **C4 assignment / subtyping.** `Assign(p, Use(Copy(q) | Move(q)))`:
  `relate(slots(type(q) at q), slots(type(p) at p), Covariant)`. The slots of
  a projected place are a sub-range of the local's slots: compute with
  `slots_of_place(place)` (walk the type with the projections; `Deref` of
  `&'r T` drops slot `r` and continues into `T`'s slots; `Field(f)` of a
  struct maps the struct's region parameters into the field's type).
  Other rvalues produce types without regions except `Aggregate`, which
  relates each operand to its field's type (with the struct's region
  parameters being the destination's slots).
- **C5 call.** For `call f(args) -> dest`: instantiate every universal region
  of `f`'s signature with a fresh variable (`inst[U]`). For each argument:
  `relate(slots(arg), inst(param slots), Covariant)`. For each return slot
  `R` and each parameter `x` in `lives(f)` (3.6): for each slot `s` of `x`:
  `outlives(inst[s], inst[R])`. Then `relate(inst(return slots),
  slots(dest), Covariant)`. Runtime functions (`RtFn`) have no regions in
  their results (all return owned values or numbers), so only C1 applies to
  their borrowed arguments.
- **C6 well-formedness.** For every local whose type is `&'o T` with `T`
  containing slots `s…`: `outlives(s, 'o)` for each (a borrow cannot outlive
  what its target borrows). Same for parameters, as a *declared* relation
  between universals (it is always true at call sites by C6 there).
- **C7 return.** Covered by C4 on `_0 = …`, because `_0`'s slots are the
  return regions.

`relate(a, b, variance)` for corresponding slot lists:
- Covariant at a `&` slot: `outlives(a_i, b_i)`.
- Invariant (everything inside a `&mut`, and struct region parameters that
  are invariant, below): both `outlives(a_i, b_i)` and `outlives(b_i, a_i)`.
- Struct region parameter variance is computed once per struct: covariant
  if every field that mentions the parameter mentions it only in covariant
  positions (under `&`, not under `&mut`) and the field is not a `mut` field;
  invariant otherwise. (A `mut &string cur;` field can be overwritten
  through a shared path by S8, so it must be invariant, the same reason
  Rust's `Cell<&T>` is invariant.)

#### 3.5.4 Solving

```text
values[r] = C1 seeds (points where r's local is live)       for local regions
values[U] = all points + {end(U)}                           for universal U
values[ρL] = {}                                             for loan regions
repeat until no change:
    for each constraint outlives(a, b):
        values[a] |= values[b]          // points and end markers
```

Make it fast enough without cleverness: compute SCCs of the constraint graph
(edge b → a for `outlives(a, b)`), give each SCC one value, propagate in
topological order once. With no SCCs this is a single pass. Bodies are small
(hundreds of points, tens of regions); bitsets keep it trivial.

**Loan region value.** After solving, loan `L`'s region `values[ρL]` is the
set of points where some live value may hold the borrow. Note `ρL` is
reached from the destination slot through C2, so it contains every point
where `_d` (or anything it flowed into) is live.

#### 3.5.5 Loans in scope

Forward dataflow over loans (`BorrowsInScope`):

- **Gen** at the loan's own location (after the statement).
- **Kill, out of region.** Precompute, per loan, the locations where it goes
  out of scope: DFS forward over the CFG from `L.loc`; at each point, if the
  point is not in `values[ρL].points`, record "kill L here" and stop that
  branch. (A universal end marker in the value means the loan flows out of
  the function: it never goes out of region inside the body; that is what
  makes returning a borrow of a local an error at the local's
  `StorageDead`.)
- **Kill, overwritten base.** `Assign(p, …)` where some loan's place has `p`
  as a proper prefix *followed by a `Deref`* (the loan is through a borrow
  stored in `p`, and `p` now holds a different borrow): kill the loan (after
  the conflict check of the assignment, which ignores such loans).
- **Kill at StorageDead(l)**: loans of places rooted at `l` die with it
  (after the check, which reports them, 3.5.7).

#### 3.5.6 Accesses and conflicts

For every statement and terminator, enumerate its *accesses* in evaluation
order:

| Source | Access |
|---|---|
| `Copy(q)` operand | Read(q) |
| `Move(q)` operand | Move(q) |
| `Ref(Shared, q)` | ShareBorrow(q) |
| `Ref(Mut, q)` | MutBorrow(q) |
| `DeepCopy(q)`, `FakeRead(q)`, `RawRef(_, q)` | Read(q) |
| destination of `Assign`/`Call` | Write(p) |
| `Drop(p)` | Drop(p) |
| `StorageDead(l)` | Dead(l) |
| `Return` | Dead(every local except `_0`) (locals still in storage at return; the builder emits their StorageDead before `Return`, so this is a backstop) |

For each access to place `q` at point `P`, for each loan `L` in scope at `P`
(excluding a loan created at `P` itself), if `places_conflict(L.place, q,
access)` and `kind_conflict(L.kind, access)`, report (3.5.7) and do not
report again for the same `(L, q's move path)` pair.

`kind_conflict`:

| Loan \ access | Read | ShareBorrow | MutBorrow | Write | Move | Drop | Dead |
|---|---|---|---|---|---|---|---|
| Shared | ok | ok | **E0302** | **E0302** | **E0305** | **E0306** | **E0306** |
| Mut | **E0304** | **E0304** | **E0303** | **E0302** | **E0305** | **E0306** | **E0306** |

A method call that needs `&mut self` appears as a MutBorrow of the receiver
(B3), so `xs::push(9)` against a shared loan is E0302 with the label
"change attempted here", matching the SPEC example.

`places_conflict(borrowed, accessed, access)`, field-disjointness:

```text
if borrowed.local != accessed.local: return false
for i in 0..min(len(borrowed.proj), len(accessed.proj)):
    match (borrowed.proj[i], accessed.proj[i]):
        (Field(a), Field(b)) if a != b: return false      // disjoint fields
        (Field(a), Field(b)):          continue
        (Deref, Deref):                continue
        _: unreachable (same prefix ⇒ same type)
// one is a prefix of the other: they overlap ...
if access is Write and accessed is a proper prefix of borrowed
   and borrowed.proj[len(accessed.proj)] == Deref:
    return false        // overwriting a borrow variable does not touch what it pointed to (it kills the loan, 3.5.5)
return true
```

So `&mut p.name` and `p.hp = 3` do not conflict; `&p.name` and
`p = other` do; `&p` and `p.hp = 3` do. A method call borrows the whole
receiver, so `p::heal()` conflicts with a loan of `p.name` (no partial
borrows across calls).

Shared borrows of the same place never conflict, and copying a `&T` is a
Read of the borrow variable, which never conflicts with the loan it holds.

**Two-phase borrows (pending Q4).** If the owner allows arguments that read
what the receiver changes (`s::insert(s::len(), "!")`-style calls on a
mutating method), mark the receiver borrow and explicit `&mut` call
arguments `Mut { two_phase: true }`. Such a loan is *reserved* from its
creation until its *activation*: the first use of the temp that holds it
(always the call, by construction). While reserved it conflicts like a
Shared loan; at activation, check a MutBorrow of its place against all
other loans in scope. If Q4 says no, `two_phase` is always false and the
example is E0304. Implement the flag either way (it is ten lines); the
builder just never sets it.

**Reborrows.** Passing on a borrow (Q3) lowers to `_t = &mut (*r)` (or
`&(*r)`): a new loan of the place `*r`, with constraint C3 tying it to `r`'s
region. While `_t` is live, `r` itself is MutBorrow'ed through its deref, so
using `r` directly is E0304; after the call `_t` is dead and `r` is usable
again. This is what makes `&mut` parameters usable more than once in a body.

#### 3.5.7 Explaining an error

For conflict errors the SPEC style needs three labels: where the place is
borrowed (the loan's span), the access (primary), and **where the borrow is
still used later**. Find the later use:

```text
explain_later_use(L, P):
    BFS over points forward from P, staying inside values[ρL].points
    at each point Q: for each local l used at Q (liveness gen set)
        if some region slot of l, after solving, contains P and
           ρL's value ⊆ that slot's value or the slot ⊇ ρL by the constraint graph:
            return (l, span of the use at Q)
    if values[ρL] contains a universal end marker: return Returned(span of the `return`)
```

Labels: `"`x` is borrowed here"` at `L.span`; the access at its span; `"the
borrow is still used here"` at the use, or `"the borrow is returned here"`.
The `help:` is from the table in section 4 and names the local found (`finish
using `first` before changing `xs``).

For E0306 (owner dies first), the primary label is the `}` (span of the
`StorageDead`/`Drop`): "`x` is dropped here, at the end of its block".

#### 3.5.8 Worked example 3

```cpp
in io;

none main() {
    mut string name = "Illy";
    &string first = &name;
    name = "Sam";
    io::outln(first);
}
```

MIR (points numbered):

```text
bb0:
 P0  StorageLive(_1)
 P1  _1 = const "Illy"
 P2  StorageLive(_2)
 P3  _2 = &_1                      // loan L0, region ρL0; _2: &'r2 string
 P4  Drop(_1)                      // B6: drop the old value of name
 P5  _1 = const "Sam"
 P6  call OutStr(copy _2, true) -> bb1
bb1: ...
```

Liveness: `_2` is live at P4, P5, P6 (used at P6). C1: `r2 ⊇ {P4,P5,P6}`.
C2: `outlives(ρL0, r2)` so `ρL0 ⊇ {P4, P5, P6}`. Loans in scope at P4: L0.
P4 is a `Drop(_1)` under a Shared loan of `_1`. A scope-end drop there would
be E0306, but this drop is the one B6 emits *for an assignment*, so it must
be reported as the assignment. The builder therefore tags every `Drop` with a
`reason: DropReason { ScopeEnd, Reassign }` (dropped again by elaboration),
and the checker treats a `Reassign` drop as a Write → E0302:

```text
error[E0302]: cannot change `name` while it is borrowed
  --> main.drh:6:5
   |
 5 |     &string first = &name;
   |                     ----- `name` is borrowed here
 6 |     name = "Sam";
   |     ^^^^ change attempted here
 7 |     io::outln(first);
   |               ----- the borrow is still used here
   |
 help: finish using `first` before changing `name`
```

Delete line 7 and `_2` is dead after P3; `ρL0 = {}`; the loan is killed at
P4 before any access; no error. That is the non-lexical part.

Add `reason: DropReason` to `StatementKind::Drop` in 2.3 accordingly; the
access table in 3.5.6 reads "Drop(p)" as Write(p) when the reason is
`Reassign`.

### 3.6 Lifetimes in signatures, and `lives`

`mir/lives.rs`.

#### 3.6.1 What a signature says

```rust
pub enum LivesSig {
    NoBorrowResult,                       // the return type has no region slots
    Declared(Vec<ParamIdx>),              // written `lives a, b` (self is ParamIdx 0 in methods)
    Inferred(Vec<ParamIdx>),              // computed below
    Unknown,                              // the body had errors: callers assume nothing and report nothing
}
```

The relations a call site may assume (C5): each return region is outlived by
every slot of every listed parameter. Nothing else.

#### 3.6.2 Inference inside one body

Run the borrow checker's region solving (3.5.4) for the body with
`lives(self) = []` provisionally (the return regions are universal and no
parameter region is declared to outlive them). Then, the *universal region
check*:

```text
for each universal U (a parameter slot):
    for each end(V) in values[U].ends with V != U:
        if V is a return region:  required.insert(param_of(U))
        elif V is another parameter's slot and not implied by C6:
            error E0320 (a borrow from one parameter is stored into another)
for each loan region ρL with some end(V) in values[ρL].ends:
    // a borrow of something local flows out through the return value
    (reported as E0307 by the loans-in-scope check at that local's StorageDead,
     because the loan never goes out of region; the explanation says "returned")
```

`required` is the set of parameters the returned borrow derives from.

#### 3.6.3 Across the call graph

Callers need callees' `LivesSig`. Process functions in **post-order of the
call graph's SCCs** (Tarjan over `Callee::Func` edges): callees first.

- A function whose return type has no slots: `NoBorrowResult`; check its body
  once.
- A function with a written `lives` clause: use it as-is for callers;
  check the body: `required ⊆ declared` else E0309 (naming the extra
  parameter and labelling the return that causes it, 3.6.5).
- A function without a clause, alone in its SCC: compute `required`; then
  (pending Q9, the rule from PLAN item 6 is shown):
  - `|required| == 0` and the result has slots: the result borrows nothing
    that outlives the call → the body necessarily returns a borrow of a local
    or a temporary → E0307 has already been reported by the loan check;
    record `Inferred([])`.
  - `|required| == 1` → `Inferred(required)`.
  - `|required| >= 2` → E0308 (missing `lives`), record
    `Inferred(required)` so callers are still checked sensibly.
- An SCC of mutually recursive functions without clauses: start every
  member at `Inferred([])`; repeat { check every member's body with the
  current sigs, errors suppressed; set each member's sig to its `required` }
  until no sig changes (sets only grow; at most `Σ params` rounds); then one
  final round with errors reported.

#### 3.6.4 `lives` on structs

`struct Parser lives src { &string src; uint32 pos; }` declares the struct's
**region parameters**: one per name after `lives`. What exactly the name
denotes is **Q10**; the machinery handles any number of region parameters,
each attached to one or more borrow fields:

- Every field type's borrow slots must be attached to a region parameter.
  A struct with a borrow field and no `lives` clause → E0314. A `lives` name
  attached to no field → E0314 ("`lives x` names nothing this struct
  borrows").
- A field whose type is another struct with region parameters
  (`Lexer lives src { Parser p; }`) maps that struct's parameters to the
  outer struct's.
- Variance per parameter: 3.5.3.
- A struct value's region slots (2.4) are its parameters, in `lives` order.

A function parameter `Parser p` then has one universal region per parameter
of `Parser`, named `p.src` in messages.

#### 3.6.5 Explaining `lives` errors

For E0308/E0309/E0307 the message must show *which return* borrows from
*which parameter*. Record, for every constraint, its origin location. To
explain "`U` must outlive return region `R`": BFS backwards over the
constraint graph from `R` to `U` (edges `b → a` for `outlives(a, b)`); on the
found path, the constraint whose origin is an assignment to `_0` gives the
`return` span ("returns a borrow of `b` here"); the constraint whose origin
is a `Ref` gives the borrow span.

#### Worked example 4

```cpp
&string longest(&string a, &string b) {
    if (a::len() > b::len()) { return a; }
    return b;
}
```

Universals: `ρa`, `ρb`, return `ρR`. `return a` lowers to `_0 = copy _1`:
C4 gives `outlives(ρa, ρR)`; `return b`: `outlives(ρb, ρR)`. After solving,
`end(ρR) ∈ values[ρa]` and `∈ values[ρb]` → `required = {a, b}` → E0308:

```text
error[E0308]: `longest` returns a borrow, but which parameter it borrows from depends on the path
  --> main.drh:1:10
   |
 1 | &string longest(&string a, &string b) {
   |         ^^^^^^^ the result borrows from `a` or `b`
 2 |     if (a::len() > b::len()) { return a; }
   |                                ------ returns a borrow of `a` here
 3 |     return b;
   |     ------ returns a borrow of `b` here
   |
 help: write `lives a, b` after the parameters: `&string longest(&string a, &string b) lives a, b {`
```

With `lives a, b`: callers get `outlives(inst[ρa], inst[ρR])` and
`outlives(inst[ρb], inst[ρR])`, so the result keeps both arguments' loans
alive while it is used.

### 3.7 Inferring `self`'s borrow mode

`ownership/self_mode.rs`, on the HIR, before MIR is built (the builder needs
the mode to emit `&` or `&mut` for receivers, B3).

```rust
pub enum SelfMode { Shared, Mut(MutReason) }
pub struct MutReason { pub span: Span, pub kind: MutKind }
pub enum MutKind {
    Writes(String),        // "self.size" (the place written)
    LendsMut(String),      // `&mut self.items`
    Calls(FuncId),         // calls a method that changes `self`
}
```

**Direct evidence** that a method needs `&mut self`, found by walking its
body. A *self place* is a place expression whose root is `self`, with field
projections (and derefs of borrow fields).

1. An assignment, compound assignment or `++`/`--` whose target is a self
   place, **unless** the place reaches a `mut` field before any `Deref`
   (S8/R1; if Q5 decides that `mut` fields are not writable through `&`,
   remove this exception). A write *through* a `&mut` field (`*self.out = …`)
   counts: a `&mut` stored in `self` can only be used through a unique path.
2. `&mut <self place>` anywhere (as a call argument or a binding).
3. A method call whose receiver is a self place and whose callee is
   currently `Mut` (including built-in mutating methods, none in step 3).

Moving out of a self place is not evidence; it is E0310 at MIR time (or a
consuming method, Q8).

**Fixpoint across methods** (mutual recursion, and methods of one struct
calling methods of a field's struct):

```text
mode[m] = Shared for every method m
worklist = all methods
callers_on_self[m] = methods that call m with a self-place receiver
while let Some(m) = worklist.pop():
    if mode[m] == Shared and (direct_evidence(m) or a call on self to some m2 with mode[m2] == Mut):
        mode[m] = Mut(reason)          // reason: the first evidence in source order
        worklist.extend(callers_on_self[m])
```

This is the least fixpoint: two methods that only call each other stay
`Shared`. It terminates because modes only go from Shared to Mut.

**Using the result.**

- The MIR builder types `self` as `&Self` or `&mut Self` and emits receiver
  borrows accordingly.
- E0312/E0311 at a call site of a `Mut` method on a non-changeable receiver
  add a note: "`heal` changes `self`: it writes `self.hp` (player.drh:7:9)".
  For `Calls`, follow the chain to the first non-`Calls` reason and print
  it: "`grow` changes `self`: it calls `resize`, which writes `self.cap`".
- The language server (step 8) shows the mode as an inlay hint; keep the
  `Vec<Option<SelfMode>>` in `Checked`.

> **LOOK FIRST (b):** how (b) passes `self` today (always a pointer, as
> PLAN's "by-pointer ABI" suggests) and whether its typeck already rejects
> writes to non-`mut` fields of `self`. If it does, that check moves to MIR
> (3.8), because whether `self` is writable is now inferred, not fixed.

### 3.8 Mutability

`mir/mutability.rs`, no dataflow except `EverInit` (3.1.6).

```rust
enum Changeable { Yes, NoBinding(Local), NoParam(Local), ThroughShared { borrow: Place }, OpenReassign(Local) }

fn changeable(place: &Place, body: &Body) -> Changeable {
    // walk from the end
    match place.proj.last() {
        None => match body.locals[place.local].mutability() {
            Mut | Open => Yes,
            Fixed if param => NoParam(place.local),
            Fixed => NoBinding(place.local),
        },
        Some(Field(f)) if field_is_mut(parent_type, f) => Yes,   // S8; Q5 may restrict this through `&`
        Some(Field(_)) => changeable(parent),
        Some(Deref) => match type(parent) {
            Ref { mutable: true, .. } => Yes,   // the &mut itself need not be in a `mut` binding
            Ref { mutable: false, .. } => ThroughShared { borrow: parent },
            Ptr { .. } => Yes,                  // unsafe; 3.9 checks the context
        },
    }
}
```

Checks:

| Operation | Requirement | Error |
|---|---|---|
| `Assign(_l, …)` whole local | `Mut`; or first assignment of a `Fixed`/`Open` non-param (3.1.6) | E0205 (all variants) |
| `Assign(p.f…, …)` (write with projections) | `changeable(p.f…) == Yes`; for an `Open` root: pending Q6 | E0205 (binding root), E0311 (through `&`) |
| `Ref(Mut, p)` | `changeable(p) == Yes` | E0312 (binding root), E0311 (through `&`) |
| receiver of a `Mut` method | as `Ref(Mut, …)`, but message names the method and the self-mode reason | E0312 / E0311 |
| `IncDec`, compound assignment | as assignment of the place | E0205 / E0311 |

`mut open` on one binding: W0323 at the parse/typeck level (it is syntax).

### 3.9 `unsafe`, `*T`, and file flags

`src/flags.rs` (new; step 7 reuses it):

```rust
pub struct FileFlags { pub entries: Vec<FileFlag>, pub unsafe_allowed: bool }
pub struct FileFlag { pub key: String, pub value: String, pub span: Span }
pub fn parse(src: &str, diags: &mut Vec<Diagnostic>) -> FileFlags;
```

- Scan lines from the start of the file. A line that starts with `//--!$`
  is a flag line: `key: value`, key `[a-z_][a-z0-9_]*` (and `dep NAME` for
  step 7's `//--!$dep json: 1.2`), value to end of line, trimmed. Keep
  scanning while lines are flag lines; the first other line (including a
  blank line, pending nothing: the SPEC says "at the very top") ends the
  block. A shebang line `#!…` as the very first line is skipped first
  (step 7 needs it; it does not hurt now).
- A `//--!$` line after the top block → E0318 ("file flags must be at the
  very top"). The lexer sees these as ordinary comments; `flags::parse`
  finds them by scanning all lines that start with `//--!$` after the block.
- `unsafe: true` / `unsafe: false`; any other value → E0318. Unknown keys →
  W0319 (step 7 will add many; warning so old files keep working).

Checks (pass `unsafety`, MIR, using `Statement::unsafe_ctx` and
`Body::unsafe_flag`):

| Thing | Needs |
|---|---|
| the `unsafe { }` block itself | the file flag, else E0317 at the `unsafe` keyword (once per block) |
| writing the type `*T` anywhere | the file flag, else E0317 (typeck reports it, at the type) |
| `Deref` of a `*T` place (read or write) | `unsafe_ctx`, else E0316 |
| making a pointer (`RawRef`) and copying one | pending Q14 |

Borrow checking ignores places with a `Deref` of a `*T` in their prefix
(they create no loans that matter: C3 stops; conflicts are not checked for
such accesses), and moving out through one is E0310 (R9).

---
## 4. Diagnostics

**Every text in this section is a DRAFT** for the owner to approve (PLAN step
3 "Open"). E0301 and E0302 are fixed by the SPEC (code and E0302's layout);
their wording beyond the SPEC example is also a draft.

Conventions, matching SPEC §6 and today's `typeck.rs` messages:
- message: lower case, what went wrong, names in backticks;
- primary label (`^^^`): the operation that is rejected;
- secondary labels (`---`): where the borrow/move happened, where it is
  still used;
- `help:` a concrete fix, quoting code the user can type;
- a place is named the way the user writes it: `p.name`, `self.hp`, and a
  deref of a borrow by the borrow's name (R4); a temporary as "this value";
- every new code gets an entry in `src/diag/codes.rs` with a title and an
  `explain` text (draft explain texts are given as one line here; write them
  out in the style of the existing ones, with a short example).

The whole `03xx` range is used, also for the two non-ownership codes this
step needs (file flags, `mut open`), so that parallel branches adding
`01xx`/`02xx` codes cannot collide. Renumber if the owner prefers.

| Code | Title | Raised by |
|---|---|---|
| E0301 | used after it moved | moves (3.1.3) |
| E0302 | changed while borrowed | borrowck |
| E0303 | two `&mut` borrows at once | borrowck |
| E0304 | used while borrowed with `&mut` | borrowck |
| E0305 | moved while borrowed | borrowck |
| E0306 | does not live long enough | borrowck |
| E0307 | returns a borrow of a local | borrowck / lives |
| E0308 | missing `lives` | lives |
| E0309 | `lives` does not match the body | lives |
| E0310 | cannot move out of a borrow | moves |
| E0311 | cannot change through a `&` borrow | mutability |
| E0312 | cannot lend `&mut` from an immutable binding | mutability |
| E0313 | assigning part of a moved value | moves |
| E0314 | struct borrows without `lives` | typeck (struct decls) |
| E0315 | cannot copy a value holding `&mut` | moves |
| E0316 | raw pointer used outside `unsafe { }` | unsafety |
| E0317 | `unsafe` without the file flag | typeck / unsafety |
| E0318 | invalid file flag | flags |
| W0319 | unknown file flag | flags |
| E0320 | borrow stored into another parameter | lives |
| E0321 | missing `&` / `&mut` at the call site | typeck |
| W0322 | `copy` of a value that copies anyway | typeck (pending Q11) |
| W0323 | `mut open` is redundant | parser/typeck |

Existing codes whose reporting moves to MIR with unchanged texts: E0105,
E0205, E0207. E0205 gains one variant for `open` (below).

### E0301 used after it moved

```text
error[E0301]: `name` is used after it moved
  --> main.drh:6:15
   |
 5 |     greet(name);
   |           ---- `name` moved here
 6 |     io::outln(name);
   |               ^^^^ used here, after the move
   |
 help: if `greet` should not take ownership, give it a `&string` parameter; or pass a copy: `greet(copy name)`
```

The help depends on what moved it: a call to a user function → the text
above (the first part only if the parameter's type is not already a borrow);
an assignment `string b = name;` → "to keep both, write `string b = copy
name;`"; a `return` cannot precede a use.

Variants (same code):
- in a loop: secondary label "`name` moved here, in the previous time round
  the loop"; help "move it only once (for example after the loop), or pass
  `copy name`".
- partial: message "`p` is used after `p.name` moved"; label at the move
  "`p.name` moved here"; help "give `p.name` a new value first, or move a copy:
  `copy p.name`".
- maybe moved: label "`name` moved here, on some paths" when the use is
  reachable without passing the move.

Explain: values of types that do not copy automatically move on assignment,
pass and return; the source cannot be used again until given a new value.

### E0302 cannot change while borrowed (SPEC layout)

```text
error[E0302]: cannot change `name` while it is borrowed
  --> main.drh:6:5
   |
 5 |     &string first = &name;
   |                     ----- `name` is borrowed here
 6 |     name = "Sam";
   |     ^^^^ change attempted here
 7 |     io::outln(first);
   |               ----- the borrow is still used here
   |
 help: finish using `first` before changing `name`
```

For a mutating method the primary label sits on the method name
(`xs::push(9)` → `^^` under `xs`, as in the SPEC; use the receiver span). If
the later use is a return: label "the borrow is returned here", help
"change `name` before borrowing it, or return a copy".

### E0303 two `&mut` at once

```text
error[E0303]: `p` is already borrowed with `&mut`
  --> main.drh:6:21
   |
 5 |     &mut Player a = &mut p;
   |                     ------ first `&mut` borrow here
 6 |     &mut Player b = &mut p;
   |                     ^^^^^^ second `&mut` borrow here
 7 |     heal(a);
   |          - the first borrow is still used here
   |
 help: only one `&mut` borrow of `p` can be in use at a time; finish using `a` before borrowing `p` again
```

### E0304 used while borrowed with `&mut`

```text
error[E0304]: cannot use `p.hp` while `p` is borrowed with `&mut`
  --> main.drh:6:15
   |
 5 |     &mut Player a = &mut p;
   |                     ------ `p` is borrowed with `&mut` here
 6 |     io::outln(p.hp::text());
   |               ^^^^ used here
 7 |     heal(a);
   |          - the `&mut` borrow is still used here
   |
 help: read `p.hp` before borrowing `p` with `&mut`, or read it through `a`: `a.hp`
```

(A shared borrow attempt reads "cannot borrow `p` while it is borrowed with
`&mut`", label "borrowed again here".)

### E0305 moved while borrowed

```text
error[E0305]: cannot move `name` while it is borrowed
  --> main.drh:6:11
   |
 5 |     &string first = &name;
   |                     ----- `name` is borrowed here
 6 |     take(name);
   |          ^^^^ moved here
 7 |     io::outln(first);
   |               ----- the borrow is still used here
   |
 help: finish using `first` before moving `name`, or move a copy: `take(copy name)`
```

### E0306 does not live long enough

```text
error[E0306]: `local` does not live long enough
  --> main.drh:8:5
   |
 6 |         string local = "x";
 7 |         keep = &local;
   |                ------ `local` is borrowed here
 8 |     }
   |     ^ `local` is dropped here, at the end of its block
 9 |     io::outln(keep);
   |               ---- the borrow is still used here
   |
 help: declare `local` outside the block, so it lives as long as `keep` is used
```

For a struct holding a borrow (`Parser q = &text, 0;` escaping `text`'s
block) the borrow label names the field: "`text` is borrowed here, and kept
in `q.src`".

### E0307 returns a borrow of a local

```text
error[E0307]: cannot return a borrow of `s`, which belongs to this function
  --> main.drh:3:12
   |
 2 |     string s = "hi"::upper();
   |            - `s` is created here and dropped when `shout` returns
 3 |     return &s;
   |            ^^ the borrow is returned here
   |
 help: return the string itself: change the return type to `string` and write `return s;`
```

For a temporary: "cannot return a borrow of a temporary value"; help the
same. For a function returning a borrow with no borrowed parameter at all,
the note line: "`shout` takes no borrows, so a borrow it returns can only
point at something it owns".

### E0308 missing `lives`

As in worked example 4 (3.6). Pending Q9, which may change when this fires.

### E0309 `lives` does not match the body

```text
error[E0309]: `pick` returns a borrow of `b`, but its `lives` names only `a`
  --> main.drh:3:12
   |
 1 | &string pick(&string a, &string b) lives a {
   |                                    ------- says the result borrows only from `a`
 2 |     ...
 3 |     return b;
   |            ^ returns a borrow of `b` here
   |
 help: add `b` to the list: `lives a, b`
```

Variant: "`lives` names `c`, which is not a parameter holding a borrow"
(label on the name; help lists the parameters that hold borrows).

### E0310 cannot move out of a borrow

```text
error[E0310]: cannot move `p.name` out of a borrow
  --> main.drh:2:12
   |
 1 | string nameOf(&Player p) {
   |               --------- `p` is a `&` borrow
 2 |     return p.name;
   |            ^^^^^^ this would move `p.name` out of a value `nameOf` does not own
   |
 help: return a copy: `return copy p.name;`
```

For `self`: secondary label "`self` is borrowed by `nameOf`" (pending Q8).
For a raw pointer: "cannot move out through a raw pointer"; same help.

### E0311 cannot change through a `&` borrow

```text
error[E0311]: cannot change `p.hp` through a `&` borrow
  --> main.drh:2:5
   |
 1 | none hurt(&Player p) {
   |           --------- `p` is a `&` borrow, which only reads
 2 |     p.title = "hurt";
   |     ^^^^^^^ change attempted here
   |
 help: take a `&mut` borrow: `none hurt(&mut Player p)`
```

Method variant: "cannot call `heal` through a `&` borrow: `heal` changes
`self`", with `note: `heal` changes `self`: it writes `self.title`
(player.drh:7:9)` (3.7). Inside a method whose `self` is inferred `&self`
this cannot happen (any change makes it `&mut self`).

### E0312 cannot lend `&mut` from an immutable binding

```text
error[E0312]: cannot lend `xs` as `&mut`: it is neither `mut` nor `open`
  --> main.drh:3:10
   |
 2 |     string xs = "a";
   |            -- declared here without `mut` or `open`
 3 |     grow(&mut xs);
   |          ^^^^^^^ `&mut` borrow here
   |
 help: declare it `open string xs` (its contents may change) or `mut string xs` (it may also be reassigned)
```

Method variant: "cannot call `heal` on `p`: `heal` changes `self`, and `p`
is neither `mut` nor `open`", plus the self-mode note.

### E0205 (existing) new `open` variant

```text
error[E0205]: cannot reassign `xs`: it is `open`, not `mut`
  --> main.drh:3:5
   |
 2 |     open string xs = "a";
   |                 -- declared `open`: its contents may change, but it is never reassigned
 3 |     xs = "b";
   |     ^^ reassigned here
   |
 help: declare it `mut string xs` to allow reassigning
```

### E0313 assigning part of a moved value

```text
error[E0313]: cannot give `p.hp` a value: `p` has moved
  --> main.drh:4:5
   |
 3 |     take(p);
   |          - `p` moved here
 4 |     p.hp = 5;
   |     ^^^^ assigned here
   |
 help: give `p` a whole new value instead: `p = ...;`
```

### E0314 struct borrows without `lives`

```text
error[E0314]: `Parser` holds a borrow, so it must say what it borrows from
  --> main.drh:1:8
   |
 1 | struct Parser { &string src; uint32 pos; }
   |        ^^^^^^   ----------- this field is a borrow
   |
 help: add `lives src`: `struct Parser lives src { ... }`
```

Variant: "`lives x` names nothing `Parser` borrows" (the exact form depends
on Q10).

### E0315 cannot copy a value holding `&mut`

```text
error[E0315]: cannot copy `w`: it holds a `&mut` borrow
  --> main.drh:4:16
   |
 4 |     Writer v = copy w;
   |                ^^^^^^ copy attempted here
   |
 note: `Writer.out` is `&mut string`; a copy would be a second `&mut` to the same string
 help: borrow `w` instead: `&Writer v = &w;`
```

### E0316 raw pointer outside `unsafe { }`

```text
error[E0316]: reading through a raw pointer needs an `unsafe { }` block
  --> main.drh:5:15
   |
 5 |     uint32 v = *p;
   |                ^^ `p` is `*uint32`
   |
 help: wrap it: `unsafe { v = *p; }`, and check that `p` points at a live `uint32`
```

### E0317 `unsafe` without the file flag

```text
error[E0317]: `unsafe` is not enabled in this file
  --> main.drh:4:5
   |
 4 |     unsafe {
   |     ^^^^^^ needs the file flag
   |
 help: add `//--!$unsafe: true` as the first line of the file
```

The same text for a `*T` type written in a file without the flag (label
"raw pointer type here").

### E0318 / W0319 file flags

- E0318 "file flags must be at the very top of the file" (label "this flag
  comes after code"; help "move it above the first line of code, with the
  other flags").
- E0318 "`unsafe` must be `true` or `false`, found `yes`".
- W0319 "unknown file flag `unsfe`" (help, if a known key is within edit
  distance 2: "did you mean `unsafe`?").

### E0320 borrow stored into another parameter

```text
error[E0320]: cannot store a borrow of `b` inside `a`
  --> main.drh:2:5
   |
 1 | none link(&mut Parser a, &string b) {
 2 |     a.src = b;
   |     ^^^^^^^^^ `a.src` would point at `b`'s string after `link` returns
   |
 note: nothing in `link`'s signature says `b` lives as long as what `a` borrows
 help: return a new `Parser` instead (`Parser link(&string b) { ... }`)
```

(Drachm has no syntax to state this relation; if the owner wants one, that
is a new open question, not part of step 3.)

### E0321 missing `&` / `&mut` at the call site

```text
error[E0321]: `grow` takes `&mut string` here, so the call must say so
  --> main.drh:3:10
   |
 3 |     grow(xs);
   |          ^^ this passes `xs` itself
   |
 help: write `grow(&mut xs)`
```

For `&` parameters this depends on Q2 (if shared borrows are automatic, the
`&` variant never fires).

### W0322 / W0323

- W0322 (pending Q11): "`copy` does nothing here: `uint32` always copies";
  help "remove `copy`".
- W0323: "`mut open` says the same as `mut`" (label on `open`; help "write
  `mut string xs`"). SPEC §2 decides that this is a warning.

---

## 5. Open language questions

These are user-visible behaviours step 3 needs that the SPEC does not settle.
**Do not decide them.** Ask the owner a few at a time with
`AskUserQuestion` (code previews in ```cpp fences), record the answers in
`docs/SPEC.md`, the design doc and this file. The options are in no order of
preference. Where a work package needs an answer that has not come yet, the
package says what to build meanwhile (usually: report E0901 for the undecided
case).

### Q1. Reading and writing through a borrow

With `&mut uint32 r = &mut n;` and `&uint32 q = &n;`: what does arithmetic or
assignment look like? (`.` and `::` look through borrows already, R4.)

- **A. Explicit `*`**, as for raw pointers: `uint32 m = *q + 1; *r = 5;`
- **B. Automatic for reads, `*` for writes**: `uint32 m = q + 1;` (a borrow
  reads as its value wherever a value is expected) but `*r = 5;`, since
  `r = …` reassigns the borrow itself.
- **C. A word for writing through** (original): reads are automatic, writes
  say where they go: `set r to 5;` vs `r = &other;` to point elsewhere.
- **D. Borrows are never reassigned** (original): a borrow binding always
  means its target, so `r = 5;` writes through, and pointing it elsewhere
  needs a new binding.

```cpp
&mut uint32 r = &mut n;
*r = 5;          // A, B
set r to 5;      // C
r = 5;           // D
```

### Q2. Does the call site write `&` for shared borrows?

S7 shows `push(&mut xs, 1)` and PLAN records "explicit `&mut` at call
sites" as decided. For shared borrows, R2 already covers builtins; this is
about user functions such as `none show(&Player p)`.

- **A. Always written**: `show(&p);` Every borrow is visible at the call.
- **B. Only `&mut` is written**: `show(p);` borrows automatically because the
  parameter is `&Player`; only changes are marked at the call.
- **C. Both accepted**: `show(p)` and `show(&p)` mean the same.
- **D. Read-only parameters by keyword** (original): a parameter written
  `string name` with a `reads` marker, e.g. `none show(reads Player p)`, is
  borrowed implicitly and cannot be changed; `&Player` remains for borrows
  that are stored or returned.

```cpp
none show(&Player p) { io::outln(p.name); }
show(&p);    // A (C)
show(p);     // B (C)
```

### Q3. Passing on a borrow you hold

Inside `none fill(&mut string s)`, calling `none add(&mut string t)`:

- **A. Write `&mut` again**: `add(&mut s);` A borrow of a borrow is the same
  borrow passed on (there are no `&&string` types).
- **B. Pass it plainly**: `add(s);` passes the borrow on for the call; `s`
  stays usable afterwards.
- **C. Both accepted.**
- **D. `lend` keyword** (original): `add(lend s);` makes "I'm handing my
  borrow on for this call" visible and distinct from creating a new one.

### Q4. Can a call's arguments read what the receiver changes?

```cpp
open string s = "ab";
s::push(s::len()::text());   // receiver is borrowed `&mut` first (left to right)
```

(`push` stands for any mutating method; lists arrive in step 6.)

- **A. Allowed**: the `&mut` borrow of the receiver only starts when the call
  itself starts (the arguments are evaluated first, while it is "reserved").
- **B. Error E0304**: write `string n = s::len()::text(); s::push(n);`
- **C. Allowed only for Copy results** (original): arguments may read the
  receiver if what they produce copies (numbers, bools), never if they
  produce a borrow into it.

### Q5. `mut` fields reached through a `&` borrow

S8 says a `mut` field "can always be changed, even on an immutable value". Is
that also true through a shared `&` borrow (`&self` methods, `&Player`
parameters)? It matters for soundness: if `p.name` is a `mut string` and
someone holds `&p.name` while a `&self` method replaces `self.name`, the
borrow points at freed memory.

- **A. Yes, for every type, but a `mut` field cannot be borrowed** (only read
  by copy or `copy`): `&p.name` is an error when `name` is `mut`.
- **B. Yes, only for fields of Copy types** (numbers, bools, small structs); a
  `mut string` field needs a `&mut` path like any other field.
- **C. Only on owned values**: a `mut` field is writable on an immutable
  *binding*, but through a `&` borrow it is read-only like the rest.
- **D. Yes, and borrows of `mut` fields are short** (original): `&p.hp` of a
  `mut` field is allowed only within one statement (it may be passed to a
  call but not kept in a binding).

This decides 3.7 rule 1's exception and 3.8's `Field(f) if mut` arm, and
whether C may get `restrict` on `&` parameters (7.3).

### Q6. What an `open` binding allows

S1: `open` = "contents may change (mutating methods, `&mut`), never
reassigned". Not settled:

- field writes: `open Player p = …; p.hp = 3;`
  - **A.** allowed (changing a field is changing contents);
  - **B.** not allowed (only methods and `&mut` change an `open` value);
- `open` parameters: `none fill(open string s)`
  - **A.** allowed (the function owns `s`, may change but not reassign it);
  - **B.** not allowed (parameters are `mut` or nothing, S3);
- an `open` binding declared without a value (`open string s;`) assigned once
  later:
  - **A.** allowed like an immutable binding (S2);
  - **B.** not allowed.

### Q7. Partial moves (decided in 3.1.4; please confirm)

This spec allows moving a field out of a struct you own, which makes the
struct unusable as a whole until the field gets a new value; it forbids
moving out of borrows. Alternatives:

- **A. As in 3.1.4.**
- **B. No partial moves**: `string n = p.name;` is an error; write
  `copy p.name`, or take the whole struct apart.
- **C. Take by word** (original): moving a field out must say so,
  `string n = take p.name;`, and `p` is unusable after; plain
  `string n = p.name;` is an error with a fix-it.

```cpp
Player p = "Illy", "Dr", 10;
string n = p.name;     // A: fine; p.title still usable, p as a whole not
io::outln(p.title);
```

### Q8. Methods that consume `self`

`self` is inferred `&self` or `&mut self` (S14). A method that moves out of
`self` (`string intoName() { return self.name; }`) cannot work with either.

- **A. Not allowed**: E0310; return `copy self.name`.
- **B. Inferred by-value `self`**: a method that moves out of `self` takes it
  by value; calling it moves the receiver (`p::intoName()`, then `p` is
  unusable).
- **C. A marker on the method** (original): `string intoName() takes self {
  … }` (wordy clause after the parameters, like `lives`), so consuming a
  receiver is always visible in the signature.

### Q9. When `lives` is required on a function

PLAN item 6 reads as: inferred from the body when the returned borrow comes
from exactly one parameter, required when it comes from several. Options:

- **A. Body-derived, as PLAN reads** (3.6.3): inferred for one, E0308 for
  several.
- **B. Signature rule**: inferred only when exactly one parameter holds a
  borrow; otherwise `lives` is always required, whatever the body does
  (changing a body never changes the signature callers see).
- **C. Always inferred**: `lives` is optional documentation, checked when
  written.
- **D. Inferred inside a module, required on exported functions** (original):
  functions listed in `out` must write `lives` (their contract is public);
  private ones never need it.

The inference algorithm is the same for all four; only when E0308 fires
differs.

### Q10. What a struct's `lives` names

`struct Parser lives src { &string src; }` uses one name that is also the
field name. With several borrow fields:

- **A. Field names**: `lives` lists exactly the borrow fields; each may borrow
  from a different place. `struct Pair lives a, b { &string a; &string b; }`
- **B. One source per struct**: `lives` names one thing all its borrow fields
  borrow from; `struct Tokens lives text { &string all; &string rest; }`
- **C. Named sources with fields assigned to them** (original):
  `struct Pair lives left, right { &string a from left; &string b from right; }`

And, for functions: may `lives` point into a struct parameter's sources?

- **i.** only whole parameters: `lives p` (the result keeps the borrow of
  `p` itself alive, even if it came from the text `p` borrows);
- **ii.** also paths (original): `&string word(&Parser p) lives p.src` (the
  result lives as long as the text, not as long as the borrow of the parser).

### Q11. `copy` on borrows and on values that copy anyway

- On a borrow `&string r`: `string s = copy r;`
  - **A.** copies the string it points at (gives an owned `string`);
  - **B.** copies the borrow (gives another `&string`);
  - **C.** error: write `copy *r` (or whatever Q1 says).
- On a Copy value (`copy n` with `uint32 n`):
  - **A.** silent;
  - **B.** warning W0322 "`copy` does nothing here";
  - **C.** error.

### Q12. The 32-byte Copy rule in detail

- Size: **A.** the struct's size in memory including alignment padding (what
  is actually copied); **B.** the sum of its fields' sizes (padding does not
  count; reordering fields never changes Copy-ness).
- Opting out/in: a library struct silently changing between copy and move
  when a field is added breaks its users.
  - **A.** no opt-out; the rule is the rule;
  - **B.** an opt-out word on the struct (original): `struct Handle stays { … }`
    never copies automatically;
  - **C.** an opt-in instead: only structs marked `struct V copies { … }`
    copy automatically, and the compiler checks they qualify.

### Q13. Borrowing a temporary

```cpp
&string t = &"  hi "::trim();   // the trimmed string is a temporary
io::outln(t);
```

- **A. The temporary lives to the end of the block** when it is borrowed in a
  declaration.
- **B. Error**: name it first (`string tt = "  hi "::trim(); &string t = &tt;`).
- **C. The declaration takes ownership silently** (original): borrowing a
  temporary in a declaration makes the binding own it, and `t` behaves as an
  immutable owned `string`.

### Q14. Raw pointers in detail

SPEC §6 gives `*T`, `*p = 7;`, the file flag and `unsafe { }`. Not settled:

- **Making one**: **A.** from a borrow, by conversion: `*uint32 p =
  (&mut n)::to<*uint32>();` **B.** by a word (original): `*uint32 p = address
  of n;` **C.** a borrow converts automatically where a pointer is expected.
- **What needs `unsafe { }`**: **A.** only reading/writing through a pointer;
  **B.** every expression that mentions a pointer value (making, copying,
  comparing, dereferencing).
- **Pointer mutability**: **A.** one `*T` that may be written through; **B.**
  `*T` read-only and `*mut T` writable.
- **Null**: **A.** `*T` may be `none`-like null (`p == none`); **B.** no null
  until C import arrives.
- **`unsafe` functions**: **A.** none, only blocks; **B.** `unsafe` before a
  function's return type marks it, and calls need a block.

Until answered, WP9 implements: the flag, `unsafe { }`, the `*T` type,
dereference inside `unsafe { }`, and E0901 for making pointers.

### Q15. Do raw pointers and `&mut` copy?

- `*T` values: **A.** copy automatically (like C); **B.** move like strings;
  **C.** copy, but only inside `unsafe { }`.
- `&mut T` values (R8 makes `&T` copy): assigning `&mut string r2 = r;`
  **A.** moves `r` (then `r` is unusable); **B.** is an error (pass it on
  with Q3's form instead).

### Q16. Self mode where there is no body to infer from

Interface methods (step 6) have no body, and a library author may want a
method to stay `&mut self` for the future even if it does not change `self`
yet.

- **A. Interfaces infer from all implementations** (a method is `&mut self`
  if any implementation's is); no syntax.
- **B. A marker in interface signatures only** (original, wordy):
  `interface Counter { none bump() changes self; }`; struct methods stay
  inferred and must not exceed what the interface says.
- **C. An optional marker everywhere**: `changes self` may be written on any
  method; it is checked (or forces `&mut self`), never required except in
  interfaces.

### Q17. User-defined cleanup when a value is dropped

Not in the SPEC. Files, sockets and locks (step 7) need it, and it changes
drop elaboration (a struct with cleanup can't be partially moved; drop order
becomes visible; liveness must count drops, 3.4).

- **A. A method with a reserved name**: `none drop() { … }` runs when the
  value is destroyed.
- **B. A block in the struct** (original): `struct File { … on drop { close(self.fd); } }`
- **C. No user cleanup**: library types wrap runtime handles that the runtime
  closes; users call `close()` explicitly.

Step 3 does not need the answer; it only needs to know whether to keep the
"open the drop into fields" shortcut of 3.3, which stays valid for structs
without cleanup under every option.

### Q18. Clause order in signatures and struct heads

- Functions: `T f(..) lives a fails E { }` or `T f(..) fails E lives a { }`?
  **A.** `lives` first; **B.** `fails` first; **C.** either order.
- Structs: `struct Parser lives src with Shape { }` or `with Shape lives src`?
  **A.** `lives` first; **B.** `with` first; **C.** either.

### Q19. Message wording

All of section 4 (drafts), and whether non-ownership codes (file flags,
`mut open`) should move out of `03xx`.

---

## 6. Work packages

Order and parallelism:

```text
WP0a syntax & types (borrows, open, copy, lives, unsafe, *T) ──┐
WP0b file flags ───────────────────────────────────────────────┤ (parallel)
WP1  MIR data structures, builder, printer, validator ─────────┘ (needs WP0a for borrow nodes; can start on numbers)
 ├─ WP2  init & move analyses (replaces Flow) ──┐
 ├─ WP3  self-mode inference (HIR) ─────────────┤ (parallel with WP2)
 ├─ WP4  backends on MIR ───────────────────────┤ (numeric programs right after WP1; strings after WP6)
 │                                              │
 ├─ WP5  mutability checks ──── after WP2, WP3  │
 ├─ WP6  drop elaboration + removal of the step 2 workaround + leak tests ── after WP2, needs WP4
 ├─ WP7  borrow checker (liveness, regions, loans, conflicts) ── after WP2, WP5
 ├─ WP8  lives: signature inference, struct lives ── after WP7
 └─ WP9  unsafe ── after WP0b, WP5
WP10 performance pass ── last
```

Each package ends with `cargo test` green on both backends and the leak
check (once WP6 lands) clean. Golden test conventions are those at the top
of `tests/run.rs`. New fail tests go in `tests/fail/own_*.drh`, new run
tests in `tests/run/own_*.drh`, MIR tests in `tests/mir/` (WP1).

The test programs below are written against the readings in 1.4. Where an
open question changes how a program must be written, it is marked
`[Qn]`; adjust it when the answer comes.

### WP0a: syntax and types

**Files:** `src/lexer.rs` (nothing new: `open`, `copy`, `lives`, `unsafe`
are already keywords), `src/parser.rs`, `src/ast.rs`, `src/types.rs`,
`src/typeck.rs`, `src/hir.rs`, `src/diag/codes.rs`.

**Build:**
1. Types: `&T`, `&mut T`, `*T` in type position (Type gets `Ref`/`Ptr`
   with interned pointee, 2.4). `*T` requires the file flag (E0317,
   reported by typeck; WP0b supplies the flag).
2. Bindings and params: `open` modifier next to `mut` (`mut open` → W0323).
   Record `Mutability` in `hir::Local`.
3. Expressions: prefix `&e`, `&mut e`, `copy e`, `*e` (deref); `unsafe { }`
   as a statement. The lexer produces `&&` as one token: in prefix position
   the parser must split it into two `&` (or reject `&&x` with a clear error
   if Q3 rules out borrows of borrows: until then, reject with E0901).
   Assignment targets become place expressions (`p.f = …`, `*r = …`).
4. `lives a, b` after a function's parameter list; `struct X lives a, b`
   (order with `with`/`fails` per Q18; until answered, accept `lives`
   directly after `)` / the struct name only).
5. Type checking: `&e` has type `&T`; field access and methods auto-deref
   (R4); a call argument for a `&`/`&mut` parameter must be a borrow of the
   right kind (E0321 for `&mut`; for `&`, Q2); `copy e` type = `e`'s type
   (or per Q11 for borrows); struct declarations: E0314.
6. HIR additions from 2.2 (spans, places, new nodes).

**Pitfalls:** binary `&` vs prefix `&` (prefix only where an operand is
expected); `*` likewise; the bitwise-parentheses rule (E0006) must not fire
on prefix `&`. `&mut x` is one unit: `&mut` then a place.

**Acceptance:** `cargo test` still green (no behaviour change for old
programs). New fail tests:

```cpp
// tests/fail/own_syntax.drh
in io;
struct Parser { &string src; }
none main() {
    mut open uint32 a = 1;
    uint32 b = 2;
    grow(b);
}
none grow(&mut uint32 n) { }
```
`.err`: `error[E0314]: `Parser` holds a borrow`, `warning[W0323]`,
`error[E0321]: `grow` takes `&mut uint32` here`.

### WP0b: file flags

**Files:** new `src/flags.rs`, `src/lib.rs` (call it before lexing, pass
`unsafe_allowed` down), `src/diag/codes.rs`.

**Build:** 3.9's parser and E0318/W0319. Unit tests in `flags.rs` for:
shebang first, several flags, flag after code, bad value, unknown key.

**Acceptance:** fail tests

```cpp
//--!$unsafe: yes
none main() { }
```
→ `error[E0318]: `unsafe` must be `true` or `false``; and a file whose second
code line is `//--!$unsafe: true` → `file flags must be at the very top`.

### WP1: MIR, builder, printer, validator

**LOOK FIRST (a), (b), (c)** (Appendix A), all of it.

**Files:** new `src/mir/{mod,index,ty,build,print,validate,traversal}.rs`,
`src/lib.rs` (`Checked`), `src/main.rs` (`--dump-mir`), `tests/run.rs` (MIR
tests).

**Build:**
1. 2.3's types; `place_ty`, `region_slots`, `needs_drop`, `is_copy`.
2. The builder, the whole lowering table of 2.5, including (c)'s control
   flow and (b)'s method calls. Drops emitted at every scope exit with
   `reason`. Receiver borrows use `SelfMode` from WP3; until WP3 lands, use
   `&mut` for every method whose body (b)'s typeck accepted as mutating
   self, else `&` (or simply always `&mut`; nothing checks it yet).
3. `print.rs`: Appendix B format; `validate.rs`: V1-V9 (V3 only after
   checks; reducibility check in debug builds).
4. `Body::fallthrough_end` for E0105.
5. MIR golden tests: `tests/mir/NAME.drh` + `NAME.mir`; each non-blank line
   of `NAME.mir` must appear in `drachm check --dump-mir` output (same
   matching as fail tests: robust to numbering changes elsewhere).
6. `--dump-mir` dumps the MIR *after* elaboration once WP6 exists; before
   that, as built.

At the end of WP1 nothing uses the MIR yet except tests; the old HIR
backends still run.

**Pitfalls:** evaluation order (B1) for method chains `a::f()->g(b)`:
receiver, then `f`'s args, call, then `g`'s args; short-circuit `&&` inside
an argument list; `IncDec` inside an argument (`f(i++, i)` must pass old `i`
then new `i`); `return` inside nested loops/ifs must emit drops for every
frame; `skip` must emit drops for the frames it leaves.

**Acceptance:** for every existing `tests/run` and `examples` program,
building MIR and validating it succeeds (add a unit test that walks all of
them). MIR tests:

```cpp
// tests/mir/scope_drops.drh
none main() {
    string a = "x";
    {
        string b = "y";
    }
    string c = "z";
}
```
`.mir` contains, in this order: `Drop(_2)` (b, at the inner `}`), then at
the end `Drop(_3)`, `Drop(_1)` (reverse order: c before a). (Write the
expected local numbers after looking at the first dump.)

```cpp
// tests/mir/break_drops.drh
none main() {
    mut uint32 i = 0;
    while (true) {
        string s = "x";
        if (i == 3) { break; }
        i++;
    }
}
```
`.mir`: the block ending in the `break` jump contains `Drop(_s)` before its
`goto`.

### WP2: initialisation and move analyses

**Files:** new `src/mir/{dataflow,moves}.rs`, `src/mir/check.rs`; remove
`Flow` from `src/typeck.rs`.

**Build:** 3.0, 3.1 (MoveData, the four analyses, use checks E0301/E0207,
E0313, E0310, E0315, E0105 from CFG reachability). 3.1.6 is in WP5.

**Pitfalls:** report the *first* use after a move, once per path; a `Move`
operand inside a loop must see the move from the previous iteration
(`ReachingMoves` handles it; test it); `return x;` is a move and must not
make the drop at the end double (it doesn't: drops are elaborated). E0207
texts must stay exactly as today.

**Acceptance:** `tests/fail/unassigned.err` and `noreturn.err` unchanged and
passing. New fail tests (one file each, `.err` lines given):

```cpp
// own_use_after_move.drh  [Q2 for `greet`'s parameter: it takes `string`, so no question]
in io;
none greet(string s) { io::outln(s); }
none main() {
    string name = "Illy";
    greet(name);
    io::outln(name);
}
```
→ `error[E0301]: `name` is used after it moved`, `` `name` moved here ``,
`used here, after the move`, `help: if `greet` should not take ownership`.

```cpp
// own_move_in_loop.drh
in io;
none take(string s) { }
none main() {
    string s = "x";
    mut uint32 i = 0;
    while (i < 2) { take(s); i++; }
}
```
→ `E0301`, `in the previous time round the loop`.

```cpp
// own_partial.drh  [Q7]
in io;
struct Player { string name; string title; }
none take(string s) { }
none main() {
    Player p = "Illy", "Dr";
    take(p.name);
    io::outln(p.title);          // fine
    Player q = p;                // E0301: `p` is used after `p.name` moved
}
```

```cpp
// own_maybe_moved.drh
none take(string s) { }
bool flip() { return true; }
none main() {
    string s = "x";
    if (flip()) { take(s); }
    take(s);
}
```
→ `E0301`, `moved here, on some paths`.

```cpp
// own_move_out_of_borrow.drh
struct Player { string name; }
string nameOf(&Player p) { return p.name; }
none main() { }
```
→ `error[E0310]: cannot move `p.name` out of a borrow`, `help: return a copy: `return copy p.name;``.

Run test (moves that are fine):

```cpp
// tests/run/own_moves.drh
in io;
string twice(string s) { return s + s; }
none main() {
    string a = "ab";
    string b = a;
    string c = twice(b);
    io::outln(c);
}
```
`.out`: `abab`.

And its fail twin: an immutable binding that has moved still cannot be
reassigned (EverInit, 3.1.6):

```cpp
// own_reassign_moved.drh
none take(string s) { }
none main() {
    string b = "x";
    take(b);
    b = "new";
}
```
→ `error[E0205]: cannot change `b`: it is not `mut``, `assigned a second time here`.

### WP3: self-mode inference

**LOOK FIRST (b):** methods, `self`, field mutability flags.

**Files:** new `src/ownership/{mod,self_mode}.rs`; `src/lib.rs`.

**Build:** 3.7: evidence walk, fixpoint, reasons. Expose `Vec<Option<SelfMode>>`.
Unit tests on small programs (no codegen needed):

```cpp
struct Counter {
    uint32 n;
    mut uint32 reads;
    none bump() { self.n = self.n + 1; }      // Mut: writes self.n
    none bumpTwice() { self::bump(); self::bump(); }   // Mut: calls bump
    uint32 get() { self.reads = self.reads + 1; return self.n; }  // Shared: `reads` is a mut field [Q5]
    none ping(uint32 k) { if (k > 0) { self::pong(k - 1); } }    // Shared (mutual recursion, no change)
    none pong(uint32 k) { if (k > 0) { self::ping(k - 1); } }    // Shared
}
```
Expected: bump Mut(Writes "self.n"), bumpTwice Mut(Calls bump), get Shared,
ping/pong Shared.

**Pitfalls:** method calls on a *field* of self whose type is another struct
(`self.inner::grow()`): look up the field type's method; the reason chain
must not loop on mutual recursion (store only the first reason).

**Acceptance:** the unit tests; plus, after WP5, the fail test in WP5 that
prints the inferred reason.

### WP4: backends on MIR

**LOOK FIRST (a), (b):** the string and struct code in both backends: ABI
for strings (pointer to a 24-byte slot?), by-pointer threshold, sret.

**Files:** `src/codegen.rs`, `src/cgen.rs` (rewritten around MIR), new
`src/layout.rs`, `src/glue.rs`, new `src/mir/cleanup.rs`.

**Build:** 2.7, 2.8, 2.9. Delete the HIR-walking code paths once all tests
pass on MIR (keep them only until then; do not maintain two paths).

Order inside the package: (1) numbers, bools, calls, control flow (all the
existing tests) on both backends; run `bench/run.sh` and compare with the
numbers in PLAN step 1 (must be within noise, ±3%); (2) strings and structs,
which need correct drops, so (2) completes after WP6 (until then, run string
tests with drops left unelaborated *and* the step 2 copies still in place —
i.e. keep (a)'s deep copies in the builder behind a temporary flag until WP6
removes them).

**Pitfalls:** Cranelift variables vs stack slots (`in_memory`); never write
to the source of a `Move`; fall-through layout in C (label only when needed,
`bbN:;` with the semicolon); C `switch` on 128-bit values is not allowed
(lower 128-bit `Switch` to an `if` chain in the C backend, or keep the builder
from producing them); signed `Switch` case values are stored as zero-extended
bits like `Constant::Int`.

**Acceptance:** every existing run/example test passes on both backends;
`optimised_output_matches` passes; benches within ±3% for collatz, fib,
mandel, primes. All MIR-specific invariants validated in debug builds during
`cargo test`.

### WP5: mutability

**Files:** new `src/mir/mutability.rs`; remove the remaining E0205 logic
from `typeck.rs`.

**Build:** 3.8 and 3.1.6; method-receiver checks with self-mode notes;
`open` rules where Q6 is answered (until then: field writes on `open` and
`open` parameters → E0901).

**Acceptance:** `tests/fail/immutable.err` unchanged and passing. New:

```cpp
// own_open.drh
none grow(&mut string s) { }
none main() {
    open string xs = "a";
    grow(&mut xs);       // fine
    xs = "b";            // E0205 `open` variant
    string ys = "c";
    grow(&mut ys);       // E0312
}
```
→ `cannot reassign `xs`: it is `open`, not `mut``, `error[E0312]: cannot lend `ys` as `&mut``.

```cpp
// own_mut_through_shared.drh
struct Player {
    string title;
    none retitle() { self.title = "x"; }
}
none hurt(&Player p) { p::retitle(); }
none main() { }
```
→ `error[E0311]: cannot call `retitle` through a `&` borrow`, `` `retitle` changes `self`: it writes `self.title` ``.

### WP6: drop elaboration, workaround removal, leak tests

**LOOK FIRST (a):** where the deep copies are inserted (typeck? HIR
builder?), and how user `copy` differs from them.

**Files:** new `src/mir/drops.rs`; `src/mir/build.rs` (drop the temporary
deep-copy flag from WP4); `src/typeck.rs` / wherever (a) inserts copies
(delete); `runtime/strings/src/lib.rs` (double-free detection);
`tests/run.rs` (leak checking).

**Build:**
1. 3.3 elaboration with flags.
2. Remove step 2's deep-copy-on-assign: inserted copies and HIR drops go;
   only `ExprKind::DeepCopy` from user `copy` remains. String assignment,
   passing and returning become moves.
3. Runtime: when `DRACHM_CHECK_LEAKS` is set (read once into a static at the
   first allocation), also keep the set of live owned pointers (a
   `Mutex<HashSet<usize>>`); `drachm_str_drop` (and any runtime path that
   frees or reallocates) checks the pointer is in the set and removes it;
   if not: print `drachm: double free of a string` to stderr and
   `abort()`. Without the variable, nothing changes (no cost).
4. Harness: `run_case` and `optimised_output_matches` run every program with
   `DRACHM_CHECK_LEAKS=1`. For a case whose expected exit code is 0, stderr
   must not contain `drachm: leaked` or `double free`. (Panicking cases exit
   101 without dropping, R6, so they are exempt.) Add the leak check to
   `examples` too.

**Pitfalls:** flag writes after a `Call` go at the start of its target block
(the builder must give each call a fresh target block with one predecessor);
partially moved structs; drop of `_0` never happens (it is returned);
parameters are dropped at function end if not moved; temporaries dropped at
statement end (B4) including temporaries in conditions of `while` (each time
round); a `Drop` with `reason: Reassign` of a never-initialised local is
removed (dead).

**Acceptance:** all tests leak-free on both backends. New run tests (each
must also be leak-free):

```cpp
// tests/run/own_drops.drh
in io;
struct Player { string name; string title; }
none take(string s) { io::outln(s); }
bool odd(uint32 i) { return i % 2 == 1; }
none main() {
    mut uint32 i = 0;
    while (i < 4) {
        Player p = "n" + i::text(), "t" + i::text();
        if (odd(i)) { take(p.name); }       // partial move on some iterations
        string s = "x" + i::text();
        if (i == 2) { take(s); }            // conditional move
        i++;
    }
    string last = "end";
    take(last);
}
```
`.out`: `n1`, `x2`, `n3`, `end` on separate lines. Leak check: clean.

```cpp
// tests/run/own_reassign.drh
in io;
none main() {
    mut string s = "a";
    mut uint32 i = 0;
    while (i < 1000) { s = s + "b"; i++; }  // each old value dropped
    io::outln(s::len()::text());
}
```
`.out`: `1001`. Clean leak check proves the 1000 old strings were freed; the
double-free detector proves none was freed twice.

```cpp
// tests/run/own_copy.drh
in io;
none main() {
    string a = "hi"::upper();
    string b = copy a;
    string c = a;           // move, no allocation
    io::outln(b + c);
}
```
`.out`: `HIHI`.

Also a MIR test: `tests/mir/drop_flag.drh` = worked example 1, `.mir` lines
`Drop(_1) if _4` (adjust numbers) and no unconditional `Drop(_1)`.

### WP7: borrow checker

**Files:** new `src/mir/liveness.rs`, `src/mir/borrowck/{mod,regions,
constraints,loans,conflicts,explain}.rs`.

**Build:** 3.4, 3.5. Two-phase per Q4 (flag implemented, set only if Q4 =
A). Reborrow lowering per Q3 (until answered: accept only Q3-A's form
`&mut s` on a `&mut` binding, as a reborrow).

**Pitfalls:** the `Reassign` drop is a Write, not E0306; loans of a
temporary used in one call die at the call (liveness does it, but check);
a loan created at `P` must not be checked against the access at `P` that
creates it; field-disjointness through a deref of `&mut` (`&mut r.a` and
`&mut r.b` with `r: &mut Pair` are fine); struct region parameter variance;
report one error per loan.

**Acceptance (the PLAN's "done when" list):**

```cpp
// own_change_while_borrowed.drh  (E0302, SPEC example adapted to strings)
in io;
none main() {
    mut string name = "Illy";
    &string first = &name;
    name = "Sam";
    io::outln(first);
}
```
→ `error[E0302]: cannot change `name` while it is borrowed`, `` `name` is borrowed here ``, `change attempted here`, `the borrow is still used here`, `help: finish using `first` before changing `name``.

```cpp
// own_two_mut.drh
struct Player { mut uint32 hp; }
none heal(&mut Player p) { p.hp = p.hp + 1; }
none main() {
    mut Player p = 1;
    &mut Player a = &mut p;
    &mut Player b = &mut p;
    heal(a);
    heal(b);              // [Q3/Q15: passing a held &mut; adjust]
}
```
→ `error[E0303]: `p` is already borrowed with `&mut``.

```cpp
// own_shared_and_mut.drh
in io;
struct Player { mut uint32 hp; }
none heal(&mut Player p) { p.hp = p.hp + 1; }
none main() {
    mut Player p = 1;
    &Player look = &p;
    heal(&mut p);
    io::outln(look.hp::text());
}
```
→ `error[E0302]: cannot change `p` while it is borrowed` (a `&mut` borrow
under a shared loan).

```cpp
// own_dies_too_soon.drh
in io;
none main() {
    mut &string keep = &"a";      // [Q13: use a named string instead if Q13 = B]
    if (true) {
        string local = "x";
        keep = &local;
    }
    io::outln(keep);
}
```
→ `error[E0306]: `local` does not live long enough`, `is dropped here, at the end of its block`.

```cpp
// own_return_local.drh
&string shout() {
    string s = "hi"::upper();
    return &s;
}
none main() { }
```
→ `error[E0307]: cannot return a borrow of `s``.

Run tests that must be **accepted** (the non-lexical cases):

```cpp
// tests/run/own_nll.drh
in io;
struct Pair { string a; string b; }
none main() {
    mut string name = "Illy";
    &string first = &name;
    io::outln(first);
    name = "Sam";                 // fine: `first` is not used after this
    io::outln(name);
    mut Pair p = "x", "y";
    &mut string pa = &mut p.a;
    &string pb = &p.b;            // fine: different field
    io::outln(pb);
    io::outln(pa);
}
```
`.out`: `Illy`, `Sam`, `y`, `x`.

### WP8: lifetimes in signatures, `lives`

**Files:** new `src/mir/lives.rs`; struct `lives` in typeck (region
parameters, variance).

**Build:** 3.6 entirely: per-body inference, call-graph SCC order, recursion
fixpoint, E0307/E0308/E0309/E0320, struct region parameters (Q10: until
answered, implement option A's reading for one name, which is also the
SPEC's only example, and report E0901 for two or more names).

**Acceptance:**

```cpp
// own_missing_lives.drh  [Q9]
&string longest(&string a, &string b) {
    if (a::len() > b::len()) { return a; }
    return b;
}
none main() { }
```
→ `error[E0308]`, `returns a borrow of `a` here`, `help: write `lives a, b``.

```cpp
// own_wrong_lives.drh
&string pick(&string a, &string b) lives a { return b; }
none main() { }
```
→ `error[E0309]: `pick` returns a borrow of `b`, but its `lives` names only `a``.

```cpp
// tests/run/own_lives.drh
in io;
&string longest(&string a, &string b) lives a, b {
    if (a::len() > b::len()) { return a; }
    return b;
}
&string first(&string a, uint32 n) { return a; }      // inferred: lives a
struct Parser lives src { &string src; uint32 pos; }
&string rest(&Parser p) { return p.src; }             // inferred: lives p
none main() {
    string x = "long one";
    string y = "short";
    io::outln(longest(&x, &y));
    io::outln(first(&y, 3));
    Parser p = &x, 0;
    io::outln(rest(&p));
}
```
`.out`: `long one`, `short`, `long one`.

```cpp
// own_lives_call_site.drh  (lives used at the caller)
in io;
&string longest(&string a, &string b) lives a, b {
    if (a::len() > b::len()) { return a; }
    return b;
}
none main() {
    string x = "long one";
    mut &string r = &x;
    if (true) {
        string y = "short";
        r = longest(&x, &y);
    }
    io::outln(r);
}
```
→ `error[E0306]: `y` does not live long enough`.

```cpp
// tests/run/own_recursive_lives.drh
in io;
&string pickA(&string a, &string b, uint32 n) {
    if (n == 0) { return a; }
    return pickA(a, b, n - 1);
}
none main() {
    string x = "a"; string y = "b";
    io::outln(pickA(&x, &y, 3));
}
```
`.out`: `a` (inferred `lives a` through the recursion; `b` is not required).

### WP9: unsafe

**Files:** new `src/mir/unsafety.rs`; typeck for `*T`; both backends for
`Deref` of pointers (already covered by place addressing).

**Build:** 3.9's checks; pointer creation per Q14 (until answered: E0901).

**Acceptance:**

```cpp
// own_unsafe_no_flag.drh
none main() { unsafe { } }
```
→ `error[E0317]: `unsafe` is not enabled in this file`.

```cpp
// own_unsafe_deref.drh
//--!$unsafe: true
none read(*uint32 p) { uint32 v = *p; }
none main() { }
```
→ `error[E0316]: reading through a raw pointer needs an `unsafe { }` block`.

A run test that writes through a pointer waits for Q14 (there is no way to
make one yet).

### WP10: performance pass

**Files:** `bench/` (new benchmarks), backends.

**Build:** the items in section 7 marked "WP10". Add benchmarks with C and
Rust twins: `strings.drh` (build a 1 MB string by appends, then count a
substring), `structs.drh` (a `Vector3` struct, 10^7 adds in a loop, Copy
semantics), `moves.drh` (pass a `Player` with two strings through a chain of
functions 10^6 times).

**Acceptance:** `bench/run.sh` shows the old four benchmarks unchanged
(±3%) and the new ones listed in `PLAN.md`'s step 3 notes with their
numbers; any benchmark more than 1.5x behind Rust gets a written reason in
PLAN.

### After all packages

Update `docs/PLAN.md` (progress table, step 3 notes: what was decided, the
answers to section 5, leftovers), `docs/SPEC.md` §6 with the answers, and the
design doc.

---

## 7. Performance notes

The owner's goal is speed on par with Rust. What this design does to
generated code, and how to keep it fast.

### 7.1 Moves and copies

- A move is a bitwise copy of the value (24 bytes for a string) and nothing
  else: no zeroing of the source, no runtime "moved" bit. Removing step 2's
  deep copies is the single biggest speedup of the step for string code.
- `Move` of a by-pointer aggregate argument passes the source's address
  (2.9); no copy at all. `Copy` of a by-pointer aggregate copies (the callee
  may change a `mut` parameter).
- B7 (calls write to a temp, then move) adds a copy that C removes for free;
  Cranelift may keep it. **WP10:** a small MIR pass "destination
  propagation": for `_t = call f(..) -> bb; bb: p = move _t` where `p` is a
  local that is not borrowed by any argument of the call, rewrite the call to
  write `p` directly. Must run after borrow checking (it is only valid
  because the checker has proved the absence of conflicts).

### 7.2 Drops and drop flags

- Most drops are static (unconditional) or removed. Flags appear only for
  conditional moves. A flag is a `bool` local: a Cranelift `Variable` (SSA,
  constant-folded by the egraph pass when its value is known on a path) and a
  plain C local (GCC/Clang fold it completely in the common cases, e.g. a
  move inside one branch).
- String drop is inlined as `if (cap != 0) drachm_str_drop(&s)` (2.7):
  literals and empty strings cost one compare.
- No unwinding (R6): no landing pads, no cleanup blocks, nothing that blocks
  optimisation across calls. This is an advantage over Rust's default
  `panic=unwind`.

### 7.3 What the C compiler can see

- All user functions are `static` in one translation unit, and `runtime.c`
  is pasted in: the C compiler inlines across all of them.
- The **string runtime is a Rust staticlib**: every string operation is an
  opaque call, and every string local whose address is passed to it lives in
  memory. **WP10:** emit the trivial operations inline in both backends
  (byte length, `isEmpty` as `len == 0`, the drop check), and consider a
  small C fast path for ASCII-only `len`/`sub` in `runtime.c`, keeping the
  Rust code for the general case (both backends must call the same code, so
  the fast path goes in the shared runtime, not in one backend).
- **`restrict` on `&mut` parameters (WP10).** The borrow checker guarantees
  that a `&mut T` parameter is the only way to reach its target during the
  call, so the C backend may declare such parameters `T *restrict`. This is
  what lets LLVM optimise Rust's `&mut`; GCC and Clang do the same with
  `restrict`. For `&T` parameters, `const T *restrict` is valid only if the
  target cannot change during the call, which depends on **Q5** (`mut`
  fields writable through `&` break it, the same way Rust's `Cell` turns off
  `noalias`): emit it only for types with no `mut` field at any depth, or
  for all `&` if Q5 = C. Getting this wrong is a miscompile, so gate it
  behind the borrow checker having run and test it with a benchmark that
  would show the difference.
- By-pointer aggregates passed to `static` functions are inlined away by the
  C compiler; Cranelift does no inlining, so small Copy structs (≤ 16 bytes)
  should be passed by value in Cranelift if (b)'s ABI does not already.
  `&self` on a small Copy struct may also be passed by value *in the ABI*
  (the method cannot observe the difference, unless Q5 allows writing `mut`
  fields through `&self`, or raw pointers compare addresses).
- Labels and gotos do not hurt (2.9); reducible CFGs are guaranteed.

### 7.4 Cranelift

- Scalars that are never borrowed stay `Variable`s, so numeric code is
  unchanged by MIR. A scalar that *is* borrowed lives in a stack slot for the
  whole function; keep borrows of loop counters out of hot loops in the
  stdlib and benchmarks.
- Loads through `&T`/`&mut T` can carry `MemFlags::trusted()` (aligned,
  non-trapping): borrows always point at live, aligned memory. This lets
  Cranelift's alias analysis and GVN do a little more.
- The cleanup pass (2.8) keeps block counts low, which matters for
  Cranelift's compile speed and register allocation.

### 7.5 Compile time

All analyses are bitset dataflow over small bodies; region solving is one
SCC-ordered pass. The cost that can grow is region values (regions × points
bits): a 2000-point function with 300 regions is 75 KB of bits, fine. If a
generated function is huge, switch region values to sparse interval sets.

---

## Appendix A: LOOK FIRST checklist

Fill in before starting; each answer changes a named part of this spec.

| # | Question | Affects |
|---|---|---|
| A1 | How is `Type` extended for strings and structs? Still `Copy`? Interned? | 2.4, WP0a |
| A2 | Where are struct definitions stored (fields, `mut` flags, visibility, methods)? | 2.3 `StructInfo`, 3.7, 3.8 |
| A3 | Is there a layout function (size/align/offsets)? In which backend? By-pointer threshold? sret for large returns? | 2.7, 2.9, WP4 |
| A4 | How are string values represented in each backend (stack slot of 24 bytes? pointer?), and literals (`cap == 0` view, or copied)? | R10, 2.9 |
| A5 | Which HIR nodes are step 2's inserted deep copies and drops, and where are they inserted? Is user `copy x` the same node? | 2.2, WP6 |
| A6 | Where do string temporaries get dropped today (end of statement?) | B4 |
| A7 | How is `self` passed and typed; how is a method call represented; how are operator methods called? | 2.5, 3.7 |
| A8 | Does typeck already reject writes to non-`mut` fields, or to `self` fields? | 3.8, WP5 |
| A9 | HIR nodes for `for`, `for … of`, `break N`, `continue N`, `skip`, `skip to next`, `match`; how each backend lowers them; their error codes. | 2.5, WP1 |
| A10 | Runtime functions for string iteration (`for (string c of s)`), string `match` and globs. | 2.5 |
| A11 | Did (c) change typeck's `Flow` for `break` (reachability after `while (true)`)? | WP2 (Flow removal) |
| A12 | New diagnostic codes added by (a)/(b)/(c), to avoid clashes. | section 4 |
| A13 | Does `lib::check` still return `hir::Program`, and do the backends still take `&hir::Program`? | 2.1 |
| A14 | Is the `drachm_str_live`/`DRACHM_CHECK_LEAKS` report already used by the test harness? | WP6 |

## Appendix B: MIR text format

Used by `--dump-mir` and `tests/mir`. One function per block of text:

```text
fn NAME(_1: TYPE, _2: TYPE) -> TYPE
  _0: TYPE
  _3: TYPE             // user name, or "temp", or "drop flag of _1"
bbN:
  STATEMENT
  ...
  TERMINATOR
```

Statements: `StorageLive(_3)`, `StorageDead(_3)`, `_3 = RVALUE`,
`Drop(_1.0)`, `Drop(_1) if _4`, `FakeRead(_2)`. Places: `_1`, `_1.2`
(field index), `(*_1).0`. Operands: `copy _1`, `move _1`, `const 5u32`,
`const 2.5f64`, `const true`, `const "text"`. Rvalues: `&_1`, `&mut _1`,
`Add(copy _1, const 1u32)`, `Convert(copy _1, uint8)`, `Aggregate
Player(move _3, copy _4)`, `DeepCopy(_1)`. Terminators: `goto -> bb3`,
`If copy _2 -> bb3, bb4`, `Switch copy _2 [0: bb3, 1: bb4, otherwise: bb5]`,
`_5 = call f(move _1, copy _2) -> bb6`, `call OutStr(...) -> bb6`, `return`,
`unreachable`, `panic(KIND)`. The format is for tests and debugging only and
may change; tests match lines, not whole dumps.
