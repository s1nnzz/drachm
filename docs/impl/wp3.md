# WP3: self-mode inference

Implementer: **a standard model is enough** (a small, fully specified fixpoint
over the HIR; the only care needed is reading the HIR shapes correctly and not
touching typeck). Runs in parallel with WP2 and WP4. Starts after WP0a and WP1
phase B are merged. Read first: `step3-ownership.md` 1.4 (R1, Q5, Q8, Q17),
3.7, and `owner-answers.md` Q5, Q8, Q16, Q17.

## Goal

For every method, decide whether it needs `&self` (`Shared`) or `&mut self`
(`Mut`, with the first reason), from the HIR alone, with no `mut`-field
exception. The MIR builder uses the answer for receiver borrows. **Nothing
user-visible changes in this package**: typeck keeps reporting E0205 for
receivers with its own (old) logic until WP5 replaces it. The new module runs
beside it.

## Files you own

New: `src/ownership/mod.rs`, `src/ownership/self_mode.rs`, `tests/self_mode.rs`
(a library-API test; integration tests in `tests/` are picked up by cargo
automatically).

Edit, each limited to the named spot:
- `src/lib.rs`: **one line**, `pub mod ownership;`, between `pub mod mir;` and
  `pub mod parser;`. Do not touch `check()` or `build()` (WP2 and WP4 own
  those bodies).
- `src/mir/mod.rs`: replace the placeholder `pub enum SelfMode { Shared, Mut }`
  (and its comment) with `pub use crate::ownership::{MutKind, MutReason, SelfMode};`.
  Nothing else in that file (WP2 and WP4 add `pub mod` lines elsewhere in it).
  `SelfMode` loses `Copy` (it holds a reason); fix the compile errors this
  causes in `src/mir/` (they are `.clone()`s or references).
- `src/mir/build.rs`: only `program()` (fill `Program.self_modes` from
  `ownership::self_mode::infer`, replacing `has_self.then_some(SelfMode::Mut)`)
  and `receiver_mutable`, which becomes a lookup in the modes (`Builder` needs a
  `modes: &[Option<SelfMode>]` field; the signature `receiver_mutable(f)`
  may change to take it). Nothing else in `build.rs`: WP2 may make a one-spot
  bug fix there too, so keep your diff to those two functions plus the field.
- `tests/mir/`: add the one test below.

Do **not** touch `src/typeck.rs` or `src/typeck/*` (WP2 is deleting `Flow`
there at the same time). Leave `SelfUse`, `report_fixed_receivers`,
`check_field_change` where they are; WP5 deletes them.

## LOOK FIRST

1. **How a method body names `self`.** Spec A7 says `self` is local 0
   (`has_self`). After WP0a, is its HIR type still the struct type, or `&Self`?
   Do not depend on the type; use `func.has_self && place.local == 0`.
2. **HIR shapes after WP0a** that carry a self place: `Stmt::Assign(Place, Expr)`
   (compound assignment already lowered to `Assign(place, Binary(.., place_read, v))`),
   `ExprKind::IncDec { place, .. }`, `Borrow { mutable, place: Box<Expr> }`,
   `Lend(Box<Expr>)`, `Take { place, leaving }`, `Call(FuncId, Vec<Expr>)` with
   the receiver as `args[0]` for `has_self` callees (A7). How a field access on
   `self` appears as an *expression* (`Field(Box<Expr>, usize)` over
   `Local(0)`), and where `Deref` nodes get inserted by `Checker::read`.
3. **Legacy nodes** (`Copy`, `Move`, `Temp`, `Scoped`, `Stmt::Drop`): they wrap
   expressions and must be walked through, never treated as evidence.
4. `Span` derives (`Clone`, `Copy`, `Debug`, `PartialEq`)? You need them for
   `MutReason`. Add `PartialEq` if missing (it is in `src/diag`; that one-line
   derive is allowed).
5. Which struct/method `FuncId` a method belongs to and its name:
   `Func.symbol` is the symbol; the user name comes from typeck's naming
   (`report_fixed_receivers` uses `g.sigs[callee].name.rsplit("::")`). Methods
   may only be identified by `has_self` in `hir::Func`. If there is no struct
   or method name in `hir::Func`, you only need `FuncId`s here; names come from
   `Func.symbol`.
6. Whether `drop` methods are identifiable: `StructDef.has_drop: Option<FuncId>`
   (WP0a).

## Data structures

```rust
// src/ownership/mod.rs
pub mod self_mode;
pub use self_mode::{infer, MutKind, MutReason, SelfMode};

// src/ownership/self_mode.rs
#[derive(Debug, Clone, PartialEq)]
pub enum SelfMode { Shared, Mut(MutReason) }

#[derive(Debug, Clone, PartialEq)]
pub struct MutReason { pub span: Span, pub kind: MutKind }

#[derive(Debug, Clone, PartialEq)]
pub enum MutKind {
    /// Assignment, compound assignment, `++`/`--`. The place as the user
    /// writes it: "self.size" (a deref of a borrow field prints as the field:
    /// "self.out").
    Writes(String),
    /// `&mut self.items`, or `lend self.out`.
    LendsMut(String),
    /// A call of a method that changes `self`, on a self place.
    Calls(FuncId),
    /// The method is `drop` (reserved name: always called with exclusive access).
    IsDrop,
}

/// One entry per `FuncId`; `None` for functions without a `self`.
pub fn infer(hir: &hir::Program) -> Vec<Option<SelfMode>>;

/// The chain of reasons for diagnostics (WP5): `[heal: Writes "self.hp"]` or
/// `[grow: Calls(resize), resize: Writes "self.cap"]`. Stops at the first
/// non-`Calls` reason; cannot loop (the reason of a `Mut` method points at a
/// method that was made `Mut` earlier).
pub fn reason_chain(modes: &[Option<SelfMode>], f: FuncId) -> Vec<(FuncId, MutReason)>;
```

## What to build

`infer` is spec 3.7:

1. A visitor `walk(func, &mut FnMut(Evidence))` over the HIR body, an
   **exhaustive** `match` over `Stmt` and `ExprKind` (no `_ =>` arm, so a new HIR
   node is a compile error here), producing for a method body in source order:
   - `Assign(place, _)` and `IncDec { place, .. }` where the place's root is
     `self` → `Writes(text)`. **No exception for `mut` fields** (Q5): through a
     borrow everything is read-only. A write through a `&mut` field
     (`*self.out = ..`, place `self.out` + `Deref`) counts, text "self.out".
   - `Borrow { mutable: true, place }` of a self place → `LendsMut(text)`;
     `Lend(e)` of a self place → `LendsMut(text)` (a `&mut` field can only be
     lent through exclusive access).
   - `Call(f, args)` with `has_self` callee `f`, where `args[0]` is a self place
     (root `self`, `Field`/`Deref` chain): candidate edge `Calls(f)`.
   - Not evidence: reads, `Borrow { mutable: false }`, `Take`/`Copy`/`DeepCopy`
     of a self place (moving out through a borrow is E0310 later), calls on
     places not rooted at `self` (a parameter, a local, a temp), writes through
     a raw pointer field `*self.p = ..` (see "Questions" in your report: the
     owner has not said; do not add evidence for it).
2. The fixpoint exactly as in 3.7 (worklist, `callers_on_self`, least fixpoint,
   store only the *first* reason, the first evidence in source order). `drop`
   (`StructDef.has_drop == Some(f)`) starts as `Mut(IsDrop)` with the method's
   own span.
3. Place text: root name `self`, then field names along the projections,
   skipping `Deref`s; field names from the struct definitions via the types of
   the prefix (`hir.struct_def(ty).fields[i].name`). If you cannot compute a
   type for a prefix, fall back to `self`.
4. Wire into the builder as described above; `reason_chain` is used by WP5, but
   write and test it now.

## Tests to add

`tests/self_mode.rs` (library API: `drachm::check(src)`,
`drachm::ownership::infer(&hir)`; find a method's `FuncId` by `Func.symbol`
ending in `Counter::bump` or whatever the symbol scheme is) with this source:

```cpp
struct Counter {
    uint32 n;
    mut uint32 reads;
    none bump() { self.n = self.n + 1; }
    none bumpTwice() { self::bump(); self::bump(); }
    uint32 get() { self.reads = self.reads + 1; return self.n; }
    none ping(uint32 k) { if (k > 0) { self::pong(k - 1); } }
    none pong(uint32 k) { if (k > 0) { self::ping(k - 1); } }
    uint32 peek() { return self.n; }
    none inc() { self.n++; }
    none drop() { }
}
struct Outer {
    Counter c;
    none go() { self.c::bump(); }
    none look() { uint32 x = self.c::peek(); }
    none other(Counter k) { k::bump(); }
}
none main() { }
```
Expected (assert the variant and, for `Writes`/`LendsMut`, the text; assert the
reason span slices to the evidence from the source):
`bump` Mut(Writes "self.n"); `bumpTwice` Mut(Calls bump); `get`
Mut(Writes "self.reads") (the `mut` field is still read-only through a borrow:
this changes behaviour once WP5 lands, see pitfalls); `ping`, `pong`, `peek`
Shared (mutual recursion with no change stays `Shared`); `inc` Mut(Writes
"self.n"); `drop` Mut(IsDrop); `Outer::go` Mut(Calls Counter::bump); `look`
Shared; `other` Shared (calls `bump` on a parameter, not on `self`).
Also assert `reason_chain` for `go` is `[go: Calls bump, bump: Writes "self.n"]`,
and that non-methods (`main`) have `None`.

A second source for the borrow forms (only if the borrow syntax of WP0a checks
in a method; otherwise use what it accepts):

```cpp
none fill(&mut string s) { }
struct Buf {
    string data;
    none clear() { fill(&mut self.data); }
    none reread() { &string d = &self.data; }
}
none main() { }
```
→ `clear` Mut(LendsMut "self.data"), `reread` Shared.

`tests/mir/self_modes.drh` + `.mir`: a program with one `Shared` method
(`uint32 peek()`) and one `Mut` method (`none inc()`) called from `main` on a
`mut` binding. The `.mir` file asserts the receiver borrows. Appendix B prints a
borrow rvalue as `&_N` / `&mut _N`; the exact temp numbers depend on the
builder, so first dump the program, then write lines that name the borrow
kind next to the callee, for example `= &_` followed on a later line by
`call peek(`. Use the `!FN: TEXT` negative form to assert that `main` has no
`&mut` borrow feeding `peek`. (Look at how `tests/mir/*.mir` currently match;
keep to whole lines that occur.)

All existing tests stay green: `mir_builds` runs the builder over every
program with methods in `tests/run` and `examples`.

## Done when

`cargo test` green (no output of any existing test changes); the new tests
pass; `grep SelfMode::Mut src/mir/build.rs` finds no hard-wired `Mut`;
`ownership::infer` is called once per program (cache it on `mir::Program`).

## Pitfalls

- **The mut-field difference.** Today typeck treats a method that writes only
  `mut` fields of `self` as non-changing, so calling it on an immutable binding
  is allowed. WP3's modes make such methods `Mut` (Q5). That is intended, but
  it becomes visible only when WP5 switches the receiver check to these modes
  (then `Counter c = ..; c::get()` is E0205). In your report list every method
  in `tests/run/*.drh`, `examples/` and `site/examples/` whose mode differs from
  what `report_fixed_receivers` would say (writes only `mut` fields of `self`
  or calls such a method), so WP5 can expect the test fallout.
- Receiver borrows only matter in MIR for later checks. The builder emitting
  `&` instead of `&mut` must not change what the backends do: both are the
  address of the place.
- Do not look at `changes self` (interfaces, Q16); nothing in step 3 reads it.
- A method on a field of a *borrowed parameter* (`k::bump()` where `k` is a
  `&Counter`) is not about `self`: skip it.
- Static methods have no `self`: `has_self == false` means `None`.
- Receivers that are temporaries or calls (`make()::bump()`): not self places.
- Do not store a reason chain in the `Calls` variant: only the callee id.
- Operator methods (`a + b` on structs) are ordinary `Call`s with `has_self`
  callees; the same rules apply to them with no special case.
