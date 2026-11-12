---
title: Units of measure in practice
description: Why units catch real bugs, and how to declare, convert and combine them, including temperatures and whole-number units.
---

# Units of measure in practice

A speed in miles per hour and a speed in metres per second are both "just
numbers" to most languages, and adding them compiles fine. Mixing units like
that has crashed real spacecraft. In Drachm you can write the unit into the
type, and the compiler refuses the mix. This guide shows how that works day to
day. The rules in table form are in [Units of measure](reference-units.md), and
[Lesson 10](lesson-10-units.md) is the gentle start.

## Why units catch bugs

A unit lives in the type, in angle brackets: `float64<m>` is a number of
metres. The compiler checks units while it compiles and then forgets them, so
a program with units runs exactly as fast as one with plain numbers.

Here is a mistake that plain numbers hide. The function wants an acceleration,
and the call passes a speed, which is a distance divided by a time and not a
distance divided by a time squared. With plain `float64` this runs and gives a
wrong answer. With units it does not compile.

```drachm
float64<N> push(float64<kg> mass, float64<m/s^2> accel) {
    return mass * accel;
}

float64<N> f = push(m, d / t);
```

```text title="drachm check"
error[E0401]: units do not match: expected `float64<m/s^2>`, found `float64<m/s>`
   --> main.drh:12:28
    |
 12 |     float64<N> f = push(m, d / t);
    |                            ^^^^^ this is `float64<m/s>`
    |
 note: a value fits a variable only when it measures the same kind of thing
 help: to get a value of another kind, multiply or divide: `distance / time`
```

Even a unit that is only another *size* of the same thing is checked. A
`unit ft;` that you declare yourself with no link to the metre is a different
kind of thing, and the compiler will not let you assign feet to metres. You
have to say how they relate, which the next section shows how to do.

## Turning units on

Units exist only if they are declared. `in si;` declares the common SI ones for
you. Without it, `m` is unknown, and the error lists how to get it:

```drachm
in io;

none main() {
    float64<m> d = 5.0<m>;
}
```

```text title="drachm check"
error[E0402]: unknown unit `m`
  --> main.drh:4:13
   |
 4 |     float64<m> d = 5.0<m>;
   |             ^ no unit with this name is declared
   |
 help: add `in si;` for the SI units, or declare it with `unit m;` (a base unit) or `unit m = ...;` (a size of other units)
```

Put `in si;` next to `in io;` at the top of the file. A number gets a unit in
two places: after a type (`float64<m>`) and directly after a literal (`9.6<s>`),
with no space. `9.6 < s` with spaces is a comparison.

## Base, scaled and derived units

There are three kinds of unit, and you can declare all of them yourself.

- A **base** unit stands alone: `unit m;`, `unit s;`, `unit kg;`. It measures
  its own kind of thing.
- A **scaled** unit is a size of another: `unit km = 1000 m;`. It measures the
  same kind of thing as `m`, so the two mix.
- A **derived** unit is made from others with `*`, `/` and `^`:
  `unit N = kg * m / s^2;`. It is the same unit as the expression it names.

`in si;` declares these for you. The program below also declares three units
of its own: `ft` as a size of the metre, `mi` as a size of the foot, and `mph`
as a derived one.

<!-- focus: 5-7 -->
Declare units at the top level. `ft` is 0.3048 metres, so feet now convert to
metres. `mi` is a size of `ft`, and `mph` is miles divided by hours, using the
SI hour `h`.

<!-- focus: 11-14 -->
Dividing a distance by a time gives a speed. You do not have to name the result
in a unit if the types already say it, but `float64<m/s>` documents it.

<!-- focus: 17-18 -->
`N` is the same unit as `kg*m/s^2`, so the product of a mass and an acceleration
fits a `float64<N>`.

<!-- focus: 21-22 -->
Derived units multiply through, and you can read the answer in any related
unit, here kilojoules.

<!-- focus: 25-26 -->
Scaled units of one kind can be mixed in a sum. The result is in the unit of
the left side.

<!-- focus: 29-34 -->
Units you declare join the rest: feet convert to metres, and a speed in `mph`
converts to `m/s`.

<!-- example: guide-units-kinds -->

Prefixes are not automatic. `km` is a unit that is declared with
`unit km = 1000 m;`, and not `k` applied to `m`. If you want `Mm`, declare it.

## Changing the unit: ::in and ::to

Two conversions look alike and do different jobs.

- `x::in<km>()` changes the **unit** and keeps the number type.
- `x::to<float32>()` changes the **number type** and keeps the unit.

Mixing them up is an error. Asking `::in` for a unit that measures another kind
of thing is E0407, and the label says so:

```drachm
float64<s> t = 2.0<m>::in<s>();
```

```text title="drachm check"
error[E0407]: cannot convert `float64<m>` to `s`
  --> main.drh:8:28
   |
 8 |     float64<s> t = 2.0<m>::in<s>();
   |                    ------ this is `float64<m>`
   |                            ^^ `s` measures a different kind of thing
   |
 help: `::in<unit>()` converts to a unit of the same kind
```

<!-- focus: 8-9 -->
`::in<km/h>()` turns 10 m/s into 36 km/h and stays a `float64`.

<!-- focus: 12-13 -->
`::to<float32>()` makes a smaller number and keeps `m/s`.

<!-- example: guide-units-convert -->

## Plain numbers in and out

Units do not wall your code off from ordinary numbers. The rules are short.

**Plain number in.** Where a unit is declared, a plain value gets that unit.
`mut float64<m> d = 5;` is 5 metres, and so is `d = 0;`. A plain number passed
to a parameter with a unit, or returned from a function with a unit, gets the
unit as well. This is on purpose: the declaration says what the number means.

**Measure out.** A measure assigned to a plain number slot becomes the amount
in the *base* unit. `float64 x = 250.0<g>` is `0.25`, because `g` is a
thousandth of `kg`, and `uint32 m = 3<km>` is 3000.

**In an expression.** `*` and `/` accept plain numbers, since they just scale:
`d * 2.0` is still metres. But `+`, `-`, `%` and comparisons need a unit on the
number, or the compiler stops you with E0408 and shows the fix. A bare `0` is
the one exception, because zero is zero in any unit. (A temperature point is
different, as you will see.)

<!-- focus: 16-20 -->
A plain number goes in with `= 5` and `d = 0`, and `d++` adds one metre.

<!-- focus: 23-26 -->
Going the other way gives the base-unit amount: 0.25 for 250 g, and 3000 for
3 km.

<!-- focus: 29-34 -->
`d * 2.0` is fine, and `d + 5.0<m>` carries its unit.

This is the error you get when a plain number meets a measure in a sum:

```text title="drachm check, with d + 5.0"
error[E0408]: a plain number cannot stand for a measure
  --> main.drh:8:27
   |
 8 |     float64<m> bad2 = d + 5.0;
   |                           ^^^ this number has no unit
   |
 help: give it a unit: `5.0<m>`
```

## Points and deltas: temperatures

Temperatures in degrees Celsius are not like metres. 20 degrees plus 20
degrees is not "40 degrees warmer" in any useful sense, and zero is a real
temperature and not "nothing". So Drachm splits such units in two.

A unit declared with `offset`, like `degC`, is a **point** on a scale. The
difference between two points is a **delta**, written `delta degC`, which is
the size of a kelvin.

- point `-` point is a delta
- point `+` delta is a point (so is point `-` delta)
- point `+` point is an error, and so is a point times or divided by anything

```text title="drachm check, with a + b and a * 2.0 on degC values"
error[E0409]: two points cannot be added
  --> main.drh:7:25
   |
 7 |     float64<degC> bad = a + b;
   |                         ^^^^^ this is a point, like `degC`
   |
 help: add a `delta` to a point, or subtract two points to get a delta

error[E0409]: a point cannot be multiplied or divided
   --> main.drh:10:24
    |
 10 |     float64<degC> d2 = a * 2.0;
    |                        ^ this is a point, like `degC`
    |
 help: subtract two points to get a delta, and scale that
```

<!-- focus: 9-10 -->
Two points subtract into a `delta degC`, and the unit text says so.

<!-- focus: 13-14 -->
A point plus a delta is a point again.

<!-- focus: 17-18 -->
A delta is the same size as a kelvin, so it converts to `K`, and the number
stays 9.5.

<!-- focus: 21-22 -->
A point converts with its offset: 12 degC is 285.15 K.

<!-- focus: 25-26 -->
A plain `0` is not a temperature. You write `0.0<degC>` to compare with
freezing, or the compiler asks for it:

<!-- example: guide-units-temp -->

```text title="drachm check, with morning > 0"
error[E0408]: a plain number cannot stand for a measure
   --> main.drh:13:28
    |
 13 |     float64<degC> ab = a > 0;
    |                            ^ this number has no unit
    |
 help: a plain zero is not a point; write `0<degC>`
```

## Whole-number units

Units go on every number type, whole numbers too. A whole number cannot hold a
fraction of a unit, so shrinking the unit throws part of the value away.
`uint32<km> km = metres::in<km>()` with 1700 metres is `1`, and the 700 m are
gone. The compiler warns when it happens without `::in`:

```drachm
uint8<km> far = 300<m>;
```

```text title="drachm check"
warning[W0102]: `300<m>` does not fit `uint8`
  --> main.drh:5:21
   |
 5 |     uint8<km> far = 300<m>;
   |                     ^^^^^^ becomes 0
   |
 note: `uint8` holds only whole numbers, from 0 to 255; the fraction is dropped
 help: use a value that fits or a bigger type; `::to<uint8>()` converts without this warning
```

When in doubt, keep the numbers in `float64` and round only at the end, or
choose the smaller unit (`m`) for the whole-number type.

## Results with no unit

Sometimes every unit cancels. A distance divided by a distance is a plain
number, with the scales folded in: `4.0<km> / 2.0<m>` is `2000.0`, not 2. A
speed times a time divided by a distance is plain too. The result of such an
expression is simply a number, so it goes into a `float64` with no unit, as
the last lines of the program show.

Also in this program, `1.0 / t` with `t` in seconds is a frequency, which the
`Hz` unit names.

<!-- focus: 6-8 -->
`1700 m` becomes `1 km` as a whole number.

<!-- focus: 15-16 -->
Decimals keep the fraction: 1.7 km.

<!-- focus: 19-24 -->
The ratio is a plain number: `4 km / 2 m` is 2000, and `v * t / 8 m` is 1.

<!-- focus: 27-28 -->
One over a time is a frequency.

<!-- example: guide-units-int -->

## Printing a measure

`::text()` prints only the number, and `::unitText()` gives the unit as text.
You lay them out yourself, as every example here does:

```drachm
io::outln(speed::text() + " " + speed::unitText());
```

A delta prints as `delta degC`, and a derived unit prints in the form you wrote
it, like `m/s`.

Next: [Working with text](guide-text.md).
