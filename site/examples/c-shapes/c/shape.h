#ifndef SHAPE_H
#define SHAPE_H

typedef struct { int x; int y; } shape_point;
typedef struct { double w; double h; } shape_size;

typedef enum { SHAPE_SQUARE, SHAPE_WIDE, SHAPE_TALL } shape_kind;

void shape_move(shape_point *p, int dx, int dy);
shape_point shape_origin(void);
shape_size shape_scale(shape_size s, double k);
shape_kind shape_kind_of(shape_size s);

#endif
