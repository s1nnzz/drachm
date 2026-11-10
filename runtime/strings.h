// The string runtime's C interface: the contract between compiled Drachm
// code (both backends) and the Rust library in runtime/strings/.
//
// Pasted into generated C for -O builds, so it must stay plain C with no
// includes beyond <stdint.h>.
//
// Rules every function follows:
// - A string is UTF-8, never NUL-terminated, and always valid UTF-8.
// - `cap == 0` means the bytes are not owned (a literal in read-only data, or
//   the empty string with ptr == NULL): never free or write through `ptr`.
//   `cap > 0` means `ptr` came from libc `malloc` and holds `cap` bytes.
//   All allocation goes through libc malloc/realloc/free, on both sides.
// - `out` parameters point at uninitialised memory; the function writes a
//   complete value there. `out` never aliases an input.
// - Inputs are borrowed: the callee neither frees nor keeps them.
// - "Character" means grapheme cluster. Indexes count characters, are
//   0-based, and negative indexes count from the end (-1 = last). Ranges are
//   inclusive at both ends. An index outside the string panics through
//   drachm_panic("string index out of range") (exit 101), except where a
//   function says it clamps.
#ifndef DRACHM_STRINGS_H
#define DRACHM_STRINGS_H

typedef struct {
    uint8_t *ptr;
    uint64_t len; // bytes
    uint64_t cap; // 0 = not owned
} drachm_str;

// ---- lifetime ----
void drachm_str_drop(drachm_str *s);                            // frees if owned; leaves *s empty
void drachm_str_copy(drachm_str *out, const drachm_str *s);     // deep copy (a not-owned input may be shared)
void drachm_str_concat(drachm_str *out, const drachm_str *a, const drachm_str *b);

// Allocation counter for tests: live owned buffers. Printed to stderr at exit
// as "drachm: leaked N strings" when DRACHM_CHECK_LEAKS is set and N != 0.
uint64_t drachm_str_live(void);

// ---- comparison (by bytes) ----
uint8_t drachm_str_eq(const drachm_str *a, const drachm_str *b);
int32_t drachm_str_cmp(const drachm_str *a, const drachm_str *b); // -1, 0, 1

// ---- methods (SPEC section 4) ----
uint64_t drachm_str_len(const drachm_str *s);                   // characters
uint8_t drachm_str_is_empty(const drachm_str *s);
void drachm_str_reverse(drachm_str *out, const drachm_str *s);  // by character
void drachm_str_sub(drachm_str *out, const drachm_str *s, int64_t start, int64_t end);
void drachm_str_upper(drachm_str *out, const drachm_str *s);
void drachm_str_lower(drachm_str *out, const drachm_str *s);
void drachm_str_trim(drachm_str *out, const drachm_str *s);     // Unicode whitespace, both ends
int64_t drachm_str_find(const drachm_str *s, const drachm_str *text); // character index, -1 if missing
uint8_t drachm_str_contains(const drachm_str *s, const drachm_str *text);
uint8_t drachm_str_starts_with(const drachm_str *s, const drachm_str *text);
uint8_t drachm_str_ends_with(const drachm_str *s, const drachm_str *text);
uint64_t drachm_str_count(const drachm_str *s, const drachm_str *text); // non-overlapping; 0 for empty text
void drachm_str_replace(drachm_str *out, const drachm_str *s, const drachm_str *from, const drachm_str *to);
void drachm_str_replace_limit(drachm_str *out, const drachm_str *s, const drachm_str *from, const drachm_str *to,
                              uint64_t n);
void drachm_str_insert(drachm_str *out, const drachm_str *s, int64_t index, const drachm_str *text); // index == len appends
void drachm_str_remove(drachm_str *out, const drachm_str *s, int64_t start, int64_t end);
void drachm_str_repeat(drachm_str *out, const drachm_str *s, uint64_t n);
// Pad to `width` characters with `fill` repeated (cut to fit); a string
// already that wide is copied unchanged. An empty fill panics.
void drachm_str_pad_left(drachm_str *out, const drachm_str *s, uint64_t width, const drachm_str *fill);
void drachm_str_pad_right(drachm_str *out, const drachm_str *s, uint64_t width, const drachm_str *fill);

// ---- number to text in a base (for io::outf %x %X %b %o) ----
// The value arrives as two 64-bit halves, already sign- or zero-extended to
// 128 bits. A negative value prints as '-' followed by its magnitude.
void drachm_str_radix(drachm_str *out, uint64_t lo, uint64_t hi, uint8_t is_signed, uint32_t base, uint8_t upper);

#endif
