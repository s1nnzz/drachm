#include "tally.h"

int tally_add(int a, int b) { return a + b; }

long tally_clamp(long value) {
    if (value > TALLY_LIMIT) return TALLY_LIMIT;
    if (value < -TALLY_LIMIT) return -TALLY_LIMIT;
    return value;
}

double tally_mean(double a, double b) { return (a + b) / 2.0; }

unsigned char tally_next_byte(unsigned char b) { return (unsigned char)(b + 1); }
