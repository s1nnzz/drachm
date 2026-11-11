# WP6: drop elaboration, drop flags, removal of the step 2 workaround, one backend path

Implementer: **needs a strong model** (a MIR pass whose bugs are double frees
and leaks, plus changes in both code generators and the builder; must stay
bit-identical between backends and not slow the benchmarks). Runs in parallel
with WP5 and the units work (file ownership below). WP2, WP3 and WP4 are merged.
Read first: `step3-ownership.md` 2.2a (legacy nodes), 2.7 (drop and copy glue,
layouts), 2.8, 2.9, 3.3 (elaboration, flags, worked example 2), 7.2, WP6 in 6;
`wp4.md` (what the MIR backends look like and why `eligible` exists); owner
answers Q7, Q8b, Q11, Q13, Q17, #6 and "drop()+take" in `owner-answers.md`; and
the code: `src/mir/{build,cleanup,analysis,moves,check}.rs`,
`src/codegen_mir.rs`, `src/cgen_mir.rs`, `src/lib.rs`, `runtime/strings/src/lib.rs`,
`tests/{run,backends}.rs`. This brief says where it differs from the spec;
**where they differ, this brief wins.**

## Goal

After this package there is exactly one code generation path:

1. Drops are **elaborated** on the MIR: a `Drop` of a possibly-moved value gets a
   drop flag, a `Drop` of a value that is certainly moved disappears, a partly
   moved struct is dropped field by field.
2. Both MIR backends implement everything the builder can produce (flags,
   `DeepCopy`, user `drop()`, `destroy x;`) so **every program is eligible**.
3. The HIR backends (`codegen::compile`, `cgen::generate`), the cutover rule
   (`cleanup::eligible`, `ineligible_reason`, `analyse`, `moved_out_temps`) and
   `DRACHM_BACKEND` are deleted.
4. Step 2's deep-copy-on-assign workaround and the legacy HIR ownership nodes
   (`Copy`, `Move`, `Temp`, `Scoped`, `Stmt::Drop`) are gone from typeck, the HIR
   and the builder. Strings and strings-holding structs move on assign, pass and
   return; `copy x` (the `DeepCopy` node) is the only duplication.
5. `x::drop()` is already an error (E0331); `destroy x;` runs `drop()` now; a
   struct's `drop()` runs when it is dropped, before its fields.
6. The leak check also detects double frees, and every test and example runs
   under both checks.

The `uses_borrows` gate in `lib::build` stays (WP7 removes it).

## Files you own

New: `src/mir/drops.rs`, `tests/run/own_*.drh` + `.out` (listed below),
`tests/mir/drop_flag.*`, `tests/leaks.rs` (if you need a Rust-level test).

Edit:
- `src/mir/build.rs`: all legacy-node handling goes (2.2a table), `Temp`
  becomes the builder's own temp creation, `DeepCopy` only from user `copy`.
- `src/hir.rs`: delete `ExprKind::{Copy, Move, Temp, Scoped}`, `Stmt::Drop`, and
  every comment that describes them. Keep `DeepCopy`.
- `src/typeck.rs`, `src/typeck/control.rs`, `src/typeck/strings.rs`: delete the
  drop plumbing: `Checker.owned`, `Checker.temps`, `scoped`, `take_temps`,
  `owned_from`, `drop_all`, the `self.owned(e)` wrappers at their ~12 call
  sites (they become the plain expression), the `owned` depth fields of
  `Target::Loop/Level`, and everything that builds `Copy/Move/Temp/Scoped/Drop`.
  Find them with `grep -n "owned\|temps\|scoped\|ExprKind::Copy\|ExprKind::Move\|Temp(\|Scoped(\|Stmt::Drop" src/typeck.rs src/typeck/*.rs`.
  **Do not touch** `report_fixed_receivers`, `check_field_change`,
  `check_assignable`, `FixedReceiver`, `SelfUse` (WP5 deletes those) or the
  number/units code paths (the units work adds hook calls in `Globals::resolve`,
  `binary_vals`, `into`, the `"to"`/`"text"` methods and the end of `check`).
  Do not reformat the file. Whoever merges second rebases; the hunks are
  disjoint by function.
- `src/mir/mod.rs`: add `pub mod drops;`; remove nothing else except what the
  deletions below force.
- `src/mir/cleanup.rs`: delete `eligible`, `ineligible_reason`, `analyse`,
  `moved_out_temps`, `Def`, `TempFacts` and their tests. Keep the cleanup
  itself, `operand_place`, `rvalue_places` if used elsewhere.
- `src/mir/analysis.rs`, `src/mir/validate.rs`: drop-flag related changes only
  (flag locals are bool scalars, never `in_memory`; validate: a `Drop` with a flag
  names a `DropFlag` local; V-rules about `Drop` after elaboration).
- `src/codegen.rs`, `src/cgen.rs`: delete the HIR walkers and everything only
  they use. Keep what `codegen_mir.rs`/`cgen_mir.rs` call (the shared helpers WP4
  moved: arithmetic, conversion, literals, C type names, `work_type`, `emit_c_main`,
  entry glue). If that leaves a small shared file, rename it (`codegen_common.rs`,
  `cgen_common.rs`) in a separate commit first so the diffs stay readable.
- `src/codegen_mir.rs`, `src/cgen_mir.rs`: flags, `DeepCopy`, user `drop()`,
  struct drop glue (below).
- `src/lib.rs`: the bodies of `build` and `build_unchecked` only: build the MIR,
  mark, cleanup, **elaborate drops**, generate. No fallback, no env var. Keep the
  `uses_borrows` gate in `build`; `build_unchecked` stays (tests only) until WP7.
- `runtime/strings/src/lib.rs` (+ `runtime/strings.h` if the C side needs a
  declaration): double-free detection.
- `tests/run.rs`: the leak check (also fails on `double free`), applied to
  `examples` and `site/examples` too. `tests/backends.rs`: see Tests.
- The docs site (below).

Do not touch: `src/diag/codes.rs` (no new diagnostics here; if you need one,
say so in the report), `src/mir/{moves,check,lateinit,mutability}.rs` (WP2/WP5;
you only *use* `MoveData`, `MaybeInit`, `MaybeUninit`, `events`), `src/ownership/*`,
`src/mir/ty.rs`.

## LOOK FIRST

1. What the builder emits today for each legacy node (2.2a table, `build.rs`
   `Scoped`/`Temp`/`Copy`/`Move` handling, `moved_out_temps` and the B7 rule: a
   call result goes to a fresh temp that is then moved into the destination).
   The target state: no legacy nodes, so the builder must itself (a) create a
   hidden temp for every owned value that is used as a place (receiver of
   `::len()`, `&string` arguments of runtime calls, `io::out` operands), register
   it in the statement frame (B4) so it gets a `Drop` at the end of the statement,
   and (b) emit `Operand::Move` for a use of a non-Copy place that is assigned,
   passed or returned.
2. `moves::events` and `MoveData`: elaboration is built on them. Note
   `Event::Init/MoveOut/Use` and `local_path`, `prefix_path`, `descendants`.
3. How a call result is written: `Call { dest, target }`; flag writes after a call
   go at the start of `target`, which by construction has one predecessor.
4. `needs_drop`, `has_drop` in `mir/mod.rs` vs the HIR `owns` (a struct with
   `drop()` differs).
5. What `StatementKind::Drop { reason }` values the builder emits (`ScopeEnd`,
   `Assign`, `Destroy`) and how `destroy x;` lowers (a `Drop` with `Destroy`
   followed by treating `x` as moved).
6. `DropFlag` local kind and `LocalDecl.in_memory` for it (never).
7. Today's runtime: `drachm_str_drop`, `drachm_str_copy`, the live counter, the
   `DRACHM_CHECK_LEAKS` report.
8. `PanicKind` is an empty enum and the builder never emits `Switch`: those two
   terminators stay `unreachable!` in the backends with a comment saying so (no
   work). `RawRef` stays unsupported until WP9 (`build_unchecked` of a program
   that takes `address of` returns `Err`, as today).

## What to build

Do it in this order, one commit each, `cargo test` green after every one.

### Step 1. Elaboration, flags, backends (the HIR path still exists)

1. `mir/drops.rs`, spec 3.3 verbatim: `pub fn elaborate(hir: &hir::Program, body: &mut Body)`
   (recomputes `MoveData`, `MaybeInit`, `MaybeUninit` on the final MIR; the
   per-`Drop` `elaborate(p, loc)` rule; `flag(p)` creation with the four flag
   steps; a user-`drop()` type is never opened). Details the spec leaves out:
   - flags are `LocalKind::DropFlag` locals of type `bool`, set with
     `Assign(flag, Use(Const(Bool)))`;
   - a `Drop` with `reason: Assign` of a local that is never initialised is
     removed (dead); with `reason: Destroy` the drop is run now and the local is
     moved afterwards (the builder's `Move`), so the scope-end drop is dead;
   - `Drop` of `_0` never exists; a returned local is moved into `_0`;
   - parameters are dropped at function end if not moved;
   - drops on `break`/`continue`/`return` edges are already separate `Drop`
     statements from the builder (B5); elaboration treats them like any other.
   Run `validate` after elaboration in debug builds.
2. Move elaboration into `lib.rs`: build, `mark_in_memory`, `cleanup`, `elaborate`,
   then `cleanup` again (elaboration leaves empty/unreachable blocks). Order
   matters: elaborate after the checks, which `lib::check` already ran.
3. Backends: implement `Drop { flag: Some(f) }` (test the flag byte, then the
   same drop as unconditional) in both; remove the `unreachable!` for flags.
4. Struct drop glue in both backends, as one generated function per struct type
   that needs it (`drop_<Struct>(ptr)`): calls the user's `drop(&mut self)` first if
   `has_drop`, then drops the fields in declaration order (strings via the
   runtime, nested structs via their glue). A `Drop` of a struct place calls the
   glue; a partly moved struct never reaches the glue (elaboration opened it).
   Copy glue for `DeepCopy`: one `copy_<Struct>(dst, src)` per struct that holds
   strings (strings via `drachm_str_copy`, plain fields by value, nested by
   glue); `DeepCopy` of a plain `string` is the runtime call.
5. Make `eligible` return true for these new cases (flags, `DeepCopy`, user
   `drop()`, user-owned locals) behind a temporary `DRACHM_BACKEND=mir` check to
   see the programs that were ineligible (11 in `tests/run` and `examples`
   today: `interp`, `layout`, `outf`, `string_jumps`, `string_methods`,
   `string_owners`, `string_panic`, `strings`, `struct_flow`, `struct_strings`
   and the bench `strings`) build and run through MIR with identical output.
   The builder still deep-copies on assignment at this point (the workaround);
   that is step 2. Programs must be leak-free as they are.

### Step 2. Remove the workaround and the legacy nodes

1. Builder: strings and strings-holding structs **move** on assign, call
   argument and return (a `Move` of a place that is not Copy; `Use(Copy)` only for
   Copy types). `ExprKind::DeepCopy` lowers to `Rvalue::DeepCopy` (also for the
   written `copy x` that WP2 noted was an empty `Scoped` around the legacy
   `Copy`: that wrapper no longer exists).
2. Delete the legacy handling in `build.rs`, the `ExprKind`/`Stmt` variants in
   `hir.rs`, and the typeck plumbing listed above. The compiler must still report
   E0301 etc. exactly as before (WP2's analyses run on the MIR and do not care
   how it was produced; if a `tests/fail` result changes, the builder is wrong).
3. Temporaries (B4): `s::len()` where `s` is a call result, `io::outln(a + b)`,
   `x::text()::upper()`: the builder creates the temp, borrows it, and drops it
   at the end of the statement; in a `while` condition, each time round; in an
   `if` condition, before the branch.
4. Delete the compiler's own string-copy insertions ("the `Copy` typeck inserts
   when it assigns a string") wherever they live; `grep -rn "DeepCopy\|deep" src`
   should show only the user `copy` path.
5. Remove the B7 rule (call result to a temp, then moved) *only if* elaboration
   makes it free; otherwise leave it. A MIR test must show no dead `Drop` of the
   moved-from temp in the final MIR.

### Step 3. Delete the HIR backends

1. `lib.rs`: delete the fallback, the `DRACHM_BACKEND` read and the HIR calls.
2. Delete `eligible` and friends (list above), `tests/backends.rs`'s eligibility
   tests, and every reference to `DRACHM_BACKEND` in `tests/`, `bench/`, docs.
3. Delete `codegen::compile`/`cgen::generate` and their private helpers.
4. `moved_out_temps` and its use in `cgen_mir.rs` / `codegen_mir.rs` go.

### Step 4. Runtime double-free detection and the harness

1. `runtime/strings/src/lib.rs`: when `DRACHM_CHECK_LEAKS` is set (read once into
   a static at the first allocation), keep the set of live owned pointers
   (`Mutex<HashSet<usize>>`); `drachm_str_drop` and any path that frees or
   reallocates checks the pointer is in the set and removes it; if not: print
   `drachm: double free of a string` to stderr and `abort()`. Without the variable
   nothing changes (no cost, no extra locking).
2. `tests/run.rs`: the existing check fails on `drachm: leaked`; add
   `double free`. For a case whose expected exit code is 0, stderr must contain
   neither. Cases that panic (exit 101) are exempt (R6: a panic does not drop).
   Apply the check to `examples/` and `site/examples/` too.

## Tests

1. `tests/run/own_drops.drh`, `own_reassign.drh`, `own_copy.drh`,
   `own_drop_method.drh`, `own_take_leaving.drh`: the five programs and `.out` in
   `step3-ownership.md` WP6, unchanged. All leak-free, both backends.
2. New run tests (each leak-free, both backends):
   - `own_destroy.drh`: `Guard g = "g"; destroy g; io::outln("after");` prints
     `drop g` then `after`, and nothing at scope end; `destroy` inside a branch
     (`if (n > 0) { destroy g; }`) with a `g` that is also used afterwards on the
     other path (a flag).
   - `own_drop_order.drh`: a struct with `drop()` holding another struct with
     `drop()`: outer first, then fields in declaration order; drops of two locals
     in reverse declaration order; a temporary dropped at the end of its statement.
   - `own_loop_moves.drh`: a string moved in one branch of a loop body and
     reassigned later in the same iteration; a `break` out of a loop with live
     strings; a `continue` likewise.
   - `own_return_paths.drh`: early `return` with strings alive in several scopes,
     returning a moved parameter, returning a field via `copy`.
   - `own_struct_move.drh`: struct with strings passed to a function by value,
     returned from it, assigned over an old value (the old one is dropped once),
     `copy`'d, partially `take`n.
   - `own_temps.drh`: `io::outln(("a" + "b")::upper())`, `("x" + n::text())::len()`
     in an `if` and in a `while` condition (each round), method chains on temps.
   - `own_drop_take.drh`: on a struct with `drop()`, `take x.f leaving v` works
     and the struct is dropped whole; plain `take x.f` stays E0325 (already a
     fail test).
3. `tests/mir/drop_flag.drh` = worked example 1 of the spec; its `.mir` must show
   `Drop(_N) if _F` and no unconditional drop of the same local. Also a MIR test
   for the partial-move example (worked example 2): `Drop(_1.0) if _5` then
   `Drop(_1.1)`.
4. Unit tests in `drops.rs` for `elaborate` on hand-built bodies: certainly
   initialised (static), certainly moved (removed), maybe (flag), partial move
   (opened), struct with `drop()` (never opened).
5. `tests/backends.rs`: replace the eligibility tests by a test that every
   `tests/run`, `examples` and `site/examples` program builds with `-O` and
   without and prints the same output (the existing `run.rs` already runs both
   ways; keep this file only if it adds something, otherwise delete it) and that
   `build_unchecked` of the user-borrow programs from `borrows_through_mir`
   still prints `2`.
6. The old `strings` bench still builds and prints the same.

## Measuring speed (must be no worse)

Same procedure as `wp4.md` (baseline worktree on this machine, two runs each,
best of the two). Acceptance: `collatz`, `fib`, `mandel`, `primes` within +3% in
both `drachm -O` and `plain` columns of the pre-WP6 commit; **`strings` is now
on the MIR path and must be within +3% or faster** (moves instead of deep
copies should make it faster; if it is slower, diff the generated C of the hot
function). Paste the tables in the report. Also record `cargo test` wall time
before and after.

## Site updates (same commit as the user-visible change; `CLAUDE.md`)

- `site/content/reference-ownership.md`: a new section **below** "Other move
  errors" (WP5 puts its "Changing things" section above it): "When a value
  dies": scope end, reverse order, temporaries at the end of their statement,
  `drop()` (what it is, that it runs before the fields, that you cannot call it
  yourself, E0331), `destroy x;`. Real examples as `site/examples/drop.drh` +
  `.out` (a `Guard` with `drop()` printing; `destroy`; scope-end order).
  `site/examples/` programs must compile with no compiler output and are leak
  checked now.
- `site/content/reference-ownership.md`, "Taking one field out": remove the
  sentence "programs that move a field out cannot be built yet" and add a
  runnable example for `take` (`take p.name leaving "nobody"`), as
  `site/examples/take.drh` + `.out`.
- `site/content/lesson-8-ownership.md`: if it says strings are copied on
  assignment or avoids `take`, update it; add a short exercise on `drop()`
  only if it fits the lesson's existing pattern (then also
  `site/examples/solution-8-3.drh`/`.out`).
- `site/content/coming-later.md`: remove "Building programs that move a field
  out with `take`". Leave borrows, `lives` and `unsafe`.
- `site/content/reference-strings.md` / `reference-structs.md`: any statement
  that strings "are copied" when assigned becomes "move".
- `site/gen-errors.sh`: only if `codes.rs` changed (it should not).

## Done when

`cargo test` green with the harness failing on leaks and double frees, in
`tests/run`, `examples` and `site/examples`, on both backends; `grep -rn
"DRACHM_BACKEND\|eligible\|moved_out_temps\|ExprKind::Copy\|ExprKind::Move\|ExprKind::Temp\|ExprKind::Scoped\|Stmt::Drop" src tests bench docs/SPEC.md`
is empty (docs/impl history files excepted); `codegen::compile` and
`cgen::generate` no longer exist; bench tables in the report; the 11 formerly
ineligible programs run through MIR.

## Pitfalls

- Flag writes after a `Call` go at the start of the call's `target` block; give
  every call a fresh target block with one predecessor (the builder already does).
- A partially moved struct: drop opens it field by field; a field never moved
  separately is dropped exactly when its parent would be (virtual path).
- A `Drop` with `reason: Assign` before assigning a possibly-uninitialised local
  needs the flag too (the old value may not exist).
- `break`/`continue` out of nested scopes drop everything between; compare the
  `tests/mir/break_drops` and `scope_drops` expectations after the change and
  update them only where elaboration legitimately removes or flags a `Drop`.
- Struct drop glue must be generated once per type, deterministically ordered (C
  text must not depend on hash order).
- `DeepCopy` of a struct that holds a `&` borrow or a pointer field copies the
  pointer bits; `&mut` holders are E0315 already.
- Temporaries in a condition of a short-circuit `&&`/`||` may not run: their
  drops must be on the path that created them (B-rules in 2.5); a flag is the
  fallback if the builder cannot place it statically.
- Panicking cases do not drop (R6); do not add cleanup on the trap path.
- Do not make leak detection cost anything when the variable is unset: read it
  once, branch on a static.
