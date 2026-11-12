---
title: Enums and optionals
description: The decisions behind enums and the optional type T?, which are built now.
---

# Enums and optionals

Enums, `match` on their variants, guards, nested patterns and optionals (`T?`) are
built: see [Enums](reference-enums.md), where the rules are stated. This page keeps
the decisions the owner made about them.

## Optionals

`T?` is the optional type `Optional<T>`: either a value (`Some`) or `none`. The
word `none` is both the "returns nothing" type and the empty optional.

Two decisions from the owner shape this part. A `T` wraps to a `T?` automatically
wherever a `T?` is expected (`uint32? x = 5;`, `return 5;`), and the test for a
missing value is written in words, `x has value` and `x is empty`, rather than
`x == none`.

```drachm
uint32? age;                 // none, until it gets a value
uint32? found = 5;           // 5 wraps to Some(5)
uint32 n = found maybe 0;    // 5

if (found has value) { }     // the test in words
if (age is empty) { }        // and the other one
```
