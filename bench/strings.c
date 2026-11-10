#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

int main(void) {
    uint64_t total = 0, hits = 0;
    size_t tail_len = 0, tail_cap = 16;
    char *tail = malloc(tail_cap);
    for (uint32_t i = 0; i < 2000000; i++) {
        char num[32];
        int n = snprintf(num, sizeof num, "item %" PRIu32 ": ", i);
        size_t reps = i % 7, len = (size_t)n + 2 * reps;
        char *line = malloc(len + 1);
        memcpy(line, num, (size_t)n);
        for (size_t r = 0; r < reps; r++) memcpy(line + n + 2 * r, "ab", 2);
        line[len] = 0;
        total += len;
        if (strstr(line, "99")) hits++;
        if (tail_len == tail_cap) tail = realloc(tail, tail_cap *= 2);
        tail[tail_len++] = line[len - 1];
        free(line);
    }
    uint64_t bs = 0;
    for (size_t k = 0; k < tail_len; k++) bs += tail[k] == 'b';
    printf("%" PRIu64 "\n%" PRIu64 "\n%" PRIu64 "\n%zu\n", total, hits, bs, tail_len);
    free(tail);
    return 0;
}
