# Step 3 implementation spec: ownership and borrow checking

Status: **ready for implementation**. Written 2026-11-10; revised 2026-11-11
with the owner's answers (`docs/impl/owner-answers.md`, SPEC §6) and checked
against the code at `2f26fb3` (steps 2 and 4 done; Appendix A is filled in).
First-wave briefs: `wp0a.md`, `wp0b.md`, `wp1.md` in this directory.

Audience: an engineer who has read `CLAUDE.md`, `docs/SPEC.md` and
`docs/PLAN.md`, but not the design discussions. Everything needed to build
step 3 is here. Appendix A records what the code looked like when this was
revised.

Rules that bind this document (from `CLAUDE.md`):

- The language owner decides every user-visible behaviour. Section 5 records the
  owner's answers and ends with a "Still open" list. Do not decide those
  yourself: the code reports `E0901` ("not supported yet") with the message
  given there, and the case is relaxed when the owner answers.
- The diagnostic texts in section 4 were written by the implementer (Q19:
  owner review pending). Use them as written; the owner reviews them in one
  pass later, from the table at the end of section 4.
- Engineering choices (data structures, algorithms) are made here; the ones
  users can see are flagged with **(visible)**.

Contents

1. Goals, non-goals, rules being implemented
2. The MIR
3. The analyses
4. Diagnostics
5. Owner answers, and what is still open
6. Work packages
7. Performance notes

Appendix A: what the code looks like (answered).
Appendix B: MIR text format.

---

## 0. State of the code

The three branches (strings, structs, control flow) are merged. Appendix A
answers what this spec needed to know; the consequences are folded into the
sections below. The facts that matter most:

- The HIR has no spans and no borrow types or nodes. `Type` is `Copy`; structs
  are `Struct { id, name: &'static str }` (`types.rs`).
- typeck inserts the step 2 workaround nodes (`Copy`, `Move`, `Temp`,
  `Scoped`, `Stmt::Drop`). The MIR builder ignores them (2.2a) so the old HIR
  backends keep working until WP4/WP6.
- typeck already lowers `for`, `while`, `break N`, `continue N`, `skip` and
  `match` to `Stmt::If / Loop / Jump` with labels. Looping over a string and
  `match` on a string are `E0901` in typeck, so MIR has nothing to do for them.
- Each struct already has size, alignment and field offsets (`hir::layout`).
- Tests already run with `DRACHM_CHECK_LEAKS=1`; double-free detection does not
  exist yet. The `03xx` codes are all unused.

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
   - `take`, `lend`, `address of`, and the user `drop()` method;
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
- `changes self` on interface methods (step 6; this step only infers struct
  methods), interfaces, and `asm` functions (Later).
- Threads and data-race checking (later).
- Optimisations on MIR beyond the small cleanup pass in 2.8.

### 1.3 The SPEC rules this step implements

Section numbers refer to `docs/SPEC.md`.

| # | Rule | Source |
|---|---|---|
| S1 | Bindings are immutable by default. `mut` = may be reassigned **and** changed (mutating methods, `&mut` borrows, field writes). `open` = contents may change (mutating methods, `&mut`, field writes), never reassigned; `open` parameters are allowed, and an `open` binding may be declared without a value and assigned once. `mut open` is redundant (warning). | §2 |
| S2 | `uint32 later;` may be declared without a value; reading before every path assigns it is an error. An immutable binding declared that way may be assigned exactly once, where it is certainly still unassigned (so not inside a loop it was declared outside of). | §2 |
| S3 | Parameters are immutable unless written `mut uint32 n`. | §2 |
| S4 | Strings, lists and most structs **move** on assignment or pass; the source is unusable afterwards (E0301 "used after it moved"). | §6 |
| S5 | Numbers, bools, and structs whose fields are all copyable and whose total size is at most 32 bytes **copy** automatically. | §6 |
| S6 | Explicit copy: `string c = copy b;`. | §6 |
| S7 | Borrows `&T` (shared, many) and `&mut T` (exclusive). The call site always writes them: `show(&p)`, `push(&mut xs, 1)`; passing on a `&mut` you hold is `lend r`. A borrow means its target: reads are automatic, `r = 5` writes through, `r = &mut other` re-points. `&T` copies, `&mut T` moves. Lending `&mut` needs a `mut` or `open` binding. | §6 |
| S8 | Struct fields are immutable unless marked `mut`. A `mut` field can be changed on a value you own, even through an immutable binding. Through any `&` everything is read-only, `mut` fields too. | §6 |
| S9 | Struct fields may hold borrows; `lives` names the borrow fields themselves: `struct Pair lives a, b { &string a; &string b; }`. | §6 |
| S10 | Raw pointers `*T` move (`copy p` duplicates), so a struct holding one is not Copy. | §6 |
| S11 | E0302's exact style (message, labels "is borrowed here" / "change attempted here" / "the borrow is still used here", `help:`). Other borrow errors are written in the same style. | §6 |
| S12 | Lifetimes are inferred. When needed, `lives` names the parameters a returned borrow comes from: `&string longest(&string a, &string b) lives a, b { }`. | §6 |
| S13 | Returning a borrow derived from exactly one borrowed parameter is inferred; ambiguous cases ask for `lives a, b`. `lives` is checked at the definition and used at call sites. | PLAN step 3 item 6 |
| S14 | `self` is never declared; its shared vs mutable borrow mode is **inferred** from the method body. A method that assigns `self.x` or calls a mutating method is `&mut self`, otherwise `&self`. Error messages say which was inferred and why. | §5, PLAN step 3 item 7 |
| S15 | Raw pointers `*T` need BOTH the file flag `//--!$unsafe: true` AND an `unsafe { }` block around everything that touches a pointer value (`address of n`, copying, comparing, `*p`). `none` is null. `unsafe` functions need an `unsafe { }` block to call. | §6 |
| S16 | File flags are `//--!$key: value` lines, allowed only at the very top of a file. | §1 |
| S17 | No garbage collector. Values are destroyed at the end of their scope, in reverse declaration order. | §6, PLAN step 2 item 6 |
| S18 | Runtime faults panic: message and exit code 101. | §7 |
| S19 | Operands are evaluated left to right. | §2 |
| S20 | Diagnostic codes `03xx` are ownership; E0301 and E0302 are fixed. Report all errors. | §10 |
| S21 | Moving a field out says `take p.name`; `take p.t leaving v` puts a value back; a plain field move is an error. Methods never consume `self`; `return self.name` from a borrowed `self` is an error. | §6 |
| S22 | A method `none drop()` runs when a value is destroyed. On such a struct plain `take x.f` is an error; `take x.f leaving v` is allowed. | §6 |
| S23 | `struct H stays { }` never copies automatically; the 32-byte rule counts memory size with padding. `copy r` on a borrow copies the target; `copy n` on a Copy value warns W0322. | §6 |
| S24 | Borrowing a temporary in a declaration keeps it alive to the end of the block. Receivers are two-phase. | §6 |

### 1.4 Readings this spec relies on

These are places where the SPEC's wording or examples settle a behaviour in
the reading given here. They are not open questions, but show them to the
owner together with section 5 so a wrong reading is caught early.

- **R1 (field writes).** A non-`mut` field can be written exactly when the
  value containing it can be changed: through a `mut` or `open` binding, or
  through a `&mut` borrow. A `mut` field can be written on a value you own
  through any binding, immutable ones included (so `&mut p.hp` of a `mut`
  field is also fine there), but never through a `&` borrow (Q5). `self` is a
  borrow, so a method that writes `self.hp` (even a `mut` field) is `&mut
  self`. A write to a `mut` field still conflicts with loans of that place.
- **R2 (builtins borrow).** `io::out`, `io::outln`, `io::outf`, the string
  operators (`+`, comparisons, interpolation `${}`) and every string method
  take their string operands by shared borrow, implicitly, and no `&` is
  written for them. They also accept a `&string` (R4). `io::outln(name);
  io::outln(name);` is valid. Q2 ("always write `&`") is about *user* functions.
- **R3 (receivers).** A method receiver is borrowed implicitly at the call:
  `xs::push(9)` and `p::heal()` write no `&`/`&mut`. The receiver binding must
  be `mut` or `open` for a mutating method.
- **R4 (a borrow means its target).** Wherever a borrow-typed expression is
  used and the expected type is not itself a borrow (operands, conditions,
  `.` and `::`, builtin arguments, declarations of non-borrow type, `copy`),
  typeck reads through it by inserting `ExprKind::Deref`. Where a borrow is
  expected (declaration of a borrow type, borrow parameter, borrow return,
  assignment to a borrow, `lend`) the borrow itself is used. Moving a non-Copy
  value out through a borrow is E0310.
- **R5 (drop order).** Locals: reverse declaration order (S17). Struct fields:
  declaration order, after the struct's own `drop()` has run. Temporaries: at
  the end of the statement that made them, except R12. Now observable through
  `drop()`.
- **R6 (panics).** A panic exits the process immediately; nothing is dropped.
  So there is no unwinding and no cleanup path in MIR.
- **R7 (`while (true)`).** A loop whose condition is the literal `true` (or a
  C-style `for` with no condition) only finishes through `break`.
- **R8 (borrow types).** `&T` copies (8 bytes, any number may exist). `&mut T`
  and `*T` move: `f(r)` or `&mut string r2 = r;` uses `r` up; `lend r` passes
  it on and keeps `r` (Q3, Q15).
- **R9 (`unsafe` is narrow).** An `unsafe { }` block allows raw-pointer
  operations and calls of `unsafe` functions; it does not switch off move or
  borrow checking of ordinary places.
- **R10 (string literals).** A string literal is a value of type `string`
  that does not own its bytes (`cap == 0` in `runtime/strings.h`); dropping it
  does nothing, and a later change (methods return new strings) never writes
  to it. So moving and copying literals is free.
- **R11 (W0322).** `copy e` warns whenever the *result* type is Copy,
  including `copy r` with `r: &uint32`.
- **R12 (temporary extension).** Only when a declaration's whole initializer
  is `&e` / `&mut e` and `e` is not a place does the temporary live to the end
  of the block (Q13). A borrow of a temporary anywhere else, or in an
  assignment, dies at the end of the statement (E0306 if used later).
- **R13 (two-phase).** Only the implicit `&mut` borrow of a method receiver is
  two-phase (Q4). Explicit `&mut x` arguments are not.
- **R14 (clause order).** `lives`, `with` and `stays` on a struct, `lives` and
  `fails` on a function, may come in any order, each at most once (Q18;
  `stays` joins the group by extension).
- **R15 (`take`).** `take` is for fields: `take p.name`. Moving a whole local
  stays plain (`string b = a;`). A plain `take` is a move (allowed on an
  immutable owned binding); `leaving v` is also a write to the field, so it
  needs a changeable place (E0205 as for any assignment). `v` is evaluated
  after the take, so it cannot read the taken field.
- **R16 (`drop()`).** Reserved method name, shape `none drop()`, non-static;
  its `self` is `&mut`. A struct with `drop()` is never Copy (a duplicate would
  run `drop` twice), and is never partially moved (Q17).
- **R17 (E0324).** Applies where a non-Copy field of an owned path is used as a
  value to be moved: assignment or declaration source, by-value argument,
  `return`, struct field value, `leaving` value. Receivers and builtin operands
  borrow, so `p.name::len()` and `io::outln(p.name)` are fine.
- **R18 (re-point or write through).** `r = rhs` re-points `r` when `rhs` has a
  borrow type (and needs `r` to be `mut`); otherwise it writes through. `r += 1`
  and `r++` always write through.

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

**No per-function gating.** `lib::check` returns no HIR when any error was
found (lib.rs:19-30), so MIR checks run only on type-clean programs. Keep that.
**(visible)** E0105, E0205 and E0207 are therefore no longer reported next to
type errors. `tests/fail/flow.drh` (E0105, E0205, E0207) and `structmut.drh`
(E0205 together with E0101/E0114/E0201/E0211) mix them: when a code moves to
MIR, move its cases into new `own_*.drh` files and keep the rest in place.

### 2.2 What happens to the HIR

The HIR stays a tree in its present shape (`hir.rs`: `Stmt::{Assign, If, Loop,
Jump, Return, Expr, Drop}`, jumps by label). Changes (WP0a, WP1):

1. **Spans.** `hir::Expr.span`; `hir::Local` gets `span`, `mutability:
   Mutability {Fixed, Mut, Open}`, `is_param`; `hir::Place.span`;
   `Branch.end` and `Loop.end` (span of the closing `}`, for E0306) and
   `Func.body_end`; `Func.name_span`, `Func.span`, `Func.is_unsafe`.
   `Expr::new(kind, ty)` gives a dummy span so untouched construction sites
   compile; real spans go on everything a diagnostic can point at (reads of
   locals, field accesses, calls and each argument, borrows, assignments,
   returns).
2. **Places.** `Place { local, proj: Vec<HirProj>, span }`,
   `enum HirProj { Field(usize), Deref }` (replaces `fields`).
3. **New nodes.** `Stmt::Decl(LocalId)` at each user declaration (before its
   initializer's `Assign`), `Stmt::Unsafe(Block)`; `ExprKind::Borrow{mutable,
   place}`, `Deref`, `DeepCopy` (user `copy`), `Take{place, leaving}`, `Lend`,
   `AddressOf`, `NullPtr`. Backends get trivial arms (`Decl` ignored, `Unsafe`
   runs its block); the others cannot reach a backend before WP4 (WP0a gates
   `lib::build`).
4. **Struct data.** `FieldDef.mutable`; `StructDef.copy` (2.7), `stays`,
   `has_drop: Option<FuncId>`, `lives: Vec<usize>` (indices of the borrow
   fields named by `lives`).
5. **Removed in WP2/WP5:** `Flow` (typeck.rs:852) **and its uses in
   `typeck/control.rs`** (`Target` carries `Flow`s; `chain`, `looped`,
   `for_each`, `match_stmt` join them), the late-init branch of
   `check_assignable`, `read_local`'s E0207. E0105/E0205/E0207 move to MIR with
   **the same texts**.
6. **Removed in WP6:** the legacy nodes of 2.2a.

#### 2.2a Legacy nodes (until WP6)

typeck still inserts step 2's ownership workaround. The MIR builder ignores it
and makes its own moves and drops, so moves are real in MIR from WP1 on while
the HIR backends keep using the legacy nodes. (WP1 phase A, which runs in
parallel with WP0a, instead translates the legacy nodes faithfully: `Copy` →
`DeepCopy`, `Stmt::Drop` and the `Scoped` list → `Drop`, no scope frames. Phase
B switches to the table below once `Stmt::Decl` and spans exist.)

| Legacy HIR | MIR builder |
|---|---|
| `Copy(e)` | `⟦e⟧` as if the `Copy` were not there (a move for non-Copy types) |
| `Move(l)` | `Operand::Move(l)` |
| `Temp(l, e)` | `l = ⟦e⟧` (StorageLive at first sight); the value is the place `l`; `l` joins the statement frame (B4) |
| `Scoped(e, locals)` | `⟦e⟧`; the list is ignored |
| `Stmt::Drop(p)` | ignored |

Hidden locals (not introduced by `Decl`) get `StorageLive` in `bb0`, never a
`StorageDead`, and only get drops through `Temp` registration. User locals and
their scopes come from `Decl` and the block structure (`Branch.body`,
`Loop.body`, `Loop.step`, `Func.body`).

### 2.3 Data structures

New module `src/mir/mod.rs`. Index types are newtypes over `u32` with a
small `IndexVec<I, T>` (a `Vec<T>` indexed by `I`); write it in
`src/mir/index.rs`, 40 lines, no crate needed.

```rust
use crate::diag::Span;
use crate::types::Type;              // after step 2 this includes Str, Struct, Ref, Ptr
use crate::hir::{FuncId, StructId};   // plain `usize` aliases; field indexes are `usize`

newtype_index!(Local);               // _0 is the return place
newtype_index!(BlockId);             // bb0 is the entry block
newtype_index!(StrId);               // index into Program::strings

pub struct Program {
    pub bodies: Vec<Option<Body>>,              // by FuncId; every function has a body today
    pub main: FuncId,
    pub strings: IndexVec<StrId, String>,       // literal pool, shared by both backends
    // structs and layouts: hir::Program::structs (extended, 2.2 item 4); no copy here
    pub self_modes: Vec<Option<SelfMode>>,       // by FuncId; None for non-methods
    pub lives: Vec<LivesSig>,                    // by FuncId; 3.6: inferred or declared
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
    pub unsafe_fn: bool,                 // declared `unsafe`
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
    /// Integer/bool switch. Defined, printed and validated, but the builder
    /// does not emit it: `match` already arrives as an if-chain (2.5).
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
    Func(FuncId),                        // user function or method (self is args[0])
    Str(hir::StrFn),                     // runtime/strings.h and drachm_text_*; string args are `&string` temps (B3)
    Out { newline: bool },               // io::out / io::outln of a `&string`
    Text,                                // hir::ExprKind::Text: number or bool to an owned string
    Radix { base: u32, upper: bool },    // hir::ExprKind::Radix
    Join,                                // hir::ExprKind::Join: any number of `&string`, owned result
}

pub enum PanicKind { /* filled by step 6: Bounds, ... */ }

pub struct Place { pub local: Local, pub proj: Projs }
pub type Projs = SmallVec<[Proj; 2]>;  // or a plain Vec; places are short

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum Proj {
    Field(usize),    // on a struct-typed prefix
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
    RawRef(Place),                      // `address of place`: makes a `*T` (needs unsafe)
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
    Mut { two_phase: bool },   // two_phase: receiver borrows only (3.5.6)
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

MIR uses `types::Type` (types.rs:9: `Copy` and `Hash`; structs are `Struct { id,
name: &'static str }`, the name interned by `types::intern`). Step 3 adds two
variants with the same leak-intern trick, so `Type` stays `Copy` and no type
table is needed:

```rust
pub enum Type { /* existing variants, */
    Ref { mutable: bool, to: &'static Type },  // &T / &mut T; `to` is never itself a Ref
    Ptr { to: &'static Type },                 // *T: one kind, may be written through, may be none
}
impl Type {
    pub fn borrow(mutable: bool, to: Type) -> Type;  // interns `to` in a static Mutex<HashSet<&'static Type>>
    pub fn ptr(to: Type) -> Type;
}
```

`Display` prints `&T`, `&mut T`, `*T`. Every exhaustive `match` on `Type` gets
arms: `hir::scalar_size` (hir.rs:96; Ref and Ptr are 8 bytes, aligned 8),
`codegen::cl_type` (codegen.rs:40; the pointer type), `cgen::c_type`
(cgen.rs:133; `T *`).

Region information is **not** stored in `Type`. The borrow checker gives each
local a vector of *region slots*, one per borrow position in its type, in a
fixed pre-order:

```rust
fn region_slots(ty: Type, structs: &[StructDef]) -> usize {
    match ty {
        Type::Ref { to, .. } => 1 + region_slots(*to, structs),
        Type::Struct { id, .. } => structs[id].lives.len(),  // `lives` names, 3.6.4
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
    diags: &'a mut Vec<Diagnostic>,     // the builder reports E0324/E0325 itself
    body: Body,                         // being built; blocks have Option<Terminator>
    cur: BlockId,                       // the block statements are appended to
    local_map: Vec<Local>,              // hir::LocalId -> mir::Local
    scopes: Vec<ScopeFrame>,            // the scope stack (drops to emit on exit)
    targets: HashMap<hir::Label, Target>, // for Jump
    unsafe_depth: u32,
}

struct ScopeFrame { id: ScopeId, owned: Vec<Local> }   // locals declared here, in order
enum Target {                           // keyed by the label typeck gave the If/Loop
    Loop { exit: BlockId, step: BlockId, scope_depth: usize },
    If { exit: BlockId, tests: Vec<BlockId>, scope_depth: usize },
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
  `return`/`break` that caused them (`Branch.end`, `Loop.end`,
  `Func.body_end`).
- B9. *Temporaries in declarations.* A declaration whose whole initializer is
  `Borrow(e)` with `e` not a place puts the temp for `e` in the enclosing
  *block* frame, just before the declared local, so it dies after it (Q13,
  R12). Everything else uses the statement frame.
- B10. *Field moves.* `as_operand` of a place of non-Copy type whose
  projections contain a `Field` and no `Deref` is **E0324**, unless it comes
  from `Take`. With a `Deref` in the projections it emits `Move` and the moves
  pass reports E0310. Receivers, builtin operands, `Ref` and `DeepCopy` use
  `as_place` and are not moves (R17).
- B11. *Two-phase.* A method receiver's `&mut` borrow is `Mut { two_phase:
  true }`; every other borrow has `two_phase: false`.

**Lowering table.** `⟦e⟧ → op` means "emit statements, produce an operand".

| HIR | MIR |
|---|---|
| `Int`, `Float`, `Bool` | `Const` |
| `Str(s)` (literal) | `Const(Str(id))`, `id` interned in `Program::strings` |
| `Local(id)` | `Copy(_l)` / `Move(_l)` by type (B2) |
| `Field(e, f)` | place `as_place(e)` + `Field(f)`; if `e` is a borrow, insert `Deref` first (R4) |
| `Neg/Not/BitNot(e)` | `_t = Neg(⟦e⟧)` → `Copy(_t)` |
| `Binary(op, l, r)` | `a = ⟦l⟧; b = ⟦r⟧; _t = Binary(op, a, b)` |
| `StrCall(f, args)` (also `+`, `==`, `<` … on strings) | each string arg is a place: `_a = &place`; `_t = call Str(f)(copy _a, …)` (R2) |
| `Shift`, `IntPow`, `Convert` | as `Binary` |
| `Logic { and, l, r }` | `_t = ⟦l⟧; If _t → bbR / bbJ`, `bbR: _t = ⟦r⟧; goto bbJ` (for `||` the branches swap). Short-circuit is control flow. |
| `IncDec { place, inc, prefix }` | `_old = copy place; place = Binary(Add/Sub, copy place, 1)`; value is `_old` (postfix) or `copy place` read into a temp (prefix) |
| `Call(f, args)` | each arg `⟦a⟧` left to right (borrowed params: B3); `_t = call f(args) -> bbNext` |
| method call `recv::m(args)` (b) | receiver: `as_place(recv)` (temp if not a place); `_s = &place` or `&mut place` by `modes[m]` (if `recv` is already a borrow: `&(*recv)` / `&mut (*recv)`, a reborrow); then args; `call m(copy/move _s, args)` (B11). A `static` method has no receiver. |
| `Borrow { mutable, e }` | `_t = Ref(Shared/Mut, as_place(e))`; on a non-place `e`, materialise a temp first (its lifetime: B9) |
| `DeepCopy(e)` | `e` is a place; a borrow place is read through first (`copy r` copies the target, Q11): `_t = DeepCopy(place)`. A Copy result was already warned (W0322) and lowers as a plain copy |
| `Deref(e)` | place `as_place(e)` + `Deref` |
| `Take { place, leaving: None }` | `_t = Use(Move(place))`; value `Move(_t)`; the place is now uninitialised (3.1) |
| `Take { place, leaving: Some(v) }` | `_t = Use(Move(place)); _v = ⟦v⟧; place = Use(move _v)` (no `Drop`: the place is uninitialised); value `Move(_t)` |
| `Lend(e)` | `_t = Ref(Mut{two_phase:false}, (*as_place(e)))`, a reborrow; value `Move(_t)` |
| `AddressOf(e)` | `_t = RawRef(as_place(e))` |
| `NullPtr` | `Const(Int(0, ptr type))` |
| struct construction (b) | each field value `⟦v⟧` in *source* order into temps; `_t = Aggregate(S, ops in field order)` |
| `Out { newline, arg }`, `Text`, `Radix` | `Callee::Out/Text/Radix`, string argument borrowed (R2) |
| `Join(parts)` (interpolation, `io::outf`) | `Callee::Join`, every part a `&string` temp |
| `Stmt::Decl(l)` | `StorageLive(_l)`; register `l` in the current frame |
| `Stmt::Assign(p, e)` (initializer or later) | B6 when the type needs drop, else `p = Use(⟦e⟧)`; a call result goes through a temp (B7) |
| `Stmt::Unsafe(block)` | like `Unsafe(block)` below |
| `Stmt::Expr(e)` | `⟦e⟧`, result discarded: a temp with drop glue is dropped at the end of the statement (B4) |
| `If`, `Loop`, `Jump`, `match` | see "Control flow" below |
| `Return(e)` | `_0 = Use(⟦e⟧)` (moves the value out, so it is not dropped); drops of all frames (B5); `Return` |
| `Unsafe(block)` | `unsafe_depth += 1` while lowering; every statement/terminator built gets `unsafe_ctx = true` |
| end of function body | if `cur` is reachable: drops of the function frame; `Return` for `none` functions, else `Unreachable` (E0105 is reported by WP2 using CFG reachability, see 3.1.5) |

**Control flow.** typeck already lowered `for`, `while`, `break N`, `continue
N`, `skip` and `match` to `Stmt::If / Loop / Jump`, every jump naming its
target by label (control.rs), so the builder does no level counting. A `Jump`
first emits `Drop`/`StorageDead` for every scope frame above the target's
`scope_depth` (innermost first), then `goto`.

| HIR | MIR |
|---|---|
| `If { label, branches }` | blocks `test_k`, `body_k` per branch, and `exit`. `test_k`: `_c = ⟦test⟧; If _c → body_k / test_{k+1}` (last one: `/ exit`); a branch with no test goes straight to its body; each body is an `IfBranch` scope ending with `goto exit`. `Target::If { exit, tests, depth }` is live while lowering the bodies |
| `Loop { label, cond, body, step }` | `goto head; head: cond → body / exit` (no cond: `goto body`, R7); body in a `LoopBody` scope, then `goto stepbb; stepbb: step; goto head`. `Target::Loop { exit, step, depth }` |
| `Jump::Break(l)` / `Continue(l)` | drops, then `goto exit` / `goto step` |
| `Jump::Leave(l)` / `Next(l, n)` | drops, then `goto exit` / `goto tests[n]` |
| `match` | already an `If` chain of comparisons (numbers and bools only), so `Switch` is never emitted. Looping over a string and `match` on a string are `E0901` in typeck (control.rs:481, 613): nothing to lower |

#### Worked example 1

```cpp
in io;

none consume(string s) { io::outln(s); }

none main() {
    string a = "hi";
    if (a::isEmpty()) {
        consume(a);
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
  call consume(move _1) -> bb3
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
  call consume(move _1) -> bb3
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
        Type::Struct(s) => cx.structs[s].has_drop.is_some() || cx.structs[s].fields.iter().any(|f| needs_drop(f.ty, cx)),
        _ => false,       // numbers, bool, none, &T, &mut T, *T (raw pointers are never dropped)
    }
}
```

`is_copy(ty)`: numbers, `bool`, `none`, `&T` → true; `&mut T`, `*T`, `Str` →
false (Q15); `Struct(s)` → `StructDef.copy`, computed once in typeck next to
`lay_out`: not `stays`, no `drop()` (R16), every field `is_copy`, and `size <=
32` where `size` is `StructDef.size`, the memory size including tail padding
(Q12: reordering fields can change Copy-ness). A `*T` or `&mut T` field makes
the struct non-Copy through the fields rule (S10 follows). Struct fields cannot
contain the struct itself by value, so the recursion ends.

**Layouts.** Already shared: `hir::layout` and `StructDef.size / align /
FieldDef.offset` (hir.rs:44-110) are computed once in typeck (typeck.rs:541)
and read by both backends; there is no new table. The C backend adds
`_Static_assert(sizeof(S) == N && offsetof(S, f) == K, "layout");` after each
struct definition so a disagreement with the C compiler fails the build.

**Glue functions**, generated once per struct type that needs them, by a
shared module `src/glue.rs` that both backends call to get a description:

- `drop_glue(S)`: if `S` has a user `drop()`, call it first with the address
  of the value; then for each field in declaration order (R5) that needs drop:
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
- `Call`: today's ABI (Appendix A, A3): every string or struct argument is a
  pointer to a fresh copy that the callee takes over; a string or struct result
  is written through a hidden first pointer (codegen.rs:55, 974). With MIR, a
  `Move` of such an argument passes the **address of the source place itself**,
  with no copy (the source is dead afterwards); a `Copy` first copies into a
  fresh stack slot, because the callee may change a `mut` parameter. The C
  backend passes strings and structs by value, `self` by pointer (cgen.rs:123).
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

#### 3.1.4 Partial moves: `take` (Q7)

Moving a field out of a struct you own says `take`: `string n = take p.name;`.

- A plain use of a non-Copy field of an owned path as a value to move
  (`string m = p.name;`, a by-value argument, `return`, ...) is **E0324** from
  the builder (B10, R17); fix-it "write `take p.name` or `copy p.name`".
- `take place` is a `Move` of that move path (any depth, no `Deref`). Then
  `p.name` and everything inside it is unusable (E0301) until reassigned; other
  fields stay usable; `p` as a whole is unusable (E0301, partial variant) until
  `p.name` or `p` is reassigned; `p.name = …` re-initialises; at scope end only
  the fields that still hold values are dropped (3.3).
- `take p.title leaving v`: move, then re-initialise in the same statement, so
  `p.title` stays usable (R15).
- A struct with `drop()`: plain `take x.f` is **E0325**; `take x.f leaving v`
  is allowed and the struct stays complete (Q17).
- Not allowed: moving out through a borrow, `self` included (`return
  self.name;` is E0310 with the Q8b help `return copy self.name;`), or through
  a raw pointer (E0310, "use `copy`"). Plain `take` through a borrow is E0310
  too; `take … leaving` through a borrow, `take` of a Copy field or of a whole
  local, and `take` of a non-place (E0327) are in the "Still open" list or
  E0327.
- Methods never consume `self` (Q8): there is no by-value `self`.

*Justification.* Move paths are per field anyway, so partial moves cost nothing
in the checker or at run time. Spelling them `take` makes the hole visible in
the source, and keeps accidental field moves from silently leaving a struct
half-empty.

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
`&mut`-borrowed) but does not move it. For a borrow `r`, `copy r` is `DeepCopy((*r))`: an owned
copy of the target (Q11). `DeepCopy` of a type that contains a
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
    if p has child move paths and !has_drop(type(p)):   // someone moved/assigned a field separately
        // A type with a user `drop()` is never opened: it is whole at every Drop
        // (plain `take` is E0325; `leaving` re-inits in the same statement), so one
        // Drop of the whole value runs `drop()` and then the fields. Otherwise
        // dropping a struct is dropping its fields.
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
        consume(take p.name);
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
- `Drop(p)` is **not** a use for region liveness, except when `type(p)` has a
  user `drop()` (itself or in a field): then it is a use of `p`, so the borrows
  a struct holds stay live until its drop point (drop-liveness, Q17). Strings
  and plain structs never read through their borrows when dropped.

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

**Two-phase borrows (Q4: allowed).** The implicit `&mut` borrow of a method
receiver is `Mut { two_phase: true }`: *reserved* from its creation until its
*activation* (the first use of the temp that holds it, always the call). While
reserved it conflicts like a Shared loan, so `s::append(s::len()::text())` is
fine (the arguments only read `s`). At activation it is checked as a MutBorrow
of its place against all other loans in scope, so `xs::push(xs::first())` is
an error (E0302, "change attempted here"; the borrow returned by `first` is
still used by the call). Explicit `&mut x` arguments are not two-phase (R13).

**Reborrows.** Passing on a borrow you hold (`lend r`, Q3) lowers to `_t = &mut (*r)` (or
`&(*r)`): a new loan of the place `*r`, with constraint C3 tying it to `r`'s
region. While `_t` is live, `r` itself is MutBorrow'ed through its deref, so
using `r` directly is E0304; after the call `_t` is dead and `r` is usable
again. This is what makes `&mut` parameters usable more than once in a body. A bare
`&mut` local passed or assigned (`f(r)`, `&mut string r2 = r;`) is a `Move`,
not a reborrow (Q15): `r` is used up.

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
  (Q9: body-derived):
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
**region parameters**, one per name after `lives`. Each name is a **borrow
field of the struct itself** (Q10), and different fields may borrow from
different places: `struct Pair lives a, b { &string a; &string b; }`.

- The `lives` list must name exactly the fields whose type is a borrow. A
  borrow field not listed, or a name that is not a borrow field → E0314.
  The list order is the slot order.
- A borrow field's type may be `&T` / `&mut T` with `T` having no region slots.
  A field whose type is a struct with `lives`, a borrow of one, or anything
  with two or more slots → `E0901` (Still open).
- Variance per parameter: 3.5.3. A struct value's region slots (2.4) are its
  parameters, in `lives` order. A function parameter `Parser p` has one
  universal region per parameter, named `p.src` in messages.
- On functions, `lives` names whole parameters only (`lives p`), never paths
  (Q10ii). Clause order: R14.

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
   place, with **no exception for `mut` fields**: `self` is a borrow, and through a
   borrow everything is read-only (Q5, R1). A write *through* a `&mut` field (`*self.out = …`)
   counts: a `&mut` stored in `self` can only be used through a unique path.
2. `&mut <self place>` anywhere (as a call argument or a binding).
3. A method call whose receiver is a self place and whose callee is
   currently `Mut` (including built-in mutating methods, none in step 3).

Moving out of a self place is not evidence; it is E0310 at MIR time. There
are no consuming methods (Q8).

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

A method named `drop` is always `Mut` (reserved name, called with exclusive
access). `changes self` (Q16) is written only on interface methods and matters
in step 6 (a struct's method may not change more than its interface allows);
nothing in step 3 reads it.

**Using the result.**

- The MIR builder types `self` as `&Self` or `&mut Self` and emits receiver
  borrows accordingly.
- E0312/E0311 at a call site of a `Mut` method on a non-changeable receiver
  add a note: "`heal` changes `self`: it writes `self.hp` (player.drh:7:9)".
  For `Calls`, follow the chain to the first non-`Calls` reason and print
  it: "`grow` changes `self`: it calls `resize`, which writes `self.cap`".
- The language server (step 8) shows the mode as an inlay hint; keep the
  `Vec<Option<SelfMode>>` in `Checked`.

> Present code (A7, A8): `self` is parameter 0 (`has_self`), always by address;
> typeck already runs this fixpoint on the HIR (`SelfUse`, typeck.rs:59;
> `report_fixed_receivers`, typeck.rs:586; `check_field_change`, typeck.rs:1367).
> WP3 moves that logic into `ownership/self_mode.rs` with the changes above.

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
        Some(Field(f)) if field_is_mut(parent_type, f) && !has_deref(parent) => Yes,  // S8: only on a value you own (Q5)
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
| `Assign(p.f…, …)` (write with projections) | `changeable(p.f…) == Yes` (an `Open` root counts: field writes are allowed, Q6) | E0205 (binding root), E0311 (through `&`) |
| `Ref(Mut, p)` | `changeable(p) == Yes` | E0312 (binding root), E0311 (through `&`) |
| receiver of a `Mut` method | as `Ref(Mut, …)`, but message names the method and the self-mode reason | E0312 / E0311 |
| `IncDec`, compound assignment | as assignment of the place (on a borrow: through it, R18) | E0205 / E0311 |
| `r = <borrow>` (re-point a borrow binding, R18) | the binding is `Mut` (or a first late-init assignment) | E0205 |
| `r = <non-borrow>` (write through `*r`) | `r` is a `&mut` | E0311 |
| `RawRef(p)` (`address of p`) | `changeable(p) == Yes` | E0901 (Still open) |

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
| an `unsafe { }` block, an `unsafe` function declaration, a `*T` type written anywhere, `address of` | the file flag, else E0317 (typeck, once per site) |
| `RawRef` (`address of`), `Deref` of a `*T` (read or write), `DeepCopy` of a `*T` (`copy p`), a `*T` operand of `==` / `!=`, the constant `none` as a pointer | `unsafe_ctx`, else E0316 |
| a call of an `unsafe` function | `unsafe_ctx`, else E0316 (call variant) |
| any other pointer-typed operand (move, pass, return, declare) outside `unsafe_ctx` | `E0901` (Still open) |
| an unsafe operation inside an `unsafe` function but outside a block | `E0901` (Still open: whether the body counts as unsafe) |

Borrow checking ignores places with a `Deref` of a `*T` in their prefix
(they create no loans that matter: C3 stops; conflicts are not checked for
such accesses), and moving out through one is E0310 (R9).

---
## 4. Diagnostics

**Wording written by the implementer; owner review pending (Q19).** Treat the
texts as final for implementation; the table at the end of this section lists
every message line for the review. E0301 and E0302 are fixed by the SPEC (code
and E0302's layout); their wording beyond the SPEC example is also by the
implementer.

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
| W0322 | `copy` of a value that copies anyway | typeck |
| W0323 | `mut open` is redundant | parser/typeck |
| E0324 | field moved without `take` | builder (B10) |
| E0325 | `take` on a struct with `drop()` | builder |
| E0326 | `lend` of something that is not a `&mut` you hold | typeck |
| E0327 | `take` / `copy` / `address of` of something that is not a variable or field | typeck |
| E0328 | `drop` has the wrong shape | typeck (declarations) |
| E0332 | assigning part of a value that has none yet | moves (`wp2.md`) |

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
- a `&mut` borrow moved (`f(r)` or `&mut string r2 = r;`): label "`r` (a `&mut`
  borrow) moved here"; help "to keep using `r` afterwards, pass it on with
  `lend r`".
- a `stays` struct: help "`Handle` is marked `stays`, so it never copies
  automatically; write `copy h`".

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
 6 |     consume(name);
   |          ^^^^ moved here
 7 |     io::outln(first);
   |               ----- the borrow is still used here
   |
 help: finish using `first` before moving `name`, or move a copy: `consume(copy name)`
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

As in worked example 4 (3.6). It fires only when several parameters could be the source (Q9).

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

For `self` (Q8b): message "cannot move `self.name` out of a borrow", secondary
label "`self` is borrowed by `nameOf`", help "`self` is borrowed; write `return
copy self.name;`". `take self.name` gets the same message with help "`take`
cannot move out of a borrow; write `copy self.name`".
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
 3 |     consume(p);
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

Variants: "`lives x` names nothing `Parser` borrows" (`x` is not a borrow
field); "field `b` is a borrow, but `lives` does not name it" (help: add `b`).

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

Other operations get the same code with their own label: `address of n`
("makes a raw pointer here"), `copy p`, `p == none`, `none` as a pointer, and
calling an `unsafe` function ("`cycles` is an `unsafe` function").

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

The same for `&` parameters (Q2): "`show` takes `&Player` here, so the call
must say so", help "write `show(&p)`". Passing a `&mut` borrow where a `&` is
expected is `E0901` (Still open).

### E0324 field moved without `take`

```text
error[E0324]: cannot move `p.name` out of `p` without `take`
  --> main.drh:5:19
   |
 5 |     string m = p.name;
   |                ^^^^^^ this would leave `p` without a name
   |
 help: write `take p.name` to move it out, or `copy p.name` to keep both
```

Note: after `take`, `p.name` has no value until it is given a new one.

### E0325 `take` on a struct with `drop()`

```text
error[E0325]: cannot take `f.path` out of a `File`: it has a `drop()`
  --> main.drh:6:18
   |
 1 | struct File { string path; none drop() { } }
   |                            ---- `drop` runs when a `File` is destroyed, so it must stay whole
 6 |     string s = take f.path;
   |                ^^^^^^^^^^^ taken here
   |
 help: put a value back in the same statement: `take f.path leaving ""`
```

### E0326 `lend` needs a `&mut` you hold

```text
error[E0326]: `lend` passes on a `&mut` borrow you hold, and `n` is not one
  --> main.drh:3:9
   |
 2 |     uint32 n = 1;
   |     ------ `n` is a `uint32`, not a `&mut`
 3 |     add(lend n);
   |         ^^^^^^ nothing to pass on
   |
 help: to lend `n` itself, write `add(&mut n)`
```

### E0327 not a variable or field

```text
error[E0327]: `take` needs a field of a variable, like `p.name`
  --> main.drh:4:18
   |
 4 |     string s = take make();
   |                ^^^^^^^^^^^ `make()` is a temporary value; there is nothing to take it out of
   |
 help: use the value directly: `string s = make();`
```

Variants: "`copy` needs a variable or a field, like `copy name`" (help: name
the value first); "`address of` needs a variable or a field, like `address of n`".

### E0328 `drop` has the wrong shape

```text
error[E0328]: `drop` must be written `none drop()`
  --> main.drh:3:5
   |
 3 |     string drop(uint32 how) { }
   |     ^^^^^^^^^^^^^^^^^^^^^^^ `drop` runs by itself when the value is destroyed, so it takes nothing and returns nothing
   |
 help: write `none drop() { ... }`, not `static`, with no parameters
```

### E0332 assigning part of a value that has none yet

```text
error[E0332]: cannot give `p.hp` a value: `p` has no value yet
  --> main.drh:4:5
   |
 3 |     mut Player p;
   |                - `p` is declared here without a value
 4 |     p.hp = 5;
   |     ^^^^ assigned here
   |
 help: give `p` a whole value first: `p = ...;`
```

The never-assigned sibling of E0313 (same rule, 3.1.3: the parents of an
assigned part must be initialised). Whether a struct may be filled in field by
field is not decided by the owner; this is the spec's reading and is listed in
the WP2 report for confirmation.

### W0322 / W0323

- W0322: "`copy` does nothing for `uint32`" (label on `copy`; help "remove
  `copy`: `uint32` always copies"). Fires when the result type is Copy (R11).
  Disabling this lint is a `[warnings]` setting in the project config (step 7);
  nothing in step 3.
- W0323: "`mut open` says the same as `mut`" (label on `open`; help "write
  `mut string xs`"). SPEC §2 decides that this is a warning.

### Message lines, for the owner's review pass (Q19)

| Code | Message line |
|---|---|
| E0301 | `name` is used after it moved |
| E0302 | cannot change `name` while it is borrowed |
| E0303 | `p` is already borrowed with `&mut` |
| E0304 | cannot use `p.hp` while `p` is borrowed with `&mut` |
| E0305 | cannot move `name` while it is borrowed |
| E0306 | `local` does not live long enough |
| E0307 | cannot return a borrow of `s`, which belongs to this function |
| E0308 | `longest` returns a borrow, but which parameter it borrows from depends on the path |
| E0309 | `pick` returns a borrow of `b`, but its `lives` names only `a` |
| E0310 | cannot move `p.name` out of a borrow |
| E0311 | cannot change `p.hp` through a `&` borrow |
| E0312 | cannot lend `xs` as `&mut`: it is neither `mut` nor `open` |
| E0205 (open) | cannot reassign `xs`: it is `open`, not `mut` |
| E0313 | cannot give `p.hp` a value: `p` has moved |
| E0314 | `Parser` holds a borrow, so it must say what it borrows from |
| E0315 | cannot copy `w`: it holds a `&mut` borrow |
| E0316 | using a raw pointer needs an `unsafe { }` block |
| E0317 | `unsafe` is not enabled in this file |
| E0318 | file flags must be at the very top of the file / `unsafe` must be `true` or `false`, found `yes` |
| W0319 | unknown file flag `unsfe` |
| E0320 | cannot store a borrow of `b` inside `a` |
| E0321 | `grow` takes `&mut string` here, so the call must say so |
| W0322 | `copy` does nothing for `uint32` |
| W0323 | `mut open` says the same as `mut` |
| E0324 | cannot move `p.name` out of `p` without `take` |
| E0325 | cannot take `f.path` out of a `File`: it has a `drop()` |
| E0326 | `lend` passes on a `&mut` borrow you hold, and `n` is not one |
| E0327 | `take` needs a field of a variable, like `p.name` |
| E0328 | `drop` must be written `none drop()` |
| E0332 | cannot give `p.hp` a value: `p` has no value yet |

---

## 5. Owner answers, and what is still open

Answered 2026-11-10 (`owner-answers.md`, SPEC §6). The options that were offered
are gone; only the answers are kept. Anything the answers leave undecided is in
"Still open" below and is `E0901` in the code.

- **Q1 answered.** A borrow always means its target: reads are automatic,
  `r = 5;` writes through, `r = &mut other;` re-points `r` (allowed because the
  right side is a reference). No explicit `*` on borrows. (R4, R18)
- **Q2 answered.** `&` is always written for shared borrows of user functions:
  `show(&p);` (and `&mut` as before). Builtins and receivers stay implicit.
- **Q3 answered.** Passing on a held borrow is `add(lend s);`. `lend` is a new
  reserved word.
- **Q4 answered.** Two-phase receivers are allowed; only borrows still alive at
  activation are errors (3.5.6).
- **Q5 answered.** `mut` fields are special only on values you own; through any
  `&` everything is read-only.
- **Q6 answered.** `open` allows field writes, `open` parameters, and declaring
  without a value and assigning once.
- **Q7 answered.** Partial moves say `take`; plain field move is E0324;
  `take p.f leaving v` puts a value back (3.1.4).
- **Q8 answered.** No consuming `self`. **Q8b:** no hidden copies; `return
  self.name;` from a borrowed `self` is an error (E0310) with the help `return
  copy self.name;`.
- **Q9 answered.** `lives` on a function is needed only when the returned borrow
  could come from several parameters (body-derived; E0308 otherwise).
- **Q10 answered.** A struct's `lives` names its borrow fields themselves, each
  possibly borrowing from a different place. **Q10ii:** function `lives` names
  whole parameters only.
- **Q11 answered.** `copy r` on a borrow copies the target (an owned value);
  `copy n` on a Copy value warns W0322 (disableable via config `[warnings]`).
- **Q12 answered.** The 32-byte rule counts memory size including padding;
  `struct H stays { }` never copies automatically.
- **Q13 answered.** Borrowing a temporary in a declaration keeps it alive to the
  end of the block (B9, R12).
- **Q14 answered.** `address of n` makes a pointer; everything touching a
  pointer value needs `unsafe { }`; one kind `*T` (writable); `none` is null,
  tested with `p == none`; `unsafe` before a function's return type, calls need
  a block. (3.9)
- **Q15 answered.** `*T` moves (`copy p` duplicates); `&mut T` moves; `&T` copies.
- **Q16 answered.** Interface methods that may change `self` say `changes self`
  after the parameters; struct methods stay inferred. Step 6 only.
- **Q17 answered.** A method `none drop() { }` runs when the value is destroyed;
  plain `take x.f` on such a struct is an error, `take x.f leaving v` is allowed.
- **Q18 answered.** `lives`/`fails` on functions and `lives`/`with` on structs
  in either order; `drachm fmt` picks one (R14).
- **Q19: wording written by implementer, owner review pending.** Section 4 is
  final for implementation; the message table at its end is for the review.
  Codes stay in `03xx`.

Assembly functions (`unsafe uint64 cycles() asm { ... }`) are out of scope
(Later, after C header import).

### Still open

The code reports `E0901` for each of these, with the message given, so that
the case is easy to relax when the owner answers. Not in the list: things this
spec reads in the obvious way (R1-R18).

Answered 2026-11-11 (see `owner-answers.md`): #1 error, fix-it `lend s`;
#3 write `&m`, bare `m` is an error; #6 `x::drop()` is an error, new statement
`destroy x;`; #9 allowed, yields read-only `*fixed T`. These are no longer
E0901 once implemented (#1, #3 in WP0a; #6 in WP6; #9 in WP9).

1. **A borrow of a borrow.** `&r` / `&mut r` where `r` is already a borrow
   binding, `&&x`, and `&&T` / `&mut &T` in a type: "borrowing a borrow is not
   supported yet; use `lend r` to pass on a `&mut`". (By Q1 `&mut r` could mean
   a borrow of the target, which would make `lend` redundant.)
2. **`lend` on a shared borrow** (`lend q` with `q: &T`): "`lend` on a `&`
   borrow is not supported yet".
3. **A `&mut` where a `&` is expected** (argument, declaration, return):
   "using a `&mut` where a `&` is expected is not supported yet".
4. **`*r` on a borrow** (not a pointer): "`*` on a borrow is not supported yet;
   a borrow already means its target".
5. **More `take` forms.** `take … leaving` through a borrow, `take` of a Copy
   field, `take` of a whole local: "this form of `take` is not supported yet".
6. **Calling `drop` yourself** (`x::drop()`): "calling `drop` yourself is not
   supported yet".
7. **Struct borrow fields with several slots:** a field of a struct type that
   has `lives`, or a borrow of one: "a field that borrows a struct with
   `lives` is not supported yet".
8. **Pointers outside `unsafe`.** Moving, passing, returning or declaring a
   `*T` value outside an `unsafe` block (only making, copying, comparing and
   dereferencing are decided): "using a raw pointer outside `unsafe { }` is not
   decided yet". Likewise an unsafe operation inside an `unsafe` function but
   outside a block: "inside an `unsafe` function, whether a block is still
   needed is not decided yet".
9. **`address of` an immutable binding** (the pointer may be written through):
   "`address of` a binding that is not `mut` or `open` is not decided yet".
10. **`none` as a value** anywhere except against a `*T` ("`none` as a value is
    not supported yet").
11. **Two-phase for explicit `&mut` arguments** (`f(&mut x, x::len())`):
    *not* E0901; it is the ordinary conflict error (E0304), the conservative
    reading (R13).

---

## 6. Work packages

Order and parallelism:

```text
WP0a syntax & types (borrows, open, copy, take, lend, lives, unsafe, *T, spans) ─┐
WP0b file flags ─────────────────────────────────────────────────────────────────┤ (parallel)
WP1  MIR data structures, builder, printer, validator ──────────────────────────┘ (phase A parallel with WP0a, on the existing HIR; phase B after WP0a)
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

Second wave: `wp2.md` (dataflow + moves; it also takes 3.1.6 late-init E0205, which
the removal of `Flow` forces), `wp3.md` (self modes), `wp4.md` (backends on MIR,
stage 1; its eligibility rule replaces the cutover rule in the WP4 section below,
and speed is compared against a baseline built on the same machine, not PLAN
numbers). Where a brief differs from this document, the brief wins.

The first-wave packages (WP0a, WP0b, WP1) have their own briefs with exact
file ownership: `wp0a.md`, `wp0b.md`, `wp1.md`. Test programs below follow the
owner's answers; `take` is a reserved word, so helper functions are called
`consume`.

### WP0a, WP0b, WP1

See `docs/impl/wp0a.md` (syntax, types, typeck, HIR additions), `wp0b.md`
(file flags) and `wp1.md` (MIR data structures, builder, printer, validator).
WP0a and WP1 need a stronger implementer than WP0b.

### WP2: initialisation and move analyses

**Files:** new `src/mir/{dataflow,moves}.rs`, `src/mir/check.rs`; remove
`Flow` from `src/typeck.rs` **and `src/typeck/control.rs`** (2.2 item 5); split
the E0105/E0207 cases out of `tests/fail/flow.drh` into new `own_*` files.

**Build:** 3.0, 3.1 (MoveData, the four analyses, use checks E0301/E0207,
E0313, E0310, E0315, E0105 from CFG reachability). 3.1.6 is in WP5. The
builder (WP1) already reports E0324/E0325; the fail tests for them live here.

**Pitfalls:** report the *first* use after a move, once per path; a `Move`
operand inside a loop must see the move from the previous iteration
(`ReachingMoves` handles it; test it); `return x;` is a move and must not
make the drop at the end double (it doesn't: drops are elaborated). E0207
texts must stay exactly as today.

**Acceptance:** `tests/fail/unassigned.err` and `noreturn.err` unchanged and
passing. New fail tests (one file each, `.err` lines given):

```cpp
// own_use_after_move.drh
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
none consume(string s) { }
none main() {
    string s = "x";
    mut uint32 i = 0;
    while (i < 2) { consume(s); i++; }
}
```
→ `E0301`, `in the previous time round the loop`.

```cpp
// own_partial.drh
in io;
struct Player { string name; string title; }
none consume(string s) { }
none main() {
    Player p = "Illy", "Dr";
    consume(take p.name);
    io::outln(p.title);          // fine
    Player q = p;                // E0301: `p` is used after `p.name` moved
}
```

```cpp
// own_maybe_moved.drh
none consume(string s) { }
bool flip() { return true; }
none main() {
    string s = "x";
    if (flip()) { consume(s); }
    consume(s);
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

```cpp
// own_self_move.drh  (Q8b)
struct Player {
    string name;
    string nameOf() { return self.name; }
}
none main() { }
```
→ `error[E0310]: cannot move `self.name` out of a borrow`, `` help: `self` is borrowed; write `return copy self.name;` ``.

```cpp
// own_field_move.drh
struct Player { string name; }
none main() {
    Player p = "Illy";
    string m = p.name;
}
```
→ `error[E0324]: cannot move `p.name` out of `p` without `take``, `help: write `take p.name``.

```cpp
// own_take_drop.drh
struct File {
    string path;
    none drop() { }
}
none main() {
    File f = "a";
    string s = take f.path;
}
```
→ `error[E0325]: cannot take `f.path` out of a `File`: it has a `drop()``, `` take f.path leaving "" ``.

```cpp
// own_move_mut_borrow.drh  (Q15)
none add(&mut string t) { }
none fill(&mut string s) {
    add(s);
    add(s);
}
none main() { }
```
→ `error[E0301]: `s` is used after it moved`, `` pass it on with `lend s` ``.

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
none consume(string s) { }
none main() {
    string b = "x";
    consume(b);
    b = "new";
}
```
→ `error[E0205]: cannot change `b`: it is not `mut``, `assigned a second time here`.

### WP3: self-mode inference

Starting point: `SelfUse` / `report_fixed_receivers` / `check_field_change` in
`typeck.rs` (59, 586, 1367) already run this fixpoint on the HIR; move that
logic here, with the changes in 3.7 (no `mut`-field exception; `drop` is `Mut`).

**Files:** new `src/ownership/{mod,self_mode}.rs`; `src/lib.rs`.

**Build:** 3.7: evidence walk, fixpoint, reasons. Expose `Vec<Option<SelfMode>>`.
Unit tests on small programs (no codegen needed):

```cpp
struct Counter {
    uint32 n;
    mut uint32 reads;
    none bump() { self.n = self.n + 1; }      // Mut: writes self.n
    none bumpTwice() { self::bump(); self::bump(); }   // Mut: calls bump
    uint32 get() { self.reads = self.reads + 1; return self.n; }  // Mut: a `mut` field is still read-only through a borrow (Q5)
    none ping(uint32 k) { if (k > 0) { self::pong(k - 1); } }    // Shared (mutual recursion, no change)
    none pong(uint32 k) { if (k > 0) { self::ping(k - 1); } }    // Shared
}
```
Expected: bump Mut(Writes "self.n"), bumpTwice Mut(Calls bump), get
Mut(Writes "self.reads"), ping/pong Shared.

**Pitfalls:** method calls on a *field* of self whose type is another struct
(`self.inner::grow()`): look up the field type's method; the reason chain
must not loop on mutual recursion (store only the first reason).

**Acceptance:** the unit tests; plus, after WP5, the fail test in WP5 that
prints the inferred reason.

### WP4: backends on MIR

Answered in Appendix A (A3, A4).

**Files:** `src/codegen.rs`, `src/cgen.rs` (rewritten around MIR), new
`src/layout.rs`, `src/glue.rs`, new `src/mir/cleanup.rs`.

**Build:** 2.7, 2.8, 2.9. Delete the HIR-walking code paths once all tests
pass on MIR (keep them only until then; do not maintain two paths).

Order inside the package: (1) numbers, bools, calls, control flow (all the
existing tests) on both backends; run `bench/run.sh` and compare with the
numbers in PLAN step 1 (must be within noise, ±3%); (2) strings and structs,
which need real drops. Cutover rule until WP6 lands: a program is compiled from
MIR only if no MIR local in it has `needs_drop(ty)`; all others still go
through the old HIR path (2.2a). WP6 deletes the old path.

**Pitfalls:** Cranelift variables vs stack slots (`in_memory`); never write
to the source of a `Move`; fall-through layout in C (label only when needed,
`bbN:;` with the semicolon); the builder never emits `Switch` (`match` is an if-chain), so the C `switch`
path can wait.

**Acceptance:** every existing run/example test passes on both backends;
`optimised_output_matches` passes; benches within ±3% for collatz, fib,
mandel, primes. All MIR-specific invariants validated in debug builds during
`cargo test`.

### WP5: mutability

**Files:** new `src/mir/mutability.rs`; remove the remaining E0205 logic
from `typeck.rs`.

**Build:** 3.8 and 3.1.6; method-receiver checks with self-mode notes;
`open` rules (Q6: field writes, `open` parameters and declare-once are all
allowed).

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

```cpp
// own_mut_field_shared.drh  (Q5)
struct Player { string name; mut uint32 hp; }
none hurt(&Player p) { p.hp = p.hp - 1; }
none main() { }
```
→ `error[E0311]: cannot change `p.hp` through a `&` borrow`.

```cpp
// tests/run/own_open_ok.drh  (Q6)
in io;
struct Player { string name; uint32 hp; }
none rename(open Player p) { p.hp = 9; }
none main() {
    open Player p = "Illy", 1;
    p.hp = 5;
    rename(&mut p);
    open string later;
    later = "x";
    io::outln(later + p.hp::text());
}
```
`.out`: `x9`. (`rename(&mut p)` passes an `open` binding to an `open` parameter
as `&mut`; adjust only if WP0a settles the parameter spelling differently.)

### WP6: drop elaboration, workaround removal, leak tests

Answered (A5, A6): the nodes are in 2.2a; delete the drop plumbing too
(`Checker::owned`, `temps`, `scoped`, typeck.rs:885-1030, and `owned` in
control.rs).

**Files:** new `src/mir/drops.rs`; `src/mir/build.rs` (drop the temporary
deep-copy flag from WP4); `src/typeck.rs` / wherever (a) inserts copies
(delete); `runtime/strings/src/lib.rs` (double-free detection);
`tests/run.rs` (leak checking).

**Build:**
1. 3.3 elaboration with flags, including the user `drop()` rule (a type with
   `drop()` is dropped whole: glue calls `drop` first, then the fields).
2. Remove step 2's deep-copy-on-assign: inserted copies and HIR drops go;
   only `ExprKind::DeepCopy` from user `copy` remains. String assignment,
   passing and returning become moves.
3. Runtime: when `DRACHM_CHECK_LEAKS` is set (read once into a static at the
   first allocation), also keep the set of live owned pointers (a
   `Mutex<HashSet<usize>>`); `drachm_str_drop` (and any runtime path that
   frees or reallocates) checks the pointer is in the set and removes it;
   if not: print `drachm: double free of a string` to stderr and
   `abort()`. Without the variable, nothing changes (no cost).
4. Harness: it already sets `DRACHM_CHECK_LEAKS=1` and fails on `drachm:
   leaked` (tests/run.rs:59-66, 190); add `double free` to what fails. For a
   case whose expected exit code is 0, stderr must contain neither. (Panicking cases exit
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
none consume(string s) { io::outln(s); }
bool odd(uint32 i) { return i % 2 == 1; }
none main() {
    mut uint32 i = 0;
    while (i < 4) {
        Player p = "n" + i::text(), "t" + i::text();
        if (odd(i)) { consume(take p.name); }  // partial move on some iterations
        string s = "x" + i::text();
        if (i == 2) { consume(s); }            // conditional move
        i++;
    }
    string last = "end";
    consume(last);
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

```cpp
// tests/run/own_drop_method.drh
in io;
struct Guard {
    string name;
    none drop() { io::outln("drop " + self.name); }
}
none main() {
    Guard a = "a";
    if (true) {
        Guard b = "b";
        io::outln("inner");
    }
    Guard c = "c";
    io::outln("end");
}
```
`.out`: `inner`, `drop b`, `end`, `drop c`, `drop a` (reverse declaration order).

```cpp
// tests/run/own_take_leaving.drh
in io;
struct Player { string name; string title; }
none main() {
    mut Player p = "Illy", "Dr";
    string t = take p.title leaving "none";
    io::outln(t + " " + p.title);
    string n = take p.name;
    p.name = "Sam";
    io::outln(n + " " + p.name);
}
```
`.out`: `Dr none`, `Illy Sam`. Leak check clean.

Also a MIR test: `tests/mir/drop_flag.drh` = worked example 1, `.mir` lines
`Drop(_1) if _4` (adjust numbers) and no unconditional `Drop(_1)`.

### WP7: borrow checker

**Files:** new `src/mir/liveness.rs`, `src/mir/borrowck/{mod,regions,
constraints,loans,conflicts,explain}.rs`.

**Build:** 3.4, 3.5. Two-phase for receiver borrows (Q4). `lend r` reborrow (Q3); a bare `&mut`
local passed on is a move (Q15).

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
    heal(b);
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
    mut &string keep = &"a";      // a declaration temporary lives to the end of the block (Q13)
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

```cpp
// own_two_phase_live.drh  (Q4: a borrow still alive at activation)
struct Bag {
    string a;
    &string first() { return &self.a; }
    none put(&string s) { self.a = copy s; }
}
none main() {
    mut Bag b = "x";
    b::put(b::first());
}
```
→ `error[E0302]: cannot change `b` while it is borrowed`.

```cpp
// tests/run/own_two_phase.drh  (Q4)
in io;
struct Counter {
    uint32 n;
    none add(uint32 k) { self.n = self.n + k; }
    uint32 get() { return self.n; }
}
none main() {
    mut Counter c = 1;
    c::add(c::get());
    io::outln(c.n::text());
}
```
`.out`: `2`.

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
fixpoint, E0307/E0308/E0309/E0320, struct region parameters (Q10: field names; several
names allowed).

**Acceptance:**

```cpp
// own_missing_lives.drh
&string longest(&string a, &string b) {
    if (a::len() > b::len()) { return a; }
    return b;
}
none main() { }
```
→ `error[E0308]`, `returns a borrow of `a` here`, `help: write `lives a, b``.

```cpp
// own_struct_lives.drh  (Q10; added by WP0a, E0314 is a declaration check)
struct Pair lives a { &string a; &string b; }
none main() { }
```
→ `error[E0314]`, `field `b` is a borrow, but `lives` does not name it`.

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

**Build:** 3.9's checks; `address of`, `none` as a null pointer, `unsafe` functions (Q14).

**Acceptance:**

```cpp
// own_unsafe_no_flag.drh  (added by WP0a, E0317 is reported by typeck)
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

```cpp
// own_unsafe_call.drh
//--!$unsafe: true
unsafe uint32 one() { return 1; }
none main() { uint32 x = one(); }
```
→ `error[E0316]: calling `one`, an `unsafe` function, needs an `unsafe { }` block`.

```cpp
// tests/run/own_unsafe_ptr.drh
//--!$unsafe: true
in io;
unsafe none bump(*uint32 p) { unsafe { *p = *p + 1; } }
none main() {
    mut uint32 n = 41;
    unsafe {
        *uint32 p = address of n;
        bump(p);
    }
    io::outln(n::text());
}
```
`.out`: `42`. (`bump(p)` inside the block moves `p`.)

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
  `restrict`. For `&T` parameters `const T *restrict` is valid as well: nothing changes through
  a `&` (Q5 is settled: not even `mut` fields), so no exception is needed. Getting this wrong is a miscompile, so gate it
  behind the borrow checker having run and test it with a benchmark that
  would show the difference.
- By-pointer aggregates passed to `static` functions are inlined away by the
  C compiler; Cranelift does no inlining, so small Copy structs (≤ 16 bytes)
  should be passed by value in Cranelift if (b)'s ABI does not already.
  `&self` on a small Copy struct may also be passed by value *in the ABI*
  (the method cannot observe the difference, unless raw pointers compare addresses).
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

## Appendix A: LOOK FIRST checklist (answered against commit `2f26fb3`)

| # | Question | Answer | Affects |
|---|---|---|---|
| A1 | How is `Type` extended? Still `Copy`? Interned? | `Type` is `Copy + Hash` (types.rs:9-22). Structs are `Struct { id, name: &'static str }`, the name leaked once by `types::intern` (types.rs:25). No `Ref`/`Ptr`. Add them with the same trick: `to: &'static Type`, interned in a static set (2.4). Exhaustive matches to extend: `hir::scalar_size` (hir.rs:96), `codegen::cl_type` (codegen.rs:40), `cgen::c_type` (cgen.rs:133). | 2.4, WP0a |
| A2 | Where are struct definitions stored? | typeck-private `StructInfo` / `FieldInfo` (`mutable`, `public`) / `MethodInfo` (typeck.rs:29-57); backends see `hir::StructDef { name, fields, size, align, owns }` and `FieldDef { name, ty, offset }` (hir.rs:44-58), which lacks `mutable`. Add `mutable`, `copy`, `stays`, `has_drop`, `lives` (2.2 item 4). Methods are ordinary `hir::Func`s. | 2.3, 3.7, 3.8 |
| A3 | Layout function? ABI? | Shared and already in HIR: `hir::layout` (hir.rs:102), filled in at typeck.rs:541; `Program::size_align` (hir.rs:61). Size includes tail padding. There is no by-pointer threshold: Cranelift passes every string/struct as a pointer to a fresh copy the callee takes over, results through a hidden first pointer (codegen.rs:51-63, 974-994); C passes by value, `self` by pointer (cgen.rs:123). | 2.7, 2.9, WP4 |
| A4 | String representation; literals? | 24-byte `drachm_str {ptr, len, cap}`: a stack slot in Cranelift (codegen.rs:249), a by-value struct in C. A literal is a view with `cap == 0` (codegen.rs `into`, `ExprKind::Str` arm), so R10 holds. | R10, 2.9 |
| A5 | Which HIR nodes are step 2's copies and drops? Is user `copy x` the same node? | `ExprKind::{Copy, Move, Temp, Scoped}` and `Stmt::Drop` (hir.rs:154-169, 281-292), inserted by `copy()`/`scoped()` (typeck.rs:916-927) and `Checker::owned/temps` (typeck.rs:885-1030, control.rs). User `copy x` does not exist yet; WP0a adds a separate `DeepCopy` node. | 2.2a, WP6 |
| A6 | Where are string temporaries dropped? | End of the statement: `Checker::take_temps` (typeck.rs:1009) collects `Temp`s into a `Scoped` wrapper. | B4 |
| A7 | How is `self` passed; method calls; operator calls? | `self` is parameter 0 (`has_self`), refers to the caller's storage when the argument `is_place()` (hir.rs:115-130, 205). A method call and an operator-method call are both `ExprKind::Call(FuncId, args)` with the receiver as `args[0]`; `static` methods have none. `self_local` returns local 0 (typeck.rs:1448). | 2.5, 3.7 |
| A8 | Does typeck reject writes to non-`mut` fields or to `self` fields? | Yes: `check_field_change` (typeck.rs:1367) reports E0205 for bindings, and for `self` records `SelfUse.changes` instead; `report_fixed_receivers` (typeck.rs:586) is the existing "changes self" fixpoint. Both honour `mut` fields (`mut_field`), which Q5 now removes for borrows. | 3.7, 3.8, WP3, WP5 |
| A9 | HIR for `for`, `break N`, `skip`, `match`; error codes? | There is none: control.rs lowers them to `Stmt::If { label, branches }`, `Stmt::Loop { label, cond, body, step }` and `Jump::{Break, Continue, Leave, Next}` (hir.rs:154-190). Codes used there: E0011, E0012, E0101, E0104, E0109, E0901. | 2.5, WP1 |
| A10 | Runtime for string iteration, string `match`, globs? | None: `for (string c of s)` is E0901 (control.rs:481) and `match` on text is E0901 (control.rs:613). Nothing to lower. | 2.5 |
| A11 | Did (c) change `Flow`? | `Flow` (typeck.rs:852) is also threaded through `Target::{Loop, Level}` in control.rs:36-51 and the `chain` / `looped` / `for_each` / `match_stmt` functions; removing it touches control.rs (2.2 item 5, WP2). | WP2 |
| A12 | New codes added by (a)/(b)/(c)? | E0009, E0108, E0112-E0114, W0208, E0209-E0213 (plus E0011/E0012 in control). All `03xx` codes are free. | section 4 |
| A13 | Does `lib::check` still return `hir::Program`? | Yes: `check(src) -> (Option<hir::Program>, Vec<Diagnostic>)` returns `None` if any error (lib.rs:19-30); `build(&hir::Program, out, optimise)` (lib.rs:36); `main.rs` `front_end` (main.rs:69). | 2.1 |
| A14 | Is the leak report used by the harness? | Yes: `run_program` sets `DRACHM_CHECK_LEAKS=1` and `leak_report` fails on `drachm: leaked` (tests/run.rs:59-66), in `run`, `examples` and `optimised_output_matches`. Runtime side: a live counter only (runtime/strings/src/lib.rs:53-61); no double-free detection. | WP6 |

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
