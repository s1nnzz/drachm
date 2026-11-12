# S5B: constructors, overloads, `new Type of (...)`, `fill`, `def`

Implementer: **standard model.** Mostly typeck and the parser, one MIR analysis
reused (per-field initialisation), no new backend work. B.2 (`fill`) has one
tricky part, the incomplete-value state, so read its LOOK FIRST twice. If the
incomplete-value analysis fights you for more than a day, stop and report: a
strong model should take that step.

Read first: `step5-overview.md` sections 1, 2.6, 3, 5, 6 and Q7 to Q10; SPEC
section 5 (constructors, declaration shorthand, `new ... of`, field names,
automatic constructor, operator overloading by right-hand type) and the "Field
by field" decisions in `owner-answers.md` ("Field-by-field filling and
defaults", Q6 on `open`, Q7 partial moves, Q14 units not relevant);
`wp5.md` (self mode, mutability), `step3-ownership.md` 2.3 to 2.6 and the
late-initialisation rules (`E0332`, `uint32 x;`); PLAN step 5 items 1, 2 and
the "Decided" paragraph; and the code: `src/parser.rs` (`member`, `struct_decl`,
`init`, `binding`), `src/ast.rs` (`Init`, `Method`), `src/typeck.rs`
(`declare_methods`, `construct`, `init`, `struct_method`, `struct_operator`,
`operator_call`, `static_call`, `check_body`, `assign_field`, `lvalue`),
`src/mir/{moves,mutability,build,check}.rs` (how `take` makes a field
uninitialised and how reads of it are rejected), `site/content/design-constructors.md`.

## Goal

B.1:
1. A struct may have constructors: methods named after the struct, no return
   type, overloaded by parameter types. They are used through the declaration
   shorthand (`Circle a = 5;`, `Person p = "Illy", 18;`) or `new Circle of (5)`
   anywhere an expression is allowed; `Circle(5)` is not a call.
2. Resolution: exact match first, then lossless conversions, then lossy with
   W0102; two equally good candidates are E0502; nothing fits E0501.
3. The automatic constructor (fields in order) exists when **no constructor is
   written** (Q8, interim). `Vector3 p = z: 3, x: 1, y: 2;` and
   `new Vector3 of (x: 1, y: 2, z: 3)` use field names with the automatic one.
4. A constructor body gives every field a value; reading `self` is not allowed
   until complete (Q9, E0507).

B.2:
5. `Player p;` (no value) then `fill p { p.name = "Sam"; p.hp = 5; }`;
   `fill partial p { ... }`; `def` templates and `defaults`; `fill p with
   defaults { ... }`.

Out of scope (E0901, with the interim in the overview): named values together
with a written constructor, one constructor calling another, `fill p with
Veteran { }` (named def, Q10), `fill` on parameters, fields or through
references, failing constructors (S5D).

## Files you own

New: `src/typeck/construct.rs` (constructor declaration, `new..of`, shorthand),
`src/typeck/overload.rs` (the shared overload resolver), `src/typeck/fill.rs`
(`fill`, `def`), `tests/run/ctor_*.drh`, `fill_*.drh`, `def_*.drh`,
`tests/fail/ctor_*.drh`, `fill_*.drh`, `def_*.drh`, `site/examples/ctor-*.drh`,
`fill-*.drh`, `def-*.drh`.

Edit: `src/parser.rs` (the constructor arm at `member` replaces the E0901;
`new`, `fill`, `def`, `struct ... defaults/default` forms), `src/ast.rs`,
`src/hir.rs` (`Func.kind`, `Stmt::Fill`, `DefDef`), `src/typeck.rs` (small
hunks calling into the new files; `operator_call` moves to `overload.rs`
unchanged in behaviour), `src/mir/build.rs` (lower `Fill`; constructor
bodies), `src/mir/{moves,mutability,check}.rs` (incomplete values), the E0332
help text in `src/diag/codes.rs`, codes E050x, site pages below, `docs/PLAN.md`.

Do not touch: `src/mir/borrowck/*`, backends, runtime, `src/typeck/control.rs`.

## LOOK FIRST

1. `operator_call` and `struct_operator`: overload by right-hand type already
   exists with the order exact / lossless / lossy. Extract, don't duplicate.
   Add a test that operator overloads still behave identically.
2. How a declaration without a value (`Player p;`) is represented in HIR/MIR,
   what `StorageLive` and the move analysis do with it, and how `take p.f`
   makes `p.f` uninitialised. A struct with no value is *entirely* uninitialised;
   `fill` initialises its fields one by one. The move/init analysis
   (`MaybeInit` per move path, `moves.rs`) has per-field paths: a `fill` block
   is a sequence of ordinary field assignments, and the completeness check at the
   `}` is "every field path of `p` is initialised on every path to here".
3. Where E0332 is produced today and what "until `p` is complete only `fill`
   touches it" must change: reads, borrows, plain field assignments and method
   calls on an incomplete `p` are E0507 (E0332 stays for a plain field
   assignment outside `fill`, with its help now pointing at `fill`).
4. The `self` of a method is a `&` / `&mut` parameter. A constructor's `self`
   is **not** a parameter: it is a local of the struct type that starts with no
   value, and the function returns it. Design the HIR so the builder sees:
   `Func { kind: Constructor { struct }, locals: [params..., self], ret: struct,
   body: [..., Return(Local(self))] }` with an implicit `fill partial`-like
   region around the body.
5. Visibility: constructors are methods, so `public:`/`private:` apply (a
   private constructor can only be used inside the struct's methods).
6. `Init::Fields` and `Init::Expr` in `typeck.rs::init`: the single-value form
   `Circle a = 5;` ("one value of another type builds a one-field struct") must
   now go through overload resolution when constructors exist.

## What to build (one commit per step, `cargo test` green after each)

### Step 1. Constructors and overloads (B.1)

- Parser: a method-like member whose name is the struct's name and that has no
  return type: `Circle(float64 radius) { ... }`. Parameter lists as for
  functions. `static`, return types, `fails` on a constructor: `fails` is S5D's;
  until then it is E0901 as today ("`fails` is not supported yet").
  `Circle(...)` called as a plain function inside expressions is **not** a
  construction: E0201 "no function `Circle`" with help "build a `Circle` with
  `new Circle of (...)`" (check the existing wording of the unknown-function
  error and add the help).
- Declaration pass: constructors are signatures in `StructInfo.ctors`. Two
  constructors with identical parameter types: E0503. Parameter names are
  free (they are not field names).
- `overload.rs`: `resolve(candidates, args) -> Resolved { index, conversions } |
  NoFit | Ambiguous(Vec<usize>)`. Ranking per argument: exact type, lossless
  conversion, lossy conversion; a candidate's rank is the worst of its
  arguments; the best rank wins; ties between candidates of the best rank are
  ambiguous unless one is strictly better on some argument and not worse on any
  (then it wins). A lossy pick emits W0102 per lossy argument as usual. Untyped
  constants adapt: `Circle(5)` with `(float64)` and `(uint32)` picks the
  integer one (exact for a whole constant defaults to `uint32`), `Circle(5.0)`
  picks float. Write the table of cases in a test file.
- `Circle a = 5;`, `Person p = "Illy", 18;`, `new Circle of (5)`,
  `new Person of ("Illy", 18)`, `new Vector3 of (x: 1, y: 2, z: 3)`,
  `new Empty of ()`. `new` is allowed anywhere an expression is; `of` and the
  parentheses are required (E0003 otherwise, with the SPEC example as help).
  `new` on a name that is not a struct is E0101. A struct with no fields and
  no constructor builds with `new Empty of ()`.
- The automatic constructor: when no constructor is written, today's
  `construct` behaviour (field order, or all by name). When one is written:
  the automatic one is gone (Q8); a call with values no written constructor
  accepts is E0501 listing the written ones. Named values with a written
  constructor: E0901.
- Constructor bodies: `self` is incomplete (B.1 uses a small special case of
  the state B.2 generalises). Rules: only `self.field = value;` assignments of
  the struct's own fields, once per immutable field (a `mut` field may be
  assigned again), reading `self`, passing `self`, calling methods on it: E0507.
  At every `return` and the end of the body every field must have a value,
  else E0504 (one error per missing field, at the closing `}` or the `return`).
  A constructor may `return;` early (bare) only when complete. Parameters and
  locals are ordinary.
  - Lowering: the builder declares `self` as a user local with no value, lowers
    the body as usual, and ends every path with `_0 = move self`. Because
    fields are initialised one by one, the **same per-field initialisation
    analysis** used for `take` decides completeness; E0504 is that analysis'
    "field path not definitely initialised at the `Return`" reported with the
    constructor wording. Do not add a second analysis.
  - Drops: if the constructor returns on a path where `self` is partly
    initialised (an early `return` after a `string` field was set but E0504
    would have fired, so it is not reachable in an accepted program) nothing
    extra is needed; a `string` field assigned in the body is dropped
    automatically if the body then panics (no unwinding exists), so no
    handling.
- Method-call syntax `Circle::unit()` (static methods) is unchanged.
- Tests (run): `ctor_basic` (SPEC's `Circle` with two overloads; calls with
  `5`, `5.0`, a `uint8`, a `float32`), `ctor_shorthand` (`Person p = "Illy",
  18;`, single value, named fields with the automatic constructor), `ctor_new`
  (`new .. of` in arguments, returns, nested, a struct holding a `string`:
  leak-free), `ctor_private`, `ctor_lossy` (`.warn`: W0102 on a `float64` to
  `uint32` pick), `ctor_static_factory` (`static Circle unit() { return new
  Circle of (1.0); }`), `ctor_operators` (operator overloading unchanged).
  Fail: `ctor_nofit` (E0501, with the real two-constructor rendering from the
  overview), `ctor_ambiguous` (E0502), `ctor_dup` (E0503), `ctor_missing_field`
  (E0504), `ctor_read_self` (E0507), `ctor_named_written` (E0901), `ctor_call`
  (E0201 with the `new` help).

### Step 2. `fill`, `fill partial`, `def`, `defaults` (B.2)

Grammar (all words are context words; see the overview section 1):

```cpp
struct Player defaults PlayerStart { string name; uint32 hp; string title; }
def PlayerStart of Player { hp = 100; title = "none"; }

Player p;                                   // no value
fill p with defaults { p.name = "Sam"; }    // starts from the default def
Player q;
fill q { q.name = "Ana"; q.hp = 5; q.title = ""; }   // every field
Player r;
fill partial r { r.name = "Bo"; }           // r stays incomplete
fill r { r.hp = 1; r.title = "t"; }         // completes it
```

Shorthand: `def Player { ... }` (a def with the struct's own name, no `of`) plus
`struct default Player { ... }` (default = the def of the same name).

- `def NAME of STRUCT { field = expr; ... }` is a top-level item, any order
  relative to the struct. A def sets any subset of the fields (E0201 for a
  non-field, E0509 for a field twice or two defs with one name; value types
  convert as assignment). Values are any expressions that do not mention
  fields (Q10 interim), evaluated each time the def is used, so a `string`
  literal gives a fresh not-owned literal and an expression gives a fresh value.
  A def is not a value or a type; it exists only to be applied.
- `struct NAME defaults DEF` / `struct default NAME`: the struct's default def.
  `fill ... with defaults` with no default def: E0510. The clause may be
  combined with `lives`/`stays` in either order like the other clauses.
- `fill X { ... }`:
  - `X` is a local variable that currently has **no value at all** (declared
    without one, or only partly filled by an earlier `fill partial`).
    Anything else is E0506 (it already has a value, or may have one).
  - Inside the block only `X.field = value;` assignments of `X`'s own fields
    (and ordinary statements: `if`, loops, locals, calls not mentioning `X`)
    are allowed. Reading `X` or a field, borrowing it, calling a method on it,
    passing it: E0507 (Q9 interim).
  - A field not given a value on **every** path to the closing `}` is E0505
    (one error per missing field), unless `partial`.
  - A field of an immutable (non-`mut`) field may be assigned once across the
    block and any later `fill`; assigning a field that already has a value is
    the usual immutable-field error (E0311-family message, reuse).
  - `with defaults` first assigns every field the def sets (in the def's
    order), then runs the block; the block may assign those fields again only
    if the field is `mut` (otherwise "already has a value": say in the help
    that the def set it). Fields the def does not set must be set by the block
    (E0505).
  - `fill partial X { }` leaves `X` incomplete: afterwards `X` is usable again
    only after a later `fill X { }` completes it. Completeness across blocks is
    the per-field analysis, so it works through branches and loops.
  - Plain `X.f = v;` outside a `fill` on an incomplete `X`: E0332 with help
    "fill it with `fill X { ... }`".
  - `fill` of `open`/`mut`/`take`-emptied values, parameters, fields, derefs:
    E0901.
- HIR: `Stmt::Fill { local, partial, defaults: Option<DefId>, body }`; defs are
  `hir::DefDef { struct_id, assigns: Vec<(field, Expr)> }`. The MIR builder
  lowers `Fill` as: the def's assignments, then the body statements (they are
  ordinary `Assign` statements to field places of the declared local). No new
  MIR statement. The init analysis does the rest. The borrow checker and the
  mutability pass need to know that field assignment to a not-yet-complete local
  is an *initialisation*, not a write through an immutable binding: reuse
  whatever makes `Player p; p = ...;` legal today (immutable binding assigned
  once).
- Drop elaboration: a partly filled `p` at scope end (after `fill partial` and
  never completed) drops only the initialised fields (the struct is already
  dropped field-by-field when fields were `take`n; reuse). A struct with a
  `drop()` that is incomplete at scope end is E0507 ("`p` has a `drop()`, so it
  must be complete before it goes out of scope").
- Tests (run): `fill_basic`, `fill_branch` (fields set in both arms of an `if`),
  `fill_partial_two` (two partial fills then a completing one), `fill_loop`
  (a field set inside a `while (true)` that always `break`s after setting it),
  `def_default`,
  `def_shorthand`, `def_mut_override` (a `mut` field set by the def and again in
  the block), `fill_owning` (strings, leak-free), `fill_in_function`.
  Fail: `fill_missing` (E0505), `fill_read` (E0507), `fill_twice_immutable`,
  `fill_on_value` (E0506), `fill_no_default` (E0510), `def_dup_field` (E0509),
  `def_unknown_field` (E0201), `fill_named_def` (E0901), `fill_partial_drop`
  (E0507), `fill_assign_outside` (E0332 with the new help).

### Step 3. Site and docs

See the checklist.

## Site updates (same commit as each step; only what builds)

- `design-constructors.md`: B.1 moves "Writing a constructor", "Building from the
  declared type", "new ... of", "Naming the fields" to
  `reference-structs.md` as a new section "Constructors" (examples
  `ctor-overloads.drh`, `ctor-new.drh`, `ctor-fields.drh` + `.out`, and the real
  E0501 / E0502 / E0504 output). B.2 moves "fill: one field at a time" and
  "def: named defaults" to a section "Filling and defaults" (examples
  `fill-basic.drh`, `fill-partial.drh`, `def-defaults.drh`). "A constructor that
  can fail" stays (S5D), "Choosing another def" and "Keys for maps" keep their
  tentative markers (Q10; map keys are step 6). Update the lead paragraph.
- `reference-structs.md`: "Building a value" gets a pointer to constructors.
- `lesson-6-structs.md`: if it says constructors are coming, link.
- `coming-later.md`, `reference-diagnostics.md`, `errors.md` (regenerate).
- `docs/PLAN.md`: progress notes and interims. `docs/SPEC.md`: nothing except if
  the code differs from section 5; report it.

## Done when

- `cargo test` green; all examples compile silently and print their `.out` on
  both builds.
- Existing programs using the field-order shorthand are unchanged.
- `grep -rn "constructors are not supported yet" src` is empty.
- Report: the overload ranking table you implemented, any program that now picks
  a different candidate than before, interims taken (named values with written
  constructors, delegation, reads of incomplete `self`), and whether the
  incomplete-value analysis needed a new pass (it should not).

## Pitfalls

- A one-value shorthand `Wrapper w = 5;` on a struct with a single `uint32`
  field and no constructor must keep working (automatic constructor path).
- `new Circle of (5)` must not be parsed as a call of a function named `new`
  (`new` is reserved).
- A parameter named like a field (`Circle(float64 r) { self.r = r; }`): the
  right side is the parameter, the left `self.r`; make sure name lookup is not
  confused.
- A constructor with a `string` parameter moved into a field: the parameter is
  moved (E0301 on later use); a borrowed parameter needs `copy`.
- Immutable fields assigned in a constructor are *initialisations*; the same
  field assigned in a method is E0311-family. Do not let constructor mode leak
  into other methods.
- Early `return` in a constructor inside a `fill`-like region: check
  completeness at the `return` too.
- The word `default` is also used by `unit`/config elsewhere? it is not a
  keyword: keep context rules narrow so `uint32 default = 3;` still parses.

## Language questions found

Q7 to Q10 in `step5-overview.md`.
