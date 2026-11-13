---
title: "Lesson 16: Collections and for-each"
description: Fixed arrays, growable lists and maps, indexing, and walking them with for and of.
---

# Lesson 16: Collections and for-each

**Goal:** keep many values together, look them up, and visit each one.

## Arrays, lists and maps

Three kinds of collection have a short spelling. A type followed by `[n]` is a
fixed-size array. A type followed by `[?]` is a list that can grow. `Map<K, V>`
looks values up by key.

<!-- focus: 4 -->
Four bytes, no more and no fewer.

<!-- focus: 5 -->
Two strings, and more can be added later.

<!-- focus: 6 -->
Each entry is a key, a colon and a value.

<!-- example: lesson-16-literals -->

## Indexing

Indexes start at 0, so `names[0]` is `"Illy"`. An index outside the collection
stops the program with a message by default.

A map lookup with `[]` gives an optional ([lesson 13](lesson-13-enums.md)),
which is `none` when the key is missing. That is why `maybe` is so often at the
end of a lookup.

<!-- focus: 7 -->
`Bob` is not in the map, so `age` is `none`.

<!-- focus: 8 -->
With `maybe 0` the missing key gives 0.

<!-- example: lesson-16-index -->

## Changing a list

A list changes through methods such as `push`, and the binding must allow it.
A list declared without a value starts empty, so `mut string[?] names;` can
be pushed to at once. `open` allows changes without letting the name be
assigned a new list ([lesson 8](lesson-8-ownership.md) covers the ownership side).

<!-- example: lesson-16-change -->

## Walking a collection

`for` with `of` visits each item in turn. The name before `of` holds the current
item.

<!-- focus: 5 -->
`total` is declared `mut` so the loop can add to it.

<!-- focus: 6-8 -->
`n` is each number in turn.

<!-- example: lesson-16-walk -->

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

<!-- solution-of: solution-16-1 -->
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

### Ranges with a step

A range `a..b` in a `for` runs from `a` to `b`, both included, as the loop above
shows. A step after the range, `by n`, moves the counter by `n` each round:
`0..10 by 3` is 0, 3, 6, 9. The loop stops at the last value not past the end,
so the 10 is never reached.

<!-- example: lesson-16-step -->

The step can be a value (`by gap`), and it must be a number of the loop's type.
A negative step counts down, so `10..0 by -2` runs down from 10. A step that
points away from the end gives an empty loop. A step of zero written in the
program is E0106; one that is zero only when the program runs stops it with a
run-time error. A float range needs its step: `0.0..1.0 by 0.25`.

### Walking a map

A map is walked with two loop variables, one for the key and one for the value.
An `OrderedMap` visits its entries in the order the keys were added. An ordinary
`Map` visits them in an order that is not promised.

<!-- example: lesson-16-map -->

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

<!-- solution-of: solution-16-2 -->
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
`join` on a list goes back. Looping with `for` over the characters of a string
works too (see [Control flow](reference-control-flow.md#looping-over-characters)).

<!-- example: lesson-16-split -->

[Collections](reference-collections.md) has the full list of methods and the
errors, and [Collections and for-each](design-collections.md) has the settings
still to come. Next, splitting a program across files:
[Lesson 17: Modules and projects](lesson-17-modules.md).
