#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

typedef struct { uint64_t *ptr; uint64_t len, cap; } list;

static void push(list *l, uint64_t v) {
    if (l->len == l->cap) {
        l->cap = l->cap ? l->cap * 2 : 4;
        l->ptr = realloc(l->ptr, l->cap * sizeof(uint64_t));
    }
    l->ptr[l->len++] = v;
}

static uint64_t biggest(uint64_t a, uint64_t b) { return a > b ? a : b; }

static uint64_t main_sum(uint64_t n) {
    list xs = {0};
    for (uint64_t i = 0; i < n; i++) push(&xs, i * 3 + 1);
    uint64_t total = 0;
    for (uint64_t j = 0; j < xs.len; j++) total += xs.ptr[j];
    free(xs.ptr);
    return total;
}

static uint64_t main_index(uint64_t n, uint64_t reads) {
    list xs = {0};
    for (uint64_t i = 0; i < n; i++) push(&xs, i ^ 5);
    uint64_t state = 12345, total = 0;
    for (uint64_t k = 0; k < reads; k++) {
        state = state * 6364136223846793005ULL + 1442695040888963407ULL;
        uint64_t at = (state >> 33) % n;
        if (at >= xs.len) abort();
        total += xs.ptr[at];
    }
    free(xs.ptr);
    return total;
}

static uint64_t main_generic(uint64_t n) {
    list xs = {0};
    for (uint64_t i = 0; i < n; i++) push(&xs, (i * 7919) % 10007);
    uint64_t best = 0;
    for (uint64_t j = 0; j < xs.len; j++) best = biggest(best, xs.ptr[j]) + (xs.ptr[j] & 1);
    free(xs.ptr);
    return best;
}

int main(void) {
    printf("%llu\n", (unsigned long long)main_sum(20000000));
    printf("%llu\n", (unsigned long long)main_index(1000000, 50000000));
    printf("%llu\n", (unsigned long long)main_generic(20000000));
    return 0;
}
