#include "notes.h"
#include <stdlib.h>
#include <string.h>

#define MAX_NOTES 8

struct notes {
    char *items[MAX_NOTES];
    int count;
};

static int alive = 0;

notes *notes_new(void) {
    notes *n = calloc(1, sizeof *n);
    alive++;
    return n;
}

void notes_add(notes *n, const char *text) {
    if (n->count == MAX_NOTES) return;
    size_t len = strlen(text);
    char *copy = malloc(len + 1);
    memcpy(copy, text, len + 1);
    n->items[n->count++] = copy;
}

int notes_count(const notes *n) { return n->count; }

const char *notes_get(const notes *n, int index) {
    if (index < 0 || index >= n->count) return NULL;
    return n->items[index];
}

void notes_walk(const notes *n, void (*visit)(int index, int length)) {
    for (int i = 0; i < n->count; i++) visit(i, (int)strlen(n->items[i]));
}

void notes_free(notes *n) {
    for (int i = 0; i < n->count; i++) free(n->items[i]);
    free(n);
    alive--;
}

int notes_alive(void) { return alive; }
