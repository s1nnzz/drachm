---
title: Collections
description: Arrays, lists and maps as designed, the settings that will change lookups, and the parts still to come.
---

# Collections

Arrays, lists, maps, ordered maps, indexing, changing a list, walking
collections and splitting text all work. The [collections reference](reference-collections.md)
describes what the compiler accepts, and the [lesson](lesson-16-collections.md)
introduces them. This page keeps the design of the settings and marks the
parts still to come.

## Changing a list

A list changes through methods and needs a binding that allows it. See
[Changing a list](reference-collections.md#changing-a-list).

## Walking a map

`for (string name, uint32 age of ages)` visits each entry, and `each` takes a
callback. A `Map`'s iteration order is unspecified, and an `OrderedMap` keeps
insertion order. See [Walking a map](reference-collections.md#walking-a-map).

## Configuring lookups

The `bounds` setting in [Config](design-config.md) controls runtime checks
for array and list indexes.

### Missing keys
With [`missing_key = "panic"`](design-config.md#missing-map-keys), a missing
`map[key]` stops the program. The `map::get(key)` method always returns an
optional.

## Taking an item out by index

`take xs[i] reorders` is built: it moves the item out and fills its place with
the last item. `take xs[i] leaving v` is built too: it keeps the order and the
length. See [Taking an item out](reference-collections.md#taking-an-item-out).

## Views of map entries

<!-- status: not-implemented -->

Walking a map gives copies of the keys and values. Looking at them without
copying, `for (&string k, uint32 v of ages)`, is E0901 today. See [Walking a map](reference-collections.md#walking-a-map).
