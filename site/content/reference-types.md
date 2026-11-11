---
title: Types and numbers
description: Built-in types, number literals, conversions, operators and text of values.
---

# Types and numbers

## Built-in types

| Type | What it holds |
|---|---|
| `uint8`, `uint16`, `uint32`, `uint64`, `uint128` | whole numbers from 0 up |
| `sint8`, `sint16`, `sint32`, `sint64`, `sint128` | whole numbers, negative too |
| `float32`, `float64` | decimal numbers |
| `bool` | `true` or `false` |
| `string`, or `str` | text (see [Strings](reference-strings.md)) |
| `none` | no value: a function that returns nothing |

Every binding and parameter has a type, written first:

```drachm
uint32 answer = 42;
mut sint8 cold = -12;
```

A binding is immutable unless it is declared `mut`. A binding may also be
declared without a value and assigned exactly once, on every path before it is
read:

```drachm
uint32 later;
if (answer > 40) {
    later = 1;
} else {
    later = 2;
}
```

## Number literals

- Decimal: `42`, `1_000_000`, `2.5`, `1e21`.
- Other bases: `0xff` (hex), `0b101` (binary), `0o17` (octal).
- A suffix sets the type: `255u8`, `-5s8`, `2.5f32`. The short forms `u`,
  `s` and `f` mean `uint32`, `sint32` and `float64`.

A number written in the source is exact until it reaches a type. Arithmetic on
constants is also exact, so `0.1 + 0.2` is `0.3`.

## Conversions

- A number converts to a type that holds all its values, without a warning:
  `uint32 wide = small;` where `small` is a `uint8`.
- A conversion that may lose value gives a warning, W0102. The value wraps
  around. Write `value::to<T>()` to say the conversion is intended:

  ```drachm
  sint64 big = 1000;
  uint8 cut = big::to<uint8>();   // 232
  ```

- In `a + b`, the right side converts to the type of `a`.
- `bool` never converts to or from a number.
- A float converted to a whole number drops the fraction first.

## Operators

- Arithmetic: `+ - * / % **`. Whole division drops the fraction. Division by
  zero panics at run time, with exit code 101. A constant division by zero is
  an error.
- Comparison: `== != < <= > >=`.
- Logic: `&& || !`. Both are short-circuit, so the right side runs only when
  it is needed.
- Bits: `& | ^ ~ << >>`. A bitwise operator next to an arithmetic or
  comparison operator needs parentheses: `((a & 1) == 0)`.
- Overflow wraps around. `uint8` 255 plus 1 is 0.
- `++` and `--` change a `mut` binding by one. Compound forms like `+=` also
  work.

Write `::` and `->` with care: `::` binds tighter than any operator, so
`7 / 2::text()` divides 7 by a string. Put the call in parentheses:
`(7 / 2)::text()`.

## Text of a value

`::text()` turns a number or `bool` into a `string`:

```drachm
io::outln((10 / 4)::text());     // 2
io::outln(true::text());         // true
```

Floats always keep a decimal point: `1e21` prints as `1000000000000000000000.0`.
Use `io::outf` with `%.N` to round to N decimal places.
