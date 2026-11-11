---
title: Collections
description: Arrays, lists, maps, ordered maps, indexing rules and for over collections.
status: not-implemented
---

# Collections

Fixed arrays, growable lists and maps are designed for step 6. The compiler
does not accept them yet, and `for` over a collection waits for them.

## Arrays, lists and maps

A type followed by `[n]` is a fixed-size array, `[?]` is a growable list, and
`Map<K, V>` is a map. The brackets are the short spelling of `Array<T, n>` and
`List<T>`.

<!-- focus: 1,2,3 -->

```drachm
uint8[4] rgba = [255, 0, 0, 255];
string[?] names = ["Illy", "Sam"];
Map<string, uint32> ages = { "Illy": 18 };
```

## Indexing

Indexes start at 0. An index outside the collection panics by default; the
`bounds` setting in [Config](design-config.md) changes that. A map lookup with
`[]` gives an optional, `none` for a missing key (the `missing_key` setting can
make it panic), and `::get` always exists.

<!-- focus: 1 -->

```drachm
uint32? age = ages["Bob"];
uint32 a = ages["Bob"] maybe 0;
```

## Changing a list

A list changes through methods and needs a binding that allows it. `open`
allows changes without allowing the name to be assigned a new list.

```drachm
open string[?] xs = [];
xs::push("a");
```

## Walking a collection

`for` with `of` visits each item. A map's iteration order is unspecified, and
an `OrderedMap` keeps insertion order.

```drachm
for (string name of names) {
    io::outln(name);
}
```

## Splitting strings

Several string methods return lists: `split(sep)`, `lines()` and `chars()` (by
grapheme), and `list::join(", ")` goes back. Looping over the characters of a
string with `for` is planned along with them.

```drachm
string[?] parts = "a,b,c"::split(",");
string back = parts::join("-");
```
