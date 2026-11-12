// Failing calls, C version: outcomes returned as structs by value.
#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

typedef struct { uint8_t fail; uint64_t v; } r64;
typedef struct { uint8_t fail; char *s; uint64_t len; } rstr;

static __attribute__((noinline)) r64 step(uint64_t x) {
    if (x % 1000003 == 7) return (r64){1, 0};
    return (r64){0, (x * 3 + 1) % 1000003};
}

static __attribute__((noinline)) uint64_t plain(uint64_t x) {
    if (x % 1000003 == 7) return 0;
    return (x * 3 + 1) % 1000003;
}

static __attribute__((noinline)) r64 level3(uint64_t x) {
    if (x % 10000 == 7) return (r64){1, 0};
    return (r64){0, x + 1};
}

static __attribute__((noinline)) r64 level2(uint64_t x) {
    r64 a = level3(x);
    if (a.fail) return a;
    return (r64){0, a.v + 1};
}

static __attribute__((noinline)) r64 level1(uint64_t x) {
    r64 a = level2(x);
    if (a.fail) return a;
    return (r64){0, a.v + 1};
}

static __attribute__((noinline)) rstr label(uint64_t i) {
    char buf[32];
    int n = snprintf(buf, sizeof buf, "%" PRIu64, i);
    char *s = malloc((size_t)n + 1);
    memcpy(s, buf, (size_t)n + 1);
    return (rstr){i % 100 == 99, s, (uint64_t)n};
}

int main(void) {
    uint64_t total = 0;
    for (uint64_t i = 0; i < 100000000; i++) {
        r64 r = step(i);
        total += r.fail ? 0 : r.v;
    }
    printf("%" PRIu64 "\n", total);

    total = 0;
    for (uint64_t i = 0; i < 20000000; i++) {
        r64 r = level1(i);
        total += r.fail ? 0 : r.v;
    }
    printf("%" PRIu64 "\n", total);

    total = 0;
    for (uint64_t i = 0; i < 2000000; i++) {
        rstr r = label(i);
        total += r.fail ? 1 : r.len;
        free(r.s);
    }
    printf("%" PRIu64 "\n", total);

    total = 0;
    for (uint64_t i = 0; i < 100000000; i++) total += plain(i);
    printf("%" PRIu64 "\n", total);
    return 0;
}
