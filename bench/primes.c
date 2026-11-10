#include <inttypes.h>
#include <stdbool.h>
#include <stdio.h>

static bool is_prime(uint64_t n) {
    if (n < 2) return false;
    for (uint64_t d = 2; d * d <= n; d++) {
        if (n % d == 0) return false;
    }
    return true;
}

int main(void) {
    uint64_t count = 0;
    for (uint64_t i = 0; i < 5000000; i++) {
        if (is_prime(i)) count++;
    }
    printf("%" PRIu64 "\n", count);
    return 0;
}
