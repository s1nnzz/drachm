#include "shape.h"

void shape_move(shape_point *p, int dx, int dy) {
    p->x += dx;
    p->y += dy;
}

shape_point shape_origin(void) {
    shape_point p = { 0, 0 };
    return p;
}

shape_size shape_scale(shape_size s, double k) {
    shape_size r = { s.w * k, s.h * k };
    return r;
}

shape_kind shape_kind_of(shape_size s) {
    if (s.w > s.h) return SHAPE_WIDE;
    if (s.w < s.h) return SHAPE_TALL;
    return SHAPE_SQUARE;
}
