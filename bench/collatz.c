#include <inttypes.h>
#include <stdio.h>

static uint32_t steps(uint64_t n) {
    uint32_t count = 0;
    while (n != 1) {
        if ((n & 1) == 0) n >>= 1;
        else n = 3 * n + 1;
        count++;
    }
    return count;
}

int main(void) {
    uint64_t best = 0;
    uint32_t best_steps = 0;
    for (uint64_t i = 1; i < 3000000; i++) {
        uint32_t s = steps(i);
        if (s > best_steps) {
            best_steps = s;
            best = i;
        }
    }
    printf("%" PRIu64 "\n%" PRIu32 "\n", best, best_steps);
    return 0;
}
