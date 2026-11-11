# WP1: MIR data structures, builder, printer, validator

Implementer: **needs a strong model** (a new IR, CFG construction with scopes,
careful evaluation order). Nothing uses the MIR except tests when this lands;
the HIR backends keep running. Read first: `step3-ownership.md` 2.1-2.5 and 2.7
(as amended), Appendix A and B. This brief only lists what differs from, or
pins down, that text.

## Two phases

- **Phase A (starts now, parallel with WP0a and WP0b).** Translate the *current*
  HIR (hir.rs at `2f26fb3`) completely and faithfully. Needs nothing from WP0a.
- **Phase B (after WP0a is merged).** Real spans, `Stmt::Decl` scope frames,
  the new HIR nodes, ignoring the legacy ownership nodes (spec 2.2a), E0324 /
  E0325. Do not start B before A is committed and WP0a has landed.

## Files you own

New: `src/mir/{mod,index,ty,build,print,validate,traversal}.rs`, `tests/mir/`.
Edit: `src/main.rs` (the `--dump-mir` flag only), `tests/run.rs` (append two
tests), `src/lib.rs` (**one line**: `pub mod mir;` between `pub mod link;` and
`pub mod parser;`). Phase B also: `src/diag/codes.rs` (append E0324 and E0325 after
the WP0a entries; WP0a has merged by then).

Do not edit any other existing file. WP0a edits `hir.rs`: you read it only
through two functions in `build.rs`, `fn place(&hir::Place) -> mir::Place` and
`fn expr_span(&hir::Expr) -> Span` (phase A returns `Span::new(0, 0)`), so the
merge fix-up after WP0a is two functions plus new match arms.

## Data structures

Spec 2.3 verbatim except:

- `hir::FuncId`/`StructId`/`LocalId` are plain `usize`: per-function tables are
  `Vec`, indexed by `FuncId`. Your own `newtype_index!` types are `Local`,
  `BlockId`, `ScopeId`, `StrId`, `MovePathId` later. `IndexVec<I, T>` in
  `index.rs` (about 40 lines).
- `Program { bodies: Vec<Option<Body>>, main: FuncId, strings: IndexVec<StrId,
  String>, self_modes: Vec<Option<SelfMode>>, lives: Vec<LivesSig> }`. Define
  `SelfMode`/`LivesSig` as one-variant placeholder enums for now (WP3, WP8 own
  them: `pub enum SelfMode { Shared, Mut }`, `pub enum LivesSig { Unknown }`).
  Structs and layouts come from `hir::Program` (no copies).
- `Callee { Func(FuncId), Str(hir::StrFn), Out { newline: bool }, Text,
  Radix { base: u32, upper: bool }, Join }` (no `RtFn`). For `Str`, `Out`,
  `Text`, `Radix`, `Join` the operands are exactly the HIR arguments in order;
  string-typed operands are `Copy` of a `&string` temp (B3).
- `Type` for locals: `types::Type` as it is today. Phase B adds `Ref`/`Ptr` from
  WP0a; `place_ty` then handles `Deref`.
- `Statement` carries `span` and `unsafe_ctx: bool` from the start (phase A:
  dummy span, `false`).
- `Body` fields as in the spec plus `fallthrough_end: Option<BlockId>`.

Public API (all in `mir/mod.rs`):

```rust
pub fn build_program(hir: &hir::Program, diags: &mut Vec<Diagnostic>) -> Program;
pub fn print_program(p: &Program, hir: &hir::Program) -> String;   // Appendix B
pub fn validate(p: &Program, hir: &hir::Program) -> Result<(), Vec<String>>;
pub fn is_copy(hir: &hir::Program, ty: Type) -> bool;       // 2.7; phase A computes
        // "all fields copy && size <= 32" itself from hir::StructDef; phase B reads StructDef.copy
pub fn needs_drop(hir: &hir::Program, ty: Type) -> bool;
```

## Phase A: what the builder does

One `Builder` per `hir::Func` as in 2.5 (B1, B2, B3, B6, B7; B4/B5/B9-B11 and
scope frames are phase B). Map HIR to MIR:

| HIR | MIR |
|---|---|
| params `0..func.params` | locals `_1..` (`_0` is the return place); `Body.arg_count = params`; `has_self` param 0 is an ordinary `Param` named `self` |
| other `func.locals` | locals after the params, same order, `LocalKind::User` or `Temp` if the name is hidden (hidden names start with `$`; check how typeck names them); `StorageLive` for all of them in `bb0` |
| `Assign(place, e)` | `place = Use(⟦e⟧)`, or for a call, via a temp (B7) |
| `If`, `Loop`, `Jump`, labels | spec 2.5 "Control flow" table: a `HashMap<Label, Target>`; **no scope frames in phase A**, so a `Jump` just `goto`s |
| `Return(e)` | `_0 = Use(⟦e⟧); return` (jump to a single return block is fine) |
| `Expr(e)` | `⟦e⟧`, result discarded |
| `Drop(place)` (legacy) | `Drop { place, flag: None, reason: ScopeEnd }` |
| `Copy(e)` (legacy) | `_t = DeepCopy(as_place(e))` |
| `Move(l)` (legacy) | `Operand::Move(l)` |
| `Temp(l, e)` | `l = ⟦e⟧`; then `l` is the place |
| `Scoped(e, locals)` | `⟦e⟧` into a temp, then `Drop(l)` for each listed local (reason ScopeEnd), value = the temp |
| `Local`, `Field`, `Int`, `Float`, `Bool`, `Str(s)` | places / `Const` (`Const::Str(StrId)` interned in `Program.strings`) |
| `Struct(fields)` | field values into temps in listed (source) order, then `Aggregate(sid, ops in field order)` |
| `Neg/Not/BitNot/Binary/Shift/IntPow/Convert/Logic/IncDec/Text/Radix` | table in 2.5; `Logic` is control flow |
| `Call(f, args)` | args left to right; an owning-type or struct argument is `Move`/`Copy` by `is_copy`; a `has_self` call's `args[0]`, when a place, is passed as `_s = &mut place` (always `&mut` until WP3; keep it in one function `receiver_mutable(f) -> bool`) |
| `StrCall(f, args)`, `Out`, `Join` | string args that are places: `_a = &place` (`&mut` for the first arg of `Append`); literals and temps are first stored in a temp |

Rules that matter:

- Evaluation order is left to right and every non-trivial operand goes through a
  fresh temp (B1). `f(i++, i)` must pass old `i` then new `i`.
- A call's result is written to a fresh temp, then moved into its place (B7),
  and every call gets a fresh target block (drop elaboration relies on it).
- `while (true)` / no-cond loops get no exit edge from the head (R7); a function
  whose last block is reachable and returns a value ends in `Unreachable` and
  its block is recorded as `fallthrough_end`.
- `Switch` exists in the data structure, printer and validator but is never
  emitted.
- Builder never reports type errors. Anything it cannot lower is `unreachable!`
  with the node name (a bug in this package, not a user error).

`print.rs`: Appendix B format. Binary ops print as the `Debug` name of
`hir::BinOp` (`Add(copy _1, const 1u32)`); a user local prints its name in a
`//` comment on its declaration line; one function per block, blank line
between functions.

`validate.rs`: V1, V2, V4-V9 of 2.3 (V3 only after WP2). Add: reducibility check
under `cfg(debug_assertions)` (every retreating edge of a DFS is a back edge to
a dominator) and "every block is reachable or removed by cleanup".

`traversal.rs`: `predecessors(&Body)`, `reverse_postorder(&Body)`,
`successors(&Terminator)`. (SCCs come with WP5.)

`main.rs`: `drachm check --dump-mir FILE` runs the normal check, and on success
prints `print_program` to stdout and exits 0; with errors it behaves as before.

## Tests (phase A)

1. **Build-everything test** (`tests/run.rs`, `#[test] fn mir_builds`): for every
   `.drh` in `tests/run` and `examples`, call `drachm::check`, then
   `mir::build_program` and `mir::validate`; fail with all messages. Uses the
   library API, so it needs no new CLI.
2. **MIR golden tests** (`tests/run.rs`, `#[test] fn mir`, same matching as fail
   tests: each non-blank line of `NAME.mir` must occur in `check --dump-mir`
   stdout): add `tests/mir/fib.drh`
   ```
   uint32 fib(uint32 n) {
       if (n < 2) { return n; }
       return fib(n - 1) + fib(n - 2);
   }
   none main() { }
   ```
   `.mir` lines: `fn fib(_1: uint32) -> uint32`, `Lt(copy _1, const 2u32)`,
   `call fib(`, `Add(`, `return`.
   `tests/mir/loop_break.drh`
   ```
   none main() {
       mut uint32 i = 0;
       while (true) {
           if (i == 3) { break; }
           i++;
       }
   }
   ```
   `.mir` lines: `Eq(copy _1, const 3u32)`, `goto -> bb`, `Add(copy _1, const 1u32)`;
   and the printed `main` must have no `unreachable` (the loop exits only
   through the break). `tests/mir/forever.drh`: `uint32 f() { while (true) { } } none main() { }`;
   `.mir` line `fn f() -> uint32`; no `return` and no `unreachable` in `f`
   (the loop only goes back to its head).
3. Unit tests in `mir/` for `reverse_postorder`, `predecessors`, `is_copy`
   (struct of two `uint64` and a `uint32` is 24 bytes: Copy; four `uint64`
   plus a `uint8` is 40: not).

## Phase B (after WP0a)

- Spans: `expr_span`, `place` read `Expr.span` / `Place.span`; all statements
  and `Arg`s get real ones; scope-end drops use `Branch.end` / `Loop.end` /
  `Func.body_end`.
- Switch the ownership handling to spec 2.2a: ignore `Copy`/`Scoped`-lists/`Stmt::Drop`, add
  scope frames from `Stmt::Decl` and the block structure (B4, B5, B8), emit
  `StorageLive/Dead`, `Drop` with `DropReason`, `Jump` drops (spec 2.5 Control
  flow), B9-B11 (temporaries in declarations, field moves, two-phase receivers).
- Lower the new nodes (2.5 table: `Borrow`, `Deref`, `DeepCopy`, `Take`, `Lend`,
  `AddressOf`, `NullPtr`, `Unsafe`); `Operand::Move` of a `&mut`/`*T` local;
  `is_copy` reads `StructDef.copy`; `unsafe_ctx` from `Stmt::Unsafe`.
- E0324 (plain move of a non-Copy field of an owned path) and E0325 (plain `take`
  on a struct with `drop()`) are reported from the builder via `diags`. Texts:
  section 4. Add the two codes with `explain` text.
- Extra tests: `tests/mir/scope_drops.drh` and `break_drops.drh` from the old WP1
  text (`step3-ownership.md` history, or: three nested-scope string locals, and
  a `break` inside a loop that owns a string), `tests/fail/own_field_move.drh`
  and `own_take_drop.drh` (texts in the step3 WP2 section), and `mir_builds`
  now also runs over a new `tests/mir/borrows.drh` using every new node.

## Done when

`cargo test` is green (all existing tests unchanged: the HIR backends are
untouched, so both backends still behave identically); `mir_builds` and `mir`
pass; `drachm check --dump-mir` on `examples/hello.drh` prints a readable dump;
in debug builds `validate` runs after every `build_program`.

## Pitfalls

- A plain struct larger than 32 bytes now builds as `Move`, not `Copy` (S5), so
  programs that reuse such a struct after passing it will get E0301 once WP2
  lands. That is the specified behaviour; note affected tests, do not hack
  `is_copy`.
- `Scoped` evaluates its value *before* dropping its locals: the value must be
  in a temp first, or the drop would free what it reads.
- `Temp(l, e)` appears inside conditions of `while` loops and inside the
  unevaluated side of `&&`: the local is "maybe initialised" there; that is
  normal (drop elaboration handles it), but the builder must not assume a
  `StorageLive` dominates every use. Hence hidden locals get theirs in `bb0`.
- `IncDec` on a field place and on a local; compound assignment arrives from
  typeck as `Assign(place, Binary(.., place_read, v))`, already lowered.
- Do not let `print` order depend on a `HashMap` (golden tests match lines, but
  stable output keeps diffs readable): iterate blocks and locals by index.
- Cranelift/C backends must not see the MIR yet; do not edit them.
