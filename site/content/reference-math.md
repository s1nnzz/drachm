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
| `sqrt(x)` | a float or a whole number | the square root; the powers of the unit halve |
| `pow(x, y)` | floats, or whole numbers | `x` to the power `y`; with a unit, `y` is a constant whole number |
| `exp(x)` | a plain float | `e` to the `x` |
| `log(x)`, `log2(x)`, `log10(x)` | a plain float or whole number | the logarithms |
| `hypot(x, y)` | floats of one unit | the square root of `x * x + y * y`; the unit stays |
| `floor(x)`, `ceil(x)`, `round(x)` | a float or a whole number | the whole number below, above or nearest (half away from zero); the unit stays |
| `sin(a)`, `cos(a)`, `tan(a)` | a plain float (radians) or an angle | the ratio, a plain float |
| `atan2(y, x)` | floats of one unit | the angle in radians: a `float64<rad>` in a file with `in si;`, a plain float otherwise |
| `abs(x)` | any number | the size without the sign; the unit stays |
| `min(a, b)`, `max(a, b)` | numbers of one unit | the smaller or the larger |
| `clamp(x, low, high)` | numbers of one unit | `x`, held between `low` and `high` |

`abs`, `min`, `max` and `clamp` also work on whole numbers of every width.
`sqrt`, `log`, `log2`, `log10` and `pow` have a whole-number version, described
in [Whole numbers](#whole-numbers) below. `exp`, `hypot`, `sin`, `cos`, `tan` and
`atan2` have none: a whole number is an error with the fix in the message,
because the conversion should be your decision:

```text
error[E0101]: `math::sin` needs a `float64` or `float32`, found `uint32`
  --> angle.drh:5:25
   |
 5 |     float64 s = math::sin(n);
   |                           ^ this is `uint32`, a whole number type
   |
 note: `math::sin` has no whole-number version
 help: convert it first: `value::to<float64>()`
```

A constant is fine: `math::sqrt(2)` takes `2` as a float, like every place that
wants a float.

### NaN, zero and the ends

- `min` and `max` give NaN when either argument is NaN. `clamp` gives NaN when `x`
  is NaN. The bounds of `clamp` are a range in either order:
  `math::clamp(5.0, 10.0, 0.0)` is `5.0`.
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
number in any file, with no import. Only the module spelling needs `in math;`.
After the first call, a chain continues with `->`.

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

Given a whole number, `sqrt`, `log`, `log2`, `log10` and `pow` use a whole-number
version and give the same whole number type. The result is the true result rounded
down:

- `sqrt(x)` is the largest `r` with `r * r <= x`: `math::sqrt(10u32)` is 3.
- `log(x)` is the natural logarithm, `log2(x)` and `log10(x)` the others, rounded
  down: `math::log10(999u32)` is 2 and `math::log2(1000u32)` is 9. They are exact
  for every width, `uint128` included.
- `pow(x, n)` is the exact whole-number power, the same as `x ** n`, and follows
  the `overflow` setting. A negative exponent follows `**` too: it gives 0, or 1
  or -1 for a base of 1 or -1.
- `floor`, `ceil` and `round` of a whole number are the number itself.

A negative number to `sqrt`, or zero or less to a logarithm, has no whole-number
result, so the program stops with `math::sqrt of a negative number` (exit code 101),
as `x / 0` stops.

Because the rounding may not be what was meant, each of these calls gives the
warning W0112, with the fix for a fractional result: convert first with
`value::to<float64>()`. A `pow` with a constant exponent of zero or more has no
fraction and gives no warning.

## Not built yet

Vectors, matrices, quaternions and exact numbers are described in
[Maths and exact numbers](design-maths.md). More functions (`asin`, `acos`, `atan`,
`sinh`, ...) can be added the same way when they are decided.
