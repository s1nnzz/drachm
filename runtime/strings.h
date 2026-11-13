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

void drachm_sleep(double seconds);

typedef struct {
    uint8_t *ptr;
    uint64_t len; // bytes
    uint64_t cap; // 0 = not owned
} drachm_str;

// FS returns zero on success, otherwise a cause code. Read and getEnv
// write the output only on success.
uint8_t drachm_fs_read(const drachm_str *path, drachm_str *out);
uint8_t drachm_fs_write(const drachm_str *path, const drachm_str *value);
void drachm_os_platform(drachm_str *out);
// The text of a C string (up to its NUL), checked as UTF-8: zero on success,
// 1 for a null pointer or bytes that are not UTF-8 (nothing is written).
uint8_t drachm_c_text(const uint8_t *p, drachm_str *out);
// The bytes of a string whose last byte is the terminating NUL (`&s as c
// string`); a NUL anywhere else is a runtime error.
uint8_t *drachm_str_c_ptr(const drachm_str *s);
uint8_t drachm_os_get_env(const drachm_str *key, drachm_str *out);

// ---- lifetime ----
void drachm_str_drop(drachm_str *s);                            // frees if owned; leaves *s empty
void drachm_str_copy(drachm_str *out, const drachm_str *s);     // deep copy (a not-owned input may be shared)
void drachm_str_concat(drachm_str *out, const drachm_str *a, const drachm_str *b);
void drachm_str_join(drachm_str *out, const drachm_str *parts, uint64_t n); // the n strings of `parts`, in order
// Append `t` to `s` in place (the one exception to "out never aliases an
// input": `t` may be `s`). `s` may be not owned; it is owned afterwards
// unless `t` is empty. Capacity grows geometrically, so appending in a loop
// is linear overall.
void drachm_str_append(drachm_str *s, const drachm_str *t);

// Allocation counter for tests: live owned buffers. Printed to stderr at exit
// as "drachm: leaked N strings" when DRACHM_CHECK_LEAKS is set and N != 0.
uint64_t drachm_str_live(void);
// Allocates a counted owned buffer of `cap` bytes for C code that builds strings (cap 0 returns NULL).
uint8_t *drachm_str_alloc(uint64_t cap);

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

// ---- characters for `for`, and globs for `match` ----
uint64_t drachm_str_byte_len(const drachm_str *s);
// The character that starts at byte `pos` (a character boundary), owned or a
// static one-byte string; empty when `pos` is at or past the end.
void drachm_str_next_char(drachm_str *out, const drachm_str *s, uint64_t pos);
// The same character as a borrowed view into `s` (cap 0, never freed).
void drachm_str_char_view(drachm_str *out, const drachm_str *s, uint64_t pos);
// `*` in `pattern` matches any run of bytes, empty included; the whole value must match.
uint8_t drachm_str_glob(const drachm_str *s, const drachm_str *pattern);

// ---- input and text to number (the interim rules of the language) ----
// Input lines arrive as bytes: any invalid UTF-8 becomes U+FFFD.
void drachm_str_from_bytes(drachm_str *out, const uint8_t *p, uint64_t n);
// Parse the whole text. Each returns 1 and the value, or 0 for bad text.
// Integers: the 64-bit halves of the value, sign-extended for signed kinds.
uint8_t drachm_str_parse_int(const drachm_str *s, uint8_t is_signed, uint32_t bits, uint64_t *lo, uint64_t *hi);
uint8_t drachm_str_parse_f64(const drachm_str *s, double *out);
uint8_t drachm_str_parse_f32(const drachm_str *s, float *out);

// ---- number to text in a base (for io::outf %x %X %b %o) ----
// The value arrives as two 64-bit halves, already sign- or zero-extended to
// 128 bits. A negative value prints as '-' followed by its magnitude.
void drachm_str_radix(drachm_str *out, uint64_t lo, uint64_t hi, uint8_t is_signed, uint32_t base, uint8_t upper);

#endif
