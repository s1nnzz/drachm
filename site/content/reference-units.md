---
title: Units of measure
description: Declaring units, numbers with units, arithmetic and conversion, points and deltas.
---

# Units of measure

A number can carry a unit: `float64<m>` is a number of metres. The compiler
checks that you never add metres to seconds, and then forgets the units: a
program with units runs exactly as fast as the same program with plain numbers.

The ready-made SI set (`in si;`) is not available yet, so declare the units
you use. Units that you write in one file are visible everywhere in it, in any
order.

## Declaring units

```drachm
unit m;                      // a new base unit
unit s;
unit kg;
unit N = kg * m / s^2;       // derived: ^ is a power inside a unit
unit km = 1000 m;            // scaled
unit Hz = 1 / s;
unit pct = 0.01;             // a ratio with a name
unit degC = K offset -273.15;  // a point on a scale (see below)
```

`unit` only starts a declaration at the start of a top-level item, so it is
still a fine name for a variable or a function. Unit names are plain ASCII
letters, digits and `_`, and live in their own namespace: a variable `s` and a
unit `s` do not clash. Two units that mean the same thing are the same type:
`float64<N>` and `float64<kg*m/s^2>` can be used for each other.

## Numbers with units

Write the unit after a number type, `float64<m/s>`, `uint8<km>`, or directly
after a number literal, `9.6<s>`, `100<m>`, `2.5f32<m/s>`. There is no space
between the number and `<`: `9.6 < s` is a comparison. A literal with a unit
gets its number type like any other literal, from the place it is used.

Whole numbers can hold units too. They cannot hold fractions of a unit, so
`uint8<km> s = n;` with `n` equal to 300 m gives 0 (and warns, see below).

## Arithmetic

| Expression | Result |
|---|---|
| `a + b`, `a - b`, `a % b` | same kind of unit, else [E0401](errors.md); the result is in the unit of `a` |
| `a < b`, `a == b`, ... | same kind of unit; compared through a common base |
| `a * b`, `a / b` | the units multiply or divide: `dist / time` is `float64<m/s>` |
| `a * k`, `k * a`, `a / k` | `k` is a plain number; it only scales `a` |
| `k / a` | the unit is `1/a`: `1.0 / period` is a frequency |
| `a ** n` | `n` must be a constant whole number ([E0404](errors.md)) |
| `-a` | same unit |
| `a++`, `a--` | adds or subtracts one of `a`'s own unit |
| `& \| ^ ~ << >>` | not allowed on a measure ([E0406](errors.md)) |

A plain number beside a measure in `+`, `-`, `%` or a comparison needs a unit
([E0408](errors.md)): write `d + 5<m>`. The one exception is zero, which has no
unit: `speed > 0` and `d + 0` are fine.

Number types work as they always do: the left side leads, so `2.5 * dist` is a
`float64<m>`, and a `uint32` times a `float64<m>` gives a `uint32<m>`.

When every unit cancels, the result is a plain number with the scale folded in:
`4.0<km> / 2.0<m>` is `2000.0`.

## Scaled units

Scaled units of one kind convert to the unit of the left side:
`10<km> + 100<m>` is `10.1<km>`, and `10<m> + 1<km>` is `1010<m>`. For two
values with different scales both sides are brought to a common base, added,
and brought back, so `2<mi> + 2<mi>` with `unit mi = 5280 ft;` is
`(5280*2 + 5280*2) ft / 5280`. When both sides have the same unit nothing is
scaled at all.

Whole numbers truncate toward zero when a unit shrinks, and the compiler
warns ([W0102](errors.md)) that the conversion can lose information, or that a
unit that grows can overflow. Say it is intended with `x::in<km>()`.

## `::in` and `::to`

`x::in<km>()` changes the **unit** and keeps the number type. `x::to<float32>()`
changes the **number type** and keeps the unit. A unit inside `::to<...>` is
[E0407](errors.md), and so is `::in` to a unit of another kind.

## Measures as plain numbers

A measure can go into a plain number slot. It becomes the amount in the base
unit: `uint32 t = 3<km>` is 3000, and `float64 f = 5.0<g>` is 0.005 when
`unit g = 0.001 kg;`. Whole numbers warn when this truncates.

The other direction, a plain number into a variable with a unit
(`float64<m> d = n;`), is not decided yet and is an error ([E0408](errors.md));
write `n * 1<m>` to make a plain number a measure.

## Printing

`::text()` prints the number only, and `::unitText()` gives the unit as a
string. You lay the two out:

```drachm
io::outln(speed::text() + " " + speed::unitText());   // 10.4 m/s
```

<!-- example: units -->

## Points and deltas

A unit declared with `offset` is a **point** on a scale, like `degC`. The
difference of two points is a **delta**, written `delta degC`.

- point `-` point is a delta; point `+` delta and point `-` delta are points.
- point `+` point, a point times anything, and `-point` are [E0409](errors.md).
- `delta degC` has the size of a kelvin: it converts freely to and from `K`.
- Points compare with points. A bare `0` is not unit-free beside a point
  (0 degC is a real temperature): write `0<degC>`.
- A point converts between units with its offset, and into a plain number it
  gives the reading in the base unit: `20.0<degC>` is 293.15.

<!-- example: units-points -->

## What is not there yet

`in si;` and the SI unit set, and units that are generic over a function's
arguments, are not available yet.
