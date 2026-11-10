#include <inttypes.h>
#include <stdio.h>

static int64_t fib(int64_t n) {
    if (n < 2) return n;
    return fib(n - 1) + fib(n - 2);
}

int main(void) {
    printf("%" PRId64 "\n", fib(40));
    return 0;
}
