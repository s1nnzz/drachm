# S6D: maps, ordered maps, hashing and keys

Implementer: standard model. It repeats what S6C.1 did for a second container,
adds synthesised hash/eq functions (like `auto_eq`, a pattern that exists) and
a `key()` rule. Needs S6C.1 merged (type kinds, glue routes, literal and
builtin-call machinery) and S6R.

Read first: `step6-overview.md` 2.4 (hashing, runtime, literals, `[]`), 4, 6
(E0626 to E0629) and Q9, Q13, Q14, Q16; SPEC "Collections" and section 5 (`key()`,
automatic `==` and hashing, the OPEN note); PLAN step 6 item 3 and "Map keys";
`step6-C-lists.md` (what you copy); `step6-R-runtime.md` (map contract and
`drachm_map_ops`); the code: `src/typeck.rs` (`auto_eq`, `eq_func`,
`struct_eq`, `eq_pending`), `src/typeck/optionals.rs` (building a `V?` result),
`src/typeck/lists.rs` (S6C), `src/mir/drops.rs`, backends (where S6C added
builtin list calls), `src/cgen_common.rs`.

## Goal

1. `Map<string, uint32> ages = { "Illy": 18, "Sam": 20 };`, `OrderedMap<K,V>`
   likewise, types as parameters, results, fields, generic arguments;
   `ages["Bob"]` giving `uint32?` (`none` when missing), `ages["Bob"] maybe 0`,
   `ages::get("Bob")` (the same lookup as a method), `len()`. Other method names (`has`, `keys`, ...) are not decided: E0201 with the note "not decided yet".
2. Automatic hashing and `==` keys: integers, `bool`, `string`, structs and
   enums whose fields are hashable, `Optional` of those, arrays of them.
   Floats, lists, maps, borrows and pointers are not hashable (E0626, Q14).
   `key()` method (`string key()`, any hashable result) defines hashing and
   equality by its result (E0628 if the shape is wrong).
3. Drop, `copy`, move of maps (leak-free), maps as values of maps, maps inside
   structs and instances (`struct Registry<V> { Map<string, V> items; }`).
4. `{ k: v }` literal; duplicate constant keys E0629; `{}` with a type;
   without expected type E0624 reworded for maps.

Out of scope (E0901): changing a map after creation (Q9: no `set`, `add`,
`remove`, assignment through `[]`; the runtime has `insert` and `remove` ready
for when the question is answered); iteration (Q16); `Map` as `Iterable`;
`keys()`/`values()`; both `key()` and custom `==` (E0627, Q13); `Map` with a
borrow or holder as key or value (Q3); `OrderedMap` methods beyond `Map`'s;
maps in `const`; map equality.

## Files you own

New: `src/typeck/maps.rs` (type resolution of `Map`/`OrderedMap`, literal,
`[]`, `get`, `len`, hash/eq synthesis, `key()`), `tests/run/map_*.drh`,
`tests/fail/map_*.drh`, `site/examples/map-*.drh`, the `map` case of
`bench/collections.{drh,c,rs}`.
Edit: `src/types.rs` (`Type::Map { key, val, ordered }`), `src/ast.rs` and
`src/parser.rs` (the `{ k: v }` literal in expression position when it cannot
be a block: an expression context only; a statement-level `{` stays a block;
`{}` empty is only legal with an expected map type), `src/hir.rs`
(`MapLookup`, `MapNew`, `MapInsert` for literals), `src/typeck.rs` (type
resolution hunk, `key()` detection next to `drop()`), `src/mir/**` and the
backends (builtin map calls, `drachm_map_ops` statics, glue), `src/diag/
codes.rs` (E0626 to E0629; E0626 is shared with C: whichever lands first
defines it), site pages, `docs/PLAN.md`.

Do not touch: `runtime/**` (report needed changes to S6R's author), lists
code beyond shared helpers, the borrow checker (a map is a move type with no
projections; `ages["Bob"]` is a call producing an owned `V?`... see below).

## What to build (one commit per step, `cargo test` green after each)

### Step 1. Type, layout, glue

`Type::Map`; size 48/8; owning; never Copy. `Map<K,V>` and `OrderedMap<K,V>`
resolve from the written generic names (reserved like `List`; a user struct
with these names is E0202). A key type must be hashable and comparable
(E0626 listing the field that is not: "`float64` cannot be a map key: `Pos`
has a `float64` field `x`"). Glue: `drop_map_<m>` / `copy_map_<m>` call the
runtime with the `ops`. The `drachm_map_ops` constant per `(K, V)` is emitted
by each backend as read-only data holding the sizes, the hash and eq function
pointers, and the glue pointers (Cranelift: a data object with relocations;
C: a `static const drachm_map_ops`).

### Step 2. Hash and equality synthesis; `key()`

Per key type, in the pattern of `auto_eq`: a HIR function `uint64 hash_K(&K)`
and `bool eq_K(&K, &K)` generated once and shared by every map of that key
type. Numbers/bool: `drachm_hash_u64` of the value; `string`: `drachm_str_hash`;
struct: fold fields in order with `drachm_hash_combine`; enum: tag then the
variant's payload; `Optional`: tag then payload; arrays: fold. Equality is the
existing automatic `==` (a user `==` is **not** consulted for a type
without `key()` except through Q13's rule: a type with a custom `==` and no
`key()` is rejected as a key with E0627 "defines `==` but no `key()`, so the
map cannot know how to hash it" — that is the conservative reading; report it
as Q13 data). With `key()`: `hash_K(&K k)` = hash of the `key()` result type,
`eq_K` = equality of the two `key()` results. Both functions get the symbol
`drachm_hash_<mangle K>` and `drachm_keq_<mangle K>`. Tests (unit): which
types are hashable, the function bodies for a two-field struct, a `key()` struct.

### Step 3. Literals, lookup, `get`, `len`

- `{ k: v, ... }` with an expected type `Map<K,V>`/`OrderedMap<K,V>`: lowered to
  `MapNew` then `MapInsert` per pair in order (later duplicates of a non-
  constant key replace earlier ones; duplicate **constant** keys are E0629).
  Keys/values convert to `K`/`V` like arguments.
- `m[k]`: HIR `MapLookup` → an **owned** `V?` rvalue: `Some(copy of the value)`
  or `none`. For Copy `V` that is a bit copy; for an owning `V` (`string`) it
  is a deep copy (`ages["x"]` on a `Map<string, string>` allocates): it is the
  only reading that fits "`[]` returns `V?`" with no borrows. Report that cost
  and note the borrowing alternative `&V?` as data for the owner; do not add
  syntax. `m::get(k)` is the same node. `len()` loads the header's `live`.
- `m[k]` with a key of the wrong type: left-side conversions; E0101.
- E0630-like errors reuse E0101. A map used before it has a value: E0207 (Q20).

### Step 4. Positions and benchmark

Maps in struct fields, in enum payloads, as generic arguments, maps of lists
(`Map<string, uint32[?]>`), lists of maps; moves; `copy m`. The `map` case of
`bench/collections.*` (5 million inserts/lookups — inserts via a literal are
not enough: the benchmark needs the runtime `insert`, so build it with the
temporary intrinsic `drachm_map_insert` exposed to tests only through a
compiler flag `--allow-map-mutation` that is **not** documented and is removed
before the commit that answers Q9). Report the table; if `map` is over 2x of
Rust, profile whether hash or probing dominates and say so.

## Tests

Run: `map_basic` (literal, lookup hit/miss, `maybe`), `map_struct_key`,
`map_enum_key`, `map_key_method`, `map_owning` (string values, leak-checked),
`map_nested` (map in map, list in map), `map_field`, `map_generic_holder`,
`map_copy_move`, `map_ordered_literal`, `map_len`, `map_many` (1000-entry
literal generated by the test file). Fail: E0626 (float key; struct with a float
field), E0627, E0628, E0629, E0624 (empty literal), E0101 (wrong key type),
E0901 for assignment through `[]` and for `for` over a map, borrows as keys.

## Site updates (same commit as each step; only what builds)

- `reference-collections.md` (created in S6C): add "Maps" (literal, lookup
  returns an optional, `maybe`, `get`, `len`, `OrderedMap`, what keys can be,
  `key()`) with examples `map-basic.drh`, `map-key.drh` (+ `.out`) and real
  E0626/E0629 output. Say plainly that changing a map and walking it are
  **not** supported yet (mark those two subsections **Not implemented yet**;
  they were `status: not-implemented` on the design page, move the markers).
- `design-collections.md`: delete "Arrays, lists and maps"'s map line and the
  map half of "Indexing"; leave "Walking" for maps marked.
- Lesson 16: no badge change (S6Z). Verify and copy to `site/examples/` the
  `age`/`maybe 0` blocks and the "look up with a fallback" solution
  (`solution-16-2.drh` + `.out`).
- `reference-types.md`: `Map`, `OrderedMap`. `reference-diagnostics.md`,
  `errors.md`, `coming-later.md`, `docs/PLAN.md` as in other packages.

## Done when

- `cargo test` green; the SPEC map snippets (`{ "Illy": 18 }`, `ages["Bob"]`,
  `ages["Bob"] maybe 0`) run on both backends, leak-free.
- Report: hashability table, the deep-copy cost of `[]` on owning values, the
  benchmark table, every interim with file and line, anything the runtime
  needs from S6R.

## Pitfalls

- The hash must be identical on both backends and across runs (fixed-key
  functions only; never the address of anything).
- `Optional<string>` keys: payload hashing must hash the tag first, so `none`
  and `Some("")` differ.
- The `ops` static must outlive every map (static storage); function pointers
  to glue in Cranelift need the glue declared before the data object.
- A map literal in a loop allocates each time: no hoisting; drop at scope end.
- `{` in expression position: a block-like statement `{ ... }` at statement
  start is a block, a map literal needs an expected type or a `:` after the
  first key expression; parser test both ways, plus `if (x) { }` unchanged.
- Struct keys by value: `ages[p]` with an owning struct key `p` must not move
  `p` (lookups borrow the key; only insert moves it).

## Language questions found

Q9, Q13, Q14, Q16, Q20 in `step6-overview.md`.
