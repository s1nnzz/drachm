# WP9: `unsafe`, raw pointers `*T` and `*fixed T`, `address of`, unsafe functions

Implementer: **standard model.** The pass is a flat scan of the MIR, the type
work is mechanical, and the backends already do the hard part (WP7 step 1). Two
details need care: the `*fixed T` type (owner answer #9) and keeping every
`Type::Ptr` match exhaustive.

Read first: `step3-ownership.md` 3.9, 2.4, WP9 in 6, section 4 (E0316, E0317);
owner answers Q14, Q15, #8, #9 in `owner-answers.md`; `wp7.md` step 1 (what the
backends already do with `Ptr` and `RawRef`) and step 6 (the gate); and the code:
`src/types.rs`, `src/mir/{mod,ty,build,check,analysis}.rs`, `src/typeck.rs`
(`address_expr`, `none_compare`, `coerce`, `resolve`, `unsafe_disabled`),
`src/parser.rs` (type syntax), `src/flags.rs` (done: the file flag), `src/lib.rs`.
This brief wins over the spec.

State on master: the parser, the file flag (`//--!$unsafe: true`), E0317, `*T`
types, `address of` (for `mut`/`open` bindings), `none` as a null pointer in
`p == none`, `unsafe { }` blocks (`Statement.unsafe_ctx`) and `unsafe` functions
(`Body.unsafe_fn`) all exist. What does not: any check that an unsafe operation
is inside `unsafe` (E0316), `*fixed T`, calling-an-unsafe-function checks,
and a build (the `uses_borrows` gate, which WP7 turns into `uses_pointers`).

## Goal

1. `unsafe` is enforced: E0316 for every operation in the table of 3.9 outside an
   `unsafe { }` block (variants below); E0317 stays typeck's.
2. `*fixed T` exists: `address of` an immutable binding gives it (today E0901);
   writing through it is E0333.
3. Pointer programs build and run on both backends; leak-free.
4. `build_unchecked` and the `uses_pointers` gate are deleted (once WP7 has
   merged step 6).

## Parallelism with WP7 and WP8, and file ownership

WP9 runs **in parallel with WP7 and WP8 from the start.** It does not need the
borrow checker (raw pointers are untracked: WP7 C3 stops at them), only the
backends' support for `Ptr`/`RawRef`, which is WP7 step 1. The order that avoids
conflicts:

1. Do WP9 step 1 (the `Type::FixedPtr` variant) **first and merge it fast**;
   it is small and touches the exhaustive matches that WP7's backend work also
   edits. If WP7 step 1 merged first, add the `FixedPtr` arms next to the `Ptr`
   arms; if yours merged first, WP7 does. Either way: pure addition.
2. Steps 2 to 4 only need typeck, a new MIR pass and `check` programs, so they do
   not depend on WP7. Their run tests wait for WP7 step 1 (use `check`-only fail
   tests until then).
3. Step 5 (lift the gate) needs WP7 step 6 merged.

| Yours alone | Shared; edit only what is listed |
|---|---|
| `src/mir/unsafety.rs` (new) | `src/types.rs`, `src/mir/ty.rs`, `src/hir.rs` (`scalar_size`): the new variant's arms. Use `is_ptr()` in new code |
| `tests/fail/own_unsafe_*.drh`, `tests/run/own_unsafe_*.drh` | `src/typeck.rs`: `resolve` (type), `address_expr`, `coerce`/`none_compare` (conversion errors), the E0316-adjacent code. Not the borrow code of WP7 |
| `site/content/reference-unsafe.md` (new) | `src/parser.rs`: the type grammar for `*fixed T` only |
| E0316, E0333 in `src/diag/codes.rs` (your own block) | `src/mir/check.rs`: one call line, after WP7's/WP8's |
| | `src/codegen_mir.rs`, `src/cgen_mir.rs`: nothing expected; the null constant of pointer type, `==` on pointers and `DeepCopy` of a pointer, if WP7 step 1 did not cover them |
| | `src/lib.rs`, `tests/borrows.rs`, `tests/backends.rs`: step 5 only |
| | `site/content/coming-later.md` (one bullet), `order.json`, `reference-diagnostics.md`, `errors.md` (entries) |

## What to build

One commit per step.

### Step 1. `Type::FixedPtr`

- `types.rs`: `Type::FixedPtr { to: &'static Type }` next to `Ptr`, with
  `Type::fixed_ptr(to)`, `Display` as `*fixed T`, and every helper that handles
  `Ptr` handles it (`bare`, `mentions_unit`, `target`). `is_ptr()` returns true
  for both; add `is_fixed_ptr()`. `Copy`-ness: neither is Copy (Q15: pointers
  move; `mir::is_copy` says false for both).
- `grep -n "Type::Ptr" src` and give every site a `FixedPtr` arm (scalar size
  8/align 8, Cranelift pointer type, C `T *` with `const T *` not needed).
- Parser: after `*`, the context word `fixed` followed by a type name is the
  fixed form (`*fixed uint32`); `*fixed` alone is a pointer to a struct that
  happens to be called `fixed`. `fixed` stays an ordinary identifier elsewhere.
  The type-modifier list (`TypeMod`) gains `FixedPtr`.
- Unit tests: parse/print/resolve `*uint32`, `*fixed uint32`, `*fixed Player`;
  `E0317` when the file flag is missing, for both.

### Step 2. `address of` and typeck

- `address_expr` (typeck): the place's mutability decides the kind. A `mut` or
  `open` binding, `self` in a method that changes it, a place behind `&mut`,
  or a `mut` field of an owned immutable value (Q5/Q-MF: it can be changed) gives
  `*T`. An immutable binding, a place behind `&`, or a non-`mut` field of an
  immutable owned value gives `*fixed T` (answer #9). Delete the E0901 for it.
  The error E0327 for non-places stays.
- Writing through a `*fixed T` is E0333 (below). Where: typeck's `lvalue` sees
  `Deref` of a `FixedPtr`; set `through_fixed` on the place and report on assign,
  compound assign, `++`, and `&mut`/`address of` for a write. Reading is fine.
  (`address of *p` with `p: *fixed T` gives `*fixed T`.)
- `none` converts to either pointer kind (`coerce`), `p == none` works for both.
- **Conversions between `*T` and `*fixed T` are not designed** (language
  question 2): any assignment, argument or comparison that would convert in
  either direction is **E0901** "converting between `*T` and `*fixed T` is not
  supported yet". Do not accept `*T` where `*fixed T` is wanted either.
- A pointer to a pointer (`**T`) was already handled how? Check: if the parser or
  `resolve` accepts it, keep that behaviour; if not, leave it E0901.

### Step 3. The `unsafety` pass

`src/mir/unsafety.rs`, `pub fn run(hir, body, diags)`; called from `check::run`
for every body of a user function (skip the compiler's own, span 0).
Walk statements and terminators with `s.unsafe_ctx`:

| Found (outside `unsafe_ctx`) | Report |
|---|---|
| `Rvalue::RawRef` | E0316 "making" variant |
| a place containing `Deref` of a pointer type, in an operand (read), as the destination (write), or under a `Ref` | E0316 "reading" / "writing" |
| `Rvalue::DeepCopy` of a pointer place | E0316 "copying" |
| `Binary(Eq/Ne, ..)` with a pointer operand | E0316 "comparing" |
| a pointer-typed `Constant::Int` (the null `none`) | E0316 "null" |
| `Callee::Func(f)` where `hir.funcs[f].is_unsafe` | E0316 "calling" |
| any other pointer-typed operand: a move, copy, argument, return, declaration | **E0901** "moving or passing a raw pointer outside `unsafe` is not decided yet" (#8, language question 1) |
| any of the above inside an `unsafe` function body but outside a block | **E0901** "whether the body of an `unsafe` function counts as unsafe is not decided yet" (#8, language question 1) |

The last two rows must not fire when the operand is part of an operation already
reported in the same statement (one diagnostic per statement). Also E0333: a
`Write`/`&mut` of a place with a `Deref` of a `FixedPtr` is reported by this pass
(typeck only marks it) if typeck cannot see it (through a method receiver:
`p::change()` where `p: *fixed T` is not a thing: methods are called on values;
leave that case out).

Declarations inside an `unsafe { }` block are local to it. The `help:` for
E0316 must say so:

```text
error[E0316]: reading through a raw pointer needs an `unsafe { }` block
  --> main.drh:2:32
   |
 2 | none read(*uint32 p) { uint32 v = *p; }
   |                                   ^^ the pointer is read here
   |
 help: do it inside a block: declare `v` first, then `mut uint32 v = 0; unsafe { v = *p; }`
```

Variants (message / primary label / help), all E0316:

| Operation | Message | Primary label | Help |
|---|---|---|---|
| `address of n` | making a raw pointer with `address of` needs an `unsafe { }` block | the pointer is made here | write `unsafe { p = address of n; }` after declaring `p` |
| `*p` read | reading through a raw pointer needs an `unsafe { }` block | the pointer is read here | declare the result first, then `unsafe { v = *p; }` |
| `*p = v` | writing through a raw pointer needs an `unsafe { }` block | the pointer is written here | `unsafe { *p = v; }` |
| `copy p` | copying a raw pointer needs an `unsafe { }` block | the pointer is copied here | `unsafe { q = copy p; }` |
| `p == none`, `p == q` | comparing a raw pointer needs an `unsafe { }` block | the pointer is compared here | `unsafe { gone = p == none; }` |
| `none` as a pointer | making a null pointer needs an `unsafe { }` block | the null pointer is made here | `unsafe { p = none; }` |
| `one()` | calling `one`, an `unsafe` function, needs an `unsafe { }` block | `one` is `unsafe` | `unsafe { x = one(); }`; secondary label at the declaration: "`one` is declared `unsafe` here" |

E0333 (new), E0302 layout:

```text
error[E0333]: cannot write through `p`: it is a `*fixed uint32`, which can only be read
  --> main.drh:5:9
   |
 3 |     uint32 n = 5;
   |     - `n` is not `mut`, so `address of n` gives a read-only pointer
 4 |     unsafe {
 5 |         *p = 6;
   |         ^^ written through a `*fixed` pointer here
   |
 help: declare `n` as `mut uint32 n`, so that `address of n` gives a `*uint32`
```

Add E0316 and E0333 to `codes.rs` (explain text with a small example) and to the
review tables in `step3-ownership.md` section 4 (done by this brief's author).

### Step 4. Tests (check-only and run)

Fail tests (`tests/fail`, `.err` lines): `own_unsafe_deref`, `own_unsafe_call`
(spec, verbatim), and `own_unsafe_addr`, `own_unsafe_write`, `own_unsafe_copy`,
`own_unsafe_compare`, `own_unsafe_null` (one E0316 variant each),
`own_unsafe_fixed_write` (E0333), `own_unsafe_ptr_moved` (a pointer used after it
moved: E0301, proves Q15 holds for pointers), `own_unsafe_convert` (E0901),
`own_unsafe_pass` (E0901, moving a pointer argument outside `unsafe`),
`own_unsafe_fn_body` (E0901), `own_unsafe_no_flag` (exists).

Run tests (`tests/run`, need WP7 step 1): `own_unsafe_ptr` (spec, verbatim, `42`),
and
- `own_unsafe_fixed`: `uint32 n = 7; unsafe { *fixed uint32 p = address of n; ... read through p ... }` prints 7
- `own_unsafe_field`: pointer to a struct field, written through, then the struct printed
- `own_unsafe_null`: `*uint32 p = none; unsafe { ... p == none ... }`
- `own_unsafe_struct_ptr`: a struct with a `*uint32` field set from `address of`, read in a method (the method stays `&self`: owner's note, writing through a pointer field does not change `self`)
- `own_unsafe_swap`: an `unsafe` function `swap(*uint32 a, *uint32 b)` called twice from `main`
- `own_unsafe_string`: pointer to a `string`, `copy`'d through it (`unsafe { string s2 = copy *p; }`)

Every run test passes both backends and the leak check. Pointers are not dropped
or tracked: a test must not let a pointer outlive its target.

### Step 5. Lift the gate (needs WP7 step 6)

- Delete the `uses_pointers` refusal in `lib::build`, `build_unchecked` and
  `unsupported()`; make `tests/borrows.rs` build its whole source through
  `drachm::build`; delete `tests/backends.rs`'s `build_unchecked` test or
  turn it into plain `tests/run` cases.
- `grep -rn "build_unchecked\|uses_pointers\|uses_borrows" src tests` is empty.

## Site updates (CLAUDE.md; only what the compiler accepts)

- `site/content/reference-unsafe.md` (new, `order.json` after `reference-ownership`
  or `reference-lives`): the file flag (check `reference-cli.md` first; if it does
  not describe flags, describe `//--!$unsafe: true` here, E0317), `*T` and
  `*fixed T`, `address of`, `none`, `unsafe { }`, `unsafe` functions, which
  operations need the block (the table above, in words), the variants of E0316 and
  E0333 with real output, and a worked example. What moving or passing a pointer
  needs is **not** documented (undecided).
- Examples `site/examples/unsafe-pointer.drh`, `unsafe-fixed.drh`,
  `unsafe-fn.drh` + `.out` (first line `//--!$unsafe: true`; compile silent,
  leak-free, both backends).
- `site/content/coming-later.md`: remove "`unsafe` with raw pointers"; add "Assembly
  functions (`asm`) and importing C headers" under Later if not there (the
  owner's request says asm is scheduled after header import).
- `site/content/reference-types.md`: the pointer types in the types table.
- `reference-diagnostics.md`, `errors.md`: new codes; rerun `site/gen-errors.sh`.

## Done when

`cargo test` green; the spec's WP9 acceptance programs behave as listed; pointer
programs build with `drachm build` and `-O`; the gate and `build_unchecked` are
gone; E0901 messages exist for the three undecided points and the report lists
them.

## Pitfalls

- `unsafe { }` is a scope: a variable declared inside is not visible after it. The
  help text and the examples must show "declare first, assign inside".
- `address of` a scalar local forces it to memory (`mark_in_memory`); with a loop
  around the block make sure the slot is not reused.
- The null constant has pointer type: the backends' constant emission must accept
  `Constant::Int(0, Ptr)` (Cranelift `iconst` of pointer width; C `((T *)0)`).
- Pointer equality is address equality; never compare through the pointer.
- `copy p` on a pointer is a bit copy; `DeepCopy` of a *struct* with a pointer
  field also copies bits (WP6 already does).
- A pointer to a local that has been dropped or gone out of scope is the user's
  problem; the compiler must not try to track it (R9).
- Do not add a `*mut`: there is one writable pointer kind (Q14).

## Language questions found

1. **(#8) What else needs `unsafe`?** Q14 lists making, copying, comparing and
   dereferencing. Moving or passing a pointer around, and the body of a function
   declared `unsafe`, are not decided. Until then E0901 (see step 3).
   - A. Only the listed four need the block; passing, returning and storing a pointer is free.
     An `unsafe` function's body is not automatically unsafe: it still writes `unsafe { }`.
   - B. Everything that touches a pointer value needs the block, moving included; an
     `unsafe` function's body is one big unsafe block.
   - C. Original: a function can take the pointer **and** declare what it does with it:
     `unsafe` is only for the four operations, and a function that takes a pointer parameter
     must itself be marked `unsafe` so callers know.
   ```cpp
   unsafe none bump(*uint32 p) { *p = *p + 1; }   // B: no inner block needed
   unsafe none bump(*uint32 p) { unsafe { *p = *p + 1; } }   // A
   none bump(*uint32 p) { }   // C: error, takes a pointer without saying `unsafe`
   ```
2. **Converting between `*T` and `*fixed T`.** Giving up the right to write
   (`*T` to `*fixed T`) is harmless; the reverse breaks the promise of the
   immutable binding. E0901 until decided.
   - A. `*T` converts to `*fixed T` silently; the reverse never.
   - B. Both ways need an explicit word inside `unsafe`: `p::freeze()` and
     `p::thaw()`.
   - C. Original: no conversion at all; `address of` is the only way to make
     either kind, and a function that wants to read takes `*fixed T`, callers
     pass `address of` an immutable copy.
   ```cpp
   mut uint32 n = 1;
   unsafe {
       *uint32 w = address of n;
       *fixed uint32 r = w;            // A: fine   B: error   C: error
       *fixed uint32 r2 = w::freeze(); // B
   }
   ```
