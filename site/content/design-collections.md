---
title: Collections
description: Arrays, lists, maps, ordered maps, indexing rules and for over collections.
status: not-implemented
---

# Collections

The [collections reference](reference-collections.md) describes the array,
list and map operations the compiler accepts today.

## Configuring lookups

The `bounds` setting in [Config](design-config.md) will control indexing
outside an array or list. The `missing_key` setting will let a missing
`map[key]` stop the program. The `map::get(key)` method will continue to
return an optional.

## Changing a list

A list changes through methods and needs a binding that allows it. `open`
allows changes without allowing the name to be assigned a new list.

```drachm
open string[?] xs = [];
xs::push("a");
```

## Walking a map

`for (string name, uint32 age of ages)` visits each entry (it works now; see
the [reference](reference-collections.md#walking-a-map)). A `Map`'s iteration
order is unspecified, and an `OrderedMap` keeps insertion order.
