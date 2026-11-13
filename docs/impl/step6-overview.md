# Step 6: generics, interfaces, collections, lambdas (overview)

This file is the plan for step 6. The packages have their own briefs; read this
file first, then your brief:

| Package | Brief | Implementer | What |
|---|---|---|---|
| S6R | `step6-R-runtime.md` | standard | list and map runtime in the strings staticlib (type-erased, element sizes and glue passed in); Rust unit tests; no compiler change |
| S6A | `step6-A-generics.md` | **strong** (A.1, A.2), standard (A.3) | `<T>` parsing and `Name<args>` / `T[?]` / `T[N]` type syntax, on-demand layout, monomorphised functions (A.1), structs, enums, `Optional<T>` (A.2), `<unit U>` parameters (A.3) |
| S6B | `step6-B-interfaces.md` | **strong** | `interface`, `with`, `extends`, `Self`, `static`, required fields, `changes self`, `<T with I>` (checked at the definition and at the instance), static dispatch |
| S6C | `step6-C-lists.md` | **strong** (C.1), standard (C.2) | `T[?]` and `T[N]`: types, literals, indexing and bounds checks, glue, backends (C.1); methods, `for` over lists, real `string[?]` from `split`/`lines`/`chars` (C.2) |
| S6D | `step6-D-maps.md` | standard | `Map<K,V>`, `OrderedMap<K,V>`, `{ k: v }`, automatic hashing, `key()`, `[]` giving `V?`, `::get` |
| S6E | `step6-E-dyn.md` | **strong** | `&Shape` / `&mut Shape`: fat borrows, vtables, dynamic calls and required fields |
| S6F | `step6-F-lambdas.md` | **strong** | lambdas `(T x) => expr` / `{ block }`, function types `(T) => R`, captures by borrow, `move`, `each` |
| S6G | `step6-G-iterable.md` | standard | `Iterable<T>` and `for (T x of thing)` over user types and maps |
| S6Z | `step6-Z-sweep.md` | standard | closing sweep: lessons 15 and 16, `guide-generics`, design pages, PLAN/SPEC, benchmarks, the page-level badges |

Nothing below changes the language. Where the owner decided something it is
quoted from `owner-answers.md` or SPEC; where nothing was decided there is an
**interim** choice that is conservative (an E0901 "not supported yet", or an
error that can later be relaxed) and the point is a numbered language question
at the end (Q1 to Q20). **Do not decide a language question yourself**; if your
work needs one answered, stop at the interim and report. The one interim that
is not an error (Q1, instantiate-time checking) is named as such and why.

Every brief says, and every implementer must follow (CLAUDE.md "Keeping the
docs site current"): the site is updated **in the same commit** as the feature
becomes usable, only documents what the compiler accepts today, and every
page or section about something the compiler does not accept yet keeps its
**Not implemented yet** / **Tentative** marker; a marker is removed in the
commit in which that thing builds, never earlier. Commits are plain human
messages with no attribution lines (CLAUDE.md overrides any default). Do not
push. The machine is shared with the owner: `CARGO_BUILD_JOBS=4`, targeted
tests while working (`cargo test --test run list_`), the full `cargo test`
once per commit at the end, `nice -n 19` for benchmarks, never two compiling
jobs yourself.

**Precondition.** Step 5 packages S5C (failures) and S5D (handlers) change the
call path and `src/prelude/core.drh`; S6A.1 must not start until S5C is
merged (it rewrites `Sig`, `check_body` callers and the call lowering that S5C
also edits). S6R has no such dependency and may start at once.

## 1. What exists today (the state your work starts from)

Facts checked in the tree on 2026-11-12; line numbers drift, grep the names.

- **Parser** (`src/parser.rs`): generic structs and enums are E0901 (`generic
  structs are not supported yet`, ~580; enums ~475). `ty()` reads marks
  (`&`, `&mut`, `*`, `*fixed`), a name, an optional unit part, an optional `?`.
  `type_unit` / `unit_close` treat **any** `<` followed by names, `* / ^ ( )`
  and integers up to a `>` as a unit (`uint32<m/s>`). So `Vec<uint32>` parses
  as the unit `uint32` today (Q6 of the units work, owner-answers). `[`...`]`
  after a type is not parsed; `x::name<targs>(args)` parses `targs` as types
  (`::to<T>()`). `TypeName` (`src/ast.rs`) has `name, span, mods, unit,
  optional`. Reserved words already include `interface with extends Self self
  static move async await`; **not** reserved: `changes`, `Iterable`, `unit`,
  `size`. Symbols `=>` and `->` exist; `[` `]` are tokens.
- **Types** (`src/types.rs`): `Type` is `Copy`; compound types point at interned
  `&'static Type` (`Ref`, `Ptr`, `FixedPtr`, `Unit`); `Struct { id, name }` and
  `Enum { id, name }` carry an index and an interned name. Names are only for
  messages; identity is the id.
- **Typeck** (`src/typeck.rs` ~3300 lines, `src/typeck/{control,enums,fill,
  construct,overload,optionals,strings,units}.rs`): everything is global and
  phased. `check()` declares struct and enum names, optionals (`declare_optionals`
  from the `T?` the parser collected in `Program.optionals`), fields, variants,
  runs `report_cycles` / `mark_owning` / `copy_types`, declares signatures and
  methods, then **`lay_out` runs once, before any body is checked**, and
  `g.struct_copy` / `g.enum_copy` are cached for the body checker. Bodies are
  checked in one loop (`check_body`). There is no way to create a struct type
  after bodies start. Symbols: `drachm_f_<name>`, `drachm_m_<len><Struct>_<word>`,
  `drachm_eq_<Struct>`; C structs `drachm_s_<name>`; glue named from the
  struct's own name or `_enum<N>` (`drops::glue_name`). None of these tolerate
  `<`, `>`, `[`, `,`.
- **Optionals** (`typeck/optionals.rs`): `EnumKind::Optional(T)` made per
  distinct `T` from the parser's list; `Optional<T>` spelled out is rejected
  (typeck.rs ~155, "waits for generics").
- **HIR** (`src/hir.rs`): `Program { funcs, structs, enums, main }`, all
  concrete. `HirProj` is `Field | Deref | Downcast`. Layout functions `layout`
  and `enum_layout`.
- **MIR** (`src/mir/*`): `Rvalue::Aggregate(StructId, ..)`, `Variant`,
  `Discriminant`, `Proj::{Field, Deref, Downcast}`; `Switch` is emitted for
  enums. Move paths are `Field` chains (`moves.rs`); enums are one path.
  Drop elaboration and glue are per struct/enum (`drops::drop_glue_types`,
  `copy_glue_types`, `holds_string`). `mir::ty::place_ty` types places.
- **Borrow checker** (`src/mir/borrowck/*`, `lives.rs`): regions come from the
  `Ref` types in locals and the `lives` clauses; a struct with borrow fields is
  a *holder* (`StructDef::lives`). All of it runs on concrete types, so a
  monomorphised instance needs no new rule (section 2.5).
- **Ownership inference** (`src/ownership/self_mode.rs`): `infer(hir)` gives
  each method `Shared` or `Mut(reason)` from its body. This is what `changes
  self` is compared against.
- **Backends**: Cranelift (`codegen_mir.rs`) and C (`cgen_mir.rs`) read the same
  MIR; `codegen_common::in_memory` decides which types travel by hidden
  pointer (structs, enums); `cgen_common` has `c_type`, `struct_defs`.
  `tests/run.rs` runs every case both ways and checks leaks.
- **Runtime**: `runtime/runtime.c` (panic exit 101, output, input);
  `runtime/strings/` is a Rust `staticlib` built by `build.rs`, embedded by
  `link.rs`, with the C contract in `runtime/strings.h` (pasted into generated C
  for `-O`). A string is `{ptr, len, cap}` 24 bytes, `cap == 0` means not owned;
  all allocation goes through libc `malloc`/`realloc`/`free`; `drachm_str_live()`
  counts live owned buffers and `DRACHM_CHECK_LEAKS=1` prints and fails on a leak.
- **Strings**: `split`, `lines`, `chars` return E0901 "lists are not supported
  yet" (`typeck/strings.rs` ~190). `for (string c of s)` and `for (&string c
  of s)` work (`for_text` in `typeck/control.rs`).
- **Site**: `design-generics.md`, `design-collections.md`, `lesson-15-generics.md`,
  `lesson-16-collections.md`, `guide-generics.md` all carry `status:
  not-implemented`; `lesson-15` and `guide-generics` also have two
  `<!-- status: tentative -->` sections (Iterable, generic over units).

## 2. Design

### 2.1 Monomorphisation (engineering; invisible to users except messages)

A generic declaration is **a template**; using it with concrete type arguments
makes an **instance**, which is an ordinary struct, enum or function from then
on. HIR, MIR, borrowck, glue and both backends therefore see no generics. This
is the PLAN's strategy ("by monomorphisation") and the reason section 2.5 can
say the borrow checker needs no change.

- **Templates**: the typeck keeps the `ast` item plus its parameter list in
  `Globals::templates` (functions, structs, enums), keyed by name. A template
  is not declared as a `StructInfo` / `Sig` until instantiated.
- **Instances** are keyed by `(template, Vec<Type>)` (`Type` is `Hash + Eq`) in
  `Globals::instances`. `instantiate_struct(name, args) -> StructId` returns the
  existing instance or: allocates the `StructInfo` (display name `Stack<uint32>`,
  interned), resolves the field types with an environment `tenv: name -> Type`
  consulted first by `Globals::resolve`, declares the method signatures the
  same way, and queues the method bodies on a worklist. Function templates:
  `instantiate_fn(name, args) -> FuncId` the same.
- **Worklist**: after the normal body loop in `check()`, drain `pending`
  (bodies of new instances). Checking a body may create more instances; loop
  until empty. Depth limit 64 instances nested from one template (E0608), so
  `grow<T>` calling `grow<List<T>>` stops.
- **Layout becomes on demand.** Today `lay_out` is a batch before bodies. The
  first commit of S6A.1 is a **pure refactor**: `owns`, `copy`, `size/align`,
  field offsets and the cycle check become memoised queries on `Globals`
  (`layout_of(Type)`, `owns(Type)`, `is_copy(Type)`), computed when asked,
  with an "in progress" mark for cycles (E0210). The batch `lay_out` at the end
  of `check()` just calls them for every struct and enum to fill `hir::Program`.
  Bodies that need `struct_copy` call the query. All 400 existing programs
  and the MIR goldens must be unchanged by this commit.
- **Naming.** A new `types::mangle(&str) -> String` maps a display name to
  `[A-Za-z0-9_]`, injectively (`<` `_L`, `>` `_R`, `,` `_C`, `[?]` `_List`,
  `[4]` `_A4`, `?` `_Q`, space dropped, `_` doubled). It is used by every
  symbol builder: `drachm_f_`, `drachm_m_`, `drachm_eq_`, `drachm_s_`,
  `glue_name`. Messages keep the written spelling (`Stack<uint32>`). Existing
  symbols for non-generic names do not change (mangling only touches the new
  characters and a doubled `_`: audit that no existing golden `.mir` or symbol
  test contains `__`; if one does, mangle only when the name contains a
  special character).
- **Where errors show.** A body is checked once per instance. An error inside
  an instance body is reported at its own source span, with a note on the
  primary diagnostic `while checking `max<string>`, instantiated here:` and a
  secondary label at the instantiating call. The same (span, code) is reported
  once however many instances hit it. A generic body never instantiated is
  still checked once, **at the definition**, when the parameters have
  constraints (2.3); an unconstrained template that is never used is not checked
  (a documented hole, W0601 "`max` is never used" is the only hint).
- **`Optional<T>`** spelled out resolves to the same enum as `T?`
  (`optional_ids`); it stays `EnumKind::Optional(T)` (no prelude enum in this
  step, so `match` and the S5 machinery are untouched; PLAN's "becomes the
  prelude enum" is left for the owner, noted in S6A.2).
- **Type syntax** (parser, S6A.1): `Name<A, B>` generic arguments; `T[?]`
  (list), `T[N]` (array, `N` an integer literal; `size` parameters are Q12);
  these compose with marks and `?` (`&string[?]`, `uint32[?]?`, `Map<string,
  uint32[?]>`). `T[?]` is `List<T>` and `T[N]` is `Array<T, N>` (SPEC); both
  spellings resolve to the same type; a user struct called `List` is E0202
  (the names are reserved by the compiler). `>>` is split by `close_angle`.
- **Disambiguating `Vec<uint32>` from a unit.** The parser has no symbol table,
  so it decides by the **head**: the unit reading of `<...>` applies only when
  the name before `<` is a number type (`uint8`..`uint128`, `sint*`, `float*`),
  and, as today, only if the tokens run to a `>` as a unit would. Any other
  head is a generic argument list: comma-separated types (and integer literals
  for `Array`), with the same lookahead (tokens up to the matching `>` must be
  type tokens, so `bool <(Point o)` stays an operator method). In an
  *expression* (`max<uint32>(5, 9)`, `Stack<uint32>::zero()`, `new Stack<uint32>
  of (...)`) `Name<` opens type arguments only if the matching `>` is followed by
  `(` or `::`. A comparison chain `a < b > (c)` is a type error anyway; add a
  parser test pinning the choice. A unit as a *user generic's* argument
  (`Vec3<m/s>`) is Q2; until then a head that is not a number type never reads
  a unit, and `unit U` parameters are inferred, never written (A.3).
- **Inference** is unification of parameter types against argument types,
  left to right; a type parameter bound by one argument must equal the type of
  the others (strict; Q7 decides literals and widening). `max(a, b)` with
  `uint32 a, b` gives `T = uint32`. Explicit `max<uint32>(5, 9)` seeds the
  environment; fewer arguments than parameters infer the rest (`make<A>()` is
  E0601 when `A` cannot be found). Generic *struct* arguments are never
  inferred: a declaration writes the whole type (`Stack<uint32> s;`); `new
  Stack<uint32> of (...)` and `Stack<uint32>::zero()` write it too. Type
  arguments of a call must not be borrows or hold borrows (Q3 interim).
- **Methods of generic structs** are instantiated with the struct. A method
  with type parameters of its own (`none map<U>(..)`) is E0901 (Q15).
  Operators, constructors, `drop()`, `fill`, `def`, `lives` on a generic struct
  work as for a plain struct, per instance.

### 2.2 Layout of the new types (S6C, S6D, S6E, S6F build these)

| Type | Rust `Type` | Size, align | Passed as | Owns | Copy |
|---|---|---|---|---|---|
| `T[?]` | `Type::List { elem }` (interned) | 24, 8: `{ptr, len, cap}` as a string | hidden pointer (like struct) | yes | no (a move type) |
| `T[N]` | `Type::Array { elem, len }` | `N * size(T)`, align of `T` (N >= 1; `T[0]` is E0624) | hidden pointer | iff `T` owns | iff `T` Copy and size <= 32 (the struct rule) |
| `Map<K,V>`, `OrderedMap<K,V>` | `Type::Map { key, val, ordered }` | 48: runtime header (S6R) | hidden pointer | yes | no |
| `&Shape` / `&mut Shape` | `Type::Ref { to: &Type::Dyn { id } }` | 16: `{data, vtable}` | hidden pointer (in memory, like a struct) | no | `&` copies, `&mut` moves (R8, owner Q15) |
| `(A, B) => R` | `Type::Func { id }` (id into an interned signature table) | 16: `{code, env}` | hidden pointer | no | copies (a view of an env that lives elsewhere) |
| `Shape` (an interface by value) | none | | | | Q4: E0617 |

`Type` stays `Copy` (interned pointers / ids). `Program::size_align`,
`owns`, `scalar_size`, `mir::is_copy`, `needs_drop`, `drops::holds_string`
(rename to `needs_deep_copy`), `analysis::mark_in_memory`,
`codegen_common::in_memory`, `cgen_common::{c_type, work_type, struct_defs}`
are each audited in S6C.1 (the first package to add a `Type` variant) and
extended; later packages add their own variants the same way. Put the audit
list in the commit message, as S5A.1 did for enums.

`mangle` names for the C backend: `drachm_s_List_..` for nothing (lists and
maps have no C struct of their own: they use `drachm_list` / `drachm_map`
from the runtime header; arrays are `uint8_t name[N * size]` inside the
containing struct or a local, with the element type's alignment forced by
`_Alignas`/`aligned` as the enum payload area already does).

### 2.3 Interfaces, constraints and dispatch (S6B, S6E)

- `interface X { sigs; static sigs; fields; }`, `extends`, `Self`: an interface
  is an `InterfaceInfo { name, extends, methods: Vec<IMethod>, fields,
  statics }` in the typeck. `IMethod { name, params, ret, changes_self: bool,
  is_static }` with `Self` kept as `Type::SelfTy` only inside interface
  signatures and substituted by the implementing type when compared.
  `struct S with A and B`: one conformance record per pair, checked after
  all signatures exist (names, parameter and return types equal after
  substituting `Self`, `static`ness equal, required fields present with
  equal types and visibility `public`). `extends`: the extended interface's
  members are part of the extending one; two members of the same name from
  different parents must have equal signatures (E0615).
- **`changes self`** (Q16 of the owner answers): parsed only in interface
  method declarations (anywhere else E0613). Conformance rule: the struct
  method's inferred mode (`ownership::self_mode::infer` on the **instance's**
  HIR) must be `Shared` unless the interface says `changes self`; a struct
  method that is `Shared` while the interface says `changes self` is fine ("may
  not change more than the interface allows"). The check therefore runs after
  `self_mode::infer`, in a new function in `ownership/` called from `lib.rs`
  next to the existing mutability checks; E0612 points at the line in the
  struct method that writes (the `MutReason` already has a span).
- **Constraints** `<T with Shape and Drawable>`: the parameter's bounds. At an
  instance, each type argument must have a conformance record for every bound
  (E0607). At the **definition**, a template with at least one bound is checked
  once with each bounded parameter replaced by `Type::Param { id, name }`, a
  rigid placeholder that exists only in this definition pass (its HIR is
  discarded, so `Param` never reaches MIR). Rules in that pass: a method call
  on a `Param` must be a method of one of its bounds (E0606: "`T` has no
  method `foo` that `Shape` promises"), with the argument and return types of
  the interface (so `Self` becomes `T`); `changes self` methods need `&mut`;
  a field access must be a required field; moving, copying (`copy` needs
  nothing), passing to other generic functions with matching bounds, storing
  in `T[?]` are fine; **operators** on a bounded `Param` are E0606 unless an
  interface declares the operator method (interfaces may declare operator
  methods like any method); unbounded parameters of the same template are
  *not* checked at the definition (they take Q1's interim). Diagnostics from
  the definition pass are marked "in the definition of `total`".
- **Static dispatch** for generics comes free from monomorphisation: in the
  instance `T` is the struct, so `s::area()` is a direct call. `static`
  interface methods (`interface Zero { static Self zero(); }`) are called as
  `T::zero()` inside the template, which at the instance is `Circle::zero()`.
  Built-in types do not implement user interfaces (Q6 interim), so `Zero` for
  `uint32` waits.
- **Dynamic dispatch** (S6E): `&Shape` is a fat borrow `{data, vtable}`. A vtable
  is a read-only static per `(struct, interface)` pair, laid out as
  `[drop_glue_or_null, size, align, method0, method1, ..., field_offset0,
  ...]` in interface order (extended interfaces first). **Required fields**
  (the PLAN says "pick one and note it") use **offsets in the vtable**:
  a read of `s.x` through a `&Positioned` loads the offset word and then the
  field at `data + offset`; the field type is fixed by the interface, so the
  load is typed. No accessor thunks. Writing a required field through
  `&mut Positioned` is allowed only if the struct field is `mut` (the existing
  rule), checked on the struct at the conversion `&sq` to `&Positioned`.
  Conversion points: an argument or `let` where the target type is `&Iface` /
  `&mut Iface` and the source is `&S` / `&mut S` for a struct `S` with the
  interface (E0640 otherwise); an `&Shape3D` to `&Shape` upcast is E0901 (Q5).
  The MIR gets `Rvalue::Unsize(Operand, StructId, IfaceId)` and `Callee::Dyn
  { iface, method }` (receiver is the fat operand); dynamic calls on a method
  that `changes self` need a `&mut Shape`, enforced by the borrow rules as for
  any `&mut` receiver. Glue: a `&Shape` owns nothing and needs no glue.
- **Interfaces by value** (`Shape s;`, `Shape[?]`, `List<Shape>`) are E0617
  with a fix-it to borrow (Q4 is the question).

### 2.4 Collections (S6R, S6C, S6D)

**Runtime.** The runtime lives in the existing Rust staticlib (a second
staticlib would link two copies of Rust's `std`, and the allocation counter
and panic path are already here): new modules `runtime/strings/src/lists.rs`
and `maps.rs`, a new C header `runtime/lists.h` pasted next to `strings.h`
for `-O`, and a link step unchanged. It is **type-erased**: every function
takes the element size and alignment, and, where elements need it, function
pointers to compiler-generated glue:

```c
typedef struct { uint8_t *ptr; uint64_t len; uint64_t cap; } drachm_list;   // same shape as a string
typedef void (*drachm_drop_fn)(void *elem);
typedef void (*drachm_copy_fn)(void *dst, const void *src);   // deep copy of one element
void  drachm_list_push(drachm_list *l, const void *elem, uint64_t size);    // moves the bytes in; grows x2
void *drachm_list_at(const drachm_list *l, uint64_t i, uint64_t size);      // bounds-checked, panics (exit 101) "index 5 is outside the list (length 3)"
void  drachm_list_pop(drachm_list *l, void *out, uint64_t size);            // caller checked len > 0
void  drachm_list_insert(drachm_list *l, uint64_t i, const void *elem, uint64_t size);
void  drachm_list_remove(drachm_list *l, uint64_t i, void *out, uint64_t size);
void  drachm_list_drop(drachm_list *l, uint64_t size, drachm_drop_fn drop_elem); // drop_elem may be NULL
void  drachm_list_copy(drachm_list *out, const drachm_list *l, uint64_t size, drachm_copy_fn copy_elem);
```

Allocation uses `drachm_str_alloc`'s counted allocator, so the leak check
covers lists and maps with no harness change (the message keeps its text; if
you rename it, change `tests/leaks.rs` in the same commit). Elements are raw
bytes: the compiler guarantees alignment <= 16 by allocating through a
helper that over-aligns (`size`/`align` are both passed to `alloc`).

Maps are an insertion-ordered table (entries vector + open-addressing index of
32-bit positions, tombstones, compaction on grow), type-erased with callbacks
`hash(const void *key) -> u64`, `eq(const void *a, const void *b) -> u8`, key
and value size/align, and `drop_key`, `drop_val`, `copy_key`, `copy_val`.
Iteration is by entry position (`drachm_map_next(m, &cursor, &k, &v) -> u8`).
`Map` and `OrderedMap` share the implementation in this step: `Map`'s order
is *unspecified* by the language and happens to be insertion order; no test
or page may rely on it (a randomised order in debug builds is a later idea).

**Compiler side.**
- Element access is a new MIR projection `Proj::Index(Local)` (the local holds
  the index as `uint64`): the place `*(list.ptr + i * size)` for lists, and
  `base + i * size` for arrays. Before it the builder emits the bounds check as
  a branch to a call of `drachm_index_panic(i, len)` (a runtime function, one
  panic message on both backends). A constant index into an array is checked
  at compile time (E0620). Move paths stop at an `Index` (like `Downcast`): an
  element is never a separate move path, so moving out of an element is
  rejected before MIR (E0622, Q11) and `moves.rs` needs no change beyond
  "unknown projection means the whole parent". The borrow checker treats
  `xs[i]` and `xs[j]` as overlapping (conservative), so `swap`-style code uses
  a method. Validation rule V11 (new): `Index` only on `List`/`Array`.
- Intrinsic calls (`push`, `len`, `pop`, `insert`, ...) are `Callee::Builtin(
  ListFn, elem_type)` calls in the MIR; both backends lower them to the runtime
  functions above with the element size and glue pointers. `len` is a load of
  the header field and is inlined in both backends.
- Glue: `drops::drop_glue_types` and `copy_glue_types` currently list structs
  then enums; they gain `List`, `Array`, `Map` types that need it. `drop_list_<m>`
  calls `drachm_list_drop` with the element's glue (or `NULL` when the element
  is plain data). `copy_list_<m>` likewise. `Array` glue is a loop in the
  generated function. A struct with a list field gets list glue called from its
  own glue (`holds_string` generalised to `needs_deep_copy`). `T[?]` of Copy
  `T` still moves (a list owns memory).
- **Hashing and equality** for map keys are *synthesised Drachm-side
  functions* per key type, like `auto_eq`: numbers and `bool` by value, `string`
  by the runtime's `drachm_str_hash` (new, in the strings crate, FNV or
  `ahash`-free SipHash with a fixed key: results must be identical on both
  backends and across runs), structs field by field combining with a fixed
  mixer, enums by tag then payload. A type with `key()` hashes and compares the
  result of `key()` (Q13). Floats are not hashable (Q14), nor lists/maps
  (E0626), nor structs containing them.
- **Literals**: `[a, b, c]` is a list when an expected type `T[?]` is known and
  an array when `T[N]`; with no expected type it is a list (E0624 `[]` with no
  type). Every element converts to the element type with the usual left-side
  rules (first element or the expected type decides). `{ k: v, ... }` likewise
  for a map; duplicate constant keys are E0629. These lower to a sequence of
  `push`/`insert` calls on a fresh local (no `Aggregate` for lists).
- **`[]`**: lists and arrays: the place `xs[i]` (assignable when `xs` allows
  changes: `mut`/`open`, or through `&mut`); maps: a read giving `V?`
  (`ages["Bob"]`), `::get` the same call spelled as a method; writing is Q9.
  `index` is any integer type, converted losslessly to `uint64`; a negative
  `sint` index is a bounds panic (strings count from the end, lists do not:
  SPEC says nothing; Q10 asks).

### 2.5 Borrow checker, `lives`, drops and generics

- **Instances are concrete**, so `lives` inference, `E0308`/`E0309`, `Ref`
  regions and holders run unchanged. The only new shapes are (a) a type
  argument that is a borrow or a holder (Q3: E0901 for now, which keeps
  `regions.rs` from ever seeing a region inside a list element, map entry or
  generic struct argument), and (b) the fat `&Shape` and function types
  (below).
- A generic **struct** may have `&T` / `&mut T` fields and a `lives` clause
  naming them (`struct View<T> lives a { &T a; }`); instantiation `View<string>`
  is a plain holder of a `&string`. A generic function returning a borrow
  (`&T first<T>(&T[?] xs)`) has its `lives` inferred per instance; if the inferred
  answer depends on `T` (a `T` that is itself a holder) it is reported at the
  instance with the instantiating call as a secondary label.
- **`&Shape`** is a `Ref` in `regions.rs` with an unsized target; loans on the
  referent behave exactly as for `&Circle` (the conversion is a reborrow:
  `Unsize` takes the place the `&S` borrowed). `Type::target()` of a `Ref` to
  `Dyn` returns the `Dyn`; deref of it is only legal as the receiver of a
  dynamic call or required-field read (the typeck guarantees; V12 validates).
- **Function types** `(A) => R` hold one region slot, like a `&`. A lambda
  expression lowers to `env = Aggregate(EnvStruct, captures..)` (a synthesised
  struct whose borrow fields are its `lives`, an ordinary holder) and
  `f = MakeFn(code, &env)`; the loan on `env` makes the function value a
  borrower of the local `env` temporary, and the env's own loans keep the
  captured variables borrowed. A function value is **passed down only**
  (Q17): it may be a parameter, a local, and called; storing it in a struct or
  list, returning it, or capturing it in an escaping position is E0652.
  `move` lambdas build the env with `Move` operands; the env is a temporary
  struct value whose ordinary glue drops the moved captures when the temporary
  dies at the end of the statement that created it.
- **Drop and copy glue of containers** is in 2.4; of instances it is the
  existing glue. `copy xs` on a list of owning `T` calls `copy_list_<m>`.
  `destroy xs;` and scope end call `drop_list_<m>`. Drop flags and partial
  moves: a list or map is one move path, like an enum.

### 2.6 Iteration protocol: `for (T x of xs)`

`control.rs::for_each` dispatches on the iterated expression: range (today),
string (`for_text`, today), then new arms, all built from the existing HIR
`Loop` / `If` so `skip`, `break N`, `continue` need nothing new:

- **Array and list**: hidden `len` (loaded once; the loop holds a shared
  borrow of `xs` for its whole extent, so the length cannot change) and hidden
  counter; the body sees the variable declared by the loop. Binding forms:
  `T x` for a Copy `T` copies the element; `&T x` binds a view of the element
  place; `&mut T x` a mutable view (the list must be changeable, and the loop
  holds a `&mut` borrow); `T x` for an owning `T` is Q8 (interim: E0631).
  The element type written must equal the element type (`uint32` over a
  `uint32[?]`; E0630 otherwise; no conversions in a `for`).
- **Map**: Q16. Interim: E0901 (iteration of maps waits).
- **A type implementing `Iterable<T>`** (S6G): the loop borrows the value
  `&mut` and desugars to `loop { T? n = it::next(); if (n is empty) break;
  T x = <payload of n>; body }` (HIR built directly, no source rewrite). Q19.
- A `string[?]` produced by `split`/`lines`/`chars` is an ordinary list of
  owned strings (the runtime builds it with `drachm_str_split` etc.
  returning a `drachm_list`; S6C.2).

## 3. Order, parallelism, file ownership

Two lanes, at most **two compiling agents at once**. The agents of one lane run
one after another. A package starts only when everything it needs has merged.

```
Lane 1 (strong model):   S6A.1 ──> S6A.2 ──> S6B ──> S6E ──> S6F
Lane 2:                  S6R ──> S6C.1 ──> S6C.2 ──> S6D ──> S6G ──> S6Z
                                 (strong)   (standard)
S6A.3 (unit parameters, standard) runs in lane 2 between S6C.2 and S6D, or in
lane 1 after S6B if lane 1 is ahead.
```

| Needs | Because |
|---|---|
| S6A.1 needs S5C merged | it edits `Sig`, call lowering, `check()` phases that S5C also edits |
| S6R needs nothing | runtime only (starts at once, before S5C is done) |
| S6C.1 needs S6A.1 and S6R | `T[?]` syntax, on-demand layout, element types by instance; runtime functions |
| S6A.2 needs S6A.1 | structs and enums reuse the instance machinery |
| S6B needs S6A.2 | `struct Stack<T> with I`, conformance on instances, constraint checks |
| S6C.2 needs S6C.1 | methods and `for` use the types; `split` etc. |
| S6D needs S6C.1 | `Map` reuses type variants, glue, literal machinery; hashing needs structs/enums only |
| S6E needs S6B | conformance records, interface info |
| S6F needs S6E and S6C.1 | fat pair mechanics; `each` on lists |
| S6G needs S6B and S6C.2 | `Iterable<T>` is an interface; `for_each` arms exist |
| S6A.3 needs S6A.2 | units in instance keys |
| S6Z needs everything | it removes page-level badges |

Expected shape: lane 1 is the long one (A.1 is the biggest single package).
If lane 2 reaches S6G before S6B merges, it does S6Z's non-dependent site work
and waits.

File ownership. "Shared hunk" means both lanes edit the file; each edit is a
small disjoint hunk and the later merger rebases.

| File | R | A | B | C | D | E | F | G |
|---|---|---|---|---|---|---|---|---|
| `src/lexer.rs` | | `changes` as context word check | | | | | | |
| `src/ast.rs` | | TypeName args/dims, generic params, templates | interface nodes | list literal, index nodes | map literal | | lambda, fn types | |
| `src/parser.rs` | | owns types, generics; shared hunk with B | `interface`, `with`, `changes self` | `[..]` literals, `xs[i]` | `{k: v}` | | `=>`, `move`, fn types | |
| `src/types.rs` | | `Param`, `mangle`, interned lists | `SelfTy` | `List`, `Array` | `Map` | `Dyn` | `Func` | |
| `src/typeck.rs` | | owns (instances, layout queries, inference) | shared hunk (conformance) | shared hunk (calls on builtin types) | shared hunk | shared hunk | shared hunk | |
| `src/typeck/generics.rs` (new) | | owns | uses | | | | | |
| `src/typeck/interfaces.rs` (new) | | | owns | | | uses | | uses |
| `src/typeck/lists.rs` (new) | | | | owns | | | uses (`each`) | |
| `src/typeck/maps.rs` (new) | | | | | owns | | | |
| `src/typeck/dynamic.rs` (new) | | | | | | owns | | |
| `src/typeck/lambdas.rs` (new) | | | | | | | owns | |
| `src/typeck/control.rs` | | | | `for_each` list/array arm: **shared hunk** | | | | `Iterable` arm: **shared hunk** |
| `src/typeck/strings.rs` | | | | `split`/`lines`/`chars` | | | | |
| `src/typeck/units.rs` | | A.3 | | | | | | |
| `src/hir.rs` | | instances need nothing; `Param` never here | | Index, List ops | Map ops | Unsize, DynCall | Lambda, FnCall | |
| `src/ownership/*` | | | conformance (`changes self`) | | | | | |
| `src/mir/{mod,build,validate,print,ty}.rs` | | | | Index, Builtin calls | | Unsize, Dyn | MakeFn, FnCall | |
| `src/mir/{moves,drops,mutability,borrowck}` | | | | glue for lists | glue for maps | `Dyn` refs | function types, env holders | |
| `src/codegen_*.rs`, `src/cgen_*.rs` | | `mangle` use | | owns list/array code | map calls | vtables, fat calls | fn pairs | |
| `runtime/strings/**`, `runtime/lists.h` | **owns** | | | C.2: split etc. | uses | | | |
| `runtime/runtime.c` | | | | `drachm_index_panic` | | | | |
| `build.rs`, `src/link.rs` | R (header embed) | | | | | | | |
| `src/prelude/` | | | | | | | | G (`Iterable`) |
| `src/diag/codes.rs` | | E0601-E0609 | E0610-E0619 | E0620-E0631 | E0626-E0629 | E0640 | E0650-E0655 | E0660-E0662 |
| `tests/run.rs`, `tests/backends.rs` | | | | | | | | |
| `site/content/*` | | generics ref | interfaces ref | collections ref | collections ref | interfaces ref | lambdas ref | iterables |
| `docs/PLAN.md`, `docs/SPEC.md` | | row 6 note | row 6 note | row 6 note | row 6 note | row 6 note | row 6 note | row 6 note |

Code blocks for codes.rs are contiguous and disjoint per package; each adds an
`explain` text of the existing style. `tests/run.rs` and `tests/backends.rs`
need no edits (cases are dropped in as files); S6C.1 adds a `leaks` case for
lists in `tests/leaks.rs` if the harness needs a list-specific check.

Every package: one commit per numbered step in its brief, `cargo test` green
before each commit, golden `.out` files read before blessing.

## 4. How to measure speed (S6C.1 records the baseline, S6Z reports)

The owner chose `-O` via C for benchmark speed, so generic code must not be
slower than hand-written code, and lists must not be slower than C arrays plus
a growth policy. `bench/collections.drh` with `collections.c` and `collections.rs`
beside it (S6C.1 adds the first two cases, S6D the third, S6Z reports):

1. `sum`: a `uint64[?]` of 50 million filled by `push` then summed with `for
   (uint64 n of xs)`; C: realloc'd array; Rust: `Vec<u64>`.
2. `index`: random-access sum through `xs[i]` with a checked index (the
   bounds check cost; the Rust twin uses `xs[i]`).
3. `generic`: `T max<T>(T a, T b)` and `Stack<T>` push/pop 100 million times
   with `T = uint64`; must compile to the same loop as the non-generic
   version (compare the C emitted by `-O`).
4. `map`: 5 million inserts and lookups of `uint64` keys; C twin: a simple
   open-addressing table; Rust: `HashMap` with the default hasher and with a
   fixed one.
5. `dyn`: 100 million `s::area()` calls through `&Shape` over two alternating
   structs (the vtable cost) against a switch in C and `&dyn` in Rust.

Report the `bench/run.sh` table (drachm -O, plain Cranelift, C, Rust). Targets
(reported against, not gates): `sum` and `generic` within 1.3x of C; `index`
within 1.5x; `map` within 2x of Rust's `HashMap`; `dyn` within 1.5x of Rust.
The gate is that both backends behave identically. Report `time cargo test`
before and after each package (once).

## 5. Site rules for the whole step

The pages `design-generics.md`, `design-collections.md`, `lesson-15-generics.md`,
`lesson-16-collections.md`, `guide-generics.md` carry `status: not-implemented`
at the top and per-section markers (`<!-- status: tentative -->`). When a
package makes a section build:

1. Move it into a **reference** page: new `reference-generics.md` (generic
   functions, structs, constraints, interfaces, `changes self`, `Self`, dynamic
   `&Shape`) and `reference-collections.md` (arrays, lists, maps, indexing,
   `for` over them, string lists), extend `reference-types.md` with `T[?]`,
   `T[N]`, `Map`, `Optional<T>`, and `reference-control-flow.md` with `for` over
   collections; lambdas go in `reference-generics.md` under "Function types and
   lambdas" until the owner wants a page of their own. Each moved section has a
   real example in `site/examples/NAME.drh` + `.out` (checked by `cargo test`)
   and the real compiler output for its errors (format of
   `reference-ownership.md`).
2. In the design page, delete the moved section and leave a one-line link, or
   replace the page-level `status:` by a `<!-- status: not-implemented -->`
   marker under each heading that is still unbuilt. The page-level status goes
   only when nothing on the page is unbuilt.
3. **Lessons 15 and 16 and `guide-generics` keep their badge until S6Z** (they
   need several packages: lesson 15 needs A, B, C, E and uses `Iterable`
   (tentative section stays tentative until G; Q19); lesson 16 needs C, D and
   G for map iteration; `guide-generics` needs A, B, C, E). Each package whose
   feature appears in one of those pages adds the matching real example in
   `site/examples/` and checks the lesson's starter/solution blocks that it
   enables (the solution programs are copied to `site/examples/solution-15-N.drh`
   + `.out`, the same convention as `solution-10-N`); S6Z removes the
   `status: not-implemented` front matter and the sentence "The compiler can't
   run this lesson yet; the code shows the planned design." when every block
   that is not marked tentative builds.
4. Roadmap `coming-later.md`: progress row 6, the "coming later" bullets. New
   pages go in `site/content/order.json`.
5. `site/gen-errors.sh` after every change to `src/diag/codes.rs`;
   `reference-diagnostics.md` lists the 06xx codes.
6. Every new example compiles silently, is leak-free and prints exactly its
   `.out` on both builds.
7. The "Generic over units" section of `design-generics.md` and `guide-generics.md`
   goes away only when A.3 builds; the spelling stays tentative if Q2 is open.

## 6. New diagnostic codes

A new range is introduced: `06xx` = generics, interfaces, collections, lambdas
(add the line to the header comment of `src/diag/codes.rs`, to SPEC section
10's range list, and to `reference-diagnostics.md`). Wording style is SPEC
section 10 / E0302: a message that states the problem with the user's names, a
primary label that says what is wrong **here**, secondary labels that show the
other place involved, and a `help:` with code the user can type. Every message
is written for the owner's batch review (same status as the 03xx and 05xx
wording). Names are the user's spelling; types print as the compiler prints
them (`Stack<uint32>`, `string[?]`).

Existing codes reused: E0101 (type mismatch, index types, element conversion),
E0103 (arguments), E0201 (no such method/field/name), E0202 (duplicate), E0210
(contains itself: `Node<T>` directly inside itself), E0213, E0301-E0311 (all
borrow/move rules apply to instances and elements unchanged), E0901.

| Code | Pkg | Situation | Message | Primary label | Help |
|---|---|---|---|---|---|
| E0601 | A.1 | cannot infer | cannot tell what `T` is in this call of `make` | no argument shows `T` | write it out: `make<uint32>()` |
| E0602 | A.1 | wrong count | `Stack` takes 1 type argument, but 2 are given | the extra argument (or: needs 1 here) | secondary: `Stack<T>` declared here; write `Stack<uint32>` |
| E0603 | A.1 | arguments disagree | `T` is `uint8` from the first argument, but `string` from the second | this one is `string`; secondary: first is `uint8` | convert one, or write the type: `max<uint32>(a, b)` |
| E0604 | A.1 | missing arguments | `Stack` needs a type argument | `Stack` alone is not a type | write `Stack<uint32>`; secondary: declared with `<T>` here |
| E0605 | A.1 | bad parameter list | type parameter `T` is declared twice (or: `T` is also the name of a struct) | declared again here; secondary: first here | rename one |
| E0606 | A.1/B | beyond the constraint | `T` has no method `scale` that its constraint `Shape` promises (or: `T` has no `>`: operators need an interface that declares them) | not offered by `Shape`; secondary: `T with Shape` here | add it to the interface, or constrain `T` with one that has it |
| E0607 | B | constraint not met | `Triangle` does not implement `Shape`, which `total` asks of `T` | `T` is `Triangle` here; secondary: `T with Shape` | write `struct Triangle with Shape` and give it `area` |
| E0608 | A.1 | runaway instance | `grow` keeps instantiating itself: `grow<uint32>`, `grow<uint32[?]>`, ... | instantiated here again with a bigger type | make the recursive call use the same `T` |
| E0609 | A.3 | unit parameter misuse | `U` is a unit, so it can only be used inside `<>` of a number type | not a type | write `float64<U>`; a type parameter would be `<T>` |
| E0610 | B | missing method | `Circle` says `with Shape`, but has no method `area` | `with Shape` here; secondary: `area` promised here | add `float64 area() { ... }` |
| E0611 | B | wrong signature | `Circle::area` returns `uint32`, but `Shape::area` returns `float64` | this return type; secondary: the interface line | change it to `float64` |
| E0612 | B | changes too much | `Circle::grow` changes `self`, but `Shape::grow` does not say `changes self` | changes `self.r` here; secondary: the interface method | add `changes self` to `Shape::grow`, or stop changing `self` |
| E0613 | B | `changes self` misplaced | `changes self` is only written in interfaces | not allowed here | a struct method's mode is worked out from its body |
| E0614 | B | required field | `Square` says `with Positioned`, but has no field `x` of type `float64` | `with Positioned` here; secondary: `x` required here | add `public float64 x;` (or: `x` is `uint32` here, `float64` there) |
| E0615 | B | interface clash | `Shape3D` extends `Shape` and `Solid`, which both have `area` with different types | clashes here | make the signatures equal (or: `Shape` extends itself) |
| E0616 | B | `Self` misplaced | `Self` only means something inside an interface | here | write the struct's name |
| E0617 | B | interface as value | `Shape` is an interface, so a value cannot be one | not a type for a value | borrow it: `&Shape` |
| E0618 | B/E | not usable as a type | `Zero` cannot be used as a type: it has a `static` method | cannot be dispatched | use it as a constraint: `<T with Zero>` |
| E0619 | E | static through dynamic | `zero` is `static`, so it needs a type, not a `&Zero` | called on a value | call it on the type: `Circle::zero()` |
| E0620 | C | constant index | index 5 is outside `uint8[4]` | this index; secondary: the array's type | valid indexes are 0 to 3 |
| E0622 | C | move out of an element | cannot move a `string` out of a list by index | moved here | copy it: `copy names[0]`, or borrow: `&names[0]` |
| E0623 | C | literal mixes types | this list holds `uint32` and `string` | this element is `string`; secondary: first is `uint32` | convert, or give the list a type |
| E0624 | C | empty literal | cannot tell what this empty list holds | no type to take from | give it a type: `string[?] xs = [];` |
| E0625 | C | array length | `uint8[4]` needs 4 values, but 3 are given | 3 values; secondary: the type | add one, or change the length |
| E0626 | C/D | not hashable | `float64` cannot be a map key | floats do not hash | give the type a `key()` returning a whole number or text |
| E0627 | D | `key()` and `==` | `Tag` has both `key()` and `==` | defined here; secondary: the other | (Q13) |
| E0628 | D | `key()` shape | `key()` must be `key()` with no parameters and a hashable result | this one | write `string key() { ... }` |
| E0629 | D | duplicate key | this map literal has the key `"Illy"` twice | again here; secondary: first here | remove one |
| E0630 | C | loop variable type | the items are `string`, but the variable is `uint32` | this variable; secondary: the list's type | write `string` |
| E0631 | C | by-value owning item | `names` holds `string`s, which move; the loop cannot copy each one by itself | `string n` would copy | write `&string n` to look at each, or `copy` it inside the loop (Q8) |
| E0640 | E | no conversion | `Square` does not implement `Shape`, so `&sq` is not a `&Shape` | here; secondary: `struct Square` | add `with Shape` |
| E0650 | F | lambda shape | this lambda takes `(string)`, but `each` wants `(uint32)` | the lambda; secondary: the parameter type | write `(uint32 n) => ...` |
| E0651 | F | lambda result | this lambda gives `string`, but `(uint32) => uint32` wants `uint32` | here | convert it |
| E0652 | F | function value kept | a function value can only be passed down, not stored or returned (Q17) | stored here | pass it to the function that uses it |
| E0653 | F | captured value changed | this lambda changes `count`, which it borrows (Q18) | changed here; secondary: captured here | not supported yet: return the value and sum outside |
| E0654 | F | not callable | `n` is a `uint32`, not a function | called here | |
| E0660 | G | not iterable | `Countdown` cannot be walked with `for`: it does not implement `Iterable<T>` | here | add `with Iterable<uint32>` and a `uint32? next()` |
| E0661 | G | wrong `next` | `next` must return `uint32?` to implement `Iterable<uint32>` | this | |
| E0662 | G | needs `mut` | the loop changes `c` as it goes, so `c` must be `mut` (Q19) | `c` is not `mut` | declare it `mut Countdown c` |
| W0601 | A.1 | unused template | `max` is never used | nothing instantiates it, so its body was not checked | remove it, or call it |

Full renderings to copy the layout from (the implementers write the real
output; these fix the shape):

```text
error[E0601]: cannot tell what `T` is in this call of `make`
  --> main.drh:9:20
   |
 2 | T make<T>() { ... }
   |        - `T` is declared here
 9 |     uint32 n = make();
   |                ^^^^^^ no argument shows `T`, and the result is not enough
   |
   help: write it out: `make<uint32>()`
```

```text
error[E0612]: `Circle::grow` changes `self`, but `Shape::grow` does not say `changes self`
  --> main.drh:12:9
   |
 3 |     none grow(float64 k);
   |     --------------------- `Shape::grow` promises not to change `self`
11 | struct Circle with Shape {
12 |     none grow(float64 k) { self.r = self.r * k; }
   |                            ^^^^^^ changes `self.r` here
   |
   help: add `changes self` to `Shape::grow`: `none grow(float64 k) changes self;`
```

```text
error[E0607]: `Triangle` does not implement `Shape`, which `total` asks of `T`
  --> main.drh:20:17
   |
 6 | float64 total<T with Shape>(T[?] shapes) { ... }
   |                 ---------- `T` must implement `Shape`
20 |     float64 a = total(ts);
   |                 ^^^^^ `T` is `Triangle` here
   |
   help: write `struct Triangle with Shape` and give it `float64 area()`
```

```text
error[E0620]: index 5 is outside `uint8[4]`
  --> main.drh:5:19
   |
 4 |     uint8[4] rgba = [255, 0, 0, 255];
   |     --------- an array of 4 values
 5 |     uint8 a = rgba[5];
   |                    ^ this index is 5
   |
   help: valid indexes are 0 to 3
```

## 7. Reporting

Each package's report lists: what differs from the brief, the benchmark table
where the brief asks for one, `time cargo test` before/after, every interim
that the code took (with its file and line), every new language question found,
and the `grep` results the "Done when" asks for.

## 8. Language questions (the owner decides; answers go to `owner-answers.md`)

Every question has its interim, the behaviour the code has until answered. The
interim never silently picks a design: it is an error saying "not supported
yet" (E0901) or the more restrictive reading. The option marked "(original)"
is a Drachm idea rather than a borrowed one; none of them is marked as
recommended. Questions are in the order the packages need them.

### Q1. What can an unconstrained `T` do in a body?

SPEC writes `T max<T>(T a, T b) { if (a > b) ... }` with no constraint, and
also says `<T with Shape>` restricts the body to Shape's methods.

```cpp
T max<T>(T a, T b) { if (a > b) { return a; } return b; }   // uses >, says nothing
float64 total<T with Shape>(T[?] shapes) { ... }              // only Shape's methods
```

- A. Check at the instance: an unconstrained body is checked once per type it
  is used with, as if `T` were written out; `max<string>` works if strings
  have `>`, and `max<Circle>` fails with the error inside `max`, pointing at
  the call.
- B. Operator interfaces: `>` needs `T with Ordered`, `+` needs `T with Add`,
  and so on, provided by the compiler for the number types and `string`;
  an unconstrained `T` can only be moved, copied, stored and passed.
- C. (original) The compiler works out what the body needs, as it already
  does for `changes self`: `max` is shown as needing `>` of `T`, a caller
  with a type that has no `>` gets the error at the call, and the signature
  (in error messages and the language server) lists the inferred needs.

Interim: A for unconstrained parameters (the one interim that is not an
error: SPEC's own `max` example and the done-when need it); constrained
parameters are checked at the definition as in 2.3. Moving to B or C later
only turns more programs into errors at the definition.

### Q2. How is a unit written as an argument of your own generic?

Data (S6Z, 2026-11-13): `struct Vec3<T, unit U>` with `Vec3<float64, unit m/s>` builds; `unit U with Shape` is E0901 ("a constraint on a unit parameter is not supported yet").

```cpp
struct Vec3<T, unit U> { float64<U> x; float64<U> y; }
Vec3<float64, m/s> v;      // ?
```

- A. By the parameter's kind: the declaration says `unit U`, so the argument
  is read as a unit (`Vec3<float64, m/s>`); a name that is both a type and a
  unit is an error asking for the other spelling.
- B. Marked at the use: `Vec3<float64, unit m/s>`.
- C. (original) A `;` separates types from units in both the declaration and
  the use: `struct Vec3<T; unit U>` and `Vec3<float64; m/s>`.

Interim: `unit U` parameters exist only on functions and are inferred from
the arguments (`float64<U> twice<unit U>(float64<U> x)` called as `twice(3.0<m>)`);
writing a unit argument explicitly, or on a struct, is E0901.

### Q3. May a type argument be a borrow, or a type that holds one?

Data (S6Z): `Stack<&string>` with `s::push(&name)` builds (`gen-borrow-stack`); `List<&string>` builds. A generic interface as a borrow, `&Read<uint32>`, is E0901.

```cpp
string[?] names = ["a"];
&string[?] views = ...;        // a list of borrows?
Stack<&string> s;              // a stack of borrows?
```

- A. Yes, and the container becomes a borrower: its lifetime is tied to
  everything borrowed inside it, inferred (the container behaves like a struct
  with `lives`).
- B. No: containers and generic structs own what they hold; to keep a
  reference, copy it or hold a struct with `lives`.
- C. (original) Yes, but the borrow must be named: `Stack<&string> lives
  names` says which variable it borrows from.

Interim: E0901 for any type argument that is a `&`, `&mut`, a pointer, or a
struct with `lives`. (Option B would then be the same error with a new
message.)

### Q4. Can an interface be the type of an owned value?

```cpp
Shape[?] shapes = [Circle(1.0), Rect(2.0, 3.0)];   // mixed shapes in one list
Shape s = Circle(1.0);
```

- A. No: an interface is only ever borrowed (`&Shape`); a mixed collection is
  an enum, which the compiler can lay out and check for completeness.
- B. Yes: an interface by value owns its struct on the heap (size unknown
  otherwise), so `Shape[?]` works and drops each struct through the vtable.
- C. (original) Yes, with a word that says where it lives: `heap Shape[?]`;
  a bare `Shape` is only a borrow target.

Interim: A. `Shape s;` and `Shape[?]` are E0617 with the fix-it `&Shape`.

### Q5. Which interfaces can be used as a type, and may `&Shape3D` become `&Shape`?

Data (S6Z): `exclusive` in a parameter is E0901 ("`exclusive` is not supported yet"); automatic upcasts are built.

```cpp
interface Zero { static Self zero(); }       // no value to call it on
interface Shape3D extends Shape { float64 volume(); }
none show(&Shape s) { }
&Shape3D b = ...;  show(b);                  // upcast?
```

- A. An interface is usable as a type only if it has no `static` methods and
  `Self` is not a parameter or result; upcasts are not allowed (take the
  parent interface in the function).
- B. Any interface is usable as a type; `static` methods and `Self` members
  are simply not callable through it; upcasts are automatic.
- C. (original) The interface says so: `interface Shape dynamic { ... }` opts
  in; without it only constraints (`<T with Shape>`) can use it.

Interim: A for usability (E0618), upcasts E0901.

### Q6. How do types you don't own (numbers, `string`, lists) implement an interface?

```cpp
interface Zero { static Self zero(); }
T sum<T with Zero>(T[?] xs) { ... }       // sum of uint32[?]?
```

- A. They don't: only your own structs say `with`.
- B. A separate declaration states it: `uint32 with Zero { static uint32 zero() { return 0; } }`
  (anywhere in the project).
- C. (original) The compiler provides a fixed set of interfaces for numbers
  and text (`Zero`, `Ordered`, `Equal`, `Text`) that cannot be extended.

Interim: A.

### Q7. What is `T` when the arguments are literals or of different number types?

```cpp
uint8 a = 5;
max(a, 300);        // a is uint8, 300 does not fit
max(4, 9);          // no variable shows a type at all
max(2.5, 1);        // float and integer
```

- A. The first argument that has a type decides `T`; untyped constants adapt
  (or fail, as `uint8 x = 300;` does); a call made only of constants takes
  the type an untyped constant gets with no context (the same as a plain
  `5` today).
- B. All typed arguments must be exactly the same type; constants adapt; no
  other conversions at all (`max(a_u8, b_u16)` is an error asking for
  `max<uint16>(a, b)`).
- C. (original) `T` is the smallest type every argument fits into without
  loss: `max(a_u8, 300)` is `uint16`, `max(2.5, 1)` is `float64`.

Interim: B, and an all-constant call uses the type of an untyped constant
today (lesson 15's `min(4, 9)` and `min(2.5, 1.5)` work).

### Q8. What does `for (T x of xs)` do for an item that owns memory?

```cpp
string[?] names = ["Illy", "Sam"];
for (string n of names) { }       // copy each? move each?
for (&string n of names) { }      // a view
```

- A. As for text (step 5 Q2): the type written decides: `string n` gives a
  copy per item (the compiler skips it when the body never keeps `n`), `&string
  n` a view, `&mut string n` a mutable view.
- B. By value over a borrowed list is an error for owning items; Copy items
  bind plainly; the owning forms are `&T` and `&mut T` only.
- C. (original) `take string n` consumes the list: each item is moved out and
  the list is used up; `string n` copies, `&string n` views.

Interim: B (E0631 offering the `&` and `copy` fixes).

### Q9. How does a map get a new entry?

Data (S6Z): `m[k] = v`, `m::set`, `m::remove`, `m::get` and `m::len` all build and run (`map-update`, and a probe in the sweep).

```cpp
Map<string, uint32> ages = { "Illy": 18 };
ages["Bob"] = 21;          // inserts? replaces? both?
ages["Illy"] += 1;         // when Illy is missing?
```

- A. `m[k] = v` inserts or replaces; `m[k] += x` on a missing key is a panic.
- B. `[]` is only for reading (it gives an optional); entries change through
  methods: `m::set(k, v)`, `m::remove(k)`.
- C. (original) `m[k] = v` replaces an existing entry (panics when missing);
  adding is a failing call so a duplicate is visible: `m::add(k, v) fails
  DuplicateKey`.

Interim: B-minimal: only the literal builds a map and `[]`/`::get` read; no
`set`, `add` or assignment through `[]` yet (E0901 "changing a map waits").

### Q10. The edges of indexing and the rest of the list methods

```cpp
string[?] xs = ["a", "b", "c"];
xs[-1];                     // from the end, like text? a panic?
xs::pop();                  // string? ; xs::remove(1); xs::insert(0, "z"); xs::clear();
```

- A. Index: only 0 and up, a negative value panics; methods `push`, `pop`
  (gives `T?`), `get(i)` (gives `T?`), `insert(i, v)`, `remove(i)` (gives `T`,
  panics when outside), `clear()`, `contains(v)`, `join(sep)`, `first()` and
  `last()` (give `T?`).
- B. Like text: negative counts from the end (`xs[-1]` is the last); same
  methods as A.
- C. (original) Every method that can miss gives an optional and nothing
  panics except `[]`: `remove(i)` gives `T?`; `xs::dropAt(i)` / `xs::takeAt(i)`
  say whether the item is kept or given back.

Interim: only the methods SPEC and the lessons show (`push`, `len`, `get`,
`[]`, `join`, plus `pop` giving `T?`, `each` with S6F); everything else is
E0201 with the note "not decided yet"; a negative index panics.

### Q11. Moving an item out of a list by index

Data (S6Z): `take xs[i] reorders` builds; `take xs[i] leaving v` is E0901.

```cpp
string first = xs[0];                   // error today: strings move
string s = take xs[0] leaving "";       // like take on a field?
string t = xs::remove(0);               // shifts the rest
```

- A. `take xs[i] leaving v`, the same form as for fields; the list keeps its
  length.
- B. Indexing never moves; only methods take items out (`remove`, `pop`).
- C. (original) `take xs[i]` swaps in the list's last item and shrinks it
  (order not kept); the order-keeping removal is `remove(i)`.

Interim: B (E0622); `take xs[i]` is E0901.

### Q12. Can the length of an array be a generic parameter?

Data (S6Z): `struct Buffer<T, uint32 N> { T[N] items; }` is a parse error (E0003, "expected `,` or `>`, found name `N`"), not the E0901 the interim names. A diagnostic gap, reported, not changed.

```cpp
struct Buffer<T, N> { T[N] items; }
uint8[4] rgba;                        // a literal length is fine today
```

- A. No: array lengths are written as numbers (or named constants once
  modules exist); no value parameters.
- B. Value parameters like `<T, uint32 N>`.
- C. (original) A kind word like `unit`: `<T, size N>`, so `unit U` and `size
  N` are the two non-type kinds.

Interim: A (`size` and `uint32 N` parameters are E0901).

### Q13. A type with both `key()` and its own `==` as a map key

```cpp
struct Tag {
    string text;
    string key() { return self.text::lower(); }
    bool ==(Tag o) { return self.text == o.text; }
}
Map<Tag, uint32> m;
```

- A. Not allowed: a type picks one (E0627).
- B. `key()` wins for maps; `==` stays what `==` is; the two may disagree.
- C. (original) The struct says which one a map uses in its head: `struct Tag
  keyed by key` or `keyed by ==`; with only one present no word is needed.

Interim: A.

### Q14. May floats be map keys?

```cpp
Map<float64, string> m;       // NaN != NaN, -0.0 == 0.0
```

- A. No: a float key is an error; use a `key()` returning whole numbers or
  text; a struct with a float field needs `key()` to be a key.
- B. Yes, hashing and comparing the bits: `NaN` equals itself, `-0.0` and
  `0.0` differ.
- C. (original) Yes, but only keys that compare equal to themselves: a `NaN`
  key is a failing insert (`fails BadKey`).

Interim: A (E0626).

### Q15. Methods with type parameters of their own

Data (S6Z): an interface method with its own type parameter, `U[?] map<U>((T) => U f)`, is E0901 as the interim says.

```cpp
struct List... { U[?] map<U>((T) => U f) { ... } }
```

- A. Allowed: instantiated per use, inferred like functions.
- B. Not allowed: a method can use only the struct's parameters; other generic
  behaviour is a free function.
- C. (original) Allowed only through an interface method, so every extra
  parameter is visible in the interface (`T? find<U with Equal>(U key)`).

Interim: B-strict: E0901.

### Q16. How a map is walked

Data (S6Z): the two-binding walk builds; `for (&string k, uint32 v of ages)` is E0901 ("looking at a map's entries without copying them is not supported yet").

```cpp
for (string k, uint32 v of ages) { }
```

- A. Two bindings: `for (string k, uint32 v of ages)`.
- B. One item type with fields: `for (Entry<string, uint32> e of ages)` and
  `e.key`, `e.value`.
- C. (original) The arrow already in the language: `for (string k -> uint32 v
  of ages)`.

Interim: E0901 (maps cannot be iterated yet; lookups and the literal work).

### Q17. Where may a function value go?

Data (S6Z): a lambda returned from a function is E0652 ("a function value cannot be returned").

```cpp
none each((string) => none f) { ... }            // passed down
(uint32) => uint32 adder(uint32 n) { return (uint32 x) => x + n; }   // returned: borrows n
```

- A. Only down: a function value can be a parameter, a local and be called;
  storing it in a struct or list and returning it are errors. `move` lambdas
  too.
- B. Borrowing lambdas only down; `move` lambdas (which own their captures)
  may be returned and stored (they live on the heap).
- C. (original) The function type says: `(string) => none` is passed down
  only, `keep (string) => none` may be stored and returned (and then must be
  `move`).

Interim: A (E0652).

### Q18. A lambda that changes what it captured

```cpp
mut uint32 count = 0;
names::each((string n) => { count++; });
```

- A. Inferred, as `self` is: the capture becomes a `&mut` borrow, and calling
  a function value that changes captures needs a function that is allowed to
  (shown in the type by inference).
- B. The function type says: `(string) => none changes` accepts lambdas that
  change their captures, plain `(string) => none` only read.
- C. (original) The lambda lists what it changes: `(string n) changes count
  => count++`.

Interim: E0653 (captures are read-only).

### Q19. What `for (T x of thing)` does to a type with `Iterable<T>`

```cpp
mut Countdown c = Countdown(3);
for (uint32 n of c) { }       // c is spent now?
uint32 total = sum(c);        // may c be walked twice?
```

- A. The loop borrows `c` as `&mut` and calls `next()`: walking advances `c`
  and it stays advanced afterwards.
- B. Two levels: `Iterable<T>` has `Iterator<T> iter()` and the iterator has
  `T? next() changes self`; the loop calls `iter()` and walks that, so the
  value itself is not spent.
- C. (original) The loop consumes the value (`for` takes it, as `take` would)
  unless you walk a copy: `for (uint32 n of copy c)`.

Interim: A, with the variable required to be `mut` (E0662); lists, arrays and
maps are walked by the built-in loops and are not `Iterable<T>` yet, so
`T with Iterable<uint32>` accepts only user types.

### Q20. The empty collection

Data (S6Z): `mut string[?] names;` followed by `names::push("Illy")` builds and runs; `open string[?] xs = [];` builds too.

```cpp
string[?] xs;                // no value yet, or an empty list?
Map<string, uint32> m;
```

- A. Like `uint32? age;` (starts as `none`): a list or map declared without a
  value starts empty and is usable at once.
- B. Like any other binding: no value until assigned (reading is E0207); the
  empty list is written `[]`.
- C. (original) Starts empty, but only for `open` bindings, since only those
  can then be filled: `open string[?] xs;`.

Interim: B.
