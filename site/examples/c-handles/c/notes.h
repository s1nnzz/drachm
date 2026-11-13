#ifndef NOTES_H
#define NOTES_H

typedef struct notes notes;

notes *notes_new(void);
void notes_add(notes *n, const char *text);
int notes_count(const notes *n);
const char *notes_get(const notes *n, int index);
void notes_walk(const notes *n, void (*visit)(int index, int length));
void notes_free(notes *n);

/* How many notes objects are alive, for checking nothing leaks. */
int notes_alive(void);

#endif
