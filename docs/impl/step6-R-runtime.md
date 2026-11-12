# S6R: list and map runtime

Implementer: standard model. Rust only, in the existing string runtime crate;
no compiler change, no language change, no site change. It can start before
step 5 is finished. The compiler packages (S6C, S6D) call what you build, so the
C contract is the deliverable: write it first, review it against the packages
that use it, then implement.

Read first: `step6-overview.md` 2.4 and 4; `runtime/strings.h` (the contract
style: out parameters, `cap == 0`, panics through `drachm_panic`);
`runtime/strings/src/lib.rs` (the allocator, `drachm_str_alloc`,
`drachm_str_live`, the panic path, how `#[no_mangle] extern "C"` functions are
written); `build.rs`; `src/link.rs` (how `strings.h` is embedded and pasted
into generated C); `tests/leaks.rs`.

## Goal

1. `runtime/lists.h`: the C contract for lists, arrays' helper, and maps (below).
2. `runtime/strings/src/lists.rs` and `maps.rs` implementing it; the crate
   name stays, the header is embedded and pasted into `-O` C next to
   `strings.h`.
3. Rust unit tests (`cargo test --manifest-path runtime/strings/Cargo.toml`)
   that exercise every function with several element sizes (1, 3, 8, 24, 40
   bytes), alignments up to 16, growth across many reallocations, and drop /
   copy callbacks that count calls.

## Files you own

New: `runtime/lists.h`, `runtime/strings/src/lists.rs`, `maps.rs`,
`hash.rs`. Edit: `runtime/strings/src/lib.rs` (`mod` lines, one hash export),
`build.rs` (`rerun-if-changed` for the header), `src/link.rs` (embed and paste
the header after `strings.h`: the same two-line change as `strings.h` has;
find it by grepping `strings.h`), `runtime/strings/Cargo.toml` only if a crate
is truly needed (prefer none; write the table yourself).

Do not touch: the compiler (`src/typeck*`, `src/mir`, backends), `runtime.c`,
the site, docs other than a header comment.

## The contract (write `lists.h` with these, same comment style as `strings.h`)

```c
typedef struct { uint8_t *ptr; uint64_t len; uint64_t cap; } drachm_list;  // a string-shaped header
typedef void    (*drachm_drop_fn)(void *elem);                  // drop one element in place
typedef void    (*drachm_copy_fn)(void *dst, const void *src);  // deep-copy one element into uninitialised dst
typedef uint64_t (*drachm_hash_fn)(const void *key);
typedef uint8_t  (*drachm_eq_fn)(const void *a, const void *b);

// ---- lists (size = element size in bytes, align = its alignment, 1..16, power of 2) ----
void  drachm_list_new(drachm_list *out);                                        // empty, cap 0, ptr NULL
void  drachm_list_with_cap(drachm_list *out, uint64_t cap, uint64_t size, uint64_t align);
void  drachm_list_push(drachm_list *l, const void *elem, uint64_t size, uint64_t align); // bytes moved in
void *drachm_list_at(const drachm_list *l, uint64_t i, uint64_t size);          // panics: index out of range
void  drachm_index_panic(uint64_t i, uint64_t len);                              // the one panic text, used by compiled bounds checks
uint8_t drachm_list_pop(drachm_list *l, void *out, uint64_t size);              // 0 when empty
void  drachm_list_insert(drachm_list *l, uint64_t i, const void *elem, uint64_t size, uint64_t align); // i <= len else panic
void  drachm_list_remove(drachm_list *l, uint64_t i, void *out, uint64_t size);  // i < len else panic; shifts down
void  drachm_list_clear(drachm_list *l, uint64_t size, drachm_drop_fn drop_elem);
void  drachm_list_drop(drachm_list *l, uint64_t size, drachm_drop_fn drop_elem); // drops each element, frees, leaves empty
void  drachm_list_copy(drachm_list *out, const drachm_list *l, uint64_t size, uint64_t align, drachm_copy_fn copy_elem); // NULL = memcpy
```

Panic text (both backends use `drachm_index_panic`): `index 5 is outside the
list (length 3)`, exit 101, via `drachm_panic`.

```c
// ---- maps: insertion-ordered table, type-erased ----
typedef struct { uint8_t *entries; uint32_t *index; uint64_t len; uint64_t cap; uint64_t live; uint64_t mask; } drachm_map; // 48 bytes
typedef struct {
    uint64_t ksize, kalign, vsize, valign;      // vsize 0 allowed (sets)
    drachm_hash_fn hash; drachm_eq_fn eq;
    drachm_drop_fn drop_key, drop_val;          // NULL = plain data
    drachm_copy_fn copy_key, copy_val;          // NULL = memcpy
} drachm_map_ops;                               // a static const per (K, V), emitted by the compiler
void   drachm_map_new(drachm_map *out);
void  *drachm_map_find(const drachm_map *m, const void *key, const drachm_map_ops *ops);    // pointer to the VALUE or NULL
uint8_t drachm_map_insert(drachm_map *m, void *key, void *val, const drachm_map_ops *ops);  // moves key and val in; if the key exists drops the NEW key, replaces and drops the OLD value; returns 1 if the key was new
uint8_t drachm_map_remove(drachm_map *m, const void *key, void *out_val, const drachm_map_ops *ops); // moves the value out; drops the stored key
uint64_t drachm_map_len(const drachm_map *m);
uint8_t drachm_map_next(const drachm_map *m, uint64_t *cursor, void **key, void **val, const drachm_map_ops *ops); // 0 at the end
void   drachm_map_drop(drachm_map *m, const drachm_map_ops *ops);
void   drachm_map_copy(drachm_map *out, const drachm_map *m, const drachm_map_ops *ops);

// ---- hashing helpers the compiler's synthesised hash functions call ----
uint64_t drachm_hash_u64(uint64_t x);                    // a fixed mixer (splitmix64 finaliser)
uint64_t drachm_hash_combine(uint64_t h, uint64_t x);
uint64_t drachm_hash_bytes(const uint8_t *p, uint64_t n);// fixed-key, deterministic across runs and platforms
uint64_t drachm_str_hash(const drachm_str *s);
```

## What to build (one commit per step, `cargo test` of the crate green)

1. The header and a Rust module `lists.rs`. Growth x2 from 4; every allocation
   through the counted allocator of the string crate (`drachm_str_alloc`-style
   function; add a `pub(crate)` over-aligned variant if alignment > 8 is needed
   and keep the live counter exact). Insertion/removal use `ptr::copy`. Zero
   sized elements: `size == 0` is legal (a list of `none`-like structs); `len`
   still counts and nothing is allocated.
2. `maps.rs`: entries vector (`key bytes | value bytes` per slot, each
   aligned), index table of `u32` positions (open addressing, linear probing,
   load <= 0.5, power-of-two capacity, empty and deleted markers), removal marks
   the entry dead (so iteration order of the rest is unchanged) and a rebuild
   compacts when dead entries exceed half. `next` skips dead entries.
   `len == live`. Keys compare through `ops.eq` after `ops.hash` equal.
3. Hash helpers: deterministic and portable (no `std` `RandomState`). The same
   64-bit result for the same bytes on every run and both backends.
4. The leak counter: every list buffer, map entries buffer and index buffer is
   counted exactly like a string buffer, so `DRACHM_CHECK_LEAKS=1` sees them.
   The message keeps its words; add a Rust test that a dropped list and map
   bring `drachm_str_live()` back to 0.
5. `src/link.rs` / `build.rs`: the header ships with the compiler; a compile
   test in `tests/` is not needed (S6C.1 is the first user), but run `cargo
   build` once and check `lists.h` is pasted by looking at what `-O` writes
   for an existing program with `--emit-c` or the equivalent flag (see
   `src/flags.rs`).

## Tests

Rust unit tests in `runtime/strings/src/` (modules `lists::tests`,
`maps::tests`): push/pop/insert/remove with sizes 1, 3, 8, 24, 40; `at` panics
(use `catch_unwind` around the Rust-side function that the `extern` wraps, or
test the checked inner function); drop callback called exactly once per live
element and zero times for removed-out ones; copy callback; map: 100 000
inserts, finds, removes, re-inserts, iteration order equals insertion order
with removals skipped, replacing an existing key drops the new key and the old
value, a key type with a custom `eq` (equal-by-case strings), growth across the
index rebuild, `drachm_map_copy` independence.

## Done when

- `cargo test --manifest-path runtime/strings/Cargo.toml` passes (also under
  `miri` if available: report if not; do not install anything).
- `cargo build` at the top level succeeds and the header is embedded; no
  existing golden test changes.
- Report: the final struct layouts (`sizeof drachm_map` must be 48), the hash
  function used, any deviation from the contract (S6C.1 and S6D are written
  against it).

## Pitfalls

- A list header must have the same layout as `drachm_str` (the compiler treats
  a list as 24 bytes `{ptr, len, cap}`), and `cap == 0` with a non-NULL `ptr`
  must never happen (it would look not-owned).
- Alignment above 8: `malloc` gives 16 on 64-bit Linux; larger alignments are
  not needed (`uint128` is 16).
- Callbacks run user `drop()` methods that may call back into the runtime
  (dropping a list of lists): do not hold a Rust borrow of the list across the
  callback; read the pointer and length first.
- Do not use `Vec`/`HashMap` inside the runtime for the user data: the
  compiler's glue relies on exact layout and on the single counted allocator.
- Panics must go through `drachm_panic` (exit 101, no unwinding across
  `extern "C"`).
