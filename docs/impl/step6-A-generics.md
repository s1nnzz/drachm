# S6A: generics by monomorphisation

A.1 (type syntax, on-demand layout, generic functions): **needs a strong
model.** It restructures the phases of `typeck::check` and touches the parser,
the symbol scheme and every place that assumes all types exist before bodies.
A.2 (generic structs and enums, `Optional<T>`): **strong** (same machinery, new
layout cases). A.3 (`<unit U>`): standard.

Read first: `step6-overview.md` sections 1, 2.1, 2.5, 3, 5, 6 (codes E0601 to
E0609) and the questions Q1, Q2, Q3, Q7, Q15 (they define what you must refuse
or leave open); SPEC "Generics" and "Collections"; PLAN step 6 items 1 and 3;
`step4.5-units.md` (unit parameters); the code: `src/parser.rs` (`ty`,
`type_unit`, `unit_close`, `name_end`, `type_end`, `at_item_start`, `struct_decl`
~580, enum ~475, the method-call `targs` ~1412, `close_angle`), `src/ast.rs`
(`TypeName`, `Program`), `src/types.rs`, `src/typeck.rs` (`check` 259-410,
`Globals`, `resolve*`, `declare_struct`, `declare_fields`, `declare_methods`,
`report_cycles`, `mark_owning`, `copy_types`, `lay_out`, `check_body`,
`args`, `static_call`, `struct_method`, `value_method`), `src/typeck/enums.rs`
(`declare_enum`, `declare_variants`), `optionals.rs`, `src/mir/drops.rs`
(`glue_name`), `src/cgen_common.rs` (struct names), `src/codegen_common.rs`,
`tests/run.rs`.

## Goal

1. (A.1) `T max<T>(T a, T b)`, explicit `max<uint32>(5, 9)`, inference by
   unification, generic functions calling each other and themselves, with
   identical output on both backends; the type syntax `Name<A, B>`, `T[?]` and
   `T[N]` parses (the list/array types themselves are S6C: here they resolve
   to E0901 "lists are not supported yet" until S6C.1 lands, but the parser
   and `TypeName` carry them); `Vec<uint32>` no longer reads as a unit.
2. (A.2) `struct Stack<T> { ... }` with fields, methods, operators,
   constructors, `drop()`, `lives`, `fill`, `def` all per instance; `enum
   Tree<T>`-style generic enums (recursion only through lists/boxes: direct
   recursion is E0210 as today); `Optional<T>` spelled out is the same type as
   `T?`; generic struct inside generic struct (`Pair<Stack<uint32>, string>`).
3. (A.3) `float64<U> twice<unit U>(float64<U> x)` called with any unit,
   inferred; units in instance keys; erased before the backends as now.
4. Errors E0601 to E0605, E0608, E0609, W0601 as in the overview; constraints
   (`with`) are parsed here (the grammar `<T with A and B>`) and stored, but
   checked in S6B: until it lands a `with` in a type parameter list is E0901.

Out of scope (E0901 with the wording): borrows or holders as type arguments
(Q3: "a type argument cannot be a borrow yet"), methods with their own type
parameters (Q15), unit arguments written out or on structs (Q2), generic
interfaces `Iterable<T>` (S6B/G), `size`/value parameters (Q12), generic
`unsafe` or `asm` functions, default type arguments, partial explicit type
arguments (`f<A>()` where more parameters remain: supply all or none).

## Files you own

New: `src/typeck/generics.rs` (templates, instances, inference, the worklist),
`tests/run/gen_*.drh`, `tests/fail/gen_*.drh`, `tests/mir/gen_*.drh` + `.mir`,
`site/examples/gen-*.drh`, `site/content/reference-generics.md`.

Edit: `src/lexer.rs` (nothing expected: `with` is already a keyword),
`src/ast.rs`, `src/parser.rs`, `src/types.rs` (`mangle`, `Type::Param`
reserved for S6B), `src/typeck.rs` (phases, `Globals::resolve*` taking `tenv`,
small hunks calling `generics.rs`), `src/typeck/enums.rs` (instances of
enums), `src/typeck/optionals.rs` (`Optional<T>`), `src/typeck/units.rs` (A.3),
`src/mir/drops.rs` (`glue_name` uses `mangle`), `src/cgen_common.rs`,
`src/codegen_common.rs`, `src/diag/codes.rs` (E0601 to E0609, W0601), site
pages in the checklist, `docs/PLAN.md` (row 6 note).

Do not touch: `src/mir/borrowck/**`, `src/ownership/**`, `runtime/**`,
`src/typeck/control.rs` (beyond a compile fix), lesson 15/16/guide badges.

## LOOK FIRST

1. Every place that reads `g.struct_copy`, `g.enum_copy`, `owns`, `lay_out`'s
   output, or iterates `g.structs` expecting the full set: list them in the
   first commit message. Which of them can ask for a type that does not
   exist yet?
2. How `resolve_binding_type` / `resolve_unit_type` use `TypeName`; every
   construction site of `ast::TypeName { .. }` (the parser builds them in
   about ten places, including the synthesised ones in `type_optional`): add
   the new fields with `..Default`-style helpers rather than editing each.
3. Where symbols are built (`drachm_m_{len}{name}_{word}`, `drachm_eq_`,
   `drachm_f_`, `glue_name`, `cgen_common` line ~100): list them all.
4. How `check_body` gets its signature and `MethodCtx`; instances re-enter it
   with a `tenv`.
5. How `declare_optionals` receives its `T`s before names resolve: a generic
   instance created while resolving a signature may itself name `T?`
   (`Stack<string?>`); `optionals` must become callable on demand too.

## What to build (A.1, one commit per step, `cargo test` green after each)

### Step 1. Refactor only: layout and ownership as on-demand queries

No new feature. `owns`, `is_copy`, `size/align`, field offsets, the cycle check
(`report_cycles`) and `mark_owning` become memoised methods on `Globals`
(`layout_of`, `owns_of`, `copy_of`) with an in-progress mark (E0210 on a
re-entry). `check()` still calls them for all structs/enums before bodies and
fills `hir::Program` at the end. Every existing golden (`tests/run`, `fail`,
`mir`, `site/examples`) is byte-identical. Commit message lists the audit (LOOK
FIRST 1).

### Step 2. Type syntax and `TypeName`

- `ast::TypeName` gains `args: Vec<TypeName>` (generic arguments; an integer
  literal argument is stored as a `TypeName` with `name` the digits and a flag
  `is_number`), `dims: Vec<Dim>` with `Dim::List` (`[?]`) and `Dim::Array(
  u64, Span)` (`[N]`), applied left to right (`uint32[?][?]`), and `Optional`
  applies to the result as it does now.
- Parser rule for `<` after a type name (overview 2.1): number-type head keeps
  the unit reading; any other head reads generic arguments. `type_end`,
  `name_end`, `unit_part_end`, `at_item_start` learn the new tokens (so
  `Stack<uint32> s;`, `List<List<uint32>> x = ...;`, `&Stack<string> f(...)`
  and `Map<string, uint32[?]> m;` are recognised as declarations and
  function heads). Expressions: `max<uint32>(5, 9)`, `Stack<uint32>::zero()`,
  `new Stack<uint32> of (...)`: type arguments when the matching `>` is
  followed by `(` or `::`.
- Parser tests (in the `parser.rs` test module): `Vec<uint32>`,
  `float64<m/s>` still a unit, `Vec<float64<m>>` (nested unit), `A<B<C>>`
  with `>>` split, `bool <(Point o)` still an operator method, `a < b` and `a
  < b && c > d` still expressions, `uint32[?]`, `uint8[4]`, `string[?]?`,
  `&Stack<uint32>`, and the three errors: `Stack<>`, `Stack<uint32`, `T[0]`.
- Resolution: `Name<args>` of a non-template is E0602 ("`Point` takes no type
  arguments"); of a template with the wrong count E0602; a template name
  without arguments E0604; `[?]`/`[N]` resolve to E0901 until S6C.1.

### Step 3. Generic functions

- Parse `Type name<T, U with A and B, unit V>(params)`: type parameters after
  the function name, before `(`. `with` constraints are stored (E0901 for now);
  `unit` parameters stored (A.3). Duplicate or clashing names E0605. A type
  parameter shadows nothing: it is an error if it equals a struct, enum or
  unit name (E0605).
- `generics.rs`: `Globals::templates`, `instantiate_fn(name, targs, span) ->
  Option<FuncId>`, the `tenv`, the worklist drained at the end of `check()`.
  The instance's `Sig` is resolved under the `tenv`; its symbol is
  `drachm_f_<mangle(name<args>)>`; its HIR function is emitted at its new
  `FuncId`. `Globals::funcs_by_name` keeps only non-generic functions; a call
  `f(args)` / `f<A>(args)` finds a template first.
- Inference (overview 2.1): unify each parameter type against the argument's
  type structurally (`T`, `&T`, `T?`, `Stack<T>`, `T[?]`); the arguments are
  checked first without an expected type (`self.expr`), then coerced to the
  instance's parameter types. Constants: Q7 interim (B): an untyped constant
  adapts to a `T` already fixed by a typed argument, and a call of only
  constants takes the type an untyped constant has with no context today.
  E0601, E0603.
- Recursion: a generic function calling itself with the same `T` reuses the
  instance (the key exists before the body is checked); with a different type
  creates another; depth > 64 is E0608.
- Error positions inside an instance (overview 2.1 "Where errors show").
- W0601 "never used" for templates with no instance and no definition-time
  check (A.1: all of them; S6B limits it to unconstrained ones).

### Step 4. Run tests, benchmarks hook

Programs: `gen_max` (uint32, float64, string if `>` on strings is defined;
else comparison on numbers only), `gen_swap` (`none swap<T>(open T a, open
T b)` for a number and a string: moves), `gen_recursive` (generic
`power<T>`), `gen_two_params`, `gen_explicit`, `gen_nested_calls` (a generic
calling a generic with `T` and `T?`), `gen_instances_distinct` (same name,
different `T`, two symbols in the output), `gen_owning` (a generic function
that takes and returns a `string` by value: leak-free). Fail cases: E0601,
E0603, E0604 (name only), E0602, E0605, E0608 (polymorphic recursion), an
error inside an instance with the "instantiated here" note, E0901 for a
borrow argument and for explicit partial arguments. MIR golden: the printed
MIR of two instances (symbols differ, bodies equal). Add the `generic` case
of `bench/collections.drh` only when S6C.1 exists; here compare the C emitted
for `max<uint64>` with a hand-written one by eye and report.

## What to build (A.2)

1. Generic structs: `struct Stack<T> { ... }` parsed (remove the E0901 at
   ~580), stored as a template; `instantiate_struct`, which resolves fields,
   method signatures, constructors and `drop()` under the `tenv`, queues
   method bodies. Display name `Stack<uint32>`; symbols mangled; `lives`
   clauses and `stays` carry over; automatic `==` per instance. A field of type
   `Stack<T>` inside `Stack<T>` is E0210 (cycle through the instance key).
2. Generic enums (`enum Tree<T> { Leaf, Node(T value) }`), remove the E0901 at
   ~475; same machinery in `enums.rs`.
3. `Optional<T>`: resolves to the `T?` enum via `optional_ids`; `Optional<T>`
   in a generic body with `T` bound works at the instance. `declare_optionals`
   becomes an on-demand `optional_of(T)`.
4. Names in messages are the written spelling; hover text and `explain` stay
   as is. `Stack<uint32>` and `Stack<uint32>` written in two places are one
   type (key equality), and `Stack<uint32>` vs `Stack<sint32>` are different.
5. Tests: `gen_stack` (push/pop with a list is S6C, so use a struct with a
   fixed number of fields or `T?` slots until lists land: `struct Pair<A, B>`,
   `struct Cell<T> { T value; }`, `Cell<string>` with `drop()` order), `gen_enum`
   (`Maybe2<T>`), `gen_nested` (`Pair<Cell<uint32>, string>`), `gen_ctor` (a
   generic with a constructor and `new Cell<uint32> of (5)`), `gen_drop_order`,
   `gen_copy_rule` (`Cell<uint32>` is Copy, `Cell<string>` is not, 32-byte
   rule on an instance), `gen_opt_spelled` (`Optional<uint32>` equals `uint32?`).
   Fail: E0602 on structs, E0604, E0210 through an instance, E0901 borrow argument.

## What to build (A.3: units)

`<unit U>` after a function name declares a unit parameter; `float64<U>` in
the signature uses it. The unit environment gets a binding `U -> Unit`
during instantiation (`typeck/units.rs` `resolve_unit_type` consults
`tenv_units` first); inference unifies unit expressions of arguments: `U` is
bound by the first argument that has a unit (`float64<m/s>` binds `U = m/s`),
and every other `float64<U>` must have the same unit after the usual
dimension rules (E0603, reworded "`U` is `m` from the first argument, but `s`
from the second"). Plain numbers beside it follow the existing operator
rules in the body, which is checked per instance with a concrete unit. E0609
for `U` used where a type goes. Instance key includes the unit (interned
`&'static Unit`; identical units written differently must share a key: key on
the normalised unit). Tests: `unit_twice` (m, km, m/s, a derived unit),
`unit_mismatch` (E0603), `unit_dimensionless`, `unit_with_si`. Q2 stays: a
written unit argument is E0901.

## Site updates (same commit as each step; CLAUDE.md; only what builds)

- New `reference-generics.md` (in `order.json`): "Generic functions" (A.1),
  "Generic structs and enums" (A.2), "Optional<T>" (A.2), "Generic over units"
  (A.3, only if built); examples `gen-max.drh`, `gen-swap.drh`, `gen-pair.drh`,
  `gen-cell.drh`, `gen-twice-unit.drh` with `.out`; real compiler output for
  E0601, E0603, E0604.
- `design-generics.md`: delete the moved sections (leave links); keep the
  badges on Constraints, Interfaces, Self, `changes self`, Iterables; keep
  "Units and generics" tentative until A.3, then replace with a link when it
  builds (spelling Q2 stays tentative for written arguments).
- `reference-types.md`: `Optional<T>`, generic types, the `Name<..>` syntax and
  that a type name followed by `<` is read as a unit only after a number type.
- `lesson-15`, `guide-generics`: do **not** remove badges (S6Z). Check that
  the generic-function block of lesson 15 now compiles (copy it to
  `site/examples/gen-max.drh` and the min exercise solution to
  `solution-15-1.drh` + `.out`, using constants Q7 interim).
- `coming-later.md` row 6 text; `reference-diagnostics.md` and `errors.md`
  via `site/gen-errors.sh`.
- `docs/PLAN.md`: step 6 row "in progress: S6A.n done" and the interims taken.

## Done when

- `cargo test` green; generic functions, structs and enums run on both backends,
  leak-free, no regression in the existing 400-odd programs and the MIR goldens
  (A.1 step 1 commit proves the refactor is invisible).
- `grep -rn "generic structs are not supported yet\|generic enums are not supported yet" src` is empty after A.2.
- Report: the audit lists, the mangling table with an example of each
  special character, the instance depth limit, the errors-in-instance
  rendering (paste one), every interim taken with file and line, and the
  compile-time cost (`time cargo test` before/after).

## Pitfalls

- An instance created while a signature is being resolved (`Stack<Stack<T>>`
  in a field) before its own fields exist: allocate the id and name first,
  fill the fields after; cycles are then detected by the in-progress mark.
- `HashMap<Type, ...>` keys: two `Type`s with different interned names but the
  same id must not be both used as keys; key on `(id)` for structs/enums, not
  the displayed name.
- A body checked twice (definition pass in S6B and instance pass) must not
  push its diagnostics twice or allocate two sets of HIR locals into the same
  function.
- `>>`, `>=`, `>>=` after a type argument list: `Map<string, List<uint32>>=`
  is not a thing to support, but `Stack<uint32>>x` must error cleanly.
- `fails` functions (S5C): the instance's signature includes the failure list
  and the outcome type is derived after instantiation, never in the template.
- Unit erasure: instances with different `U` must erase to the same machine
  code but remain different keys (two symbols); that is fine.

## Language questions found

Q1, Q2, Q3, Q7, Q15 in `step6-overview.md`. If you find a case where the
interim for Q7 rejects a program in the existing site pages (other than lesson
15's constants), stop and report it as Q7 data.
