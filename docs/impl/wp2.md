# WP2: dataflow framework, initialisation and move analyses

Implementer: **needs a strong model** (dataflow over a CFG, precise
diagnostics, removing `Flow` from typeck without changing any message). Runs
in parallel with WP3 and WP4 (file ownership below). Starts after WP0a and
WP1 phase B are merged. Read first: `step3-ownership.md` 2.3, 2.5 (B4-B10),
3.0, 3.1, 4 (E0301, E0310, E0313, E0315, E0324, E0325), and the WP2 section of
6 (its test programs are reused here). This brief says where it differs from
or pins down that text; **where they differ, this brief wins.**

## Goal

Everything typeck's `Flow` does today (E0105 missing return, E0207 read before
assignment, the "second assignment" and "inside a loop" variants of E0205)
is reported by analyses on the MIR with **the same texts**, and `Flow` is
gone from typeck. In addition: E0301 (use after move, with its variants),
E0310, E0313, E0315 and the new E0332. No backend changes; no drop
elaboration (WP6); no borrow checking (WP7).

## Files you own

New: `src/mir/dataflow.rs`, `src/mir/moves.rs`, `src/mir/check.rs`,
`src/mir/lateinit.rs`, `tests/fail/own_*.drh` + `.err` files listed below.

Edit:
- `src/typeck.rs` and `src/typeck/control.rs`: remove `Flow`, `Checker.flow`,
  `Target`'s `Flow` fields, `chain`/`looped`/`for_each`/`match_stmt` joining,
  `read_local`'s E0207, the E0105 fall-off check at typeck.rs:~715 (not the
  `return;` one at ~1176, which is a type error and stays) and the
  `flow.maybe` / `loop_depth` branches of `check_assignable`. **Leave
  everything about `SelfUse`, `fixed_receivers`, `check_field_change` alone**
  (WP3 and WP5 own those; WP3 does not edit typeck.rs at all).
  `LocalInfo.loop_depth` goes if nothing else uses it.
- `src/mir/traversal.rs`: **append** `pub fn sccs(body: &Body) -> Vec<Vec<BlockId>>`
  (Tarjan, iterative) and `pub fn in_cycle(body: &Body) -> IndexVec<BlockId, Option<u32>>`
  (SCC number for blocks in a cycle of 2+ blocks or with a self-loop, else
  `None`). Nobody else edits that file in this wave.
- `src/mir/mod.rs`: add `pub mod check; pub mod dataflow; pub mod lateinit;
  pub mod moves;` as a block **directly after the existing `pub mod validate;`
  line** (WP4 adds its lines before `pub mod build;`; WP3 edits the
  `SelfMode` block lower down). Nothing else in that file.
- `src/lib.rs`: the **body of `check()` only** (see "Wiring"). WP4 owns
  `build()`'s body; WP3 owns the one `pub mod ownership;` line.
- `src/diag/codes.rs`: append E0332 after the last `03xx` entry, before `E0901`.
  No other package touches this file in this wave.
- `tests/fail/{flow,noreturn,immutable}.{drh,err}`: split as described below.

Do not touch: `src/mir/build.rs` (WP3 edits two functions in it; if you find a
builder bug, make the minimal fix, keep it to its own commit and tell the
coordinator), `src/mir/{print,validate,ty}.rs`, `src/codegen.rs`, `src/cgen.rs`,
`tests/run.rs`.

## LOOK FIRST (read the code as merged, then fill these in at the top of your commit message or a scratch note)

1. **Wiring.** Does `lib::check` already call `mir::build_program` after
   typeck (WP1 phase B said `main.rs` ignores `build_program` diags until then)?
   Where are the E0324/E0325 builder diags rendered? Whatever exists, the end
   state must be: `lib::check` returns `(None, diags)` when typeck, the builder
   or `mir::check::run` reported an error, and `drachm check` prints all of them.
2. **Spans.** Which statements/terminators/`Arg`s carry real spans after phase
   B? An `Operand` has no span: for a use inside `Assign` use the statement's
   span; for a call argument use `Arg.span`. If a case lacks one (`Drop`
   at scope end, `StorageDead`), the check never reports there (see pitfalls),
   so this only matters for labels of moves. Report any construct whose span is
   still `Span::new(0, 0)`.
3. **Types.** Is `LocalDecl.borrow` gone, and does `Type::Ref { mutable, to }`
   exist? Name of the `place_ty` helper in `mir/ty.rs` and whether it needs the
   `hir::Program`. You need "is this local/place a `&mut`", "does this type
   contain a `&mut` field" and "is this a raw pointer".
4. **`take`.** Exact lowering of `Take` with and without `leaving` (spec 2.5:
   `_t = Use(Move(place))` then, for `leaving`, `place = Use(move _v)`). The
   move-path code relies on exactly that shape.
5. **`self`.** Parameter 0 of a method is a `&Self`/`&mut Self` after WP1B, so
   its fields appear as `(*_1).f`: a place with a `Deref` has no move path.
6. **`Mutability`** values for `open` bindings (typeck stores `Open`; `mut open`
   is `Mut` plus W0323).
7. Whether `Body.fallthrough_end` is `Some` only when that block is reachable
   (the builder prunes unreachable blocks).
8. The names of hidden locals' `LocalKind` (spec: `Temp`); hidden user-less
   locals never get diagnostics naming them. If a diagnostic would have to name a
   temp, use "this value".

## Data structures

### `dataflow.rs` (spec 3.0, with these concrete signatures)

```rust
#[derive(Clone, PartialEq, Eq)]
pub struct BitSet { words: Vec<u64>, len: usize }
impl BitSet {
    pub fn new(len: usize) -> BitSet;          // all clear
    pub fn insert(&mut self, i: usize) -> bool; // true if it was clear
    pub fn remove(&mut self, i: usize) -> bool;
    pub fn contains(&self, i: usize) -> bool;
    pub fn union(&mut self, other: &BitSet) -> bool;     // true if self changed
    pub fn subtract(&mut self, other: &BitSet);
    pub fn clear(&mut self);
    pub fn iter(&self) -> impl Iterator<Item = usize> + '_;
}

pub trait Analysis {
    const FORWARD: bool;
    fn domain_size(&self, body: &Body) -> usize;
    fn boundary(&self, body: &Body, state: &mut BitSet);   // entry (forward) / every exit (backward)
    fn statement(&self, state: &mut BitSet, stmt: &Statement, loc: Location);
    fn terminator(&self, state: &mut BitSet, term: &Terminator, loc: Location);
}

pub enum Item<'a> { Statement(&'a Statement), Terminator(&'a Terminator) }

pub struct Results<A: Analysis> { pub analysis: A, entry: IndexVec<BlockId, BitSet> }
impl<A: Analysis> Results<A> {
    pub fn compute(a: A, body: &Body, preds: &IndexVec<BlockId, Vec<BlockId>>) -> Results<A>;
    /// Forward: the state just before `loc`. Backward: just after it.
    pub fn state_at(&self, body: &Body, loc: Location) -> BitSet;
    /// Every location of every reachable block, in RPO (forward) or
    /// post-order (backward), with the state described above.
    pub fn visit(&self, body: &Body, f: impl FnMut(Location, &BitSet, Item<'_>));
}
```

Join is union. A block unreachable from `bb0` keeps the empty entry state and is
never visited. Backward is not used in this package but must work and be
tested (WP7 liveness uses it): for backward analyses `entry[b]` is the state at
the *end* of `b`.

### `moves.rs` (spec 3.1.1, adapted: no `SmallVec`, no `Hash` on `Place`)

```rust
newtype_index!(MovePathId);
newtype_index!(MoveOutId);
newtype_index!(InitId);

pub struct MovePath {
    pub local: Local,
    pub fields: Vec<usize>,                 // Field projections only; empty = the local itself
    pub parent: Option<MovePathId>,
    pub children: Vec<MovePathId>,          // in field order
}

pub enum MoveCause {
    /// Passed whole as argument `index` of a call to a user function.
    CallArg { callee: FuncId, index: usize },
    /// `x = move y` or any other plain move into a place or a struct.
    Assign,
    /// `_0 = move y` (a `return`).
    Return,
    /// From a `take` (the temp `_t = move p.f`).
    Take,
    /// A `Drop` statement (scope end or before an assignment): the place is
    /// uninitialised afterwards. Never a user-visible "moved here".
    Drop,
    StorageDead,
}

pub struct MoveOut { pub path: MovePathId, pub loc: Location, pub span: Span, pub cause: MoveCause }
pub struct Init    { pub path: MovePathId, pub loc: Location }

pub struct MoveData {
    pub paths: IndexVec<MovePathId, MovePath>,
    pub local_path: IndexVec<Local, MovePathId>,        // root path of every local
    by_place: HashMap<(Local, Vec<usize>), MovePathId>,
    pub moves: IndexVec<MoveOutId, MoveOut>,
    pub inits: IndexVec<InitId, Init>,
    pub moves_at: HashMap<Location, Vec<MoveOutId>>,
    pub inits_at: HashMap<Location, Vec<InitId>>,
}
impl MoveData {
    pub fn build(body: &Body) -> MoveData;
    /// The path of `place` if it has one (no `Deref` in it).
    pub fn path_of(&self, place: &Place) -> Option<MovePathId>;
    /// Longest prefix of `place` before its first `Deref`; always exists.
    pub fn prefix_path(&self, place: &Place) -> MovePathId;
    pub fn is_ancestor(&self, a: MovePathId, of: MovePathId) -> bool;   // a == of counts
    pub fn descendants(&self, p: MovePathId) -> Vec<MovePathId>;        // p included
}

pub struct MaybeInit<'a>(pub &'a MoveData);        // domain: move paths
pub struct MaybeUninit<'a>(pub &'a MoveData);
pub struct EverInit<'a>(pub &'a MoveData);
pub struct ReachingMoves<'a>(pub &'a MoveData);    // domain: MoveOutIds
// each implements `Analysis` with the gen/kill table of 3.1.2
```

Create a path for **every** place that is the target of a `Move`/`Assign`/call
`dest`/`Drop`/`DeepCopy`/`Ref` source or any use, plus all their ancestors, so
that `prefix_path` never fails. Making all `Field`-only prefixes of used places
paths is fine (bodies are small).

### `check.rs`

```rust
/// Run every ownership analysis that exists so far on every body, appending
/// diagnostics. Order per body: moves/uses (this package), then lateinit.
pub fn run(hir: &hir::Program, mir: &Program, diags: &mut Vec<Diagnostic>);
```
Later packages add their passes to the end of `run` (WP5 mutability, WP7).
Bodies are processed in `FuncId` order; the diagnostics of one body are sorted
by primary span start before being appended, so output is deterministic.

### `lateinit.rs`

```rust
pub fn check(hir: &hir::Program, body: &Body, md: &MoveData, ever: &Results<EverInit>,
             cycles: &IndexVec<BlockId, Option<u32>>, out: &mut Vec<Diagnostic>);
```
This is spec 3.1.6 (the spec assigns it to WP5; the removal of `Flow` forces it
here, WP5 folds it into `mutability.rs` later). Only `Mutability::Fixed`,
non-parameter, user locals; `Mut` and `Open` are skipped (the `open` variant of
E0205 is WP5's).

## What to build

### Use checks (spec 3.1.3)

Write one function `uses(stmt_or_term) -> Vec<Use>` with `Use { place: Place,
span: Span, kind: UseKind }`, `UseKind { Read, FieldWriteBase, Assign }`, and
the order rules of 3.1.2: operands of an `Assign`'s rvalue are checked against
the state before the statement, then their moves take effect, then the
destination is initialised. Details that the spec leaves out:

- `Ref`, `RawRef`, `DeepCopy`, `FakeRead` sources are `Read` uses.
- `Operand::Move(q)` is a `Read` use and a MoveOut of `prefix_path(q)` only if
  `q` has no `Deref`. A `Move` with a `Deref` is **E0310** (below), and no
  MoveOut is recorded.
- A `Call`: args in order (use, then move), then init of `dest`.
- `Drop` and `StorageDead` are **not uses** (a `Drop` of a moved or never
  assigned place is normal; elaboration deals with it in WP6). A `Drop`
  of `p` is recorded as a MoveOut with cause `Drop` so that "assign after drop"
  re-initialises. `StorageLive(l)` re-arms `l` (it makes all of `l`'s paths
  uninit and clears `EverInit`/`ReachingMoves` for them).
- Assigning to `q.f` with no `Deref`: the parents of `q.f` must be fully
  initialised or the error is **E0313** (a parent has a reaching MoveOut) or
  **E0332** (no reaching MoveOut: it never had a value). Assigning a whole local
  needs nothing. Assigning through a `Deref` (`(*r).f = ...`) is a `Read` use of
  `r`.
- Report once per move path per body (`reported: BitSet`), as today.

Failing use → E0301 or E0207 exactly as 3.1.3. For the E0301 help choose from
the first listed MoveOut by `cause` and the moved type:
`CallArg` of a user function whose parameter is not a borrow → "if `greet`
should not take ownership, give it a `&string` parameter; or pass a copy:
`greet(copy name)`"; `Assign` → "to keep both, write `string b = copy name;`"
(use the real source text: `copy` + the moved place); `&mut` typed place →
the `lend` help and the "(a `&mut` borrow)" label; `StructDef.stays` → the
`stays` help; partial variant → message "`p` is used after `p.name` moved".
"On some paths" label when the use is also reachable with the path in
`MaybeInit` or when `MaybeInit` contains it (some paths moved, some did not).
The loop variant label when the MoveOut's block is not before the use's block in
RPO and the use is reachable from the move only around a back edge.

Place text in messages: the user's spelling (`p.name`, `self.hp`): build it
from `LocalKind` names and `hir` field names; a deref of a local named
`r` prints as `r`.

### E0310, E0315

E0310: `Operand::Move(q)` where `q.proj` contains `Deref`. Variants (section 4):
`&` borrow, `self` (Q8b help `return copy self.name;`), raw pointer ("cannot
move out through a raw pointer"). Decide by the type of the local the first
`Deref` applies to and whether the local is `self`. A `take` through a borrow
reaches here too (it lowered to `Move`): its help is "`take` cannot move out of
a borrow; write `copy self.name`"; tell it apart by the statement being the
`Take` temp assignment (`LocalKind::Temp` destination, `_t = Use(Move(..))`
immediately following nothing else) or, better, by a flag the builder sets: if
the builder gives no way to tell, use the generic `return copy` help and say so
in your report.

E0315: `Rvalue::DeepCopy(place)` where the type of `place` contains a
`Type::Ref { mutable: true, .. }` field anywhere (recursively through struct
fields). Needs a helper `holds_mut_borrow(hir, ty)`; put it in `moves.rs`.

### E0105 (spec 3.1.5)

If `body.fallthrough_end` is `Some(bb)` and `bb` is reachable from `bb0`
(`reverse_postorder` contains it): E0105 at `body.span` with today's texts
(typeck.rs:~717): message `{function|method} `NAME` can finish without
returning a value`, note ``promises to return `T` ``, help "add a `return` at
the end, or to every branch". Move the strings verbatim. Use the function's
user name (`Body.name`; for methods it is `Struct::name`, match what typeck
printed today: look at what `f.name` was).

### E0205 late initialisation (spec 3.1.6)

For `Assign(_l, ..)` / call `dest = _l` to a whole `Fixed` user local that is not
a parameter: `l ∉ EverInit` before it → fine. Else E0205 with typeck's
current texts and strings, variant chosen as the spec says:
- block in a cycle (`in_cycle`) and `StorageLive(l)` not in the same SCC →
  "cannot assign `x` inside a loop: it is not `mut`", note "the loop would
  assign it again each time round", label at the declaration "declared here,
  outside the loop, without `mut`", help "declare it as `mut T x`, or move the
  declaration into the loop";
- otherwise "cannot change `x`: it is not `mut`", note "assigned a second time
  here", label "declared here without `mut`", help "declare it as `mut T x` to
  allow changes".
Copy these from typeck.rs:~1413-1427 before you delete them. The first
assignment is `EverInit`-free by construction, **but** a local declared without
a value whose only assignment is inside a loop must give the loop variant even on
the first (static) assignment: that is what the SCC test is for. Note a
`Drop` with `reason: Assign` inserted before a late assignment is not an
assignment.

### Wiring and the removal of `Flow`

`lib::check` after a clean typeck: build MIR with `mir::build_program`
(diagnostics of the builder go into `diags`), and if there are no errors run
`mir::check::run`; return `(None, diags)` on any error. Only the HIR is
returned (WP6 introduces `Checked`; do not do it here). WP4 will build the MIR
a second time inside `lib::build`; that duplication is intended for this wave.

Error-mixing consequences (spec 2.1, visible): a program with a type error no
longer gets E0105/E0207/late-init E0205 reports. Fix the tests like this:

- `tests/fail/noreturn.drh`: keep `h` (the `return;` type error, typeck) and
  drop `f`, `g`; new `tests/fail/own_missing_return.drh` has `f` and `g`
  (`.err`: the two E0105 lines of the old file).
- `tests/fail/immutable.drh`: keep `bump` (parameter), `y++`, `z += 1`
  (typeck, `changes`); new `tests/fail/own_late_init.drh` has the `x = 2`
  (initialised, then assigned), `once` and `looped` cases. Update both `.err`
  files; the summary line counts change (`3 errors` / `3 errors`).
- `tests/fail/flow.drh` and `unassigned.drh` contain only MIR-reported codes
  plus the W0110 warning: they stay as they are and must pass unchanged
  (`9 errors, 3 warnings`, `5 errors`).
- `tests/fail/structmut.drh` and any other file: grep every `.err` for E0105,
  E0207, "second time", "inside a loop"; each hit that sits next to a typeck
  error must be split the same way.

## Tests to add

All fail tests: one `.drh` + `.err` in `tests/fail/`; each `.err` line below
must appear in stderr. Take the programs from the WP2 section of
`step3-ownership.md` (they are correct for the owner's answers), with these
expectations spelled out:

1. `own_use_after_move.drh`: ``error[E0301]: `name` is used after it moved``,
   `` `name` moved here ``, `used here, after the move`,
   ``help: if `greet` should not take ownership, give it a `&string` parameter; or pass a copy: `greet(copy name)` ``.
2. `own_move_in_loop.drh`: ``error[E0301]: `s` is used after it moved``,
   `` `s` moved here, in the previous time round the loop ``,
   ``help: move it only once (for example after the loop), or pass `copy s` ``.
3. `own_partial.drh`: ``error[E0301]: `p` is used after `p.name` moved``,
   `` `p.name` moved here ``, ``help: give `p.name` a new value first, or move a copy: `copy p.name` ``;
   and **no** diagnostic at the line `io::outln(p.title);` (the program of the
   spec has it; the fine uses are also covered by the Rust tests of item 13).
4. `own_maybe_moved.drh`: ``error[E0301]: `s` is used after it moved``,
   `` `s` moved here, on some paths ``.
5. `own_move_out_of_borrow.drh`: ``error[E0310]: cannot move `p.name` out of a borrow``,
   `` `p` is a `&` borrow ``, ``help: return a copy: `return copy p.name;` ``.
6. `own_self_move.drh`: ``error[E0310]: cannot move `self.name` out of a borrow``,
   `` `self` is borrowed by `nameOf` ``, ``help: `self` is borrowed; write `return copy self.name;` ``.
7. `own_field_move.drh` and `own_take_drop.drh` (builder diags, WP1 phase B
   already reports E0324/E0325; these are their fail tests, WP1 may already have
   added them: do not duplicate): ``error[E0324]: cannot move `p.name` out of `p` without `take` ``,
   ``help: write `take p.name` to move it out, or `copy p.name` to keep both``;
   ``error[E0325]: cannot take `f.path` out of a `File`: it has a `drop()` ``,
   `` help: put a value back in the same statement: `take f.path leaving ""` ``.
8. `own_move_mut_borrow.drh` (Q15): ``error[E0301]: `s` is used after it moved``,
   `` `s` (a `&mut` borrow) moved here ``, ``help: to keep using `s` afterwards, pass it on with `lend s` ``.
9. `own_reassign_moved.drh` (spec text): ``error[E0205]: cannot change `b`: it is not `mut` ``,
   `assigned a second time here`.
10. Reassigning (no file): a moved local given a new value is usable
    again (no error):
    ```cpp
    none consume(string s) { }
    none main() {
        mut string s = "a";
        consume(s);
        s = "b";
        consume(s);
    }
    ```
    A fail test cannot hold it: put it in the Rust test of item 13.
11. `own_assign_part_moved.drh`: E0313:
    ```cpp
    struct Player { string name; mut uint32 hp; }
    none consume(Player p) { }
    none main() {
        mut Player p = "Illy", 3;
        consume(p);
        p.hp = 5;
    }
    ```
    → ``error[E0313]: cannot give `p.hp` a value: `p` has moved``,
    `` `p` moved here ``, ``help: give `p` a whole new value instead: `p = ...;` ``.
12. `own_assign_part_unset.drh`: E0332 (new):
    ```cpp
    struct Player { string name; mut uint32 hp; }
    none main() {
        mut Player p;
        p.hp = 5;
    }
    ```
    → ``error[E0332]: cannot give `p.hp` a value: `p` has no value yet``,
    `` `p` is declared here without a value ``, ``help: give `p` a whole value first: `p = ...;` ``.
    (Whether a struct may be filled in field by field is the owner's call; this
    is the spec's reading, 3.1.3. Mention the test in your report.)
13. Rust tests in `src/mir/check.rs` (`#[cfg(test)]`), each building a source
    with `drachm::check` + `mir::build_program` + `check::run`:
    reassign after move is fine (item 10); `take ... leaving` keeps the field
    usable; a `while` loop whose body moves a variable declared *inside* the
    loop is fine; `if c { consume(s); return; } consume(s);` is fine (the
    moving branch does not reach the second use); a `Drop` before assignment of a
    moved string produces no error.
14. `own_mut_copy.drh` (E0315), using the syntax WP0a accepts for storing a
    `&mut` in a struct (LOOK FIRST in `tests/borrows.rs`):
    ```cpp
    struct Writer lives out { &mut string out; }
    none main() {
        mut string s = "";
        Writer w = &mut s;
        Writer v = copy w;
    }
    ```
    → ``error[E0315]: cannot copy `w`: it holds a `&mut` borrow``,
    ``note: `Writer.out` is `&mut string`; a copy would be a second `&mut` to the same string``,
    ``help: borrow `w` instead: `&Writer v = &w;` ``.
15. Dataflow unit tests in `dataflow.rs`: `BitSet` operations; a toy forward
    "reaching assignment" analysis and a toy backward liveness on a hand-built
    `Body` (build via source + builder, not by hand); fixpoint on a loop; a
    block unreachable from `bb0` is never visited.
16. `moves.rs` unit tests: `MoveData::build` on `Player p; take p.name` gives
    paths `p`, `p.name`; `MaybeUninit` after a conditional move is "maybe";
    `EverInit` forgets at `StorageDead`.

## Done when

`cargo test` is green with the fail tests above; `tests/fail/{flow,
unassigned}.err` pass unchanged; every program in `tests/run` and `examples/`
still checks with **no MIR diagnostics** (if one gets E0301 because a struct
over 32 bytes is now moved, not copied: fix the program in `tests/run` by
adding `copy`, list it in your report; for `examples/` or `site/examples/` do
not edit, report); `grep -n Flow src/typeck.rs src/typeck/control.rs` finds
nothing; `drachm explain E0332` works.

## Pitfalls

- Report the *first* use only, once per path; do not report the second use of
  a moved value, nor uses after an E0207.
- `Drop` and `StorageDead` of a moved place are not errors (see above). If you
  check them you will flag every scope end.
- `ReachingMoves` kill rule: an init of `p` kills moves of `p`, its ancestors
  and its descendants. `p = new` after `take p.name` kills the `p.name` move.
- A `Move` inside a loop must see the previous iteration's move: that falls
  out of the fixpoint over the back edge; test it.
- Call terminator order: argument uses and moves happen before `dest` is
  initialised (`x = f(x)` with a moving `x` is fine).
- `&&` / `||` temps and `while` conditions are "maybe initialised" by the
  builder's design; they are not user variables, never report them by name.
- `return x;` is `_0 = move x`: a MoveOut of `x`, not a use-after-move later.
- E0207's texts and E0105's texts must stay byte-identical to today's; diff the
  `.err` files of the old tests before and after.
- The E0205 variant test uses CFG cycles, not syntax: a `for` over a range
  desugars to a loop, so `for (...) { once = i; }` is a cycle.
- Keep `check::run` panic-free on any body the builder produced, including
  bodies with no `StorageLive` for a temp (`Temp` locals are not user locals).
