// The list and map runtime's C interface: the contract between compiled
// Drachm code (both backends) and the Rust library in runtime/strings/
// (src/lists.rs, src/maps.rs, src/hash.rs).
//
// Pasted into generated C for -O builds right after strings.h (the join
// function uses drachm_str), so it must stay plain C with no includes beyond
// <stdint.h>, and it must come after strings.h.
//
// Rules every function follows:
// - Elements are raw bytes, moved and copied as bytes. Every function that
//   takes an element gives its size in bytes and, where it allocates, its
//   alignment (a power of 2, 1..16). Glue callbacks come from the compiler;
//   NULL means plain data: no drop, and copy is memcpy.
// - A list is {ptr, len, cap}, the same shape as a string. `cap == 0` means
//   not owned: never free or write through `ptr`. `cap > 0` means `ptr` came
//   from libc malloc and holds `cap` elements. An empty list is all zero
//   (ptr NULL, len 0, cap 0) and is usable at once. Zero-sized elements never
//   allocate: cap and ptr stay 0 and NULL, len still counts.
// - `out` parameters point at uninitialised memory; the function writes a
//   complete value there and never reads it. `out` never aliases an input.
// - Inputs are borrowed (the callee neither frees nor keeps them), except an
//   element pointer given as "moved in": its bytes are taken and the caller
//   must not drop them afterwards.
// - Index parameters are int64. A negative index counts from the end (-1 is
//   the last item), for lists and for arrays. An index outside the list (after
//   that adjustment) panics through drachm_index_panic (exit 101).
// - Callbacks (drop, copy, hash, eq) may run user code that calls back into
//   the runtime for other values. A callback must not touch the collection it
//   is an element of; the runtime reads its own pointers and lengths before
//   each callback.
// - A pointer returned by a lookup points into the collection's own buffer. It
//   is valid until the next call that may move elements (push, insert, remove,
//   insert into a map, and so on).
#ifndef DRACHM_LISTS_H
#define DRACHM_LISTS_H

typedef struct {
    uint8_t *ptr;
    uint64_t len; // elements
    uint64_t cap; // elements; 0 = not owned
} drachm_list;

typedef void (*drachm_drop_fn)(void *elem);                    // drops one element in place
typedef void (*drachm_copy_fn)(void *dst, const void *src);    // deep-copies one element into uninitialised dst
typedef uint64_t (*drachm_hash_fn)(const void *key);
typedef uint8_t (*drachm_eq_fn)(const void *a, const void *b); // 1 when equal

// ---- indexes (lists and arrays) ----
// Turns `i` into an index in 0..len-1: a negative `i` counts from the end.
// Panics "index 5 is outside the list (length 3)" when there is no such index.
uint64_t drachm_list_index(uint64_t len, int64_t i);
// The panic itself, for compiled bounds checks. Does not return.
void drachm_index_panic(int64_t i, uint64_t len) __attribute__((noreturn, cold));
// The same for an index that was an unsigned number (it may be 2^63 or more).
void drachm_index_panic_u(uint64_t i, uint64_t len) __attribute__((noreturn, cold));

// ---- lists ----
void drachm_list_new(drachm_list *out);                                        // empty: ptr NULL, len 0, cap 0
void drachm_list_with_cap(drachm_list *out, uint64_t cap, uint64_t size, uint64_t align); // empty, room for cap elements
void drachm_list_copy(drachm_list *out, const drachm_list *l, uint64_t size, uint64_t align, drachm_copy_fn copy_elem);
void drachm_list_push(drachm_list *l, const void *elem, uint64_t size, uint64_t align);   // elem moved in; grows x2
// Pointer to element `i`, panics when there is no such index.
void *drachm_list_at(const drachm_list *l, int64_t i, uint64_t size);
// Pointer to element `i`, or NULL when there is no such index (never panics).
void *drachm_list_get(const drachm_list *l, int64_t i, uint64_t size);
// Moves the last element into `out` and shrinks. Returns 0 (and leaves `out`
// alone) when the list is empty. `out` must not be NULL.
uint8_t drachm_list_pop(drachm_list *l, void *out, uint64_t size);
// elem moved in at index `i`, the items from there on move up. i == len appends.
void drachm_list_insert(drachm_list *l, int64_t i, const void *elem, uint64_t size, uint64_t align);
// Moves element `i` into `out`, the items after it move down (order kept).
void drachm_list_remove(drachm_list *l, int64_t i, void *out, uint64_t size);
// Moves element `i` into `out`, moves the last element into its slot and
// shrinks (order not kept). Costs one move.
void drachm_list_swap_remove(drachm_list *l, int64_t i, void *out, uint64_t size);
// `take xs[i] leaving v`: element `i` moves into `out`, `elem` (moved in) takes
// its place. The length and the order do not change.
void drachm_list_replace(drachm_list *l, int64_t i, const void *elem, void *out, uint64_t size);
// Index of the first element equal to `*v` (by `eq`), or -1 when none is.
int64_t drachm_list_index_of(const drachm_list *l, const void *v, uint64_t size, drachm_eq_fn eq);
// Drops every element in order and keeps the buffer; the list ends with len 0.
void drachm_list_clear(drachm_list *l, uint64_t size, drachm_drop_fn drop_elem);
// Drops every element in order, frees the buffer and leaves the list empty.
void drachm_list_drop(drachm_list *l, uint64_t size, drachm_drop_fn drop_elem);
// For a list of strings: the elements joined with `sep` between them. An empty
// list gives the empty string.
void drachm_list_join(drachm_str *out, const drachm_list *l, const drachm_str *sep);

// ---- string splitting (runtime/strings/src/strings_split.rs) ----
// Each gives a new list of owned strings, replacing whatever `out` held.
void drachm_str_split(drachm_list *out, const drachm_str *s, const drachm_str *sep); // empty sep gives characters
void drachm_str_lines(drachm_list *out, const drachm_str *s);
void drachm_str_chars(drachm_list *out, const drachm_str *s);

// ---- maps: insertion-ordered table, type-erased ----
// `entries` holds one slot per insertion (dead slots included until a rebuild),
// `index` maps hash positions to slots. A zero map is an empty map.
typedef struct {
    uint8_t *entries;
    uint32_t *index;
    uint64_t len;   // slots used, dead ones included
    uint64_t cap;   // slots allocated
    uint64_t live;  // entries in the map (what drachm_map_len returns)
    uint64_t mask;  // index slots - 1 (index slots are a power of 2)
} drachm_map; // 48 bytes

// One per (K, V) pair, emitted by the compiler as a static const.
// vsize 0 is a set (no value). Hash and eq are required; the rest may be NULL.
typedef struct {
    uint64_t ksize, kalign, vsize, valign;
    drachm_hash_fn hash;
    drachm_eq_fn eq;
    drachm_drop_fn drop_key, drop_val;
    drachm_copy_fn copy_key, copy_val;
} drachm_map_ops;

void drachm_map_new(drachm_map *out);                                    // empty
// Pointer to the value stored under `key`, or NULL. For a set, a non-NULL
// result means the key is there (its value bytes are zero-sized).
void *drachm_map_find(const drachm_map *m, const void *key, const drachm_map_ops *ops);
// key and val are moved in (val NULL when vsize is 0). A new key is stored
// and the call returns 1. An existing key keeps its slot: the NEW key is
// dropped, the OLD value is dropped and replaced by the new one, and the call
// returns 0.
uint8_t drachm_map_insert(drachm_map *m, void *key, void *val, const drachm_map_ops *ops);
// Removes `key` if present; returns 1 then. The stored key is dropped. The
// value is moved into out_val, or dropped when out_val is NULL.
uint8_t drachm_map_remove(drachm_map *m, const void *key, void *out_val, const drachm_map_ops *ops);
uint64_t drachm_map_len(const drachm_map *m);                            // live entries
// The panic of `m[k] += x` when k is not in the map. Does not return.
void drachm_map_key_panic(void) __attribute__((noreturn, cold));
// Walks the entries in insertion order, skipping removed ones. Start with
// *cursor == 0. Returns 1 and the key and value pointers for the next entry
// (*cursor moves past it), or 0 at the end. The pointers point into the map
// and are valid until the map changes. For a set, *val is set but not read.
uint8_t drachm_map_next(const drachm_map *m, uint64_t *cursor, void **key, void **val, const drachm_map_ops *ops);
void drachm_map_drop(drachm_map *m, const drachm_map_ops *ops);          // drops every key and value, frees all, leaves empty
void drachm_map_copy(drachm_map *out, const drachm_map *m, const drachm_map_ops *ops); // independent deep copy, same order

// ---- hashing: the helpers the compiler's synthesised hash functions call ----
// Deterministic and portable: the same input gives the same 64-bit result on
// every run, on both backends and on every platform (no per-process seed).
uint64_t drachm_hash_u64(uint64_t x);                                    // splitmix64 finaliser
uint64_t drachm_hash_combine(uint64_t h, uint64_t x);
uint64_t drachm_hash_bytes(const uint8_t *p, uint64_t n);
uint64_t drachm_str_hash(const drachm_str *s);                           // by bytes, like drachm_str_eq

#endif
