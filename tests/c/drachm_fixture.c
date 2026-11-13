#include "drachm_fixture.h"
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int touched = 0;
static int live = 0;
static int counters = 0;
int fx_global = 7;

int fx_add(int a, int b) { return a + b; }
long fx_big(long a) { return a * 2; }
unsigned char fx_byte(unsigned char b) { return (unsigned char)(b + 1); }
signed char fx_sbyte(signed char b) { return (signed char)(b - 1); }
short fx_neg16(short v) { return (short)-v; }
unsigned short fx_u16(unsigned short v) { return (unsigned short)(v + 1); }
unsigned int fx_u32(unsigned int v) { return v + 1u; }
long long fx_ll(long long v) { return v * 3; }
double fx_scale(double x, float k) { return x * k; }
bool fx_is_even(int n) { return n % 2 == 0; }
size_t fx_len(const char *s) { return strlen(s); }
char fx_char(char c) { return (char)(c + 1); }
void fx_touch(void) { touched++; }
int fx_touched(void) { return touched; }

const char *fx_greeting(void) { return "hello from C"; }
char *fx_dup(const char *s) {
    size_t n = strlen(s);
    char *p = malloc(n + 1);
    memcpy(p, s, n + 1);
    live++;
    return p;
}
void fx_free(void *p) { if (p) { free(p); live--; } }
int fx_live(void) { return live; }
int fx_sum_bytes(const uint8_t *data, size_t n) {
    int total = 0;
    for (size_t i = 0; i < n; i++) total += data[i];
    return total;
}
int fx_divide(int a, int b, int *out) {
    if (b == 0) return -1;
    *out = a / b;
    return 0;
}
void *fx_identity(void *p) { return p; }
const char *fx_bad_text(void) { return "\xff" "bad"; }

struct fx_counter { int value; };
fx_counter *fx_counter_new(int start) {
    fx_counter *c = malloc(sizeof *c);
    c->value = start;
    counters++;
    return c;
}
void fx_counter_add(fx_counter *c, int n) { c->value += n; }
int fx_counter_get(const fx_counter *c) { return c->value; }
void fx_counter_free(fx_counter *c) { free(c); counters--; }
int fx_counters_alive(void) { return counters; }
void fx_point_move(fx_point *p, int dx, int dy) { p->x += dx; p->y += dy; }
fx_point fx_point_make(int x, int y) { fx_point p = { x, y }; return p; }
int fx_rect_area(fx_rect r) { return r.w * r.h; }
fx_size fx_size_make(double w, double h) { fx_size s = { w, h }; return s; }
fx_mixed fx_mixed_make(int a, float b, double c) { fx_mixed m = { a, b, c }; return m; }
double fx_mixed_sum(fx_mixed m) { return m.a + m.b + m.c; }
fx_tri fx_tri_make(int a) { fx_tri t = { a, a + 1, a + 2 }; return t; }
int fx_tri_sum(fx_tri t) { return t.a + t.b + t.c; }
int fx_wide_sum(fx_wide w) { return w.a + w.b + w.c + w.d + w.e; }

fx_color fx_next_color(fx_color c) { return c == FX_RED ? FX_GREEN : c == FX_GREEN ? FX_BLUE : FX_RED; }
const char *fx_color_name(fx_color c) { return c == FX_RED ? "red" : c == FX_GREEN ? "green" : c == FX_BLUE ? "blue" : "unknown"; }

int fx_apply(fx_int_fn f, int v) { return f(v); }
int fx_apply_n(int (*f)(int), int v, int times) { for (int i = 0; i < times; i++) v = f(v); return v; }

int fx_printf(const char *fmt, ...) { va_list a; va_start(a, fmt); int r = vprintf(fmt, a); va_end(a); return r; }
