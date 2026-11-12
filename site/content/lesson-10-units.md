---
title: "Lesson 10: Units of measure"
description: Numbers that carry a unit, how the compiler checks them, converting with ::in, and points and deltas.
---

# Lesson 10: Units of measure

**Goal:** attach units to numbers so the compiler catches mixing metres with
seconds, and convert between units on purpose.

## A number with a unit

A number type can take a unit in angle brackets: `float64<m>` is a number of
metres. A literal gets its unit the same way, with no space: `400.0<m>`. The
SI units come with `in si;`. At run time the units are gone, so the program is
as fast as one with plain numbers.

<!-- focus: 4-6 -->
A function can take and return measures. `distance / time` divides the units as
well as the numbers, so the result is metres per second.

<!-- focus: 9-12 -->
`::text()` prints the number only, and `::unitText()` gives the unit as a string.
You lay the two out yourself.

<!-- focus: 14-15 -->
`::in<km/h>()` changes the unit and keeps the number type. Here 5 m/s becomes
18 km/h.

<!-- focus: 17-20 -->
Multiplying makes new units: kilograms times metres per second squared is a
newton, and the compiler knows that `N` means the same thing.

<!-- focus: 22-23 -->
Adding two lengths in different units works: the result is in the unit of the
left side.

<!-- example: lesson-10-units -->

## What the compiler stops

Adding unlike units is an error, so a mix-up cannot reach the running program.

```drachm
float64<m> d = 5.0<m>;
float64<s> t = 2.0<s>;
float64<m> bad = d + t;
```

```text title="drachm check"
error[E0401]: units do not match: `float64<m>` and `float64<s>`
  --> main.drh:5:26
   |
 5 |     float64<m> bad = d + t;
   |                      - this is `float64<m>`
   |                          ^ this is `float64<s>`
   |
 note: values can only be added or subtracted when they are the same kind of unit
 help: to relate different kinds, multiply or divide them: `distance / time`
```

### Your turn: a speed

Fill in the calculation so that the program prints the speed.

```drachm title="starter"
in io;
in si;

none main() {
    float64<m> distance = 90.0<m>;
    float64<s> time = 12.0<s>;
    // your code here: a float64<m/s> called speed
    io::outln(speed::text() + " " + speed::unitText());
}
```

```text title="expected output"
7.5 m/s
```

<!-- solution-of: solution-10-1 -->
```drachm solution
in io;
in si;

none main() {
    float64<m> distance = 90.0<m>;
    float64<s> time = 12.0<s>;
    float64<m/s> speed = distance / time;
    io::outln(speed::text() + " " + speed::unitText());
}
```

### Your turn: add a run and a walk

`run` is in kilometres and `extra` in metres. Print the total in metres by
converting `run` with `::in`.

```drachm title="starter"
in io;
in si;

none main() {
    float64<km> run = 5.0<km>;
    float64<m> extra = 750.0<m>;
    // your code here: a float64<m> called total
    io::outln(total::text() + " " + total::unitText());
}
```

```text title="expected output"
5750.0 m
```

<!-- solution-of: solution-10-2 -->
```drachm solution
in io;
in si;

none main() {
    float64<km> run = 5.0<km>;
    float64<m> extra = 750.0<m>;
    float64<m> total = run::in<m>() + extra;
    io::outln(total::text() + " " + total::unitText());
}
```

## Points and deltas

A temperature of 20 degrees Celsius is a position on a scale, not an amount, so
adding two of them means nothing. A unit declared with `offset` is a **point**,
and `degC` comes with `in si;` as one. The difference of two points is a
**delta**, written `delta degC`.

<!-- focus: 7 -->
Point minus point is a delta.

<!-- focus: 10 -->
Point plus delta is a point again. Point plus point is an error.

<!-- focus: 13 -->
A point converts with its offset, so 20 degC is 293.15 K.

<!-- example: lesson-10-points -->

A `delta degC` is as big as a kelvin, so it converts to and from `K` freely.
The [units reference](reference-units.md#points-and-deltas) has the full rules.

### Your turn: how much warmer

Print the change from `start` to `end`. The result is a delta, so declare it as
one.

```drachm title="starter"
in io;
in si;

none main() {
    float64<degC> start = 18.0<degC>;
    float64<degC> end = 23.5<degC>;
    // your code here: a float64<delta degC> called change
    io::outln(change::text() + " " + change::unitText());
}
```

```text title="expected output"
5.5 delta degC
```

<!-- solution-of: solution-10-3 -->
```drachm solution
in io;
in si;

none main() {
    float64<degC> start = 18.0<degC>;
    float64<degC> end = 23.5<degC>;
    float64<delta degC> change = end - start;
    io::outln(change::text() + " " + change::unitText());
}
```

Your own units are declared with `unit`, as in `unit mi = 5280 ft;`. The
[reference](reference-units.md) lists the rules for every operator. The next
lesson covers the one place where the compiler trusts you instead:
[Lesson 11: Unsafe and pointers](lesson-11-unsafe.md).
