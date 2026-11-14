#include "events.h"
#include <string.h>

static int left = 3;

int event_next(event *out) {
    memset(out, 0, sizeof *out);
    switch (left--) {
    case 3: out->key.kind = EVENT_KEY; out->key.code = 65; out->key.down = 1; return 1;
    case 2: out->motion.kind = EVENT_MOTION; out->motion.x = 1.5; out->motion.y = 2.5; return 1;
    case 1: out->key.kind = EVENT_KEY; out->key.code = 65; out->key.down = 0; return 1;
    default: return 0;
    }
}
