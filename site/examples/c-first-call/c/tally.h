#ifndef TALLY_H
#define TALLY_H

#define TALLY_LIMIT 100
#define TALLY_NAME "tally"
#define TALLY_DOUBLE(x) ((x) * 2)

int tally_add(int a, int b);
long tally_clamp(long value);
double tally_mean(double a, double b);
unsigned char tally_next_byte(unsigned char b);

#endif
