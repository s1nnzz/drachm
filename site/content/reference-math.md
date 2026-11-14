---
title: Maths
description: The math module, the number constants, and how units go through the functions.
---

# Maths

The `math` module holds the number functions. Import it with `in math;` and call
a function as `math::name(...)`:

<!-- example: math-functions -->

Every function works in the number type of its arguments: `float64` stays
`float64` and `float32` stays `float32`. The C library computes the results, on
both backends, so NaN, infinity and negative zero come out the same either way.

## The functions

| Function | Takes | Gives |
|---|---|---|
| `sqrt(x)` | a float | the square root; the powers of the unit halve |
| `pow(x, y)` | floats | `x` to the power `y`; with a unit, `y` is a constant whole number |
| `exp(x)`, `log(x)`, `log2(x)`, `log10(x)` | a plain float | `e` to the `x`, and the logarithms |
| `hypot(x, y)` | floats of one unit | the square root of `x * x + y * y`; the unit stays |
| `floor(x)`, `ceil(x)`, `round(x)` | a float | the whole number below, above or nearest (half away from zero); the unit stays |
| `sin(a)`, `cos(a)`, `tan(a)` | a plain float (radians) or an angle | the ratio, a plain float |
| `atan2(y, x)` | floats of one unit | the angle in radians, a plain float |
| `abs(x)` | any number | the size without the sign; the unit stays |
| `min(a, b)`, `max(a, b)` | numbers of one unit | the smaller or the larger |
| `clamp(x, low, high)` | numbers of one unit | `x`, held between `low` and `high` |

`abs`, `min`, `max` and `clamp` also work on whole numbers of every width. The
other functions need a float. A whole number is an error with the fix in the
message, because the conversion should be your decision:

```text
error[E0101]: `math::sqrt` needs a `float64` or `float32`, found `uint32`
  --> area.drh:5:25
   |
 5 |     float64 r = math::sqrt(n);
   |                            ^ this is `uint32`, a whole number type
   |
 help: convert it first: `value::to<float64>()`
```

A constant is fine: `math::sqrt(2)` takes `2` as a float, like every place that
wants a float.

### NaN, zero and the ends

- `min` and `max` give NaN when either argument is NaN. `clamp` gives NaN when `x`
  is NaN. If `low` is above `high`, `clamp` gives `high`.
- `abs(-0.0)` is `0.0`. `round`, `floor` and `ceil` keep the sign of a zero.
- `sqrt(-1.0)` and `log(-1.0)` are NaN, `log(0.0)` is minus infinity: floats never
  fail.
- The `abs` of the smallest signed whole number (`-128` as a `sint8`) does not fit.
  It follows the function's `overflow` setting, as `-x` does: it wraps by default,
  stops at the end with `saturate`, and panics with `panic`.

## Two spellings

A module function and a method are the same function:

<!-- example: math-methods -->

`math::sqrt(x)` and `x::sqrt()` are one function. The method spelling works on any
number, in any file that has `in math;`. After the first call, a chain continues
with `->`.

## Constants

The constants belong to the number type, at the precision of that type, and
need no import:

<!-- example: math-constants -->

`float64::pi`, `float32::pi`, `float64::e`, `float32::e`, `float64::tau` and
`float32::tau` (`tau` is two times `pi`).

## Units

Units go through the functions. `sqrt` halves the powers:

<!-- example: math-units -->

- `sqrt(9.0<m^2>)` is `3.0<m>`, and `sqrt(4.0<km^2>)` is `2.0<km>`. A unit with an
  odd power has no square root ([E0410](errors.md)): `sqrt(8.0<m^3>)` is an error.
- `abs`, `min`, `max`, `clamp`, `floor`, `ceil` and `round` keep the unit. The
  arguments of `min`, `max`, `clamp`, `hypot` and `atan2` must be the same kind of
  unit ([E0401](errors.md)); they are brought to the first one's unit, so
  `math::min(2.0<m>, 150.0<cm>)` is `1.5<m>`. A plain number beside a measure is
  [E0408](errors.md), except zero.
- `sin`, `cos` and `tan` take a plain number, which is radians, or an angle.
  `rad` and `deg` come with `in si;`, and `math::sin(90.0<deg>)` is 1. A value
  with any other unit, such as metres, is [E0411](errors.md). `exp`, `log`,
  `log2`, `log10` and the exponent of `pow` take plain numbers the same way.
- `pow(x, n)` with a unit on `x` needs a constant whole `n`, and the powers multiply:
  `math::pow(3.0<m>, 2)` is `9.0<m^2>`. A `n` that is not a constant is
  [E0404](errors.md), as with `**`. With a plain `x`, any plain `n` will do.
- The result is a number with a unit like any other: it can be stored in a
  `float64<m>`, added to other lengths, and so on.

## Whole numbers

<!-- example: math-ints -->

## Not built yet

Vectors, matrices, quaternions and exact numbers are described in
[Maths and exact numbers](design-maths.md). More functions (`asin`, `acos`, `atan`,
`sinh`, ...) can be added the same way when they are decided.
