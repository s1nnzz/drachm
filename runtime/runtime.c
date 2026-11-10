// Drachm runtime: what compiled programs call for printing, panics and the
// few operations that are too big to emit inline.
//
// Plain builds compile this file on its own and link it. Optimised (-O)
// builds paste it at the top of the generated C with DRACHM_INLINE defined,
// so that the C compiler can inline every helper.
//
// Both backends must behave identically, so anything with room for
// disagreement (float to integer wrapping, 128-bit conversions, powers,
// float printing) lives here and is shared.
#include <inttypes.h>
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

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

DRACHM_FN _Noreturn void drachm_panic(const char *msg) {
    fflush(stdout);
    fprintf(stderr, "drachm: panic: %s\n", msg);
    exit(101);
}

DRACHM_FN _Noreturn void drachm_div_zero(void) { drachm_panic("division by zero"); }

// ---- output ---------------------------------------------------------------

static inline void drachm_write(const char *s, size_t len, int newline) {
    fwrite(s, 1, len, stdout);
    if (newline) putc('\n', stdout);
}

DRACHM_FN void drachm_out_str(const char *s, uint64_t len, int newline) { drachm_write(s, (size_t)len, newline); }

DRACHM_FN void drachm_out_bool(uint8_t v, int newline) {
    if (v) drachm_write("true", 4, newline);
    else drachm_write("false", 5, newline);
}

DRACHM_FN void drachm_out_u128(uint64_t lo, uint64_t hi, int newline) {
    drachm_u128 v = drachm_join(lo, hi);
    char buf[40];
    char *p = buf + sizeof buf;
    do {
        *--p = (char)('0' + (int)(v % 10));
        v /= 10;
    } while (v != 0);
    drachm_write(p, (size_t)(buf + sizeof buf - p), newline);
}

DRACHM_FN void drachm_out_s128(uint64_t lo, uint64_t hi, int newline) {
    if (hi >> 63) {
        drachm_pair m = drachm_split(0 - drachm_join(lo, hi));
        putc('-', stdout);
        drachm_out_u128(m.lo, m.hi, newline);
    } else {
        drachm_out_u128(lo, hi, newline);
    }
}

DRACHM_FN void drachm_out_u64(uint64_t v, int newline) { drachm_out_u128(v, 0, newline); }

DRACHM_FN void drachm_out_s64(int64_t v, int newline) { drachm_out_s128((uint64_t)v, v < 0 ? ~UINT64_C(0) : 0, newline); }

// Floats print as the shortest digits that read back as the same number,
// always with a decimal point and never in exponent form.
static void drachm_out_float(double v, int single, int newline) {
    if (v != v) {
        drachm_write("nan", 3, newline);
        return;
    }
    if (v - v != 0) {
        if (v < 0) drachm_write("-inf", 4, newline);
        else drachm_write("inf", 3, newline);
        return;
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

    char out[400];
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
    drachm_write(out, (size_t)len, newline);
}

DRACHM_FN void drachm_out_f64(double v, int newline) { drachm_out_float(v, 0, newline); }

DRACHM_FN void drachm_out_f32(float v, int newline) { drachm_out_float((double)v, 1, newline); }

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
