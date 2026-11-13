---
title: "Lesson 16: Collections and for-each"
description: Fixed arrays, growable lists and maps, indexing, and walking them with for and of.
status: not-implemented
---

# Lesson 16: Collections and for-each

The array, list and map lookup examples run today. Map iteration is planned.

**Goal:** keep many values together, look them up, and visit each one.

## Arrays, lists and maps

Three kinds of collection have a short spelling. A type followed by `[n]` is a
fixed-size array. A type followed by `[?]` is a list that can grow. `Map<K, V>`
looks values up by key.

<!-- focus: 1 -->
Four bytes, no more and no fewer.

<!-- focus: 2 -->
Two strings, and more can be added later.

<!-- focus: 3 -->
Each entry is a key, a colon and a value.

```drachm
uint8[4] rgba = [255, 0, 0, 255];
string[?] names = ["Illy", "Sam"];
Map<string, uint32> ages = { "Illy": 18 };
```

## Indexing

Indexes start at 0, so `names[0]` is `"Illy"`. An index outside the collection
stops the program with a message by default.

A map lookup with `[]` gives an optional ([lesson 13](lesson-13-enums.md)),
which is `none` when the key is missing. That is why `maybe` is so often at the
end of a lookup.

<!-- focus: 1 -->
`Bob` is not in the map, so `age` is `none`.

<!-- focus: 2 -->
With `maybe 0` the missing key gives 0.

```drachm
uint32? age = ages["Bob"];
uint32 a = ages["Bob"] maybe 0;
```

## Changing a list

A list changes through methods such as `push`, and the binding must allow it.
`open` allows changes without letting the name be assigned a new list
([lesson 8](lesson-8-ownership.md) covers the ownership side).

```drachm
open string[?] xs = [];
xs::push("a");
```

## Walking a collection

`for` with `of` visits each item in turn. The name before `of` holds the current
item. Planned map loops will use two bindings. An `OrderedMap` will visit its
entries in insertion order, while an ordinary map has no promised order.

<!-- focus: 2 -->
`total` is declared `mut` so the loop can add to it.

<!-- focus: 3-5 -->
`n` is each number in turn.

```drachm
uint32[?] scores = [4, 8, 15];
mut uint32 total = 0;
for (uint32 n of scores) {
    total = total + n;
}
```

The same `for` runs over a range, and `by` sets the step:

```drachm
for (uint32 i of 0..10 by 3) { }       // 0, 3, 6, 9
```

### Your turn: add them up

Print the sum of `scores`.

```drachm title="starter"
in io;

none main() {
    uint32[?] scores = [4, 8, 15];
    mut uint32 total = 0;
    // your code here
    io::outln(total::text());
}
```

```text title="intended output"
27
```

```drachm solution
in io;

none main() {
    uint32[?] scores = [4, 8, 15];
    mut uint32 total = 0;
    for (uint32 n of scores) {
        total = total + n;
    }
    io::outln(total::text());
}
```

### Your turn: look up with a fallback

Print the age of Illy and then of Bob, giving 0 when the key is missing.

```drachm title="starter"
in io;

none main() {
    Map<string, uint32> ages = { "Illy": 18 };
    // your code here
}
```

```text title="intended output"
18
0
```

```drachm solution
in io;

none main() {
    Map<string, uint32> ages = { "Illy": 18 };
    io::outln((ages["Illy"] maybe 0)::text());
    io::outln((ages["Bob"] maybe 0)::text());
}
```

## Splitting strings

Several string methods return lists: `split(sep)`, `lines()` and `chars()`, and
`join` on a list goes back. Looping directly over the characters of a string
with `for` is planned along with them.

```drachm
string[?] parts = "a,b,c"::split(",");
string back = parts::join("-");
```

[Collections](design-collections.md) has the rest of the design, including the
settings that change what an out-of-range index does. Next, splitting a program
across files: [Lesson 17: Modules and projects](lesson-17-modules.md).
