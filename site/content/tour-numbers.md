---
title: Numbers and types
description: Whole numbers, decimals, bools, conversions and text of numbers.
---

# Numbers and types

Every binding has a type, written before its name. A binding cannot change
unless it is declared with `mut`.

<!-- example: numbers -->

- `uint32 answer = 40 + 2;` is an unsigned 32-bit whole number. `sint8` is a
  signed 8-bit one. Sizes go from 8 to 128 bits.
- `answer / 4` is a whole division, so it drops the fraction. Convert to
  `float64` first to get `10.5`.
- `::text()` turns any number or `bool` into a `string`. `io::outln` only
  prints text, so `io::outln(answer)` is an error.
- Constant arithmetic is exact. `0.1 + 0.2` is `0.3` until it reaches a type.
- A smaller number converts to a bigger type by itself. A conversion that can
  lose value gives a warning. Write `::to<T>()` to say you meant it.

Next: [Strings](tour-strings.md).
