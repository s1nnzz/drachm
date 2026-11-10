// Drachm runtime: builtins that compiled programs link against.
#include <inttypes.h>
#include <stdio.h>
#include <stdlib.h>

void drachm_print_i64(int64_t v) { printf("%" PRId64 "\n", v); }

void drachm_print_bool(int8_t v) { puts(v ? "true" : "false"); }

void drachm_div_zero(void) {
    fflush(stdout);
    fputs("drachm: panic: division by zero\n", stderr);
    exit(101);
}
