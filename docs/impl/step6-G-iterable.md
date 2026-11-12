# S6G: `Iterable<T>` and `for` over user types

Implementer: standard model. Small: one prelude interface and one `for_each`
arm. Needs S6B (interfaces, including generic interfaces as templates, see
S6B "Out of scope") and S6C.2 (the `for` machinery) merged. S6D is not
required except for the map decision at the end.

Read first: `step6-overview.md` 2.6 and Q16, Q19; SPEC "Collections" ("Your own
types work with `for (T x of thing)` by implementing `Iterable<T>`, whose `T?
next()` returns `none` when done"); `step5-D-handlers.md` (it creates
`src/prelude/core.drh` and the way a prelude is parsed and declared; reuse it);
`step6-B-interfaces.md`; the code: `src/typeck/control.rs` (`for_each`,
`for_text`, `looped`), `src/typeck/lists.rs` (S6C.2's list arm), `src/prelude/`,
`site/content/design-generics.md` "Your own iterables".

## Goal

1. A built-in generic interface, written in the prelude (not in Rust):

   ```cpp
   interface Iterable<T> {
       T? next() changes self;
   }
   ```

   (The SPEC fixes `T? next()`; `changes self` is required because an
   iterator advances: the lesson's `Countdown` writes `self.n--` in `next`,
   which S6B's conformance rule would reject without it.)
2. `struct Countdown with Iterable<uint32> { mut uint32 n; uint32? next() { ... } }`
   (the site example) compiles; `for (uint32 n of c) { ... }` over a `mut
   Countdown c` runs: the loop borrows `c` as `&mut` for its whole extent and
   desugars to a `Loop` of `next()` calls (overview 2.6, Q19 interim A).
3. The loop variable type must equal `T` (E0630 reused); a non-`mut` binding is
   E0662; a type without `Iterable<T>` is E0660; a wrong `next` is E0661 (S6B's
   E0611 may already catch it: only add E0661 if the message is better).
4. A generic function over iterables works with S6B's machinery:
   `uint32 sum<I with Iterable<uint32>>(open I it) { mut uint32 t = 0; for (uint32 x of it) { t += x; } return t; }`.
5. `skip`, `break N`, `continue` inside the loop behave as in every other
   `for` (they come from `looped`).

Out of scope (E0901): lists, arrays, maps and strings as `Iterable<T>` values
(they have built-in loops; `sum<I with Iterable<uint32>>` over a `uint32[?]`
is E0607 with the note "lists are walked with `for` directly so far"), a
two-level `Iterator<T>` (Q19 B), consuming loops (Q19 C), map iteration (Q16:
if the owner answers during S6G's time, implement it here as a second arm; the
runtime function `drachm_map_next` exists), `Iterable` with other item
shapes, infinite loops detection (none).

## Files you own

New: `tests/run/iter_*.drh`, `tests/fail/iter_*.drh`, `site/examples/iter-*.drh`.
Edit: `src/prelude/core.drh` (the interface; if S5D's prelude loader is not
merged, stop and report), `src/typeck/interfaces.rs` (small: resolving the
prelude's `Iterable<T>` as a known interface id, `Globals::iterable`),
`src/typeck/control.rs` (**shared hunk**: one arm `iterable_loop` after the
list arm), `src/typeck/lists.rs` only to share the binding-declaration helper,
`src/diag/codes.rs` (E0660 to E0662), site pages, `docs/PLAN.md`.

Do not touch: S6B's conformance code beyond the lookup, MIR, backends,
runtime.

## What to build (one commit per step, `cargo test` green after each)

1. Prelude interface and lookup: `Globals::iterable: Option<InterfaceId>`;
   `with Iterable<uint32>` on a struct instantiates the generic interface at
   `uint32` (S6B templates) and checks `next` (E0611 if `next` returns
   anything but `uint32?`).
2. The `for` arm: the iterated expression must be a place of a struct type with
   a conformance record for some `Iterable<X>` (a temporary — a call result
   — is allowed too: it is stored in a hidden `mut` local owned by the loop);
   `X` becomes the item type; the binding forms are `X v` (Copy `X` or an
   owning `X` moved out of the optional's `Some`, since the optional is a
   fresh value: **no copy needed**, so owning items work here, unlike over
   lists), `&X v` is E0901 (items are values). Desugar to:
   hidden `&mut it = &mut c;` then `Loop { cond: none; body: [ n = it::next();
   If n is empty { Break }; v = payload(n); user body ] }` built as HIR
   directly (reuse `looped` for labels and `skip` targets). The optional
   `n` is dropped each iteration like any temporary.
3. E0660 with the help "add `with Iterable<uint32>` and a `uint32? next()`";
   E0662; type mismatch E0630.
4. Generic use: `sum<I with Iterable<uint32>>` (definition pass: `I` has `next`
   from the bound, so the loop is valid at the definition; the loop arm must
   accept a `Type::Param` whose bound includes `Iterable<X>`).
5. Tests: `iter_countdown` (the site example), `iter_owning_items` (a type
   yielding `string`s: leak-checked), `iter_break_skip`, `iter_temporary`
   (`for (uint32 n of Countdown(3))`), `iter_generic_sum`, `iter_two_loops`
   (the second loop continues from the exhausted state: documents Q19 A),
   `iter_mut_required` (E0662), `iter_not_iterable` (E0660), `iter_wrong_item` (E0630).

## Site updates (same commit; only what builds)

- `reference-generics.md`: "Your own iterables" with `iter-countdown.drh` and
  `iter-generic.drh` (+ `.out`) and the real E0660/E0662 output; keep the
  shape of `Iterable` and its use on a copy as **Tentative** (Q19) inside the
  section.
- `design-generics.md`, `guide-generics.md` and lesson 15: the "Your own
  iterables" sections have `<!-- status: tentative -->`; **keep** that marker
  (the owner has not fixed the library interfaces; Q19 is open), but now
  the code blocks build: add them to `site/examples/` and say in the text
  "this builds; the shape may change". The page-level badges are S6Z's.
- `reference-control-flow.md`: `for` over a type with `Iterable<T>`.
- `reference-diagnostics.md`, `errors.md`, `coming-later.md`, `docs/PLAN.md`.

## Done when

- `cargo test` green; the `Countdown` example from SPEC/lesson 15 runs on both
  backends and prints the expected countdown; generic `sum` works.
- Report: the exact desugaring (paste the printed MIR of a small loop), how
  owning items were handled, the Q19 data (a program where the exhausted-state
  surprise matters), every interim with file and line.

## Pitfalls

- The hidden `&mut` borrow must end after the loop (a `push`-style use of `c`
  after the loop is fine) and be live during the body (using `c` in the body is
  E0303/E0304 as for lists).
- `next()` returning an optional of a Copy payload is a Copy enum; owning
  payloads move out: use the existing `take` pattern machinery for the payload
  binding, do not invent a second one.
- A `Countdown` whose `next` is `Shared` (does not change) conforms only if
  the interface says `changes self`, which it does: a type whose method changes
  nothing is allowed (infinite iterator): no warning.
- `with Iterable<uint32>` appearing before the interface machinery of S6B
  handled generic interfaces: if S6B left generic interface templates out,
  stop and report rather than patching B.

## Language questions found

Q16, Q19 in `step6-overview.md`.
