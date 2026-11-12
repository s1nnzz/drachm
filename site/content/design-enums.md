---
title: Enums and optionals
description: Enums that carry data, match on their variants, and the optional type T?.
---

# Enums and optionals

Enums that carry data, `match` on their variants, guards and nested patterns
are built: see [Enums](reference-enums.md). What is still designed only is
the optional type `T?`, below. The compiler does not accept `T?` or `none` as a
value yet.

## Optionals
<!-- status: not-implemented -->

`T?` is the optional type `Optional<T>`, and it is an ordinary enum of the
standard library: either a value (`Some`) or `none`. The word `none` is both
the "returns nothing" type and the empty optional.

```drachm
enum Optional<T> { Some(T value), none, }
```

A variable of optional type with no value starts as `none`. A map lookup with a
missing key gives `none`. `maybe` unwraps an optional with a fallback, and
`match` can test it.

<!-- focus: 1,2,4,6 -->

```drachm
uint32? age;                           // none
uint32? found = ages["Bob"];
uint32 n = found maybe 0;

match (found) {
    case uint32 a { io::outln(a::text()); }
    case none     { io::outln("no age"); }
}
```

Two decisions from the owner shape this part. A `T` wraps to a `T?` automatically
wherever a `T?` is expected (`uint32? x = 5;`, `return 5;`), and the test
for a missing value is written in words, `x has value` and `x is empty`, rather
than `x == none`.
