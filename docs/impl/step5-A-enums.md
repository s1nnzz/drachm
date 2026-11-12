# S5A: enums with data, `match` on them, and optionals

A.1 (enums): **needs a strong model.** It adds a type kind to every layer
(types, typeck, HIR, MIR, borrow and move analyses, drop glue, both backends,
`Switch`). A mistake is a layout bug that only shows as wrong output on one
backend, or a leak/double free in glue. A.2 (optionals): standard model; it is
small once A.1 has merged.

Read first: `step5-overview.md` sections 1, 2.1, 2.2, 3, 5, 6 (the codes) and
the question list (Q3 to Q7, Q5 in particular: they define what you must refuse
with E0901); SPEC section 4 "Optionals" and "Enums", section 7 "Optionals in
`match`"; PLAN step 5 items 3 and 4; `step3-ownership.md` 2.3 to 2.9 (move
paths, drop elaboration, glue) and `wp6.md`; and the code: `src/types.rs`,
`src/hir.rs`, `src/typeck.rs` (grep `struct_id`, `declare_struct`, `lay_out`,
`copy_structs`, `mark_owning`, `report_cycles`), `src/typeck/control.rs`
(`match_stmt`, `Coverage`, `warn_missing`), `src/parser.rs` (`struct_decl`,
`match_stmt`, `ty`), `src/mir/{mod,build,validate,print,moves,drops,analysis,
check}.rs`, `src/mir/borrowck/{regions,conflicts}.rs`, `src/codegen_mir.rs`,
`src/cgen_mir.rs`, `src/codegen_common.rs`, `src/cgen_common.rs`
(`struct_defs`, `struct_order`), `site/content/design-enums.md`.

## Goal

1. `enum Shape { Circle(float64 r), Rect(float64 w, float64 h), Empty, }`
   declares an enum; `Shape s = Rect(2.0, 3.0);`, `Shape::Empty`, bare variants
   resolved from the expected type, enums as parameters, results, fields of
   structs, locals; Copy and move rules; drop and copy glue.
2. `match (s) { case Circle(float64 r) { } case Rect(float64 w, float64 h) { } case Empty { } case _ { } }`
   with typed bindings, exhaustiveness (W0110 listing missing variants), and
   unreachable cases (W0111).
3. (A.2) `T?`, `Optional<T>` spelled out, `none` as a value, `uint32? x;`
   starts as `none`, `case uint32 a` / `case none` patterns, `maybe` on an
   optional, `Some(x)` as a bare variant.
4. Identical output on both backends, leak-free, no regressions in the
   existing 400-odd programs.

Out of scope (E0901 with the wording below): payloads that own memory bound in
patterns (Q3), nested patterns, guards, or-patterns across variants (Q4),
enum `==` and enum methods (Q6), variants holding borrows or pointers,
recursive enums (E0210 reworded: "`Tree` contains itself", help "a variant
cannot hold its own enum by value; boxes come with generics"), generic enums
(step 6), automatic `T` to `T?` and `== none` (Q5).

## Files you own

New: `src/typeck/enums.rs` (declaration, layout, construction, patterns,
exhaustiveness), `tests/run/enum_*.drh`, `tests/fail/enum_*.drh`,
`tests/mir/enum_*.drh` + `.mir`, `site/examples/enum-*.drh`, `option-*.drh`,
`site/content/reference-enums.md`, `site/content/lesson-10-enums.md`.

Edit: `src/types.rs`, `src/ast.rs`, `src/parser.rs`, `src/hir.rs`,
`src/typeck.rs` (small hunks; call into `enums.rs`), `src/typeck/control.rs`
(`match_stmt` dispatches to a new function `enum_match` in `enums.rs` when the
value is an enum, so only one hunk), `src/mir/*` as in the overview table,
`src/codegen_*`, `src/cgen_*`, `src/diag/codes.rs` (E052x and E053x), site
pages in the checklist, `docs/PLAN.md` (step 5 progress, "A.1 done" notes),
`docs/SPEC.md` (only the range list in section 10).

Do not touch: `src/mir/borrowck/{solve,constraints,loans}.rs` (only
`regions.rs` and `conflicts.rs` for the `Downcast` rule), `src/typeck/units.rs`
beyond the child-walk arms the new HIR nodes need, `runtime/*`.

## LOOK FIRST

1. Every place that matches `Type::Struct` or calls `struct_id()` (about 60):
   decide for each whether an enum must be handled. Put the list in the first
   commit message body. `mir::is_copy`, `needs_drop`, `drops::holds_string`,
   `hir::Program::{size_align, owns}`, `analysis::mark_in_memory`,
   `codegen_common::in_memory`, `cgen_common::{c_type, struct_defs, work_type}`.
2. How `Aggregate` is lowered in both backends (field stores at offsets) and how
   glue functions are named, emitted and called (`drops::drop_glue_structs`,
   `copy_glue_structs`, the calls in `codegen_mir.rs`/`cgen_mir.rs`).
3. `TerminatorKind::Switch`: where it is printed/validated, and the two
   `unreachable!` arms.
4. `match_stmt` and `Coverage`: the integer machinery is intervals of an
   order-preserving key. Enum exhaustiveness is a different, simpler machinery
   (a set of variant indexes); do not bend `Coverage`.
5. `moves.rs`: how a path is built from `Field` projections, and the rule "a
   struct with `drop()` is never opened". An enum is **one** move path (no
   child paths): moving it moves all of it.
6. `lay_out`, `copy_structs`, `report_cycles`: copy the structure, do not
   share the code unless it falls out naturally.

## What to build (A.1: one commit per step, `cargo test` green after each)

### Step 1. Types, declaration, layout (no values yet)

- Parser: `enum Name { Variant, Variant(Type name, ...), }` at top level; a
  trailing comma is fine; payload fields need a type and a name; an empty
  enum is E0003 "an enum needs at least one variant". `Ast::Enum`.
- `Type::Enum`, `EnumDef`, `VariantDef` (overview 2.1). Declaration pass next to
  `declare_struct`: duplicate variants E0520; cycle check through payloads
  (E0210); payload types that are borrows/pointers E0901; units in payloads are
  fine (erased); layout (tag size, offsets, size, align), `owns`, `copy`
  (all payloads Copy, size <= 32 bytes, no `stays`).
- Name resolution: an enum name is a type. A name that is both a struct and an
  enum is E0202.
- Unit tests: layout of `Shape` (tag u8; payload at offset 8; size 24: check
  the numbers by hand), copy flag of `Msg { Text(string s) }`, E0520.

### Step 2. Values: construction and locals

- Expression forms: `Shape::Rect(2.0, 3.0)`, `Shape::Empty`, bare `Rect(2.0, 3.0)`
  / `Empty` when an expected type is known (declaration with a written type,
  assignment to a typed place, call argument, `return` in a function with a
  written return type, struct field value). Bare with no expected type: E0523.
  A bare name that is also a struct/function/local: E0524 at the declaration
  (Q7).
- Payload values convert like arguments (lossless silent, lossy W0102).
  Wrong count E0522; unknown variant E0521.
- HIR: `ExprKind::Variant { enum_id, variant, args }`. MIR: `Rvalue::Variant`,
  builder lowering (evaluate args into operands, then the rvalue), `print.rs`,
  `validate.rs` (operand types equal the variant's field types; V10 for
  `Downcast`).
- Backends: store the tag, store the fields at their offsets; `Variant` into a
  local, a field, a parameter; pass and return enums (hidden pointer, like
  structs); `copy e` of a Copy enum copies bits, of an owning one calls copy glue.
- Drop glue (switch on tag) and copy glue, per owning enum, in both backends,
  including enums inside structs and structs inside enums (`Held { Named(Person p) }`
  where `Person` owns a string: the payload is a struct, glue calls the struct's).
- Move analysis: an enum local is one path. `moves.rs` needs no new case; prove
  it with a test (move, reassign, conditional move + drop flag on an enum).
- Tests (run): construct and pass each variant kind, enums in struct fields,
  enum returned from a function, conditional move with drop flag, an enum
  holding a `string` payload created and dropped (leak check), assigned over
  (old value dropped).

### Step 3. `match` on an enum

- `case Variant`, `case Variant(Type name, ...)`, `case Variant(_)`, `case A, B`
  (several variants **without** bindings only; with bindings it is Q4: E0901),
  `case _`. Type names in bindings must match the payload type exactly (a
  lossless widening is not applied in patterns): E0526. A pattern naming a
  variant of another enum: E0525. Binding an owning payload: E0901 (Q3), unless
  the binding is `_`.
- The value is evaluated once into a hidden local **as a place**: `match (s)`
  must not move `s` (same hazard as S4R). A Copy enum may be copied; an
  owning one is matched in place by reference (the builder reads the tag and
  payload through the place).
- HIR: keep `Stmt::If` with tests of the form `Discriminant(value) == tag`
  and, new, `Stmt::Bind`-free binding: the case body starts with `Decl(x)` +
  `Assign(x, Payload{..})` for each binding (the HIR `Payload` expression of the
  overview 2.4, which lowers to `Downcast`+`Field` in MIR). For three or more
  variants the MIR builder turns the chain of discriminant tests into one
  `Switch` (a pass in `build.rs` over the HIR `If`: only when every branch is
  a discriminant test or `_`, otherwise keep the `If` chain). The `skip`
  rules are unchanged (it is the same `If`).
- Exhaustiveness: the set of variant indexes covered; none missing or `case _`
  makes it exhaustive; otherwise W0110 listing the missing variants by name
  (`warn_missing`'s style, "this `match` does not handle `Rect` and `Empty`").
  Unreachable: a variant already covered, or any case after `case _` (W0111).
- Backends: `Switch` in Cranelift (dense tags: `br_table`; else compare chain)
  and C (`switch`), no fallthrough, `default` for the (validated impossible)
  rest. Remove both `unreachable!` arms. `validate.rs`: every case value is a
  tag of the matched enum and `otherwise` exists.
- Borrowck: `Downcast` places overlap every place through the same local
  (conservative) in `conflicts.rs`; `regions.rs` asserts enums have no region
  slots.
- Tests (run): `match` over every shape of the SPEC example; bindings used in
  arithmetic; a `match` inside a loop with `break`/`continue`/`skip to next`;
  a function returning an enum matched at the call; `match` leaves `s` usable
  after; an owning-enum match with no bindings (leak-free).
  Tests (fail): `enum_unknown_variant` (E0521), `enum_bare` (E0523), `enum_dup`
  (E0520), `enum_arity` (E0522), `enum_pattern` (E0525, E0526), `enum_bind_string`
  (E0901), `enum_recursive` (E0210), `enum_name_clash` (E0524). Warn:
  `enum_missing` (W0110 with variant names), `enum_unreachable` (W0111).

### Step 4. Site, docs

Per the overview section 5 and the checklist below.

## What to build (A.2: optionals; one commit per step)

### Step 5. `T?` and `none`

- Parser: `Type?` in every type position (after the name, before units?
  `float64<m>?` reads "optional of `float64<m>`"); `Optional<T>` spelled
  out is E0901 until generics (say "write `T?`"). `none` as a value
  expression in a context with an optional expected type is `Variant(Optional,
  none)`; elsewhere E0530 (unless it is a pointer context, which keeps its
  current meaning: expected type `*T`).
- `uint32? x;` declares `x` as `none`. This is the one place where an optional
  differs from other types without a value: the declaration is `Assign(x,
  none)`. A struct field of optional type given no value in a `fill` is *not*
  defaulted (B.2 reports it missing).
- `Some(5)` as a bare variant (expected type `uint32?`) and `Optional::Some`
  is not written. A function `uint32? find(...)` returns `Some(5)` or `none`.
  An optional of a unit type, of a struct, of a `string` all work. `T??` is E0901.
- `T` to `T?` automatically and `x == none`: E0901 (Q5) with help "write
  `Some(5)`" / "use `match (x)`".
- Using an optional where its `T` is expected (arithmetic, argument): E0531.
- Optionals are enums of kind `Optional`: layout, glue, copy flag (Copy if `T`
  is Copy and the result is <= 32 bytes) fall out of A.1.

### Step 6. `match` and `maybe`

- `match (x) { case uint32 a { } case none { } }`: `case T name` is `Some`
  with a binding of `T` (an owning `T` binding is E0901, Q3), `case none` is
  `none`; exhaustive when both are there (or `_`); W0110 says "does not handle
  `none`" / "a value".
- `x maybe fallback`: for an optional `x` it is `match (x) { case T v => v;
  case none => fallback }` with `fallback` evaluated only when none; result
  type is `T` (the fallback converts to `T`). For a `string?` the Some payload
  is moved out when `x` is an rvalue (a call result): that is a move out of a
  temporary, allowed because the whole temporary is consumed, and tests prove
  no leak and no double free. For a **place** `x` of an owning optional,
  `x maybe fallback` would need to move out of `x`: E0901 (Q3) with help "use
  `take`" is not available; say "an optional that owns memory cannot be read
  with `maybe` from a variable yet". Numbers and Copy structs: fine from a
  place. `maybe` precedence/extent follows Q15's interim (a parenthesised or
  single-operand fallback). `maybe` on something not optional and not a failing
  call: E0532. (S5C extends the same syntax to failing calls; share the
  parser node `ExprKind::Maybe { value, fallback }` between A.2 and S5C: whoever
  lands first defines it, the other adds the typing arm.)
- Tests (run): `opt_basic` (declare, default none, assign Some, match both arms),
  `opt_maybe` (numbers, Copy struct, an rvalue `string?`), `opt_return` (a
  function returning an optional on two paths), `opt_struct_field`, `opt_loop`.
  Fail: `opt_none_plain` (E0530), `opt_as_plain` (E0531), `opt_maybe_plain`
  (E0532), `opt_auto_wrap` (E0901), `opt_eq_none` (E0901).

## Tests that apply to both

- `tests/mir/enum_*.drh` + `.mir`: the printed MIR of a `match` with three
  variants (a `Switch`), of a two-variant `If`, and of drop glue calls.
- `tests/leaks.rs` style: every owning-enum run test is leak-checked (the
  harness does it).
- `-O` and plain output identical (the harness does it).
- Property: the existing suites unchanged (`cargo test`).

## Site updates (same commit as each step; CLAUDE.md; only what builds)

- `site/content/design-enums.md`: A.1 removes the badges and moves "Declaring an
  enum", "Making a value", "Matching on variants" to the new
  `site/content/reference-enums.md` (examples `site/examples/enum-basic.drh`,
  `enum-match.drh`, `enum-owning.drh` with a `string` payload matched without
  bindings); A.2 moves "Optionals" there too (examples `option-basic.drh`,
  `option-maybe.drh`). What stays unbuilt on the design page keeps a marker:
  the Q3/Q4/Q5/Q6 forms (`<!-- status: tentative -->`). Update the lead
  paragraph and delete the page's `status:` front matter when nothing
  unbuilt remains outside tentative markers.
- `reference-types.md`: enum types, `T?`. `reference-control-flow.md`: `match`
  on enums, exhaustiveness warnings with the real W0110/W0111 output.
  `reference-ownership.md`: enums are Copy when all payloads are and <= 32
  bytes; one move path.
- `lesson-10-enums.md` (new, in `order.json` after lesson 9) with three
  exercises and `solution-10-N.drh` + `.out`.
- `coming-later.md`: progress row 5 text, remove the enum/optional bullets.
- `reference-diagnostics.md` and `errors.md` (`site/gen-errors.sh`) for E052x,
  E053x.
- `docs/PLAN.md` step 5: note A.1/A.2 done and list the interims taken.

## Done when

- `cargo test` green; enums and optionals work on both backends; the W0110 and
  W0111 outputs match the overview's wording.
- `grep -rn "unreachable!(\"the builder never emits" src` has no `Switch` hit.
- `grep -rn "none. as a value is not supported yet" src` is empty or only the
  pointer-context uses remain (say which).
- Report: the list of audited `struct_id()` uses, the layout table you chose
  (tag sizes), and any place the SPEC example `case Circle(float64 r)` could
  not be followed.

## Pitfalls

- A `match` that moves its scrutinee (hazard from S4R), or drops it twice.
- Tag type sizes in the C backend (`uint8_t`) vs Cranelift (`I8`): compare as
  the same unsigned width on both sides.
- The enum's alignment when a variant has a `uint128` payload (16-aligned);
  the cause word later shifts payload offsets: compute offsets from one
  function used by both backends and the glue.
- Drop glue for a payload struct that has its own `drop()` (user drop runs
  before the payload fields are dropped; check the struct glue order is reused).
- `Switch` on a tag in unreachable blocks after cleanup passes: `cleanup.rs`
  must understand `Switch` successors (it may only know `If`/`Goto`).
- Enum values as C structs containing a union: use a `char payload[N]` aligned
  array and cast at the field access, or a real `union`; either way alignment
  must match the Cranelift layout exactly. Test with a `uint128` payload.
- Two enums with the same variant name are fine; a variant named like a
  *field* or *method* is irrelevant.
- `Optional` instances are keyed by `T`; `uint32?` in two places is one enum
  (intern the `EnumDef`).

## Language questions found

Q3 to Q7 in `step5-overview.md`. If you find that `case T name` for an optional
`string?` is unavoidable in the tests you wrote, stop and report it as Q3 data.
