#include <inttypes.h>
#include <stdbool.h>
#include <stdio.h>

static bool is_prime(int64_t n) {
    if (n < 2) return false;
    for (int64_t d = 2; d * d <= n; d++) {
        if (n % d == 0) return false;
    }
    return true;
}

int main(void) {
    int64_t count = 0;
    for (int64_t i = 0; i < 5000000; i++) {
        if (is_prime(i)) count++;
    }
    printf("%" PRId64 "\n", count);
    return 0;
}
