#include <stdint.h>
#include <stdio.h>

typedef struct { uint64_t (*area)(const void *); } vt;
typedef struct { const void *data; const vt *vt; } shape;

static uint64_t square_area(const void *p) { uint64_t s = *(const uint64_t *)p; return s * s; }
static uint64_t strip_area(const void *p) { return *(const uint64_t *)p + 3; }

static const vt square_vt = { square_area };
static const vt strip_vt = { strip_area };

__attribute__((noinline)) static uint64_t total(shape a, shape b, uint64_t pairs) {
    uint64_t sum = 0;
    for (uint64_t i = 0; i < pairs; i++) {
        shape s = sum % 3 == 0 ? b : a;
        sum += s.vt->area(s.data);
    }
    return sum;
}

int main(void) {
    uint64_t q = 7, s = 11;
    shape a = { &q, &square_vt }, b = { &s, &strip_vt };
    printf("%llu\n", (unsigned long long)total(a, b, 100000000));
    return 0;
}
