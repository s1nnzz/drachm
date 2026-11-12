# S6C: arrays, lists, indexing, `for` over them, string lists

C.1 (types, literals, indexing, glue, both backends, benchmarks): **needs a
strong model.** It adds type kinds and a MIR projection that every analysis
and both backends must agree on, and glue for containers; a mistake shows as a
leak, a double free or a one-backend-only miscompile. C.2 (methods, `for`,
`split`/`lines`/`chars`): standard. Needs S6A.1 (syntax, on-demand layout)
and S6R (runtime) merged.

Read first: `step6-overview.md` 2.2, 2.4, 2.5, 2.6, 4, 6 (E0620 to E0625,
E0630, E0631) and the questions Q3, Q8, Q10, Q11, Q12, Q20; SPEC "Collections";
`owner-answers.md` (Q7 `take`, Q12 the 32-byte rule, step 5 Q2 for text loops);
PLAN step 6 items 3 and 5; `step5-A-enums.md` (the shape of "add a type kind
to every layer": copy its LOOK FIRST discipline); `step6-R-runtime.md` (the
contract you call); the code: `src/types.rs`, `src/hir.rs` (`size_align`,
`owns`, `layout`, `ExprKind`, `HirProj`), `src/typeck.rs` (`Val`, `lvalue`,
`place_expr`, `init`, `coerce`, `copy_expr`, `take_expr`, `value_method`),
`src/typeck/control.rs` (`for_each`, `for_text`, `looped`),
`src/typeck/strings.rs` (the `split|lines|chars` arm), `src/mir/{mod,build,
validate,print,ty,moves,drops,analysis}.rs`, `src/mir/borrowck/{regions,
conflicts}.rs`, `src/codegen_common.rs` (`in_memory`), `src/codegen_mir.rs`,
`src/cgen_common.rs`, `src/cgen_mir.rs`, `runtime/strings.h`.

## Goal

1. (C.1) `uint32[?] xs = [1, 2, 3];` and `uint8[4] a = [1, 2, 3, 4];`
   declarations, literals, `[]`, locals/params/results/struct fields/inside
   enums and `Optional`, `xs[i]` read and write, `xs[i].field`,
   `&xs[i]`, `&mut xs[i]`, bounds checks that panic (exit 101, the text in
   S6R), `push`, `len`, `pop` (→ `T?`), moves and drops of lists (leak-free), `copy
   xs`, equality `==` of lists/arrays (element-wise, needs element `==`),
   `T[?]` as a generic type argument (`Stack<uint32[?]>`) and inside generic
   structs (`struct Stack<T> { T[?] items; none push(T item) { self.items::push(item); } }`
   from the SPEC runs).
2. (C.2) `for (uint32 n of xs)`, `for (&string s of names)`, `for (&mut T x of xs)`;
   methods `get(i)` (→ `T?`), `join(sep)` for `string[?]`, `each` is S6F;
   `"a,b,c"::split(",")`, `::lines()`, `::chars()` return real `string[?]`;
   `string[?]` equality; `list::join`.
3. Both backends identical, leak-free, zero regressions.

Out of scope (E0901): lists of borrows/holders (Q3), `take xs[i]` (Q11),
negative indexes (panic), slices and ranges as indexes (`xs[1..3]`), methods not
in the interim list of Q10 (E0201 with the note "not decided yet"), maps
(S6D), `Array<T, N>` with `N` a parameter (Q12), arrays of length 0 (E0624
reused? no: the message says "an array needs at least one value"), arrays above
1 MiB by value (error E0901, "large arrays are not supported yet"), iterating a
list twice at once (`for` over `xs` inside `for` over `xs` is fine: two shared
borrows), `Iterable` use of a list (Q19 interim).

## Files you own

New: `src/typeck/lists.rs` (types, literals, indexing, methods, `for`
arm helper), `tests/run/list_*.drh`, `tests/fail/list_*.drh`,
`tests/mir/list_*.drh` + `.mir`, `bench/collections.{drh,c,rs}` (cases `sum`,
`index`, `generic`), `site/examples/list-*.drh`, `site/content/reference-collections.md`.
Edit: `src/types.rs` (`Type::List`, `Type::Array`, interned), `src/ast.rs`
(`ExprKind::ListLit`, `ExprKind::Index`), `src/parser.rs` (literal `[a, b]` and
`xs[i]` in expressions), `src/hir.rs` (`HirProj::Index`, `ExprKind::ListNew`,
`ListFn`), `src/typeck.rs` (small hunks: type resolution of `Dim`, the call
sites that dispatch on receiver type), `src/typeck/control.rs` (**shared
hunk**: one new arm in `for_each` calling `lists::for_each_list`),
`src/typeck/strings.rs`, `src/mir/**` (as in the overview table), `src/mir/
borrowck/{regions,conflicts}.rs` (the `Index` overlap rule only), the four
backend files, `runtime/runtime.c` (nothing: `drachm_index_panic` is in the
strings crate), `src/diag/codes.rs` (E0620 to E0625, E0630, E0631), site
pages, `docs/PLAN.md`.

Do not touch: `src/mir/borrowck/{solve,constraints,loans}.rs`, `runtime/strings/**`
beyond a split/lines/chars function in C.2 (ask: if S6R did not provide
`drachm_str_split`, add `strings_split.rs` as a separate file in the crate and
list it in the report), `src/typeck/units.rs`.

## LOOK FIRST

1. Every place that matches `Type::Struct` / `Type::Enum` / `struct_id()` /
   `enum_id()` or tests "plain data vs in memory vs owning": write the audit
   list into the first commit message, deciding for each whether `List` and
   `Array` need handling (as S5A.1 did). Particular care: `mir::is_copy`,
   `needs_drop`, `drops::holds_string` (rename `needs_deep_copy`),
   `hir::Program::{size_align, owns}`, `analysis::mark_in_memory`,
   `codegen_common::in_memory`, `cgen_common::{c_type, work_type}`.
2. How `Aggregate` is lowered and how glue is named, emitted and called in both
   backends (`drops::drop_glue_types`, `copy_glue_types`, the `Drop` terminator
   arm); lists reuse that route with a function that calls the runtime.
3. How `Proj::Downcast` is excluded from move paths (`moves.rs`) and how
   `place_ty` / `place_ty_in` / `validate` (V10) treat it: `Index` follows the
   same pattern (V11).
4. How `Rvalue::Ref` of a field place is lowered in each backend; `&xs[i]`
   is a `Ref` of an `Index` place.
5. `lay_out`'s alignment handling for `uint128` and how enum payload
   alignment was solved in C (`_Alignas`): arrays inside structs reuse it.
6. `looped` and `hidden_named` in `control.rs`: the shape of the existing
   range-loop lowering you are extending.

## What to build (C.1: one commit per step, `cargo test` green after each)

### Step 1. Types and layout, no values

`Type::List { elem }` and `Type::Array { elem, len }` (interned `&'static
Type`, `Hash`/`Eq`, `Display` as `uint32[?]` / `uint8[4]`), resolution of
`Dim`s in `Globals::resolve` (the E0901 from S6A goes away), `size_align`,
`owns` (list: always; array: element), Copy rule for arrays (element Copy and
size <= 32 bytes; a list is never Copy), `scalar_size` none. In memory for the
ABI. Unit tests: sizes (list 24/8; `uint8[4]` 4/1; `uint64[3]` 24/8; an array
of a 24-byte struct), the Copy flag of `uint8[4]` and `uint64[5]`, a struct
containing a list is owning and not Copy, E0210 for `struct A { A[?] kids; }`
being **fine** (indirection through the list; add the test).

### Step 2. Literals and the runtime calls

HIR `ListNew`, `ListPush` as `Callee::Builtin` calls; MIR lowering of `[a, b]`
as `new` then pushes into a fresh local; arrays as `Aggregate`-like stores
at element offsets (a new `Rvalue::ArrayLit(Vec<Operand>)`). Element
conversion (left-side rules), E0623, E0624, E0625. `copy xs`. Both backends:
call `drachm_list_push` etc. with the element size/align; element `Move`
operands are stored by address (take the address of the operand's local; for a
constant, materialise it in a temporary). Glue: `drop_list_<m>`, `copy_list_<m>`,
array glue functions; the struct and enum glue call them for list/array
fields. Tests: build and drop lists of `uint32`, `string`, a struct with a
string, a list of lists (`uint32[?][?]`: inner glue called per element), arrays
of strings; `copy`; all leak-checked.

### Step 3. Indexing

`xs[i]` as `HirProj::Index(Box<Expr>)` on places; MIR `Proj::Index(Local)` with
the bounds check emitted by the builder (branch to a call of
`drachm_index_panic(i, len)` in a cold block); constant index into an array
checked at compile time (E0620); index of any integer type converted to
`uint64` (a signed negative value is an out-of-range panic with its true
value). Reads copy Copy elements; an owning element read is E0622 unless
under `&`, `copy`, a method call receiver or a field access chain. Writes:
`xs[i] = v;` drops the old element (existing assign logic: the place is
initialised), `xs[i].f = v;`. Mutability: writing through `xs[i]` needs the
list changeable (`mut`/`open`/`&mut`); the mutability analysis sees the base
local. Borrowck: `xs[i]` and `xs[j]` overlap; `&mut xs[0]` and `&xs[1]`
conflict (E0303/E0304 as for one place). Moves: `Index` places are never move
paths; the HIR builder refuses (E0622) before MIR. Both backends:
address = `ptr + i * elem_size` (list: load `ptr` from the header), array:
`base + i*size`. V11 validation. Tests: read/write/nested (`grid[i][j]`),
out-of-range panic exit code 101 and message on both backends, E0620, E0622,
E0303 with two `&mut xs[i]`, E0311 write through `&`.

### Step 4. Methods and equality

`push(T)` (needs a changeable list; `push` of an owning value moves it in),
`len()` (→ `uint64`; inlined load), `pop()` (→ `T?`), `get(i)` (→ `T?`; here
or C.2: here, it shares the optional machinery), and Q10's interim limits.
`==`/`!=` on lists and arrays: a synthesised function per type like `auto_eq`
(length then element-wise; needs element `==`, E0626-style error "`Shape`
cannot be compared with `==`" reuses E0527). The `bench/collections` cases
`sum`, `index`, `generic` and their C and Rust twins (see overview 4); report the
`bench/run.sh` numbers (baseline for later packages).

### Step 5. Generic interplay

`Stack<T>` from the SPEC (`T[?] items; none push(T item) { self.items::push(item); }`)
and a `Stack<string>`; lists inside enum payloads and `Optional` (`uint32[?]?`);
`T[?]` parameters in generic functions (`T first<T>(&T[?] xs)`, returns a
copy? For owning `T` it is E0622; the borrow form `&T first<T>(&T[?] xs)` works
with inferred `lives`). Fix whatever the instance machinery missed (report it).

## What to build (C.2)

1. `for_each_list` in `lists.rs`: hidden `len`/counter locals via
   `hidden_named`; bindings `T x` (Copy `T`: a copy), `&T x`, `&mut T x`; an
   owning `T x` is E0631 (Q8 interim); element type mismatch E0630. The loop
   keeps a shared (or `&mut`) borrow of the list for the whole loop (a `Ref`
   to the list place held in a hidden local, dropped at loop end) so `push`
   inside is E0302/E0303 with the existing wording. `break N`, `continue`,
   `skip` unchanged. Arrays the same with a constant bound. Closing the
   `for-chars` pattern: share code with `for_text` where natural.
2. `get`, `join(sep)` (`string[?]` → `string`), `first()`, `last()` only if Q10's
   interim list says so (it does not: leave them out).
3. `split(sep)`, `lines()`, `chars()` return `string[?]`: the typeck arm in
   `strings.rs` stops being E0901 and builds a Builtin call to new runtime
   functions that return a `drachm_list` of `drachm_str` (owned copies, each
   one counted). `chars()` is by grapheme like `len`. Tests: all three, an empty
   string, separators at the ends, a list of strings dropped without leaks.
4. List of strings `==`, `copy`, `join`.
5. Programs from the SPEC and the lessons: `names` / `scores` / `parts`.

## Tests

`tests/run/list_*.drh`: `list_basic`, `list_owning` (strings, leak-checked),
`list_nested`, `list_array`, `list_struct_field`, `list_in_enum`, `list_copy`,
`list_index_rw`, `list_oob` (code 101 + `.err`), `list_generic_stack`,
`list_for_copy`, `list_for_view`, `list_for_mut`, `list_split_lines_chars`,
`list_eq`. `tests/fail/`: one per code E0620 to E0625, E0630, E0631, the
borrow errors, E0901 for `take xs[i]`, for a list of borrows, and for a map.
`tests/mir/list_*.mir`: an index with its bounds check, a drop of a list of
strings (glue call), a `for` over a list.

## Site updates (same commit as each step; CLAUDE.md; only what builds)

- New `reference-collections.md` (in `order.json`): "Arrays and lists",
  "Indexing", "Changing a list", "Walking a collection", "Splitting strings",
  with examples `list-basic.drh`, `list-index.drh`, `list-for.drh`,
  `list-split.drh` and real output for E0620, E0622, E0630, E0631; the
  out-of-range panic shown as program output with its exit code.
- `design-collections.md`: delete the moved sections; **leave** the map parts,
  the `bounds` setting (Config, step 7: tentative marker if it has none) and
  the Iterable part marked. `reference-types.md`: `T[?]`, `T[N]`.
  `reference-control-flow.md`: `for` over a list. `reference-strings.md`:
  `split`, `lines`, `chars` return lists; replace "planned" sentences.
  `reference-ownership.md`: lists are move types; arrays are Copy under the
  rule.
- Lesson 16: do **not** remove the badge (S6Z). Verify and copy to
  `site/examples/` the blocks that now build: arrays, lists, indexing, `open
  string[?] xs = [];` + `xs::push("a")` (Q20 interim B: written `[]`), the
  `scores` loop (`solution-16-1.drh` + `.out`) and the `split`/`join` block.
- `coming-later.md` row 6 and the "lists" bullets; `reference-diagnostics.md`
  and `errors.md` via `site/gen-errors.sh`; `docs/PLAN.md` (row 4's "lists in
  step 6" note and row 2's "`split`/`lines`/`chars` later").

## Done when

- `cargo test` green; the SPEC list examples and the `Stack<T>` struct run on
  both backends, leak-free.
- `grep -rn "lists are not supported yet" src` is empty after C.2.
- `bench/run.sh` shows `sum`/`index`/`generic` against the targets in the
  overview; report the table and, if `sum` is over 1.3x, the C the `-O` build
  emitted for the loop.
- Report: the audit list, element access lowering in each backend (a few lines
  of emitted C), the interims taken (file and line), compile time.

## Pitfalls

- A list header is 24 bytes like a string: do not confuse the two in `mark_in_memory`.
- `Move` of an element into a push: the source local must not be dropped after
  (the existing move analysis handles it as long as the call is a normal call
  with a `Move` operand; test with a `string`).
- An index expression evaluated once: `xs[f()] += 1` calls `f` once.
- Two-phase borrows (owner Q4 of 2026-11-10): `xs::push(xs::len()::text())` is
  allowed, `xs::push(xs[0])` for an owning element is E0622 first, not a
  borrow error.
- A list of lists: dropping the outer calls the inner glue per element; copying
  the outer deep-copies.
- Zero-sized elements (a struct with no fields) must not divide by zero when
  computing capacity.
- C backend: arrays as struct members need `_Alignas`/`__attribute__((aligned))`
  matching the Cranelift layout; test with a `uint128` array.
- Do not let `Index` places reach `moves.rs` move paths: an unknown projection
  must conservatively mean "the whole parent", not panic.

## Language questions found

Q3, Q8, Q10, Q11, Q12, Q20 in `step6-overview.md`. If a SPEC or lesson snippet
cannot be run with the interims (for example one that needs `take xs[i]` or an
automatically empty `string[?] xs;`), report it as data for that question.
