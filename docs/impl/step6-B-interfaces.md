# S6B: interfaces, `with`, constraints (static dispatch)

Implementer: **needs a strong model.** It adds a declaration kind, a
conformance checker that depends on the ownership inference, and a second
checking mode for generic bodies (the definition pass). Needs S6A.2 merged.

Read first: `step6-overview.md` sections 2.3 (all of it), 3, 5, 6 (E0606,
E0607, E0610 to E0619) and the questions Q1, Q4, Q5, Q6; SPEC sections 5
("interface", `with`, `Self`, `changes self`, required fields, `extends`) and
"Generics"; `owner-answers.md` Q16 (`changes self` only in interfaces, struct
methods stay inferred), Q8 and Q-MF (methods that write only `mut` fields: a
third inferred kind; decide how it compares, below); PLAN step 6 item 2;
`step6-A-generics.md` (the instance machinery and `tenv`); the code:
`src/ownership/self_mode.rs`, `src/mir/mutability.rs` (how Shared / Mut /
"only mut fields" are enforced), `src/typeck.rs` (`declare_methods`,
`MethodInfo`, `struct_method`, `value_method`, `check_body`), `src/parser.rs`
(`struct_decl`, method parsing, `with` handling if any), `src/lib.rs` (the
order of passes after typeck), `site/content/design-generics.md`.

## Goal

1. `interface Shape { float64 area(); static Shape unit(); float64 x; }`
   declarations: method signatures, `static` signatures, required fields,
   `extends` (one or several), `Self` in parameter and result position, and
   `changes self` after the parameters of a method (only there: E0613
   elsewhere).
2. `struct Circle with Shape and Drawable`: conformance checked after all
   signatures exist: E0610 (missing), E0611 (signature differs), E0612 (the
   struct method changes `self` but the interface does not say `changes
   self`), E0614 (required field missing or of another type or private).
3. `<T with Shape>` and `<T with A and B>` on functions and structs:
   instance check (E0607) and the definition check of 2.3 (E0606): only the
   interface's methods and fields are usable on a `T`; `Self` becomes `T`;
   `static` methods are called as `T::zero()`.
4. Static dispatch: instances call concrete methods directly (nothing in the
   MIR changes).
5. `Zero`-style statics: `T::zero()` in a template body works at the
   instance when `T` is a struct with the interface.

Out of scope (E0901): built-in types implementing an interface (Q6), interface
as a value type (E0617, Q4), `&Shape` (S6E: until it lands `&Shape` is
E0901 "dynamic dispatch is not supported yet"), generic interfaces
(`Iterable<T>` arrives in S6G, which needs this package's machinery for
`interface X<T>`: implement generic interface declarations **here** as
templates but gate their use: `with Iterable<uint32>` is accepted by the
parser and the checker for user-declared generic interfaces; S6G adds the
built-in one), generic methods in interfaces (Q15), default method bodies
(never: SPEC), interface fields written through `&Positioned` (S6E).

## Files you own

New: `src/typeck/interfaces.rs` (declaration, conformance, bounds, definition
pass), `src/ownership/conform.rs` (the `changes self` comparison on HIR),
`tests/run/iface_*.drh`, `tests/fail/iface_*.drh`, `site/examples/iface-*.drh`.
Edit: `src/ast.rs` (`Interface`, `IMethod`, `Struct.with: Vec<TypeName>`,
`GenericParam.bounds`), `src/parser.rs` (the `interface` item; `with` in a
struct head; `changes self`; bounds in `<...>`; `Self` as a type name; `changes`
is a **context word**: special only after the `)` of an interface method
declaration, ordinary everywhere else, like `unit`: add the parser test
`uint32 changes = 3;`), `src/types.rs` (`Type::Param`, `Type::SelfTy` used only
inside the checker's interface signatures; `Display`, `bare`, hashing),
`src/typeck.rs` (small hunks: `Globals::interfaces`, struct head `with`,
bounds in `resolve`), `src/typeck/generics.rs` (bounds check at instantiation,
definition pass entry), `src/lib.rs` (call `ownership::conform` after
`self_mode::infer`), `src/diag/codes.rs` (E0606 to E0619 minus the ones S6E
owns: E0618 and E0619 are S6E's; E0617 is yours), site pages.

Do not touch: `src/mir/**` (static dispatch needs nothing), backends,
`runtime/**`.

## LOOK FIRST

1. How `self_mode::infer` is used by `lib.rs`/`mutability.rs` and how Q-MF's
   "changes only `mut` fields" kind is represented; decide the comparison:
   interface says nothing => the struct method must be `Shared`; says `changes
   self` => anything goes. A method that writes only `mut` fields is
   "changes only mut fields" (callable on an owned immutable value): for
   conformance it counts as **changing** (the interface cannot promise
   less); note this in the report.
2. How a method call on `self`/a value finds the `MethodInfo` (`g.method`), and
   where a `Type::Param` would have to be intercepted: `value_method`,
   `field_of`, `static_call`, `binary_vals`/`struct_operator`, `coerce`, `copy_expr`.
3. How `E0213` and visibility (`check_method_visible`) treat private methods:
   an interface method is public; a struct method implementing it must be
   public (before any `private:` label), E0610 with a note if it is private.
4. How diagnostics inside the definition pass are suppressed for anything that
   depends on an unconstrained parameter (the Q1 interim).

## What to build (one commit per step, `cargo test` green after each)

### Step 1. Declarations and `with` (no generics yet)

Parse and declare interfaces (names live in a namespace shared with structs and
enums: a clash is E0202). `extends` resolves after all names exist; cycles
E0615. The member set of an interface is its own plus its parents' (parents
first); equal-name members from two parents must have equal signatures (E0615).
Structs: `with A and B` (the list; `with` before `lives`/`defaults` clauses in
any order, Q18 of the owner answers). Conformance pass (E0610, E0611, E0614);
`Self` substitution; `static` equality; operator methods may be interface
members. The check of E0612 is step 2. Tests: a struct implementing two
interfaces, `extends` two levels, a required field, `Self` returning a
constructor-style static, each error.

### Step 2. `changes self`

Parse `changes self` after the parameter list in interface methods; E0613
elsewhere (a struct method, a free function). `ownership/conform.rs`:
for each conformance record, compare the method's inferred mode with the
interface method; E0612 with the `MutReason` span as the primary label (use
`self_mode` reasons: `Writes`, `LendsMut`, `Calls`). Tests: the Counter example
of the lesson (`bump` changes, `get` reads), a struct method that writes in a
callee (`Calls` reason), a method that writes only a `mut` field, the
negative and the positive.

### Step 3. Constraints at instances

`<T with Shape and Drawable>` on functions and generic structs (the parser
stores `bounds`; S6A already accepts the grammar). At instantiation, each
bound must have a conformance record for the type argument (E0607 with the
two labels). A struct type argument that is itself an instance
(`Stack<uint32>` with `Shape`) works if its conformance holds. Q6 interim: a
number, `string`, list, map, `Optional` or any non-struct as a type argument
for a constrained parameter is E0607 with the note "only structs can
implement interfaces so far". Tests: `total<T with Shape>` over a one-field
generic wrapper (lists arrive in S6C), `T::zero()` static call, two bounds.

### Step 4. The definition pass

For a template with at least one bound: after the declaration phase, check
its body once with the bounded parameters bound to `Type::Param` (2.3):
method lookup on `Param` goes to its bounds (members of extended interfaces
included, `changes self` modes honoured: a `changes self` method on a
`&T` receiver is E0311 as for any `&` borrow), field access to required
fields, operators E0606 unless an interface declares them, everything else
as for a normal type. A call to another generic with bounds checks that the
bounds are implied (a `T with Shape` can be passed where `U with Shape`
is wanted, not where `with Drawable` is). The HIR of this pass is discarded;
its diagnostics are kept, with the label "in the definition of `total`".
Diagnostics that depend on an unconstrained parameter in the same template are
dropped (Q1: A). Tests: a body that calls a method the bound does not
promise (E0606), an operator on a bounded `T` (E0606), a call needing
`&mut` on a `&T` (E0311), a correct body that is never instantiated (no W0601
for bounded templates; they were checked).

### Step 5. Tests, site

Run tests for every step above; plus `iface_shapes_total` (two structs, a
generic `total` summing `area()` of two values), `iface_extends`, `iface_counter`,
`iface_generic_struct` (`struct Box2<T with Shape> { T inner; float64 area()
{ return self.inner::area(); } }` itself implementing `Shape`).

## Site updates

- `reference-generics.md` (created by S6A): add "Constraints with with",
  "Interfaces", "Self", "Methods that change self" with the examples
  `iface-shape.drh`, `iface-counter.drh`, `iface-total.drh` (+ `.out`) and the
  real E0607, E0610, E0612 output.
- `design-generics.md`: delete those sections, leave links; "Interface as a
  type" and "Your own iterables" stay marked. `guide-generics.md` and
  lesson 15 keep their badges (S6Z); verify the lesson's `Shape`/`Circle`
  block, the `Counter` block and the "implement an interface" exercise's
  solution (which uses `&Shape`, so it is E0901 until S6E: do not add its
  solution file yet).
- `reference-diagnostics.md`, `errors.md` via `site/gen-errors.sh`;
  `coming-later.md` row 6; `docs/PLAN.md` note.

## Done when

- `cargo test` green; interfaces, `extends`, `changes self`, constraints and
  `T::static()` work on both backends; all `Shape`/`Counter`/`Zero` snippets
  of the SPEC (outside `&Shape`) compile and run.
- Report: the Q-MF decision for conformance, the list of operations checked
  in the definition pass and what the pass does for operators, a measurement
  of compile time for a template with three bounds, every interim with file
  and line.

## Pitfalls

- The definition pass must not allocate `FuncId`s or leave `Sig`s behind;
  run it in a scratch `Globals` scope or truncate what it added.
- `Self` inside an interface used by two structs: compare after
  substituting each implementing type, not once.
- A struct that implements an interface through a generic instance:
  `struct Box2<T with Shape> with Shape`: conformance is per instance; check
  at instantiation too, not only at the template.
- `changes` must stay usable as an ordinary name (`mut uint32 changes;`).
- Method visibility: the interface method `area` is public even when the
  struct writes it under `private:` (E0610 with the explanation).

## Language questions found

Q1, Q4, Q5, Q6 in `step6-overview.md`. Report any case where an operator
appears in a bounded body and the only way forward is a built-in interface (Q1
option B) as Q1 data.
