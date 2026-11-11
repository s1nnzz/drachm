# WP4: both backends consume MIR (stage 1: numeric programs)

Implementer: **needs a strong model** (two code generators, performance
sensitive, must stay bit-identical). Runs in parallel with WP2 and WP3. Starts
after WP0a and WP1 phase B are merged. Read first: `step3-ownership.md` 2.7
(layouts), 2.8, 2.9, 7.3, 7.4, and the existing `src/codegen.rs` / `src/cgen.rs`
(you are replacing their statement/expression walkers).

## Goal

A program whose MIR is simple enough (defined as `eligible` below) is compiled
from the **MIR** by both backends, with identical behaviour and no slower than
today. Every other program keeps going through the HIR backends until WP6.
The HIR backend code stays until WP6 deletes it (this changes the spec's "do
not maintain two paths": during this stage it is the safety net and the A/B
baseline; do not modify its behaviour).

Correction to the spec's cutover rule ("MIR only if no local needs drop"):
every benchmark prints with `io::outln(x::text())`, which makes a temporary
`string`, so that rule would leave all benches on the old path and the speed
comparison would measure nothing. The rule below admits compiler-made string
temporaries.

## Files you own

New: `src/mir/analysis.rs` (the `in_memory` marking), `src/mir/cleanup.rs`
(spec 2.8 and `eligible`), `src/codegen_mir.rs`, `src/cgen_mir.rs` (the MIR
generators; separate files so the old generators stay untouched and your merge
with nobody else's work is trivial), `tests/backends.rs`.

Edit:
- `src/codegen.rs`, `src/cgen.rs`: only to make existing helpers reachable
  (`pub(crate)`) or to move pure helpers (arithmetic, conversion, division,
  literals, C type names, `work_type`) into shared form that **both** old and
  new generators call. Moving is better than copying. The behaviour of the HIR
  path must not change by a byte of output for programs that use it. Nobody
  else edits these two files in this wave.
- `src/lib.rs`: **the body of `build()` only**, plus new `pub mod codegen_mir;
  pub mod cgen_mir;` lines (alphabetical places) and one new function
  `build_unchecked` (below). WP2 owns `check()`; WP3 owns the line
  `pub mod ownership;`.
- `src/mir/mod.rs`: add `pub mod analysis; pub mod cleanup;` **directly before
  the existing `pub mod build;`** (WP2 adds its lines after `pub mod validate;`;
  WP3 edits the `SelfMode` block). Nothing else.
- `src/hir.rs`: nothing. If you need a field, say so in your report instead.

Do not touch: `src/typeck*`, `src/mir/{build,print,validate,ty,traversal}.rs`,
`tests/run.rs`, `src/diag/*`, `link.rs`, `main.rs`, `bench/*`.

## LOOK FIRST

1. The final shapes after WP1B: `Type::Ref`/`Ptr` in `types.rs`, whether
   `LocalDecl.borrow` is gone, `place_ty` and `operand_ty` helpers in
   `mir/ty.rs` (you need the type of every operand and place), `Body.name`
   (no `drachm_f_` prefix: take the symbol from `hir.funcs[f].symbol`).
2. Which statements `Drop` appears for before elaboration (the builder emits
   `Drop` for every local whose type `needs_drop`, unconditionally; for
   compiler string temps that are never moved this is exactly right).
3. `lib::build` after WP0a: the `uses_borrows` gate that returns
   "borrows and pointers cannot be built yet". **Keep it** for the public
   `build`: borrow checking (WP7) does not exist, so user-written `&` programs
   must stay unbuildable. The compiler's own receiver borrows (`&self`) are not
   user-written and are not what `uses_borrows` reports; confirm that.
4. How `Callee::Str(StrFn)` / `Text` / `Radix` / `Join` / `Out` map to runtime
   functions today: the old generators call them by name (`rt(...)` in
   `codegen.rs`, `drachm_str_*` in `cgen.rs`). Reuse the same names and
   argument conventions: the runtime in `runtime/` is shared and unchanged.
5. How the old Cranelift ABI passes struct/string parameters (a pointer to a
   fresh copy that the callee owns; results through a hidden first pointer;
   `self` by address). Your MIR calling convention must be call-compatible with
   the old one **within one program** only if some functions go MIR and others
   HIR. They do not: eligibility is per program, so the two paths never mix and
   you may pick any convention that is identical in both of your backends. Keep
   the old one unless you have a reason (less to learn).
6. `emit_c_main`, the entry glue in both backends: unchanged, shared.

## Eligibility (the cutover rule)

```rust
// src/mir/cleanup.rs
/// Can this program be compiled from MIR in stage 1? Pure function of the MIR.
pub fn eligible(hir: &hir::Program, p: &Program) -> bool;
```
True when, for every body:
- no `Switch`, `Panic`, `RawRef` (pointers wait for WP9) and no `DeepCopy`;
- no `Statement::Drop` with a `flag` (elaboration does not exist yet);
- no user, parameter or return local (`LocalKind::{User, Param, Return}`)
  whose type `needs_drop`;
- every local that `needs_drop` is a `Temp` with exactly this life: assigned
  once by a `Callee::Text`, `Radix`, `Join` or user-function call `dest`, or by
  `Use(Const(Str))`; used only as the operand of an `Rvalue::Ref` (and `Drop`);
  never the operand of `Move`/`Copy`, never in `Aggregate`, never passed to a
  call directly (so it can never be moved, and its unconditional `Drop` at the
  end of the statement is exact);
- no struct type with a `drop()` (`has_drop`), and no `Type::Ptr`.
Everything else returns false. `eligible` is allowed to be conservative and
may be widened as long as the leak and double-free tests stay clean. WP6
deletes it.

`tests/backends.rs` prints the list of eligible and ineligible programs of
`tests/run` and `examples` in the test output; in your report give the counts.
Numeric-only programs with plain structs (no `string`) and method calls must be
eligible.

## What to build

1. **`analysis.rs`**: `pub fn mark_in_memory(hir: &hir::Program, body: &mut Body)`
   sets `LocalDecl.in_memory` for: struct- and string-typed locals; any local
   that is the base of a place used in `Rvalue::Ref`; parameters of struct/string
   type. Scalars never borrowed stay `false`.
2. **`cleanup.rs`**: spec 2.8: remove `StorageLive/Dead`, `FakeRead`, `Nop`;
   drop unreachable blocks; merge `A -> Goto(B)` when `B` has one predecessor;
   retarget jumps to empty `Goto` blocks. Never reorder statements or change
   operands. Block ids must stay valid (renumber densely and rewrite every
   reference, or keep a `BlockId` map). Run `validate` after it in debug builds
   (it must still hold V1, V2, V4, V5).
3. **`build()` in `lib.rs`**:
   ```rust
   pub fn build(program: &hir::Program, out: &Path, optimise: bool) -> Result<(), String>;  // unchanged signature
   pub fn build_unchecked(program: &hir::Program, out: &Path, optimise: bool) -> Result<(), String>;
   // build = the uses_borrows gate, then build_unchecked. build_unchecked: used by tests of
   // programs with user borrows before WP7 exists; document "tests only".
   ```
   Inside: build the MIR (`mir::build_program` into a throwaway diags `Vec`; the
   errors were reported by `check`), run `mark_in_memory` and `cleanup` on every
   body, then if `eligible` **and** the environment variable `DRACHM_BACKEND`
   is not `hir`, call `codegen_mir::compile` / `cgen_mir::generate`; else the
   old calls. `DRACHM_BACKEND=hir` is the A/B switch for tests and benchmarks;
   `DRACHM_BACKEND=mir` makes an ineligible program an `Err` instead of falling
   back (used by tests). The MIR is built twice per compile (once in `check`);
   fine for now.
4. **`codegen_mir.rs`** (Cranelift), spec 2.9:
   - one Cranelift block per MIR block, created up front; emit in RPO;
     `seal_all_blocks()` once at the end;
   - locals with `in_memory == false` are `Variable`s (`declare_var` /
     `def_var` / `use_var`) exactly like today's scalars, so numeric code is the
     same SSA it is now; `in_memory` locals get a stack slot sized from
     `hir.size_align`;
   - `Rvalue`s reuse the existing `binary`, `divide`, `convert`, `halves`
     helpers; integer `/` and `%` keep their zero check; shifts and `IntPow`
     as today;
   - `Operand::Move` of a struct/string argument passes the source's address
     (no copy); `Copy` of an in-memory aggregate first copies into a fresh stack
     slot (the callee may change a `mut` parameter);
   - `Drop { place, flag: None }` of a `string`: the inline `cap != 0` test and
     the runtime drop call, as today (`drop_value`);
   - calls: user functions through the ABI of LOOK FIRST 5, runtime calls by
     name; `Unreachable` is `trap(user 1)`, as today's dead end.
5. **`cgen_mir.rs`** (C), spec 2.9: every MIR local declared at the top of the
   function (drop flags will be `uint8_t`, none yet); blocks in RPO; label
   `bbN:;` only for blocks jumped to other than by fall-through; `Goto` to the
   next emitted block prints nothing; `If` prints `if (c) goto A; goto B;` with
   the fall-through side dropped (negate the condition if needed); reuse
   `work_type`, the wrapping casts, 64-bit division fast path and float
   constants of today's `cgen.rs`; `Unreachable` is `__builtin_unreachable();`.
   Add the `_Static_assert` for struct layouts after each struct definition
   (spec 2.7).
6. **Deterministic C.** The generated C text must not depend on hash order
   (benchmarks and bug reports diff it).

## Tests to add

1. `tests/backends.rs`:
   - `mir_backend_matches_hir`: for every `.drh` in `tests/run` and `examples/`
     for which `drachm::mir::cleanup::eligible` holds (use the library API to
     check, run `drachm::check`, build the MIR), build four executables
     (plain and `-O`, each with `DRACHM_BACKEND=hir` and `DRACHM_BACKEND=mir`
     passed to the `drachm` binary), run all four with `DRACHM_CHECK_LEAKS=1`, and
     require identical stdout, exit code, and no leak report. Copy the helpers
     from `tests/run.rs` (do not edit that file).
   - `ineligible_falls_back`: a program with a `string` local builds fine with no
     env var and is rejected by `DRACHM_BACKEND=mir` with an error naming
     "not supported by the MIR backend yet".
   - `borrows_through_mir`: via `drachm::build_unchecked`, a small program using
     user-written `&` and `&mut` of numbers and plain structs (e.g.
     `none bump(&mut uint32 n) { n = n + 1; } none main() { mut uint32 a = 1;
     bump(&mut a); io::outln(a::text()); }` printing `2`; and a `&Point`
     read parameter) builds both ways and prints the same. Use the syntax WP0a
     accepts (it has `tests/borrows.rs` examples).
2. Unit tests in `cleanup.rs` (merge chains collapse; unreachable blocks go;
   a loop is untouched; `validate` holds afterwards) and `analysis.rs`
   (borrowed scalar is in memory, unborrowed scalar is not, struct is).
3. All existing tests unchanged: `run`, `examples`, `optimised_output_matches`,
   `mir_builds`, `mir`.

## Measuring speed and compile time (must be no worse)

`bench/run.sh` builds release, compiles each `bench/*.drh` with `-O` and plain,
checks that Drachm, C and Rust print the same, and prints best-of-5 wall times
in ms with columns `drachm -O`, `c`, `rust`, `plain`. The numbers in PLAN.md
come from other machines; **never compare to them.** Compare against a baseline
built from the commit just before your first commit, on this machine, in one
sitting:

```sh
git worktree add $SCRATCH/base <the commit before WP4>   # SCRATCH = your scratchpad directory
(cd $SCRATCH/base && bench/run.sh) | tee $SCRATCH/base1.txt
bench/run.sh | tee $SCRATCH/new1.txt
(cd $SCRATCH/base && bench/run.sh) | tee $SCRATCH/base2.txt
bench/run.sh | tee $SCRATCH/new2.txt
```
Alternatively use one binary and `DRACHM_BACKEND=hir bench/run.sh` against
plain `bench/run.sh` (the env var is inherited by the `drachm` runs inside the
script). Take the better of the two runs for each side and each column.
Acceptance: for `collatz`, `fib`, `mandel`, `primes` the new `drachm -O` and
`plain` columns are within +3% of the baseline (faster is fine). `strings` runs
on the old path (user strings are ineligible), so it is unchanged by
construction; check it is within noise as a sanity check on your harness
(if it differs by more than 3%, the machine is too noisy: rerun). If a bench
regresses: for `-O` diff the generated C (call `cgen_mir::generate` and
`cgen::generate` on the same HIR in a throwaway test and compare the hot
function), for plain look at the loop's Cranelift IR (`CRANELIFT` verbose
options or a quick `eprintln!` of the function). Paste the final two tables in
your report.

Also time `cargo test` before and after; it should not grow noticeably.

## Done when

`cargo test` green, including `tests/backends.rs`; benches within +3% as
above, tables in the report; the four numeric benches are actually eligible
(assert it in `tests/backends.rs`: `eligible` is true for each of
`bench/{collatz,fib,mandel,primes}.drh`); `DRACHM_BACKEND=mir drachm run
examples/hello.drh` works on both backends; in debug builds `validate` runs
after cleanup.

## Pitfalls

- Never write to the source of a `Move`. `Copy` of an in-memory aggregate needs
  a copy.
- A MIR local with no definition on some path (late-initialised `uint32 x;`):
  Cranelift's `use_var` on a variable with no reaching `def_var` yields zero in
  the frontend; confirm this for the version in `Cargo.toml`, otherwise
  `def_var` a zero in the entry block for every scalar local (dead stores cost
  nothing after optimisation). In C all locals are declared at the top, so a
  `goto` over an initialiser is not a problem; do not initialise in the
  declaration for performance-sensitive scalars.
- The builder gives every call a fresh target block and writes call results to a
  temp (B7). In C these are free; in Cranelift make sure the temp is a
  `Variable`, not a stack slot, for scalar results.
- 8/16-bit integers and the `work_type` widening: C arithmetic on `uint8_t`
  promotes to `int`; use the existing wrapping casts per `Rvalue`, not new ones.
- `bbN:;` needs the semicolon (a label before a declaration or `}`).
- Block layout is RPO but the loop condition block must still be reached by
  fall-through where possible, or the C compiler's loop rotation is the only
  thing saving you. Compare the generated C of `collatz` before/after.
- `Callee::Out` / `Text` etc. take `&string` temps; the operand is a pointer,
  the runtime functions take a pointer already (as today's `Place` operands).
- Do not add a hash map keyed by `Local` iteration order to anything that is
  printed or emitted.
- The `uses_borrows` gate stays in `build()`. Only `build_unchecked` skips it.
- Struct `owns` (HIR) differs from MIR `needs_drop` for structs with
  `drop()`; rely on the MIR functions in `mir/mod.rs`.
