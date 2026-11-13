#include "harbour.h"
#include <math.h>
#include <stdlib.h>
#include <string.h>

struct harbour {
    char *name;
    sky_point where;
    int tons[HARBOUR_BERTHS];
    int used;
};

static int open_harbours = 0;

harbour *harbour_open(const char *name, sky_point where) {
    harbour *h = calloc(1, sizeof *h);
    h->name = strdup(name);
    h->where = where;
    open_harbours++;
    return h;
}

void harbour_close(harbour *h) {
    free(h->name);
    free(h);
    open_harbours--;
}

const char *harbour_name(const harbour *h) { return h->name; }

sky_point harbour_position(const harbour *h) { return h->where; }

int harbour_dock(harbour *h, const char *ship, int tons) {
    (void)ship;
    if (h->used == HARBOUR_BERTHS) return -1;
    h->tons[h->used] = tons;
    return h->used++;
}

berth_state harbour_berth(const harbour *h, int berth) {
    if (berth < 0 || berth >= HARBOUR_BERTHS) return BERTH_CLOSED;
    return berth < h->used ? BERTH_TAKEN : BERTH_FREE;
}

void harbour_each(const harbour *h, void (*visit)(int berth, int tons)) {
    for (int i = 0; i < h->used; i++) visit(i, h->tons[i]);
}

double harbour_distance(sky_point a, sky_point b) { return hypot(a.x - b.x, a.y - b.y); }

int harbour_open_count(void) { return open_harbours; }
