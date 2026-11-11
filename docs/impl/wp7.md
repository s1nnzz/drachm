# WP7: the borrow checker, and borrows that build and run

Implementer: **needs a strong model.** This is the NLL algorithm (liveness,
region constraints, a solver, loans in scope, conflicts) plus diagnostics that
must explain themselves, plus changes in both code generators. A bug in the
checker is either a false error on a good program (annoying, caught by the
existing 150+ programs) or a missed error (a dangling pointer at run time, and
nothing catches it). WP0a to WP6 and U1 to U4 are merged. U5/U6 (units) run in
parallel and own `src/typeck/units.rs`, `src/prelude/`, `docs/SPEC.md` section 4
and the units site pages. WP8 and WP9 run in parallel with this one (file
ownership at the end; read it before you start). The input work (`step4.6-input.md`)
can run in parallel too, but touches both backends: see its brief.

Read first: `step3-ownership.md` 2.3, 2.4, 2.5 (B3, B4, B6, B9), 2.9, 3.4, 3.5
(all of it, once end to end), 3.6.1 and 3.6.5, WP7 in 6, section 4 (E0302 to
E0307 and the table of message lines); owner answers Q1 to Q4, Q10, Q13, Q15,
Q17, #1, #2, #3 in `owner-answers.md`; `wp6.md` (how the MIR passes are wired
and what the backends look like now); and the code: `src/mir/{mod,build,check,
moves,mutability,analysis,dataflow,traversal,validate,print}.rs`, `src/lib.rs`,
`src/codegen_mir.rs`, `src/cgen_mir.rs`, `src/typeck.rs` (grep `uses_borrows`),
`tests/{borrows,backends,run}.rs`. This brief says where it differs from the
spec; **where they differ, this brief wins.**

## Goal

1. `mir::check::run` rejects every program that changes, moves, or lets die a
   value while a borrow of it is still going to be used, and every program that
   returns a borrow of something the function owns. Codes E0302 to E0307, in the
   SPEC layout (primary label, borrow label, "the borrow is still used here",
   a `help:` with code to type). No false errors on any existing program in
   `tests/run`, `examples`, `site/examples`.
2. The `uses_borrows` gate in `lib::build` is gone for borrows: a program using
   `&`, `&mut`, `lend`, `&T` struct fields and `lives` builds and runs on both
   backends with identical output, leak-free. Raw pointers (`*T`, `address of`)
   stay gated until WP9 (the gate becomes `uses_pointers`).
3. Two-phase receiver borrows (Q4): `s::append(s::len()::text())` is accepted,
   `xs::push(xs::first())` is E0302.
4. `lend r` reborrows (Q3) and a bare `&mut` local passed on is a move (Q15)
   behave as the spec says, and `&mut` parameters are usable more than once.
5. The function signature facts the checker needs (`LivesSig`) come from a
   small, replaceable provisional rule until WP8 lands (below).

Out of scope: inferring `lives` (WP8), raw pointers and the `unsafety` pass
(WP9), `restrict`/noalias on borrowed parameters (WP10).

## Files you own

New:
- `src/mir/liveness.rs`
- `src/mir/borrowck/mod.rs` (entry points), `regions.rs` (slots, variance,
  universal regions), `constraints.rs`, `solve.rs`, `loans.rs`, `conflicts.rs`,
  `explain.rs`, `sigs.rs` (the provisional `LivesSig` rule)
- `tests/fail/own_*.drh` + `.err` (listed below), `tests/run/own_*.drh` + `.out`,
  `tests/borrowck/*.drh` + `.bck` (below)

Edit:
- `src/mir/mod.rs`: `pub mod liveness; pub mod borrowck;`; replace the
  placeholder `LivesSig` by the spec's enum (step 2 below); nothing else.
- `src/mir/check.rs`: add the borrow checker **at the end of `run`**, after the
  per-body pass (one call; WP8 and WP9 each add one line next to it).
- `src/lib.rs`: `build` and `build_unchecked` (gate split, `unsupported()`).
- `src/hir.rs` + `src/typeck.rs`: only the `uses_borrows` flag: rename it
  `uses_pointers` and set it only where a **raw pointer** is involved (see
  step 6). Do not touch anything else in typeck.
- `src/mir/build.rs`: only if a span or a missing `FakeRead` needs fixing
  (LOOK FIRST 2); keep such edits tiny.
- `src/mir/analysis.rs`, `src/mir/validate.rs`: only what backend support for
  `Ref`/`Ptr` locals needs.
- `src/codegen_mir.rs`, `src/cgen_mir.rs`, `src/codegen_common.rs`,
  `src/cgen_common.rs`: step 1. These two files are also touched by the input
  brief (`Callee` arms); the hunks are disjoint (yours are in place addressing,
  `Rvalue` arms, struct layout and glue; theirs are new `Callee` arms).
- `src/diag/codes.rs`: add E0302 to E0307 (one block, after E0301's neighbours,
  with the existing explain-text style: one short paragraph and one example).
  WP8 adds E0308 and E0320, WP9 adds E0316, E0333; each in its own block.
- `src/main.rs`: `check --dump-borrowck` (below).
- `tests/borrows.rs`, `tests/backends.rs`, `tests/run.rs` (only to teach
  `tests/borrowck`).
- The docs site (below) and `docs/impl/step3-ownership.md` section 4 (message
  lines, if you change any wording).

Do not touch: `src/mir/{moves,mutability,lateinit,drops,cleanup}.rs`, the typeck
beyond the flag, `src/flags.rs`, `src/units.rs`, `src/typeck/units.rs`,
`src/prelude/`, `docs/SPEC.md` (the owner's design doc is updated by whoever
interviews the owner; if the checker's behaviour differs from section 3.5 say so
in your report).

## LOOK FIRST

1. What the builder emits for each borrow form (`build.rs` around 830 to 990):
   `&x`, `&mut x`, `lend r` (`_t = &mut (*r)`), method receivers
   (`Mut { two_phase: true }`), `&` of a temporary (B9: the temporary lives to
   the end of the block), the `Reassign` drop (`DropReason::Assign`). Print the
   MIR (`check --dump-mir`) for the programs in the acceptance list and
   check each statement against the spec's worked example 3.
2. Spans. A conflict error needs the span of the **borrow expression** (`&name`,
   not the whole declaration), of the **access** (`name` in `name = "Sam";`,
   the receiver in `xs::push(9)`), and of the **use** (`first` in
   `io::outln(first)`). Check `Statement.span` for each. If a `Ref` statement
   carries the span of the surrounding statement, fix it in the builder.
3. `moves::events` (the per-location event list) is what WP2 uses for uses and
   moves. You do **not** build on it for loans, but its handling of `Move` vs
   `Copy` operands and of `take` temps is the reference for what an operand
   means. Where an access is a `take p.name` (moving a field out into a temp),
   treat it as `Move(p.name)`.
4. `mir::has_drop` / `needs_drop`: the drop-liveness rule in 3.4 needs "this
   type has a user `drop()` somewhere in it".
5. How structs with `lives` are described in `hir::StructDef.lives` (indexes of
   the borrow fields, in `lives` order) and `hir::Func.lives` (`Option<Vec<usize>>`
   of parameter indexes). `check_struct_lives` in typeck already validates them
   (E0314); you rely on that and may `debug_assert` it.
6. `dataflow.rs`: reuse its `BitSet`, forward/backward solvers and `Results`.
   Do not write a second framework.
7. What the backends do today with `Ref` (`addr_of` in `codegen_mir.rs`, `&lvalue`
   in `cgen_mir.rs`), `Proj::Deref` and a `Type::Ref` local, and the three
   existing programs in `tests/backends.rs` / `tests/mir/borrows.drh`.

## What to build

One commit per step, `cargo test` green after every one. Step 1 and steps 2 to 5
do not depend on each other; steps 2 to 5 must be done in order; step 6 needs
both.

### Step 1. Backends can run borrows (the gate stays)

Goal: `build_unchecked` builds and correctly runs every borrow-using program the
type checker accepts, with both backends identical and leak-free. The checker
does not exist yet, so the test programs must be written to be safe by hand.

Audit and fix, with a program for each line (they go in `tests/backends.rs`,
which becomes a list of `(source, expected stdout)` pairs; keep it a loop over
both backends and run each with `DRACHM_CHECK_LEAKS=1`):

1. `&string` parameter read with `::len()`, `::upper()`, printing, `==`; `&mut string`
   parameter appended to, assigned through (`s = "x";` writes through, old value
   dropped once: a `Drop` of `*s` then the store) and re-pointed
   (`mut &string cur = &a; cur = &b;`).
2. `&Struct` / `&mut Struct` parameters with string and number fields; a method
   taking `self` as a borrow that returns a field by `copy`.
3. A **returned** borrow: `&string first(&Pair p) { return &p.a; }` and a method
   `&string name() { return &self.name; }`. A `Ref` is a scalar (pointer-sized):
   returned in a register, never through the hidden out pointer. Check both
   backends' return and call-result code.
4. Structs with `&` fields (`struct Parser lives src { &string src; uint32 pos; }`):
   layout (8 bytes, 8-aligned), `Aggregate` stores of a pointer, field loads, the
   struct itself passed, returned and `copy`'d (`DeepCopy` copies the pointer
   bits; the string it points at is not copied), drop glue and copy glue skip
   `Ref`/`Ptr` fields, the automatic `==` (if the compiler generates one for
   structs with borrow fields: compare through the pointer for `Ref`, by bits
   for `Ptr`; if typeck rejects it, fine, say so in the report).
5. `lend`: `bump(lend n)` inside a function with `&mut uint32 n`, twice in a row.
6. Two-phase receivers: `c::add(c::get())`, `s::append(s::len()::text())`.
7. Borrows of fields and of elements of fields: `&p.name`, `&mut p.hp`, a
   borrowed local that is also a loop variable.
8. A `&` borrow of a temporary in a declaration (`&string t = &"a";`,
   `&string u = &x::upper();`): the builder keeps the temp alive (B9); check
   the temp is dropped exactly once at the end of the block.
9. Borrows inside loops and branches, borrows stored in a `mut &string` that is
   re-pointed in a loop.
10. `RawRef` and `Ptr`-typed locals: implement them (same address computation
    as `Ref`; `Type::Ptr` is 8 bytes, a pointer-sized scalar in both backends).
    Remove the two `unreachable!("raw pointers are not supported ...")` arms,
    so that WP9 has nothing to do in the backends except tests. `build_unchecked`
    keeps refusing them in `unsupported()` **until step 6**, where the refusal
    moves under the `uses_pointers` gate.
    Dereferencing a `Ptr` place (`*p = 7`, `Proj::Deref` of a `Ptr`) is the same
    load/store as a `Ref`.
11. `validate.rs`: the V-rules accept `Ref`/`Ptr` operands where the spec says
    (no `Move` out of `*r` of a non-Copy type; a `Ref` is Copy for `&`, move-only
    for `&mut`). Add validator tests for hand-built bodies that break each rule.

Hazards: a Cranelift `Variable` for a scalar local whose address is taken
must be `in_memory` (`analysis::mark_in_memory` decides: check it also covers a
`Ref` taken inside a loop, and a `Ref` of a parameter). A `Move` of a string
into a call passes the address of the source place itself (2.9): with borrows
the source may be reached through a `Deref`; then the callee would free memory
the borrower owns. The builder never emits `Move` out of a deref (E0310 is
reported earlier) so assert it in the backends.

C backend: do **not** add `restrict`. Borrowed C types are `T *`, strings
`drachm_str *`.

### Step 2. Regions, liveness, signatures

1. `mir/liveness.rs` (3.4): backward analysis over `dataflow.rs`; per-point
   queries `live_before(loc) -> BitSet` computed by walking the block backwards
   from its stored end set. Include the drop-liveness rule.
2. `borrowck/regions.rs`: `region_slots(ty) -> usize`, `slots_of_place(body, place)
   -> Range<usize>` into the local's slot vector (3.5.3 C4; the struct case is
   simple because a `lives` field is `&T` or `&mut T` with a slot-free `T` (E0314
   and the E0901 cases guarantee it): slot `i` of a struct is its `i`-th `lives`
   field's outer slot), struct parameter variance (3.5.3; covariant unless the
   field is `&mut` or a `mut` field), and the universal-region table of a body:
   one per parameter slot, then one per return slot, named for messages. These
   are `pub`: WP8 uses them.
3. `LivesSig`: replace the placeholder in `mir/mod.rs` by exactly the spec enum
   (`NoBorrowResult`, `Declared(Vec<usize>)`, `Inferred(Vec<usize>)`, `Unknown`;
   `usize` = parameter index, `self` = 0 for methods). `build_program` fills
   `mir.lives` through `borrowck::sigs::provisional(hir, f)`:
   - the return type has no region slots: `NoBorrowResult`;
   - `hir.funcs[f].lives` is `Some(v)`: `Declared(v)`;
   - otherwise exactly one parameter has a borrow slot: `Inferred([it])`;
   - otherwise (zero or several): `Inferred(all parameters with a borrow slot)`.
   That is more than WP8 will compute (E0308 fires there), so a program that
   needs two parameters' loans is checked conservatively. **WP8 replaces
   `provisional`; write nothing else that depends on how it works.** `Unknown` is
   what C5 treats as "assume nothing and report nothing" (used by WP8 for bodies
   that already had errors).
4. Unit tests for slot counting, variance, and liveness on hand-built bodies.

### Step 3. Constraints and solving

`borrowck/constraints.rs`, `solve.rs`: rules C1 to C7 and 3.5.4 exactly. Notes
beyond the spec:
- Points: `point_index(loc)`; the terminator is a point.
- Local regions are variables per slot; `_0`'s slots are the return universals.
  A parameter's slots are universals, and a **local that is assigned from a
  parameter** relates through C4 as usual.
- C1 uses "live on entry to the point". A `Ref` statement's destination is
  defined at the statement, so the destination's region contains the points
  from the *next* point on as far as it is live.
- C3 reborrow walk: stop after the first `Deref` of a `&`; continue through `&mut`;
  a `Deref` of a pointer type (`Ty::is_ptr`) stops with no constraint. **Use
  `is_ptr()`/`target()`, never match `Type::Ptr` directly:** WP9 adds a second
  pointer variant (`*fixed T`) and `is_ptr` will cover both.
- C5: for each `Callee::Func` call, `inst` renames every universal of the callee's
  signature; slots of a callee's parameter type come from `region_slots`. For
  `Out`/`Str`/`Text`/`Join`/`Radix` callees only C1 applies.
- Solve with SCCs as the spec says. Universals contain all points and their own
  end marker; each value is `{points, ends}` bitsets.
- **Interface for WP8 (keep it exactly; WP8 is written against it).** In
  `borrowck/mod.rs`:
  ```rust
  pub struct BodyResult {
      pub diags: Vec<Diagnostic>,   // empty when `report` is false
      pub solution: solve::Solution, // region values, after solving
  }
  /// Check one body against the given signatures.
  pub fn analyse_body(hir: &hir::Program, sigs: &[LivesSig], body: &Body, report: bool) -> BodyResult;
  /// WP7's driver: every body in FuncId order, with `sigs::provisional`.
  /// WP8 replaces the call to this in `check::run` by its own driver.
  pub fn check_program(hir: &hir::Program, mir: &Program, diags: &mut Vec<Diagnostic>);
  ```
  `solve::Solution` is `pub` and exposes: the universal-region table (for each
  universal: its parameter index, its slot index within that parameter or `Return`,
  its name for messages), `values: IndexVec<RegionVid, RegionValue>` with
  `points` and `ends`, and the constraint list, **each constraint carrying its
  origin** (`Location` + kind: `Borrow`, `Reborrow`, `Assign`, `Call`, `Return`,
  `Liveness`) because WP8's explanations walk the constraint graph (3.6.5).
  `report: false` runs the same analysis and returns no diagnostics.
- Debug output: `check --dump-borrowck` (main.rs, after `--dump-mir`) prints,
  per function: the point numbering as in the spec's worked example 3, each
  region with its value, each loan (place, kind, region, the points where it
  goes out of scope). Format it so lines are greppable: `loan L0: &_1 at P3 in {P4,P5,P6}`.
  `tests/borrowck/NAME.drh` + `NAME.bck` use the same "each line of the expectation
  must occur in the dump" rule as `tests/mir` (copy that test into `tests/run.rs`).
  Programs: worked examples 3 and 4, a loop-carried borrow, a two-phase borrow, a
  struct holder.

### Step 4. Loans and conflicts

`borrowck/loans.rs`, `conflicts.rs`: 3.5.2, 3.5.5, 3.5.6.

- One `Loan` per `Rvalue::Ref` (including `lend` reborrows and receivers).
- `BorrowsInScope`: forward dataflow with gen at the loan's location, kill when
  the successor point is outside `values[ρL].points`, kill on overwrite of the
  base borrow variable, kill at `StorageDead` of the root. Precompute the
  out-of-region kills by DFS (3.5.5).
- Accesses and the table in 3.5.6, with these refinements the spec omits:
  1. **`Dead(l)` / `Drop(l)` of an owner do not conflict with a loan whose place
     goes through a `Deref` of a reference** (the data behind a borrow is not
     owned by the variable, so the variable dying does not invalidate it). Without
     this, every `lend` reborrow of a `&mut` parameter is an error at the end of
     the function. A `Drop` of a `&` / `&mut` typed place is not an access at all.
  2. Loans whose place goes through a `Deref` of a raw pointer never exist (C3
     stops), and an access through a pointer deref is never checked.
  3. `Assign` to the destination of a call is a `Write` that happens when the
     call returns: check it on the call's **target edge**, not before the
     arguments are evaluated.
  4. The statement that creates a loan is not checked against that loan. The
     two-phase receiver temp is created before its arguments are evaluated and
     activated at the call: while *reserved* it conflicts like a `Shared` loan
     (so `s::append(s::len()::text())` passes), and at activation (the call
     terminator, the first use of the temp) it is checked as a `MutBorrow` of
     its place against every other loan in scope. Creation is checked as a
     `ShareBorrow`. Find the activation location once per two-phase loan.
  5. A `Reassign` drop is a Write, never E0306 (spec pitfall).
  6. `FakeRead(q)` is a Read. A `Copy` of a `&T` is a Read of the borrow variable
     and never conflicts with the loan it holds (it is not the borrowed place).
  7. The kind table gives the *code*; the access decides the wording variant
     (assignment, `+=`, mutating method receiver, `&mut` borrow, move, scope end).
- Dedup: one error per loan (report the first conflicting access in reverse
  post-order), after sorting by span so output is stable.
- Escape detection: a loan whose value contains an `end(U)` marker never goes
  out of region. At the owner's `StorageDead` (or `Drop`) that is a conflict
  that is reported as **E0307** when some `end` is a *return* region ("returned"),
  and as **E0306** when it is only a parameter's end (a borrow of a local stored
  into `*out`; label "kept in `out`, which the caller keeps"; E0320 is WP8's and
  is for borrows of one *parameter* stored into another).

Still no wording work beyond a one-line message plus the three labels; step 5
does the rest. Write the tests of step 5 first as `tests/fail` cases whose `.err`
files hold only the error line, and they pass at the end of this step.

### Step 5. Explaining errors

`borrowck/explain.rs`: 3.5.7 and the wording below. The "later use" search must
return a `Use { local, span, kind }` where `kind` is one of: plain use, used by
`return`, used by a user `drop()` at scope end, used again after the back edge of
a loop (the use is at a point that precedes the access in the loop body).

Wording. All messages are written for the owner's review (Q19) and are listed
in `step3-ownership.md` section 4. Every one has the E0302 layout. Names are
the user's spelling (`p.name`, `first`), a temporary is "this value", and a
borrow held in a struct field is "kept in `h.src`".

| Code | Situation | Message | Primary label | Help |
|---|---|---|---|---|
| E0302 | assignment, `+=`, `++` | cannot change `name` while it is borrowed | change attempted here | finish using `first` before changing `name` |
| E0302 | mutating method on the borrowed value | cannot change `xs` while it is borrowed | change attempted here (on the receiver) | finish using `first` before changing `xs` |
| E0302 | `&mut x` argument while a `&` loan is live | cannot change `p` while it is borrowed | `&mut` borrow attempted here | finish using `look` before lending `p` as `&mut` |
| E0302 | the later use is a `return` | (same) | (same) | change `name` before borrowing it, or return a copy |
| E0302 | the later use is on the next time round a loop | (same) | (same); the use label reads "the borrow is used again here, on the next time round the loop" | finish using `first` inside the loop, or borrow `name` again each time round |
| E0302 | the later use is a user `drop()` | (same) | (same); the use label reads "`h` is dropped here, and its `drop()` can still use the borrow" | end `h` before changing `name`: `destroy h;` |
| E0303 | second `&mut`, any form | `p` is already borrowed with `&mut` | second `&mut` borrow here | only one `&mut` borrow of `p` can be in use at a time; finish using `a` before borrowing `p` again |
| E0303 | a `&mut` while a `&` loan is live | is E0302, not E0303 | | |
| E0304 | read of the place | cannot use `p.hp` while `p` is borrowed with `&mut` | used here | read `p.hp` before borrowing `p` with `&mut`, or read it through `a`: `a.hp` |
| E0304 | a `&` borrow of the place | cannot borrow `p` while it is borrowed with `&mut` | borrowed again here | finish using `a` before borrowing `p` again |
| E0304 | the use is a `lend` reborrow's parent | cannot use `r` while a part of it is lent | used here | finish using `t` first; the lend ended after the call that took it |
| E0305 | move while borrowed | cannot move `name` while it is borrowed | moved here | finish using `first` before moving `name`, or move a copy: `consume(copy name)` |
| E0306 | owner dies at the end of a block | `local` does not live long enough | `local` is dropped here, at the end of its block | declare `local` outside the block, so it lives as long as `keep` is used |
| E0306 | owner is a parameter taken by value, or a local at function end | (same) | `s` is dropped here, when `f` returns | return the value itself, not a borrow of it |
| E0306 | stored into a `&mut` parameter | (same) | `s` is dropped here, but `out` keeps the borrow after `f` returns | store a copy: `*out = copy s` |
| E0306 | stored in a struct field | `text` does not live long enough | (same); the borrow label reads "`text` is borrowed here, and kept in `q.src`" | declare `text` outside the block, so it lives as long as `q` is used |
| E0307 | return a borrow of a local | cannot return a borrow of `s`, which belongs to this function | the borrow is returned here | return the string itself: change the return type to `string` and write `return s;` |
| E0307 | of a temporary | cannot return a borrow of a temporary value | the borrow is returned here | (same) |
| E0307 | of a parameter taken by value | cannot return a borrow of `n`, which belongs to this function | the borrow is returned here; note: `n` is taken by value, so it is dropped when `f` returns | take it as a borrow: `&uint32 n`, or return `n` itself |
| E0307 | function has no borrow parameters | (same); extra note line | | the note reads "`shout` takes no borrows, so a borrow it returns can only point at something it owns" |

The `help:` that names a holder uses the local found by the later-use search; if
that is a temporary, say "the borrow" instead of a name.

Also cut cascades: if one body already has a E0301 or E0310 error for the same
place, the borrow error for it is not repeated (compare the move path), and a body
whose MIR failed `validate` is skipped.

### Step 6. Lift the gate

1. `hir::Program`: rename `uses_borrows` to `uses_pointers`; in typeck set it only
   where a raw pointer is involved: `resolve` when a modifier is `TypeMod::Ptr`
   (not a `&`), `address_expr`, `Deref` of a `Ptr`, `NullPtr`, `none_compare` on a
   pointer, and an `unsafe` function declaration. The `&`/`&mut` places that set
   it today (assign through a `Ref`, `coerce_borrow`) stop setting anything.
2. `lib::build`: refuse only `uses_pointers`, with the existing message reworded:
   "raw pointers cannot be built yet (step 3, WP9)". `build_unchecked` keeps the
   `unsupported()` refusal for `RawRef`/`Ptr` locals; WP9 deletes both.
3. `check::run` ends with the borrow checker for every body. `lib::check`
   already stops on errors before building.
4. Tests: move every program of `tests/backends.rs` that has no pointers to
   `tests/run/borrow_*.drh` + `.out` (they now also run the checker); keep a
   small `build_unchecked` test for the pointer programs; `borrows.rs` asserts
   that the pointer part of its source is still refused and that the borrow
   part builds (split its source in two). All the `tests/fail` and `tests/run`
   cases of this brief pass.
5. Run the existing suite, `examples`, `site/examples`: no new diagnostics. Run
   `bench/run.sh` before and after (the benchmarks have no borrows, so only
   compile time could change): paste the table. Time `cargo test`.

## Tests

Fail tests (`tests/fail/NAME.drh` + `.err` with the lines the error must print;
the sources are in the PLAN's "done when" list and in `step3-ownership.md` WP7;
copy them verbatim):

`own_change_while_borrowed`, `own_two_mut`, `own_shared_and_mut`,
`own_dies_too_soon`, `own_return_local`, `own_two_phase_live`; new ones:
- `own_move_while_borrowed` (E0305, the section 4 example)
- `own_use_while_mut_borrowed` (E0304 read, then a second case in the same file
  for the `&` borrow variant; one error per loan means two loans, two errors)
- `own_borrow_in_loop` (E0302 with the "next time round the loop" label:
  `&string first = &name;` before a `while`, `name = ...;` in the body, `first`
  used after the assignment inside the body on the next round)
- `own_return_param_local` (E0307 of a by-value parameter)
- `own_struct_holder_dies` (E0306 with "kept in `q.src`")
- `own_store_in_out` (E0306 "kept in `out`")
- `own_lend_then_use` (E0304 on using `r` while the `lend` temp is alive: needs
  the temp to be alive, e.g. the callee returns a borrow derived from it)
- `own_receiver_reborrow` (a method call on `r` where `r: &mut Counter` keeps
  `r` usable afterwards: this is a **run** test; its failing twin moves `r` by
  `&mut Counter r2 = r;` and then uses `r`: E0301)
- `own_assign_through_borrowed` (`*r = ...` while a shared reborrow of `*r` is
  live: E0302)
- `own_drop_holder` (a struct with `drop()` holding `&x`: changing `x` before the
  holder dies is E0302 with the `drop()` label)

Run tests (accepted programs, `.out` files; all leak-free on both backends):
`own_nll`, `own_two_phase` (from the spec), and
- `borrow_basic`: `&`/`&mut` parameters of numbers, strings, structs; return of a
  borrow with `lives`; prints values changed through `&mut`.
- `borrow_reborrow`: a `&mut` parameter passed on with `lend` twice, a method call
  on a `&mut` binding, then the binding used directly.
- `borrow_fields`: `&mut p.a` and `&p.b` live together; `&mut r.a` and `&mut r.b`
  through `r: &mut Pair`; a method call after the field borrows ended.
- `borrow_struct_holder`: `struct Parser lives src`, constructed from `&text`,
  passed as `&mut Parser` to a method that advances `pos`, `text` printed after
  the parser's last use.
- `borrow_loops`: re-pointing a `mut &string` in a loop; a loan that ends before
  the back edge; a `break` with a live borrow.
- `borrow_temps`: `&string t = &x::upper();` printed in the block, dropped once.
- `borrow_two_phase`: `c::add(c::get())`, `s::append(s::len()::text())`,
  `v::set(v::get() + 1)`.
- `borrow_drop_holder`: holder with `drop()` printing, borrow ends exactly when
  the holder is dropped; order of drops visible in the output.
- `borrow_params_mix`: `show(&p)`, `show(&m)` for a `&mut` binding `m` (the
  explicit shared view, #3), `bump(lend m)`.

A **property check** (a unit test in `borrowck/mod.rs`): run the checker on every
file in `tests/run`, `examples`, `site/examples`, `tests/mir`, and require zero
borrow diagnostics; this is the false-positive net.

## Site updates (same commit as the user-visible change; CLAUDE.md)

Only document what the compiler accepts at the commit. Because WP8 may not have
landed, every example that returns a borrow from a function with **two or more**
borrowed parameters writes `lives` itself; do not describe inference. Do not
describe raw pointers.

- `site/content/reference-ownership.md`: new section "Borrowing" (above "When a
  value dies" if that is where the file stands; keep the existing section order
  of WP5/WP6): `&` and `&mut` at the call site (E0321), reads and writes through a
  borrow (Q1), `lend`, a bare `&mut` passed on moves, the rules in plain words
  (many `&` or one `&mut`; borrows end at the last use), then each of E0302 to
  E0307 with the real output of the compiler for a small program (the format of
  the E0311 block that is already there). Examples as `site/examples/borrow.drh`,
  `borrow-lend.drh`, `borrow-two-phase.drh` + `.out`.
- `site/content/lesson-9-borrowing.md` (new; add to `site/content/order.json`
  after lesson 8): goal, `&string` parameter, `&mut`, the three errors a beginner
  meets first (E0302, E0303, E0307) each with a fix, and three exercises in the
  pattern of lesson 8 with `site/examples/solution-9-N.drh` + `.out`.
- `site/content/lesson-8-ownership.md`: if it says "borrowing comes later" or
  similar, link to lesson 9.
- `site/content/reference-types.md`: `&T` and `&mut T` types, struct `&` fields
  with `lives` (only the written form), `mut &T`.
- `site/content/reference-structs.md`: struct `lives`, the E0314 example if not
  already there, and the Parser-style example.
- `site/content/coming-later.md`: remove "Borrows, with `&string` and `&mut`
  parameters and `lend`"; leave `lives` inference, `open` fields and `unsafe`.
- `site/content/reference-diagnostics.md` and `site/content/errors.md`: add the
  new codes to the list; regenerate with `site/gen-errors.sh` (codes.rs changed).
- All new `site/examples/*.drh` compile with no compiler output, are leak-free
  and print exactly their `.out`.

## Done when

- `cargo test` is green (all the above), including the false-positive net.
- `grep -rn "uses_borrows" src tests docs/SPEC.md` is empty (docs/impl history
  excepted).
- A program with `&`, `&mut`, `lend` and `lives` builds with `drachm build` and
  `drachm build -O` and runs; the pointer programs are still refused with the
  WP9 message.
- The report lists: where the implementation differs from 3.5, the bench table
  and test wall time before/after, and anything the spec says that turned out
  wrong.

## Pitfalls

- A loan created at `P` must not be checked against the access at `P` that creates
  it, and loans of a temporary used in one call die at the call (liveness does it:
  test it with `io::outln(&x::upper())` followed by a change of `x`).
- Field disjointness through a deref of a `&mut`: `&mut r.a` and `&mut r.b` with
  `r: &mut Pair` are fine; a method call borrows the whole receiver.
- The `StorageDead`/`Drop` exemption for loans through a deref (step 4, 1).
- Struct region variance: a `mut &string cur` field makes the parameter invariant.
- `Reassign` drops: Write, not E0306. A `Reassign` drop of a local that has a
  user `drop()` is also a use of what it holds (drop-liveness).
- Loops: the DFS for out-of-region kills must follow back edges; a loan live at
  the loop head from the previous iteration is in scope at the access.
- Parameters are locals: their `StorageDead` at return is an access; loans of a
  by-value parameter that flow into the return region are E0307.
- `check::run` skips the compiler's own functions (span 0, 0): keep doing so.
- Never match `Type::Ptr` directly (WP9 adds `Type::FixedPtr`).
- Error order must be deterministic: sort by span, then code.
- Compile time: bodies are small, but do not make the solver quadratic in
  statements. Bitsets over points, SCC propagation.
- If a `tests/run` program shows a false error, the checker is wrong, not the
  program. Do not edit existing run tests to appease it (new `own_*` tests are
  yours; the old ones are WP2 to WP6's).

## Language questions found

None new. The wording of every message above awaits the owner's Q19 review.
`lend` on a `&T` (#2) and a borrow field whose type is itself a struct with
`lives` (#7) stay E0901 as they are today.
