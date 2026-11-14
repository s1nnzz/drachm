// Drachm runtime: what compiled programs call for printing, panics, turning
// values into text, and the few operations that are too big to emit inline.
// String operations themselves are in the Rust library (strings.h).
//
// Plain builds compile this file on its own and link it. Optimised (-O)
// builds paste it at the top of the generated C with DRACHM_INLINE defined,
// so that the C compiler can inline every helper.
//
// Both backends must behave identically, so anything with room for
// disagreement (float to integer wrapping, 128-bit conversions, powers,
// number and float text) lives here and is shared.
#include <inttypes.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "strings.h"

#ifdef _WIN32
#include <fcntl.h>
#include <io.h>

// The C library on Windows turns "\n" into "\r\n" on text streams. Drachm
// programs write exactly the bytes they print, as they do everywhere else.
// Input is binary too: a line's "\r\n" arrives as it is, and drachm_in_line
// removes the "\r" itself, the same on every system.
__attribute__((constructor, unused)) static void drachm_binary_stdio(void) {
    _setmode(_fileno(stdin), _O_BINARY);
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
}
#define drachm_isatty(fd) _isatty(fd)
#else
#include <unistd.h>
#define drachm_isatty(fd) isatty(fd)
#endif

#ifdef DRACHM_INLINE
#define DRACHM_FN static inline __attribute__((unused))
#else
#define DRACHM_FN
#endif

typedef unsigned __int128 drachm_u128;
typedef __int128 drachm_s128;

// A 128-bit value passed as two halves, which every C ABI agrees on.
typedef struct {
    uint64_t lo, hi;
} drachm_pair;

static inline drachm_u128 drachm_join(uint64_t lo, uint64_t hi) { return ((drachm_u128)hi << 64) | lo; }

static inline drachm_pair drachm_split(drachm_u128 v) {
    drachm_pair p = {(uint64_t)v, (uint64_t)(v >> 64)};
    return p;
}

// ---- panics ---------------------------------------------------------------

// Never static: the string library (Rust) calls it too. Exits with _Exit so
// that no exit handlers run: a panic is not a leak.
_Noreturn void drachm_panic(const char *msg) {
    fflush(stdout);
    fprintf(stderr, "drachm: panic: %s\n", msg);
    _Exit(101);
}

DRACHM_FN _Noreturn void drachm_div_zero(void) { drachm_panic("division by zero"); }

// A range loop whose `by` step is zero at run time (the step never moves on).
DRACHM_FN _Noreturn void drachm_range_step_zero(void) { drachm_panic("range step is zero"); }

// A failed `assert`: `msg` is the message (the condition's text and its place).
DRACHM_FN _Noreturn void drachm_assert_failed(const uint8_t *msg, uint64_t len) {
    fflush(stdout);
    fprintf(stderr, "drachm: panic: %.*s\n", (int)len, (const char *)msg);
    _Exit(101);
}

// ---- output ---------------------------------------------------------------

DRACHM_FN void drachm_out_str(const char *s, uint64_t len, int newline) {
    if (len) fwrite(s, 1, (size_t)len, stdout);
    if (newline) putc('\n', stdout);
}

// ---- values as text -------------------------------------------------------
//
// Each writes a new string to `out`, following the rules in strings.h. Owned
// buffers come from drachm_str_alloc so the live-string counter stays right.

static void drachm_text_bytes(drachm_str *out, const char *s, size_t len) {
    uint8_t *p = drachm_str_alloc(len);
    if (len) memcpy(p, s, len);
    out->ptr = p;
    out->len = len;
    out->cap = len;
}

DRACHM_FN void drachm_text_bool(drachm_str *out, uint8_t v) {
    // Not owned: the text lives in read-only data.
    out->ptr = (uint8_t *)(v ? "true" : "false");
    out->len = v ? 4 : 5;
    out->cap = 0;
}

DRACHM_FN void drachm_text_u128(drachm_str *out, uint64_t lo, uint64_t hi) {
    drachm_u128 v = drachm_join(lo, hi);
    char buf[40];
    char *p = buf + sizeof buf;
    do {
        *--p = (char)('0' + (int)(v % 10));
        v /= 10;
    } while (v != 0);
    drachm_text_bytes(out, p, (size_t)(buf + sizeof buf - p));
}

DRACHM_FN void drachm_text_s128(drachm_str *out, uint64_t lo, uint64_t hi) {
    drachm_u128 v = drachm_join(lo, hi);
    int negative = hi >> 63;
    if (negative) v = 0 - v;
    char buf[41];
    char *p = buf + sizeof buf;
    do {
        *--p = (char)('0' + (int)(v % 10));
        v /= 10;
    } while (v != 0);
    if (negative) *--p = '-';
    drachm_text_bytes(out, p, (size_t)(buf + sizeof buf - p));
}

DRACHM_FN void drachm_text_u64(drachm_str *out, uint64_t v) { drachm_text_u128(out, v, 0); }

DRACHM_FN void drachm_text_s64(drachm_str *out, int64_t v) { drachm_text_s128(out, (uint64_t)v, v < 0 ? ~UINT64_C(0) : 0); }

// Floats print as the shortest digits that read back as the same number,
// always with a decimal point and never in exponent form. Writes at most 400
// bytes to `out` and returns the length.
static int drachm_float_text(char *out, double v, int single) {
    if (v != v) {
        memcpy(out, "nan", 3);
        return 3;
    }
    if (v - v != 0) {
        if (v < 0) {
            memcpy(out, "-inf", 4);
            return 4;
        }
        memcpy(out, "inf", 3);
        return 3;
    }
    // Shortest scientific form that round-trips: "[-]d.ddde[+-]xx".
    char sci[40];
    for (int prec = 0; prec < 17; prec++) {
        snprintf(sci, sizeof sci, "%.*e", prec, v);
        if (single ? strtof(sci, NULL) == (float)v : strtod(sci, NULL) == v) break;
    }
    char digits[24];
    int n = 0;
    const char *p = sci;
    int negative = *p == '-';
    if (negative) p++;
    for (; *p != 'e'; p++)
        if (*p != '.') digits[n++] = *p;
    int exp = atoi(p + 1);

    int len = 0;
    if (negative) out[len++] = '-';
    if (exp >= 0) {
        for (int i = 0; i <= exp; i++) out[len++] = i < n ? digits[i] : '0';
        out[len++] = '.';
        if (n > exp + 1)
            for (int i = exp + 1; i < n; i++) out[len++] = digits[i];
        else out[len++] = '0';
    } else {
        out[len++] = '0';
        out[len++] = '.';
        for (int i = 1; i < -exp; i++) out[len++] = '0';
        for (int i = 0; i < n; i++) out[len++] = digits[i];
    }
    return len;
}

DRACHM_FN void drachm_text_f64(drachm_str *out, double v) {
    char buf[400];
    drachm_text_bytes(out, buf, (size_t)drachm_float_text(buf, v, 0));
}

DRACHM_FN void drachm_text_f32(drachm_str *out, float v) {
    char buf[400];
    drachm_text_bytes(out, buf, (size_t)drachm_float_text(buf, (double)v, 1));
}

// io::outf's `%.N`: exactly `places` decimals, correctly rounded from the
// binary value. NaN and the infinities print as they do in text.
DRACHM_FN void drachm_text_fixed(drachm_str *out, double v, uint32_t places) {
    if (v != v || v - v != 0) {
        char buf[8];
        drachm_text_bytes(out, buf, (size_t)drachm_float_text(buf, v, 0));
        return;
    }
    int len = snprintf(NULL, 0, "%.*f", (int)places, v);
    if (len < 0) drachm_panic("number too long to format");
    uint8_t *p = drachm_str_alloc((uint64_t)len + 1);
    snprintf((char *)p, (size_t)len + 1, "%.*f", (int)places, v);
    out->ptr = p;
    out->len = (uint64_t)len;
    out->cap = (uint64_t)len + 1;
}

// io::outf's `%0N`: number text (decimal or another base) padded on the left
// to `width` characters with zeros after any sign. `inf` and `nan` are padded
// with spaces instead, as C's printf does.
DRACHM_FN void drachm_text_zero_pad(drachm_str *out, const drachm_str *s, uint64_t width) {
    uint64_t len = s->len;
    if (len >= width) {
        drachm_text_bytes(out, (const char *)s->ptr, (size_t)len);
        return;
    }
    uint64_t sign = len > 0 && s->ptr[0] == '-';
    int digits = len > sign && s->ptr[sign] != 'i' && s->ptr[sign] != 'n';
    uint64_t pad = width - len;
    uint8_t *p = drachm_str_alloc(width);
    if (digits) {
        memcpy(p, s->ptr, (size_t)sign);
        memset(p + sign, '0', (size_t)pad);
        memcpy(p + sign + pad, s->ptr + sign, (size_t)(len - sign));
    } else {
        memset(p, ' ', (size_t)pad);
        if (len) memcpy(p + pad, s->ptr, (size_t)len);
    }
    out->ptr = p;
    out->len = width;
    out->cap = width;
}

// ---- input ----------------------------------------------------------------
//
// Standard input is read here, with C stdio, for both backends.

// When standard input is a terminal, a prompt written with io::out must show
// before the program waits for a line. Pipes and files are not flushed, so a
// program that reads a lot stays fast.
static void drachm_flush_for_read(void) {
    static int terminal = -1;
    if (terminal < 0) terminal = drachm_isatty(0) ? 1 : 0;
    if (terminal) fflush(stdout);
}

// Reads one line into `out` (owned, or the empty not-owned string at the end
// of input). The line ends at "\n", which is not kept; a "\r" just before it
// is not kept either. A last line without a newline is a line.
DRACHM_FN void drachm_in_line(drachm_str *out) {
    drachm_flush_for_read();
    char *buf = NULL;
    size_t len = 0, cap = 0;
    int c, newline = 0;
    while ((c = getc(stdin)) != EOF) {
        if (c == '\n') {
            newline = 1;
            break;
        }
        if (len == cap) {
            size_t grown_cap = cap ? cap * 2 : 128;
            char *grown = realloc(buf, grown_cap);
            if (!grown) {
                free(buf);
                drachm_panic("out of memory");
            }
            buf = grown;
            cap = grown_cap;
        }
        buf[len++] = (char)c;
    }
    if (newline && len && buf[len - 1] == '\r') len--;
    if (len == 0) {
        out->ptr = NULL;
        out->len = 0;
        out->cap = 0;
    } else {
        drachm_str_from_bytes(out, (const uint8_t *)buf, len);
    }
    free(buf);
}

// 1 when no input is left. Peeks one byte, which stays for the next read.
DRACHM_FN uint8_t drachm_in_ended(void) {
    drachm_flush_for_read();
    int c = getc(stdin);
    if (c == EOF) return 1;
    ungetc(c, stdin);
    return 0;
}

// ---- text to number ---------------------------------------------------------

// The panic for text that is not a number of the type named. The text is
// shown up to 40 bytes (cut at a character boundary), control bytes as '?'.
_Noreturn static void drachm_parse_failed(const drachm_str *s, const char *type_name) {
    char msg[160];
    size_t n = (size_t)snprintf(msg, sizeof msg, "cannot read \"");
    size_t shown = s->len;
    if (shown > 40) {
        shown = 40;
        while (shown > 0 && (s->ptr[shown] & 0xC0) == 0x80) shown--;
    }
    for (size_t i = 0; i < shown; i++) {
        uint8_t c = s->ptr[i];
        msg[n++] = (c < 0x20 || c == 0x7f) ? '?' : (char)c;
    }
    if (shown < s->len) {
        memcpy(msg + n, "...", 3);
        n += 3;
    }
    snprintf(msg + n, sizeof msg - n, "\" as %s", type_name);
    drachm_panic(msg);
}

// `s` as an integer of `bits` bits (sint or uint); panics on bad text.
DRACHM_FN drachm_pair drachm_parse_int(const drachm_str *s, uint8_t is_signed, uint32_t bits) {
    drachm_pair p;
    if (!drachm_str_parse_int(s, is_signed, bits, &p.lo, &p.hi)) {
        char name[16];
        snprintf(name, sizeof name, "%s%u", is_signed ? "sint" : "uint", (unsigned)bits);
        drachm_parse_failed(s, name);
    }
    return p;
}

// `s` as a float of `bits` bits; panics on bad text.
DRACHM_FN double drachm_parse_float(const drachm_str *s, uint32_t bits) {
    if (bits == 32) {
        float f;
        if (!drachm_str_parse_f32(s, &f)) drachm_parse_failed(s, "float32");
        return f;
    }
    double d;
    if (!drachm_str_parse_f64(s, &d)) drachm_parse_failed(s, "float64");
    return d;
}

// How the text `s` reads as a number, for `::to<T>()` (the parse itself is
// drachm_parse_int / drachm_parse_float): 0 when it reads fine, 1 when it is
// empty (only whitespace), 2 when it is not a number, 3 when it is a number
// the type cannot hold. `kind` is 0 for integers, 1 for float32, 2 for float64.
static int drachm_text_space(uint8_t c) { return c == ' ' || (c >= 9 && c <= 13); }

DRACHM_FN uint8_t drachm_parse_status(const drachm_str *s, uint8_t kind, uint8_t is_signed, uint32_t bits) {
    uint64_t lo, hi;
    double d;
    float f;
    if (kind == 0 ? drachm_str_parse_int(s, is_signed, bits, &lo, &hi) : kind == 1 ? drachm_str_parse_f32(s, &f) : drachm_str_parse_f64(s, &d)) return 0;
    size_t a = 0, b = (size_t)s->len;
    while (a < b && drachm_text_space(s->ptr[a])) a++;
    while (b > a && drachm_text_space(s->ptr[b - 1])) b--;
    if (a == b) return 1;
    size_t i = a;
    if (s->ptr[i] == '-' || s->ptr[i] == '+') i++;
    size_t digits = 0;
    while (i < b && s->ptr[i] >= '0' && s->ptr[i] <= '9') i++, digits++;
    if (kind == 0) return (i == b && digits > 0) ? 3 : 2;
    // A float that reads fine as text but not as a finite number overflowed.
    size_t after = 0;
    if (i < b && s->ptr[i] == '.') {
        i++;
        while (i < b && s->ptr[i] >= '0' && s->ptr[i] <= '9') i++, after++;
    }
    if (digits + after == 0) return 2;
    if (i < b && (s->ptr[i] == 'e' || s->ptr[i] == 'E')) {
        i++;
        if (i < b && (s->ptr[i] == '+' || s->ptr[i] == '-')) i++;
        size_t exp = 0;
        while (i < b && s->ptr[i] >= '0' && s->ptr[i] <= '9') i++, exp++;
        if (exp == 0) return 2;
    }
    return i == b ? 3 : 2;
}

// ---- failure causes -----------------------------------------------------------
//
// A failure carries a hidden word: 0, or a pointer to the first link of a
// chain of messages (`fail X because e;`), newest first. Links and their
// messages are counted allocations, so DRACHM_CHECK_LEAKS covers them.

typedef struct drachm_cause {
    drachm_str msg;
    uint64_t next;
} drachm_cause;

DRACHM_FN void drachm_cause_drop(uint64_t chain) {
    while (chain) {
        drachm_cause *c = (drachm_cause *)(uintptr_t)chain;
        uint64_t next = c->next;
        drachm_str_drop(&c->msg);
        drachm_str node = {(uint8_t *)c, 0, sizeof(drachm_cause)};
        drachm_str_drop(&node);
        chain = next;
    }
}

// A new link holding a copy of `msg`, in front of the chain in `*slot`, which
// moves into it (the slot is emptied).
DRACHM_FN uint64_t drachm_cause_wrap(const drachm_str *msg, uint64_t *slot) {
    drachm_cause *c = (drachm_cause *)drachm_str_alloc(sizeof(drachm_cause));
    drachm_str_copy(&c->msg, msg);
    c->next = *slot;
    *slot = 0;
    return (uint64_t)(uintptr_t)c;
}

// The report of a failure that left `main`, on standard error:
//   error: <message>
//     because: <cause>
// one `because` line for each link. Frees the chain in `*slot`.
DRACHM_FN void drachm_report(const drachm_str *msg, uint64_t *slot) {
    fflush(stdout);
    fputs("error: ", stderr);
    if (msg->len) fwrite(msg->ptr, 1, (size_t)msg->len, stderr);
    fputc('\n', stderr);
    for (uint64_t chain = *slot; chain;) {
        const drachm_cause *c = (const drachm_cause *)(uintptr_t)chain;
        fputs("  because: ", stderr);
        if (c->msg.len) fwrite(c->msg.ptr, 1, (size_t)c->msg.len, stderr);
        fputc('\n', stderr);
        chain = c->next;
    }
    fflush(stderr);
    drachm_cause_drop(*slot);
    *slot = 0;
}

// `drachm_report`, then the program ends at once with `code`: whatever the
// stack still owns is not freed (the run-time leak check does not run).
DRACHM_FN void drachm_report_exit(const drachm_str *msg, uint64_t *slot, uint8_t code) {
    drachm_report(msg, slot);
    fflush(stdout);
    fflush(stderr);
    _exit((int)code);
}

// ---- conversions ----------------------------------------------------------

// Float to integer under overflow = wrap: drop the fraction, then wrap around
// the width. NaN and the infinities become 0. Narrower targets take the low
// bits of these.
DRACHM_FN uint64_t drachm_f64_to_u64(double x) {
    if (x > -9223372036854775808.0 && x < 9223372036854775808.0) return (uint64_t)(int64_t)x;
    if (x != x || x - x != 0) return 0;
    x = fmod(x, 18446744073709551616.0);
    if (x < 0) x += 18446744073709551616.0;
    return (uint64_t)x;
}

DRACHM_FN drachm_pair drachm_f64_to_u128(double x) {
    const double two127 = 170141183460469231731687303715884105728.0;
    if (x > -two127 && x < two127) return drachm_split((drachm_u128)(drachm_s128)x);
    if (x != x || x - x != 0) return drachm_split(0);
    x = fmod(x, 2 * two127);
    if (x < 0) x += 2 * two127;
    return drachm_split((drachm_u128)x);
}

DRACHM_FN double drachm_u128_to_f64(uint64_t lo, uint64_t hi) { return (double)drachm_join(lo, hi); }
DRACHM_FN double drachm_s128_to_f64(uint64_t lo, uint64_t hi) { return (double)(drachm_s128)drachm_join(lo, hi); }
DRACHM_FN float drachm_u128_to_f32(uint64_t lo, uint64_t hi) { return (float)drachm_join(lo, hi); }
DRACHM_FN float drachm_s128_to_f32(uint64_t lo, uint64_t hi) { return (float)(drachm_s128)drachm_join(lo, hi); }

// ---- overflow = saturate | panic -------------------------------------------

// Arithmetic, shifts, powers and conversions of a function whose overflow
// setting is not `wrap`. Operands arrive as 128-bit values extended by their
// own signedness; the result is the bit pattern (the caller keeps the low
// bits). `mode` is 1 = saturate, 2 = panic. Both backends call these, except
// that the C backend does + - * inline with __builtin_*_overflow, to the same
// rules.

#define DRACHM_OVF_SATURATE 1

static inline __attribute__((unused)) drachm_u128 drachm_ovf_max(uint32_t bits, int sgn) {
    if (sgn) return ((drachm_u128)1 << (bits - 1)) - 1;
    return bits == 128 ? ~(drachm_u128)0 : (((drachm_u128)1 << bits) - 1);
}

static inline __attribute__((unused)) drachm_u128 drachm_ovf_min(uint32_t bits, int sgn) {
    return sgn ? -((drachm_u128)1 << (bits - 1)) : 0;
}

// verdict: 0 the value fits, 1 it is above the type's range, -1 below.
static inline __attribute__((unused)) drachm_pair drachm_ovf_done(int mode, uint32_t bits, int sgn, int verdict, drachm_u128 r, const char *msg) {
    if (verdict == 0) return drachm_split(r);
    if (mode != DRACHM_OVF_SATURATE) drachm_panic(msg);
    return drachm_split(verdict > 0 ? drachm_ovf_max(bits, sgn) : drachm_ovf_min(bits, sgn));
}

// How a mathematical value that fit in 128 bits sits against a narrower type.
static inline __attribute__((unused)) int drachm_ovf_range(uint32_t bits, int sgn, drachm_u128 r) {
    if (bits == 128) return 0;
    if (sgn) {
        drachm_s128 v = (drachm_s128)r, hi = (drachm_s128)drachm_ovf_max(bits, 1);
        return v > hi ? 1 : v < -hi - 1 ? -1 : 0;
    }
    return r > drachm_ovf_max(bits, 0) ? 1 : 0;
}

// op: 0 add, 1 sub, 2 mul, 3 div, 4 neg (b is ignored).
DRACHM_FN drachm_pair drachm_ovf_arith(int op, int mode, uint32_t bits, int sgn, uint64_t alo, uint64_t ahi,
                                       uint64_t blo, uint64_t bhi) {
    drachm_u128 a = drachm_join(alo, ahi), b = drachm_join(blo, bhi), r = 0;
    int of = 0, verdict = 0;
    if (sgn) {
        drachm_s128 x = (drachm_s128)a, y = (drachm_s128)b, z = 0;
        switch (op) {
        case 0: of = __builtin_add_overflow(x, y, &z); verdict = y > 0 ? 1 : -1; break;
        case 1: of = __builtin_sub_overflow(x, y, &z); verdict = y < 0 ? 1 : -1; break;
        case 2: of = __builtin_mul_overflow(x, y, &z); verdict = (x < 0) == (y < 0) ? 1 : -1; break;
        case 3:
            if (y == 0) drachm_div_zero();
            of = x == (drachm_s128)drachm_ovf_min(bits, 1) && y == -1;
            verdict = 1;
            z = of ? 0 : x / y;
            break;
        default:
            of = x == (drachm_s128)drachm_ovf_min(bits, 1);
            verdict = 1;
            z = of ? 0 : -x;
            break;
        }
        r = (drachm_u128)z;
    } else {
        drachm_u128 z = 0;
        switch (op) {
        case 0: of = __builtin_add_overflow(a, b, &z); verdict = 1; break;
        case 1: of = __builtin_sub_overflow(a, b, &z); verdict = -1; break;
        case 2: of = __builtin_mul_overflow(a, b, &z); verdict = 1; break;
        case 3:
            if (b == 0) drachm_div_zero();
            z = a / b;
            break;
        default: of = a != 0; verdict = -1; break;
        }
        r = z;
    }
    if (!of) verdict = drachm_ovf_range(bits, sgn, r);
    return drachm_ovf_done(mode, bits, sgn, verdict, r, "arithmetic overflow");
}

// base ** exp. `exp` is extended by the exponent's own signedness;
// `exp_negative` is the whole-number reading of a negative exponent, as in
// drachm_ipow128 (never an overflow).
DRACHM_FN drachm_pair drachm_ovf_pow(int mode, uint32_t bits, int sgn, uint64_t blo, uint64_t bhi, uint64_t elo,
                                     uint64_t ehi, int exp_negative) {
    drachm_u128 base = drachm_join(blo, bhi), exp = drachm_join(elo, ehi);
    if (exp_negative) {
        if (base == 1) return drachm_split(1);
        if (sgn && base == ~(drachm_u128)0) return drachm_split((exp & 1) ? base : 1);
        if (base == 0) drachm_div_zero();
        return drachm_split(0);
    }
    const char *msg = "arithmetic overflow";
    int up = sgn && (drachm_s128)base < 0 && (exp & 1) ? -1 : 1;
    drachm_u128 result = 1;
    while (exp != 0) {
        if (exp & 1) {
            if (sgn ? __builtin_mul_overflow((drachm_s128)result, (drachm_s128)base, (drachm_s128 *)&result)
                    : __builtin_mul_overflow(result, base, &result))
                return drachm_ovf_done(mode, bits, sgn, up, 0, msg);
        }
        exp >>= 1;
        // The square is only needed if a higher bit of the exponent is set,
        // and then the result is at least that big.
        if (exp != 0) {
            if (sgn ? __builtin_mul_overflow((drachm_s128)base, (drachm_s128)base, (drachm_s128 *)&base)
                    : __builtin_mul_overflow(base, base, &base))
                return drachm_ovf_done(mode, bits, sgn, up, 0, msg);
        }
    }
    return drachm_ovf_done(mode, bits, sgn, drachm_ovf_range(bits, sgn, result), result, msg);
}

// A shift whose amount may be outside 0..bits (or negative). Saturate: a
// negative amount shifts the other way by its size (`8 >> -1` is 16), and
// the bits pushed past the width fall off like any shift (`200 >> -1` in
// uint8 is 144). An amount of the width or more makes the bits all fall off,
// leaving 0 (or -1 for a negative value shifted right). Panic stops on a
// negative amount.
DRACHM_FN drachm_pair drachm_ovf_shift(int left, int mode, uint32_t bits, int sgn, uint64_t vlo, uint64_t vhi,
                                       uint64_t nlo, uint64_t nhi, int amount_signed) {
    drachm_u128 v = drachm_join(vlo, vhi), n = drachm_join(nlo, nhi);
    int negative = amount_signed && (drachm_s128)n < 0;
    if (negative && mode == DRACHM_OVF_SATURATE) {
        drachm_u128 m = -n;
        int below = sgn && (drachm_s128)v < 0;
        if (left) {
            // `x << -m` is `x >> m`, which rounds toward negative infinity.
            if (m >= bits) return drachm_split(below ? ~(drachm_u128)0 : 0);
            return drachm_split(sgn ? (drachm_u128)((drachm_s128)v >> m) : v >> m);
        }
        // `x >> -m` is `x << m`, and like any left shift the bits pushed
        // past the width fall off.
        if (m >= bits) return drachm_split(0);
        return drachm_split(v << m);
    }
    int outside = negative || n >= bits;
    if (outside) {
        if (mode != DRACHM_OVF_SATURATE) drachm_panic("shift amount out of range");
        return drachm_split(!left && sgn && (drachm_s128)v < 0 ? ~(drachm_u128)0 : 0);
    }
    if (left) return drachm_split(v << n);
    return drachm_split(sgn ? (drachm_u128)((drachm_s128)v >> n) : v >> n);
}

// An integer to a narrower or differently signed integer type.
DRACHM_FN drachm_pair drachm_ovf_conv(int mode, uint32_t to_bits, int to_sgn, int from_sgn, uint64_t lo, uint64_t hi) {
    drachm_u128 a = drachm_join(lo, hi);
    int verdict = 0;
    if (from_sgn && (drachm_s128)a < 0) {
        if (!to_sgn) verdict = -1;
        else if (to_bits < 128 && (drachm_s128)a < -(drachm_s128)drachm_ovf_max(to_bits, 1) - 1) verdict = -1;
    } else if (a > drachm_ovf_max(to_bits, to_sgn)) {
        verdict = 1;
    }
    return drachm_ovf_done(mode, to_bits, to_sgn, verdict, a, "conversion overflow");
}

// A float to an integer: the fraction is dropped and an out of range value
// clamps; +inf and -inf clamp to the ends too, and NaN gives 0. Panic mode
// stops on NaN and the infinities.
DRACHM_FN drachm_pair drachm_ovf_f2i(int mode, uint32_t to_bits, int to_sgn, double x) {
    const char *msg = "conversion overflow";
    if (x != x) {
        if (mode != DRACHM_OVF_SATURATE) drachm_panic(msg);
        return drachm_split(0);
    }
    if (x - x != 0) {
        if (mode != DRACHM_OVF_SATURATE) drachm_panic(msg);
        return drachm_ovf_done(mode, to_bits, to_sgn, x > 0 ? 1 : -1, 0, msg);
    }
    double t = trunc(x);
    double hi = ldexp(1.0, to_sgn ? (int)to_bits - 1 : (int)to_bits);
    double lo = to_sgn ? -hi : 0.0;
    if (t >= hi) return drachm_ovf_done(mode, to_bits, to_sgn, 1, 0, msg);
    if (t < lo) return drachm_ovf_done(mode, to_bits, to_sgn, -1, 0, msg);
    return drachm_split(to_sgn ? (drachm_u128)(drachm_s128)t : (drachm_u128)t);
}

// ---- 128-bit division -----------------------------------------------------

DRACHM_FN drachm_pair drachm_div_u128(uint64_t alo, uint64_t ahi, uint64_t blo, uint64_t bhi) {
    drachm_u128 b = drachm_join(blo, bhi);
    if (b == 0) drachm_div_zero();
    return drachm_split(drachm_join(alo, ahi) / b);
}

DRACHM_FN drachm_pair drachm_rem_u128(uint64_t alo, uint64_t ahi, uint64_t blo, uint64_t bhi) {
    drachm_u128 b = drachm_join(blo, bhi);
    if (b == 0) drachm_div_zero();
    return drachm_split(drachm_join(alo, ahi) % b);
}

// The smallest value divided by -1 wraps (to itself), with remainder 0.
DRACHM_FN drachm_pair drachm_div_s128(uint64_t alo, uint64_t ahi, uint64_t blo, uint64_t bhi) {
    drachm_u128 a = drachm_join(alo, ahi), b = drachm_join(blo, bhi);
    if (b == 0) drachm_div_zero();
    if (b == ~(drachm_u128)0) return drachm_split(0 - a);
    return drachm_split((drachm_u128)((drachm_s128)a / (drachm_s128)b));
}

DRACHM_FN drachm_pair drachm_rem_s128(uint64_t alo, uint64_t ahi, uint64_t blo, uint64_t bhi) {
    drachm_u128 a = drachm_join(alo, ahi), b = drachm_join(blo, bhi);
    if (b == 0) drachm_div_zero();
    if (b == ~(drachm_u128)0) return drachm_split(0);
    return drachm_split((drachm_u128)((drachm_s128)a % (drachm_s128)b));
}

// ---- integer power --------------------------------------------------------

// base ** exp, wrapping. Callers widen the base to 64 bits (sign-extending a
// signed one, so -1 is all ones) and the exponent likewise, and say whether
// the exponent is negative. A negative exponent is whole-number division of
// 1 by the power: 0, except for bases 1 and -1, and a panic for base 0.
DRACHM_FN uint64_t drachm_ipow64(uint64_t base, uint64_t exp, int exp_negative, int base_signed) {
    if (exp_negative) {
        if (base == 1) return 1;
        if (base_signed && base == ~UINT64_C(0)) return (exp & 1) ? base : 1;
        if (base == 0) drachm_div_zero();
        return 0;
    }
    uint64_t result = 1;
    while (exp != 0) {
        if (exp & 1) result *= base;
        base *= base;
        exp >>= 1;
    }
    return result;
}

DRACHM_FN drachm_pair drachm_ipow128(uint64_t blo, uint64_t bhi, uint64_t elo, uint64_t ehi, int exp_negative,
                                     int base_signed) {
    drachm_u128 base = drachm_join(blo, bhi), exp = drachm_join(elo, ehi);
    if (exp_negative) {
        if (base == 1) return drachm_split(1);
        if (base_signed && base == ~(drachm_u128)0) return drachm_split((exp & 1) ? base : 1);
        if (base == 0) drachm_div_zero();
        return drachm_split(0);
    }
    drachm_u128 result = 1;
    while (exp != 0) {
        if (exp & 1) result *= base;
        base *= base;
        exp >>= 1;
    }
    return drachm_split(result);
}
