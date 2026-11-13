#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

struct entry {
    uint64_t key;
    uint64_t value; // Zero marks an empty slot.
};

struct map {
    struct entry *entries;
    size_t capacity;
    size_t len;
};

static uint64_t hash_key(uint64_t key) {
    key = (key ^ (key >> 30)) * UINT64_C(0xbf58476d1ce4e5b9);
    key = (key ^ (key >> 27)) * UINT64_C(0x94d049bb133111eb);
    return key ^ (key >> 31);
}

static size_t find_slot(const struct map *map, uint64_t key) {
    size_t mask = map->capacity - 1;
    size_t slot = (size_t)hash_key(key) & mask;
    while (map->entries[slot].value != 0 && map->entries[slot].key != key) {
        slot = (slot + 1) & mask;
    }
    return slot;
}

static void grow(struct map *map) {
    struct map old = *map;
    map->capacity = old.capacity == 0 ? 8 : old.capacity * 2;
    map->entries = calloc(map->capacity, sizeof *map->entries);
    if (map->entries == NULL) {
        abort();
    }
    for (size_t i = 0; i < old.capacity; i++) {
        struct entry entry = old.entries[i];
        if (entry.value != 0) {
            map->entries[find_slot(map, entry.key)] = entry;
        }
    }
    free(old.entries);
}

static void insert(struct map *map, uint64_t key, uint64_t value) {
    if (map->len == map->capacity - map->capacity / 4) {
        grow(map);
    }
    size_t slot = find_slot(map, key);
    if (map->entries[slot].value == 0) {
        map->len++;
    }
    map->entries[slot] = (struct entry){key, value};
}

int main(void) {
    const uint64_t count = 5000000;
    struct map values = {0};
    for (uint64_t i = 0; i < count; i++) {
        insert(&values, i, i * 3 + 1);
    }

    // A stride coprime to count visits every inserted key once.
    uint64_t key = 0;
    uint64_t total = 0;
    for (uint64_t i = 0; i < count; i++) {
        total += values.entries[find_slot(&values, key)].value;
        key += 7919;
        if (key >= count) {
            key -= count;
        }
    }

    printf("%" PRIu64 "\n", total);
    free(values.entries);
    return 0;
}
