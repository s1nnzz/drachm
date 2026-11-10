#include <inttypes.h>
#include <stdio.h>

static uint32_t escape(double cr, double ci) {
    double zr = 0.0, zi = 0.0;
    uint32_t i = 0;
    while (i < 200 && zr * zr + zi * zi <= 4.0) {
        double t = zr * zr - zi * zi + cr;
        zi = 2.0 * zr * zi + ci;
        zr = t;
        i++;
    }
    return i;
}

int main(void) {
    uint32_t size = 1200;
    uint64_t total = 0;
    for (uint32_t y = 0; y < size; y++) {
        for (uint32_t x = 0; x < size; x++) {
            double cr = (double)x * 3.0 / size - 2.0;
            double ci = (double)y * 2.0 / size - 1.0;
            total += escape(cr, ci);
        }
    }
    printf("%" PRIu64 "\n", total);
    return 0;
}
