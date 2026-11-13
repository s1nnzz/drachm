/* A small C library for the C-import tests. Compiled by the tests, so they
   need no system library. */
#ifndef DRACHM_FIXTURE_H
#define DRACHM_FIXTURE_H

#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

/* constants */
#define FX_COUNT 12
#define FX_NAME "fixture"
#define FX_MASK (1u << 5)
#define FX_RATIO 2.5
#define FX_SQUARE(x) ((x) * (x))

/* scalars */
int fx_add(int a, int b);
long fx_big(long a);
unsigned char fx_byte(unsigned char b);
signed char fx_sbyte(signed char b);
short fx_neg16(short v);
unsigned short fx_u16(unsigned short v);
unsigned int fx_u32(unsigned int v);
long long fx_ll(long long v);
double fx_scale(double x, float k);
bool fx_is_even(int n);
size_t fx_len(const char *s);
char fx_char(char c);
void fx_touch(void);
int fx_touched(void);

/* pointers and strings */
const char *fx_greeting(void);
char *fx_dup(const char *s);
void fx_free(void *p);
int fx_live(void);
int fx_sum_bytes(const uint8_t *data, size_t n);
int fx_divide(int a, int b, int *out);
void *fx_identity(void *p);
const char *fx_bad_text(void); /* bytes that are not UTF-8 */

/* structs */
typedef struct fx_point { int x; int y; } fx_point;
typedef struct { double w; double h; } fx_size;
typedef struct fx_rect { fx_point origin; int w; int h; } fx_rect;
typedef struct fx_counter fx_counter;
fx_counter *fx_counter_new(int start);
void fx_counter_add(fx_counter *c, int n);
int fx_counter_get(const fx_counter *c);
void fx_counter_free(fx_counter *c);
int fx_counters_alive(void);
void fx_point_move(fx_point *p, int dx, int dy);
fx_point fx_point_make(int x, int y);
int fx_rect_area(fx_rect r);
fx_size fx_size_make(double w, double h);
typedef struct { int a; float b; double c; } fx_mixed;
fx_mixed fx_mixed_make(int a, float b, double c);
double fx_mixed_sum(fx_mixed m);
typedef struct { int a; int b; int c; } fx_tri;
fx_tri fx_tri_make(int a);
int fx_tri_sum(fx_tri t);
typedef struct { int a; int b; int c; int d; int e; } fx_wide;
int fx_wide_sum(fx_wide w);

/* enums */
typedef enum { FX_RED, FX_GREEN = 5, FX_BLUE } fx_color;
enum fx_mode { FX_MODE_A = 1, FX_MODE_B = 2 };
fx_color fx_next_color(fx_color c);
const char *fx_color_name(fx_color c);

/* callbacks */
typedef int (*fx_int_fn)(int);
int fx_apply(fx_int_fn f, int v);
int fx_apply_n(int (*f)(int), int v, int times);

/* unions */
typedef union fx_number { int i; float f; } fx_number;
typedef struct fx_key_event { unsigned type; unsigned code; unsigned char down; } fx_key_event;
typedef struct fx_motion_event { unsigned type; double x; double y; } fx_motion_event;
typedef union fx_event {
    unsigned type;
    fx_key_event key;
    fx_motion_event motion;
    struct { unsigned small : 3; } bits;
    unsigned char padding[40];
} fx_event;
float fx_number_float(fx_number *n);
void fx_event_key(fx_event *e, unsigned code, int down);
void fx_event_motion(fx_event *e, double x, double y);
int fx_event_poll(fx_event *e); /* a short fixed queue: 1 while events remain */

/* things the compiler does not support yet */
typedef struct fx_flags { unsigned a : 3; unsigned b : 5; } fx_flags;
int fx_printf(const char *fmt, ...);
static inline int fx_twice(int x) { return x * 2; }
extern int fx_global;

#endif
