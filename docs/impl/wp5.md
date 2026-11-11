# WP5: mutability checks (E0205, E0311, E0312) on the MIR

Implementer: **a standard model is enough** (one pass over the MIR with a small
table, many message variants that must match existing texts; no dataflow beyond
what WP2 built). Runs in parallel with WP6 (file ownership below) and with the
units work. WP2, WP3 and WP4 are merged. Read first: `step3-ownership.md` 3.7
(self modes), 3.8 (the table), 3.1.6, 4 (E0205, E0311, E0312 and their
variants), owner answers Q5, Q6, Q8, Q16 in `owner-answers.md`, `wp2.md` (how
its checks are wired), and the code: `src/mir/{check,lateinit,moves}.rs`,
`src/ownership/self_mode.rs`, and in `src/typeck.rs` the functions named below.
This brief says where it differs from the spec; **where they differ, this
brief wins.**

## Goal

Every "this thing may not change" error is reported by one pass on the MIR,
`src/mir/mutability.rs`, and typeck reports none of them. That covers:

- a write to an immutable local, parameter or field (E0205, all existing
  variants, same texts);
- a second or in-loop assignment of a late-initialised immutable binding
  (E0205, today in `lateinit.rs`; the file folds into `mutability.rs`);
- reassigning an `open` binding (E0205, new `open` variant, Q6);
- a `&mut` borrow, or a call of a method that changes `self`, on something
  that cannot be lent mutably (E0312);
- a write, a `&mut` borrow, or a method call that changes `self`, through a
  `&` borrow (E0311, new code, Q5).

No backend changes, no new MIR statements, no borrow checking (WP7).

## Files you own

New: `src/mir/mutability.rs`, `tests/fail/own_open.*`,
`tests/fail/own_mut_through_shared.*`, `tests/fail/own_mut_field_shared.*`,
`tests/fail/own_receiver_*.*`, `tests/run/own_open_ok.*`.

Edit:
- `src/mir/check.rs`: **append** the call to `mutability::check` at the end of
  `check_body` (WP2 said new passes go at the end of `mir::check::run`'s
  per-body work) and fix the module doc comment. Nothing else in the file.
- `src/mir/lateinit.rs`: delete it. Its logic (live SCC of a binding, `EverInit`,
  in-loop test) moves into `mutability.rs` unchanged in behaviour. Remove
  `pub mod lateinit;` from `src/mir/mod.rs` and add `pub mod mutability;` in the
  same place (the `moves`/`lateinit` line group; WP6 adds `pub mod drops;`
  elsewhere in that file, so the hunks do not overlap).
- `src/typeck.rs`: remove exactly these, and nothing else:
  `report_fixed_receivers` (~798) and `Globals.fixed_receivers` / `FixedReceiver`
  with its push at ~1879; `check_field_change` (~1686) and its two callers
  (~1570, ~1575, ~2337); `check_assignable` (~1731) and its callers;
  the `SelfUse` machinery if nothing is left reading it (`self_use`, ~65, ~102,
  ~120: WP3 moved the inference to `ownership/self_mode.rs`); the
  `PlaceInfo.mut_field` / `through_deref` fields **only if** no remaining code
  reads them (line ~2508 and ~2572 do: check before deleting). Do **not** touch
  the drop/copy plumbing (`owned`, `temps`, `scoped`, `take_temps`, `Stmt::Drop`
  insertion, `ExprKind::Copy/Move/Temp/Scoped` construction): WP6 owns it. Do not
  reformat the file; the units work also adds a few hook calls to it.
- `src/ownership/self_mode.rs` (+ `src/ownership/mod.rs` re-export): add
  `pub fn writes_only_mut_fields(hir: &hir::Program, f: FuncId) -> bool`
  (see "The mut-field method case").
- `src/diag/codes.rs`: add `E0311` and `E0312` **directly after the `E0310`
  entry**, with `explain` texts in the style of the neighbours. Units add their
  block after `E0332`, WP6 adds nothing.
- `tests/fail/{immutable,flow,noreturn}.*` stay as they are (see Tests).
- The docs site (below).

Do not touch: `src/mir/build.rs`, `src/hir.rs`, `src/codegen*.rs`, `src/cgen*.rs`,
`src/lib.rs`, `tests/run.rs`, `tests/backends.rs`, `runtime/`. If the builder
must change, say so in the report; WP6 owns it in this wave.

## LOOK FIRST

1. Which spans the statements you will report on carry (`Assign`, `Ref`, a
   `Call`'s `Arg.span`). The place a diagnostic points at is the user's text
   (`Cx::place_text` in `check.rs` already turns `(*p).name` into `p.name`; reuse
   or move it to a shared helper in `mutability.rs`).
2. How the builder lowers a receiver: for a method call it emits a temp
   `_t = &_k` or `_t = &mut _k` (by `self_modes`) in the block before the
   `Call`, and `_t` is `args[0]`. You recover "this `Ref(Mut, p)` is a receiver
   borrow of method `f`" by looking at the `Call` whose `args[0]` is the temp
   (the statement is in the same block). User-written `&mut x` arguments are the
   same shape but the callee's `args[i]`, i > 0 or a non-method callee.
3. How `Take ... leaving` and `x.f++` lower (an `Assign` to a place; `++` reads
   the same place in its `Binary`). You cannot tell `++` from `x = x + 1`; keep
   today's headline text (`cannot change ...`) for both.
4. Where params with `Mutability::Fixed` are initialised in `EverInit`
   (`MoveData::local_path`, boundary state): a write to a `Fixed` parameter is
   always an error ("cannot change parameter"), a write to a `Fixed` user local
   is an error only when it is not the first assignment.
5. The current texts. Print the output of `tests/fail/{immutable,own_*}` and
   `drachm check` of a program with each variant below **before** you start, and
   keep them byte for byte (headline, labels, notes, helps). Where this brief
   gives a new text, it is taken from spec section 4.

## What to build

`mutability::check(hir, body, md, ever, cycles, modes, out)` per body, after the
move checks.

### The table (spec 3.8, with the corrections below)

`changeable(place)` walks the projections from the end:

| Place | Result |
|---|---|
| a bare local | `Mut` or `Open`: yes. `Fixed` parameter: `NoParam`. `Fixed` other: `NoBinding` |
| `x.f` where `f` is a `mut` field and no `Deref` occurs in the part before `.f` | yes (owner Q5: `mut` fields are special **only on a value you own**) |
| `x.f` otherwise | `changeable(x)` |
| `*r` where `r: &mut T` | yes (the `&mut` need not sit in a `mut` binding) |
| `*r` where `r: &T` | `ThroughShared { borrow: r }` |
| `*p` where `p` is a raw pointer | yes (WP9 checks `unsafe`) |

The `&self` of a method is a `&Self` or `&mut Self` parameter, so its fields are
`(*_1).f`: a `mut` field of `self` is **not** changeable when `self` is a `&`
borrow (Q5, and WP3 makes any such method `&mut self`).

| MIR | Check | Error |
|---|---|---|
| `Assign(_l, rv)`, whole local, `Mut` | fine | |
| `Assign(_l, rv)`, `Fixed` or `Open` non-param | fine only as the first assignment, and not inside a loop that began after the declaration (the old `lateinit` logic), else E0205; for `Open` after init: the `open` variant | E0205 |
| `Assign(_l, rv)`, `Fixed` or `Open` parameter | E0205 ("cannot change parameter", `open` variant for `Open`) | E0205 |
| `Assign(p.f.., rv)` with projections | `changeable(p.f..)` | E0205 (binding root), E0311 (through `&`) |
| `Assign(_r, Use(Ref ..))` where `_r` is a borrow-typed binding (re-point, R18) | the binding must be `Mut` | E0205 |
| `Assign((*_r), ..)` write through a `&mut` binding that is `Fixed` | fine (Q1: writes through) | |
| `Rvalue::Ref(Mut, p)` (user `&mut p`, `lend`, receiver) | `changeable(p)`; for a bare `Fixed` local the `Open` and `Mut` bindings pass | E0312 (binding root), E0311 (through `&`) |
| `Rvalue::Ref(Shared, p)` | always fine | |
| `RawRef` | not yet; WP9 | |

`Open` is changeable for field writes and `&mut` borrows but not for whole
reassignment; an `open` parameter is the same (Q6).

Do not report on compiler-generated functions (`hir.funcs[..].span == Span::new(0,0)`,
the automatic `==`), as `mir::check::run` already skips them.

### The mut-field method case (read the language question at the end first)

Today `k::hit()` on an immutable owned `Counter k`, where `hit` writes only a
`mut` field, compiles (`tests/run/structs.drh`, `site/examples/structs.drh`;
`Big::touch` in `tests/run/layout.drh`). WP3's inference makes `hit` and `touch`
`&mut self`, so under the strict table above `k::hit()` is E0312. The owner's
answers do not say which is intended (end of this file). **Until the owner
answers, keep today's behaviour**, so nothing already working breaks:

- `ownership::writes_only_mut_fields(hir, f)` is true when `f` is a method
  whose every piece of `Mut` evidence is a write to `self.<path>` where the path
  contains a `mut` field (no `Deref` before it), and there is no `&mut self..`/
  `lend` evidence, `f` is not `drop`, and every method it calls on `self` also
  satisfies this. It needs a pass over the same evidence `infer` collects; extend
  `Evidence` rather than writing a second walker.
- At a receiver `Ref(Mut, p)` of method `f` where `writes_only_mut_fields(f)`:
  if `p` is rooted in an owned binding with no `Deref` (any mutability), allow it;
  if `p` goes through a `&` borrow, E0311 (Q5: through `&` everything is
  read-only, no exception); if through a `&mut`, fine.
- Every place where this exception is applied carries `// OPEN(wp5MutFieldMethod)`
  and is one function, `receiver_exempt`, so that the owner's answer is a
  one-function change plus test edits.

### Messages

- Keep every existing E0205 headline, label, note and help (call-site variants
  from `report_fixed_receivers`: "cannot call `m` on `x`: it changes the value,
  and `x` is not `mut`" is replaced by the spec's E0312 method variant below,
  because a method call that changes `self` is a `&mut` borrow; that is the one
  intended text change and it moves from E0205 to E0312. `tests/fail` files that
  contained it are updated in the same commit, listed in the report).
- E0312 binding variant and method variant, E0311 field and method variant,
  E0205 `open` variant: spec section 4 (the blocks quoted there), including the
  self-mode note "`heal` changes `self`: it writes `self.hp` (player.drh:7:9)"
  built from `ownership::reason_chain`.
- One diagnostic per statement; if a `&mut` temp is created for a call and the
  same place also errors elsewhere, report each site once.

## Tests

1. `tests/fail/immutable.*` unchanged and passing. Every other existing
   `tests/fail` file passes unchanged except those that held the old
   receiver text (update and list them).
2. New fail tests (expected `.err`: headline lines only, as the harness does):
   - `own_open.drh`: the spec's WP5 example (`open string xs`: `grow(&mut xs)`
     fine, `xs = "b"` E0205 open variant, `string ys` then `grow(&mut ys)` E0312).
   - `own_mut_through_shared.drh`: `struct Player { string title; none retitle()
     { self.title = "x"; } }`, `none hurt(&Player p) { p::retitle(); }` is E0311
     with the method note.
   - `own_mut_field_shared.drh` (Q5): `struct Player { string name; mut uint32 hp; }`,
     `none hurt(&Player p) { p.hp = p.hp - 1; }` is E0311.
   - `own_receiver_fixed.drh`: a method that writes a non-`mut` field, called on a
     `Fixed` local, a `Fixed` parameter and a temporary (`make()::grow()`): three
     E0312, each with the self-mode note.
   - `own_param_write.drh`: write to a `Fixed` parameter and to a `Fixed`
     parameter's field.
   - `own_fixed_field.drh`: `Player p` immutable; `p.name = "x"` is E0205 and
     `p.hp = 5` (a `mut` field) is fine; same inside a loop.
   - `own_repoint.drh`: `&mut string r = &mut a; r = &mut b;` with `r` not `mut` is
     E0205 and `r = "x"` (write through) is fine.
3. New run test `tests/run/own_open_ok.drh` from `step3-ownership.md` WP5 (Q6),
   `.out` `x9`, passing on both backends and leak-free once WP6 lands (until
   then, strings in it are what they are today). If the brief's parameter
   spelling for passing an `open` binding does not parse, use what WP0a accepts
   and say so.
4. Unit tests in `mutability.rs` for `changeable` over hand-built bodies (bare
   locals of each mutability, `mut` field on owned, `mut` field behind a `&`
   deref, `&mut` deref, `&` deref).
5. The exemption: `tests/run/structs.drh` and `layout.drh` build and run
   unchanged.

## Site updates (same commit; `CLAUDE.md` "Keeping the docs site current")

- `site/content/reference-ownership.md`: a new section "Changing things"
  (or extend the existing text about `mut`): `mut`, `open`, `mut` fields, what a
  `&` borrow may not do, with real examples. Put it **above** "Other move
  errors" so WP6's new `destroy`/`drop()` section appends cleanly below.
  Only describe `&`/`&mut` borrows if `drachm check` accepts them (it does) and
  say they cannot be built yet (the page already says so).
- `site/content/reference-types.md` or `reference-structs.md`: wherever `mut`
  fields are described, state the Q5 rule in one sentence.
- A runnable example `site/examples/mutability.drh` + `.out` for the part that
  builds today (`open` binding, `mut` field on an immutable struct, `p.hp = 5`).
  Borrow-based examples can only be `drachm check`-only; use the markdown page
  with a plain fenced block, not an `<!-- example: -->` include, for those.
- `site/content/lesson-8-ownership.md`: only if it currently says anything this
  changes; otherwise leave.
- Rerun `site/gen-errors.sh` (new codes E0311, E0312). `site/content/coming-later.md`:
  no change in this package.

## Done when

`cargo test` green (both backends); every test above present; `drachm check` on
`tests/fail/own_*` prints the new texts; `typeck.rs` contains no `E0205`;
`grep -rn "lateinit" src` is empty; the `OPEN(wp5MutFieldMethod)` marker exists
exactly once in code and the report quotes the three affected programs
(`structs.drh`, `layout.drh`, the site `structs` example).

## Pitfalls

- A `Fixed` local assigned in `if`/`else` branches (once on each path) is fine:
  `EverInit` is a may-analysis over paths, so the second branch does not see the
  first. The existing lateinit tests cover it; keep them.
- Hidden temps (`LocalKind::Temp`) are always assignable and lendable; only
  `User` and `Param` locals can be `Fixed`.
- Do not forget `Mutability::Open` parameters when reading `LocalKind::Param`.
- `&m` on a `&mut` binding (owner #3) is a shared reborrow of `*m`: the place is
  `Deref(m)` with `Ref(Shared, ..)`, never an error here.
- Never panic on a MIR shape you do not expect: skip it (the `validate` pass
  and WP2 already exist), then add a test.

---

## Language question for the owner (write-up; WP5 keeps today's behaviour meanwhile)

**Q-MF. Calling a method that writes only a `mut` field on an immutable value.**

Your answers say: `mut` fields are only special on values you own (Q5); through
`&` everything is read-only; methods only borrow `self` (Q8); the compiler
infers whether a method changes `self` (Q16, step 3 spec). The compiler's
inference counts a write to `self.hits` as changing `self`, even when `hits` is
a `mut` field, because inside the method `self` is a borrow and "through a
borrow everything is read-only, changing needs `&mut`".

So for

```cpp
struct Counter {
    string label;
    mut uint32 hits;
    none hit() { self.hits++; }
}

none main() {
    Counter k = "page", 0;   // not `mut`
    k.hits++;                // fine: your own value, a `mut` field (Q5)
    k::hit();                // ???
}
```

the two rules pull apart: `hit` needs a `&mut` borrow of `k`, and `k` is an
immutable binding. Nothing in your answers says which wins. Today's compiler
(before this step) accepts `k::hit()`, and two tests and the docs site rely on
it. Options:

- **A. Strict: the call is an error.** A method call that changes `self`
  needs a `mut`/`open` binding, whatever it writes. `mut` fields only help code
  outside the struct.
  ```cpp
  Counter k = "page", 0;
  k.hits++;        // fine
  k::hit();        // error[E0312]: cannot call `hit` on `k`: `hit` changes `self`, and `k` is neither `mut` nor `open`
  mut Counter m = "page", 0;
  m::hit();        // fine
  ```
- **B. The compiler tracks a third kind of method: "changes only `mut` fields".**
  Inferred, no new syntax. Such a method may be called on an owned immutable
  value (the field is `mut` there), not through a `&` borrow (Q5), and fine
  through `&mut`.
  ```cpp
  Counter k = "page", 0;
  k::hit();        // fine: hit only touches the `mut` field `hits`
  none look(&Counter c) { c::hit(); }   // error[E0311]: through a `&` borrow
  ```
- **C. Say it in the method (original to Drachm): the method names what it
  touches.** A method that writes only `mut` fields must say `changes fields`
  after its parameters (the same place as Q16's `changes self`); without the
  words it is an ordinary changing method (option A's rule), and with them it
  behaves like B. The words are checked: writing a non-`mut` field in such a
  method is an error.
  ```cpp
  none hit() changes fields { self.hits++; }
  Counter k = "page", 0;
  k::hit();        // fine
  ```

Until you answer, the compiler behaves like B (what it did before), marked
`OPEN(wp5MutFieldMethod)`.
