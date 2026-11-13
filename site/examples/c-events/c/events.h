#ifndef EVENTS_H
#define EVENTS_H

/* An event is a union: `kind` says which member the library filled in. */
enum { EVENT_KEY = 1, EVENT_MOTION = 2 };
typedef struct { int kind; int code; int down; } key_event;
typedef struct { int kind; double x; double y; } motion_event;
typedef union { int kind; key_event key; motion_event motion; } event;

/* Fills `out` with the next event; 0 when there are no more. */
int event_next(event *out);

#endif
