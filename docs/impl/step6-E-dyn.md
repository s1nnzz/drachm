# S6E: dynamic dispatch through `&Shape`

Implementer: **needs a strong model.** It adds a fat-pointer type to the
borrow checker's world and a new call kind to both backends. Needs S6B merged
(interfaces, conformance records).

Read first: `step6-overview.md` 2.2 (the `&Shape` row), 2.3 (dynamic dispatch,
required fields, conversion points), 2.5 (borrow checker), 4 (`dyn` benchmark),
6 (E0617 to E0619, E0640) and Q4, Q5; SPEC section 5 ("An interface name used as
a type means any struct with it, dispatched at runtime"); PLAN step 6 item 2
and the Decided list ("`&Shape` dynamic dispatch (vtables)", "interfaces with
required fields need the field at a known offset or an accessor in the vtable:
pick one and note it" — the overview picks offsets in the vtable);
`step6-B-interfaces.md`; the code: `src/types.rs` (`Ref`, `target`, `bare`),
`src/mir/borrowck/regions.rs` and `conflicts.rs` (how a `Ref` creates a region
and a loan), `src/mir/mod.rs` (`Callee`, `Rvalue`), `src/mir/validate.rs`,
`src/codegen_common.rs` (`in_memory`, `signature`), `src/codegen_mir.rs` and
`src/cgen_mir.rs` (the call arm, how an in-memory argument is passed),
`src/typeck.rs` (`borrow_value`, `coerce_borrow`, `args`, `value_method`),
`tests/borrows.rs`.

## Goal

1. `none report(&Shape s) { io::outln(s::area()::text()); }` called as
   `report(&sq)` with `Square sq` (and with `Circle`), working with any number
   of implementing structs in one program.
2. `&mut Shape` with `changes self` methods (`interface Counter`): `bump(&mut c)`
   works; `&Counter` calling `bump` is E0311 as for any `&`.
3. Required fields through a fat borrow: `s.x` on a `&Positioned` reads (and, on
   `&mut`, writes a `mut` field) via the vtable offset.
4. `static` members through a dynamic value are errors (E0619); an interface
   with `static` methods or `Self` in parameter/result position cannot be a
   type (E0618, Q5 interim A); interface by value is E0617 (B's); upcast E0901.
5. Borrow checking of `&Shape` identical to `&Circle`: the borrow of `sq`
   lasts as long as the `&Shape` is used; moving `sq` while borrowed is E0305.
6. Both backends identical; the `dyn` benchmark.

Out of scope (E0901): `&Shape` inside lists/maps/generic arguments (Q3: a
borrow as a type argument), storing `&Shape` in a struct field without `lives`
(the existing struct rules apply: it is a borrow field), upcasting, `Shape`
by value or boxed (Q4), generic methods in interfaces, `&Shape` for an
interface that has generic parameters (`&Iterable<uint32>` is allowed only
once S6G exists and is its own item).

## Files you own

New: `src/typeck/dynamic.rs` (conversion points, calls, field reads),
`src/mir/vtable.rs` (vtable construction and naming, shared by the backends),
`tests/run/dyn_*.drh`, `tests/fail/dyn_*.drh`, `tests/mir/dyn_*.drh` + `.mir`,
`tests/borrowck/dyn_*.drh` + `.bck` (the borrow checker's own format; see
`tests/borrows.rs`), `site/examples/dyn-*.drh`, the `dyn` case of
`bench/collections.*`.
Edit: `src/types.rs` (`Type::Dyn { id, name }`; `Type::Ref { to: Dyn }` is the
fat borrow; `bare`, `Display`, `target` unchanged), `src/hir.rs`
(`ExprKind::Unsize`, `ExprKind::DynCall`, `ExprKind::DynField`), `src/typeck.rs`
(small hunks in `coerce_borrow`/`borrow_value`/method lookup calling
`dynamic.rs`), `src/mir/{mod,build,validate,print,ty}.rs` (`Rvalue::Unsize`,
`Callee::Dyn`, V12), `src/mir/borrowck/{regions,conflicts}.rs` (treating the
`Dyn` target as opaque), `src/codegen_common.rs`, `src/codegen_mir.rs`,
`src/cgen_common.rs`, `src/cgen_mir.rs`, `src/diag/codes.rs` (E0618, E0619,
E0640), site pages, `docs/PLAN.md`.

## LOOK FIRST

1. How a `Ref` becomes a region slot and which code assumes the pointee has a
   size (`size_align(Type::Ref)` is 8 now; a fat borrow is 16 and in memory:
   every use of `in_memory`, `mark_in_memory`, `signature`, `c_type`).
2. How a method call's receiver is passed (a pointer to the struct): the
   vtable entry for `area` must be the **same function** the static call
   uses, taking `&Circle`; so the thunk-free layout needs only the data
   pointer as receiver. Check `Self` by-value parameters never appear (E0618).
3. How `drop_glue`/`copy_glue` treat `Ref`s (they do nothing): `&Shape` needs
   none.
4. The inferred self-mode of the **implementing** method versus the
   interface's declared `changes self`: the dynamic call site is checked
   against the **interface's** declaration (S6B guarantees the struct
   changes no more).

## What to build (one commit per step, `cargo test` green after each)

### Step 1. The type and the conversion

`Type::Dyn`, `&Shape`/`&mut Shape` in parameter, result, local and field
positions (fields follow the struct `lives` rules). The conversion `&S` to
`&Iface` where `S: Iface`: implicit at argument positions, `let`
declarations with a written `&Iface` type, returns, and assignments to a
`&Iface` place; `&mut S` to `&mut Iface` likewise; `&mut S` to `&Iface` follows
the existing `&m` rule (`show(&m)` must be written; owner answers step 3 #3).
No conversion = E0640 with the labels from the table. MIR `Rvalue::Unsize(
Operand /*the &S*/, StructId, IfaceId)` builds the 16-byte value `{data,
&vtable}`. Borrowck: a `Rvalue::Unsize` is a reborrow of the place the operand
borrowed (the loan and its region flow through unchanged); `regions.rs` gets
one new arm. Validation V12: `Deref` of a `Ref` to `Dyn` appears only in
`Callee::Dyn` receivers and `DynField` reads.

### Step 2. Vtables and dynamic calls

`mir/vtable.rs`: for each `(struct, interface)` that is converted anywhere,
a read-only static `drachm_vt_<mangle S>_<mangle I>`:
`[size, align, method_0, ..., method_n, field_offset_0, ..., field_offset_m]`
with methods in interface order (extended interfaces' members first), each
entry the concrete method's symbol. Backends emit it (Cranelift: data object
with function relocations; C: `static const void *const vt[] = {...}` with
casts, and `size_t` for offsets). `Callee::Dyn { iface, method }`: load the
slot from the vtable, call it with the data pointer as receiver and the other
arguments as for a static call; the result return convention is the callee's
(same as the concrete method's, since the signature is the same after `Self`
substitution). Both backends; `-O` C lets the compiler devirtualise only
when it can see through the static table: do not rely on it. Tests: two and
four implementing structs in one program, a method taking and returning
strings (owning results through the hidden return pointer), a method with
default conversions in its arguments.

### Step 3. Fields and modes

`DynField`: load the offset word, add to the data pointer, load a value of the
interface's declared field type (copy for Copy fields; owning fields are read
only under `&`/`copy`: E0622-style "cannot move out of a borrow", existing
E0310). A `mut` field write via `&mut Positioned`. `changes self` methods need
`&mut` receivers (E0311 otherwise, existing). E0618/E0619 checks. Tests:
`dyn_fields`, `dyn_counter_mut`, the E0311 and E0305 cases in
`tests/borrowck`, E0618, E0619, E0640, E0617 (S6B's) still firing.

### Step 4. Benchmark and cleanup

The `dyn` case: 100 million `area()` calls over two alternating structs.
Report the table. If the loop is more than 1.5x of Rust's `&dyn`, look at the
generated C (vtable load, indirect call) before concluding anything.

## Site updates (same commit as each step; only what builds)

- `reference-generics.md`: add "Interface as a type" (the `&Shape` section:
  what it means, the dispatch happens at run time, the table "Interface as a
  type or as a constraint") with examples `dyn-report.drh`, `dyn-mut.drh`,
  `dyn-fields.drh` and real E0640, E0617, E0618, E0311 output.
- `design-generics.md`: delete "Interface as a type" and the
  `&Shape` half of "Interfaces"; the Q4/Q5 variants (`heap Shape`, upcasts)
  stay as `<!-- status: tentative -->`.
- Lesson 15: no badge removal (S6Z). Add the "implement an interface"
  exercise's solution as `site/examples/solution-15-2.drh` + `.out` (it uses
  `&Shape`; it builds now) and verify the starter reports exactly the error a
  learner should see (E0640, report the text).
- `guide-generics.md`: the "Interface as a type, or as a constraint" section
  builds now; no badge removal (S6Z).
- `reference-diagnostics.md`, `errors.md`, `coming-later.md`, `docs/PLAN.md`.

## Done when

- `cargo test` green; the SPEC's `none report(&Shape s)` runs on both backends;
  dynamic and static dispatch of the same method give the same output.
- `bench/run.sh` reports the `dyn` row.
- Report: vtable layout as emitted, cost per call (instructions in C), every
  interim with file and line, any place the borrow checker's `Ref` assumptions
  (`size 8`) bit.

## Pitfalls

- A `&Shape` is 16 bytes and passed in memory: every place that treats `Ref` as
  one scalar register needs an `is_fat` check (`codegen_common::in_memory`
  first).
- Two conversions of the same struct to the same interface must share one
  vtable symbol (the C linker rejects duplicates, Cranelift declares once).
- A struct that implements an interface through a **generic instance**
  (`Box2<Circle>`): the vtable names the instance's methods.
- Alignment of the data pointer when the struct is over-aligned
  (`uint128` field): `align` in the vtable is for future boxed values; today
  nothing reads it, keep it in the layout so S-later does not break the ABI.
- Removing a struct's `with` after conversions exist must not leave a stale
  vtable: vtables are built from the HIR after typeck, never cached across
  compilations.

## Language questions found

Q4, Q5 in `step6-overview.md`. Report a mixed collection you needed (a list of
`&Shape`) as Q3/Q4 data.
