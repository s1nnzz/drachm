/* The harbours the Drachma calls at: a small C library, linked into the
   showcase through the [native] table of drachm.toml. */
#ifndef HARBOUR_H
#define HARBOUR_H

#define HARBOUR_BERTHS 3
#define HARBOUR_MOTTO "safe water, honest scales"
#define HARBOUR_TOLL(tons) ((tons) / 10 + 2)

typedef enum { BERTH_FREE, BERTH_TAKEN = 2, BERTH_CLOSED } berth_state;
typedef struct { double x; double y; } sky_point;
typedef struct harbour harbour;

harbour *harbour_open(const char *name, sky_point where);
void harbour_close(harbour *h);
const char *harbour_name(const harbour *h);
sky_point harbour_position(const harbour *h);
/* The berth the ship got, or -1 when the harbour is full. */
int harbour_dock(harbour *h, const char *ship, int tons);
berth_state harbour_berth(const harbour *h, int berth);
void harbour_each(const harbour *h, void (*visit)(int berth, int tons));
double harbour_distance(sky_point a, sky_point b);
/* A signal from the harbour mast: a union, `kind` says which member is set. */
enum { SIGNAL_NONE, SIGNAL_BELL, SIGNAL_LAMP };
typedef struct { int kind; int strikes; } bell_signal;
typedef struct { int kind; int colour; double bearing; } lamp_signal;
typedef union { int kind; bell_signal bell; lamp_signal lamp; } harbour_signal;
/* Fills `out` with the next signal; 0 when the mast is quiet. */
int harbour_signal_next(harbour *h, harbour_signal *out);
/* Harbours still open, to show that nothing leaks. */
int harbour_open_count(void);

#endif
