---
title: "Lesson 13: Enums and optionals"
description: Values that are one of several variants, taking them apart with match, and the optional type T?.
---

# Lesson 13: Enums and optionals

Enums and optionals (`T?`) both work in the compiler today.

**Goal:** model a value that is one of several shapes, and handle a value that
might not be there.

## An enum

An enum is a value that is exactly one of several named variants. A variant can
carry data, written in parentheses after its name, and a trailing comma is fine.

<!-- focus: 2-4 -->
`Shape` is a circle with a radius, a rectangle with two sides, or empty.

<!-- focus: 8 -->
When the type is known from the declaration, the variant name alone is enough.

<!-- focus: 9 -->
`Shape::Empty` always works, with the type written out.

```drachm
enum Shape {
    Circle(float64 r),
    Rect(float64 w, float64 h),
    Empty,
}

none main() {
    Shape s = Rect(2.0, 3.0);
    Shape t = Shape::Empty;
}
```

## Taking a variant apart

`match` looks at which variant a value is. Each `case` names a variant and gives
its data typed names. Like every `match` ([lesson 7](lesson-7-match-and-skip.md)),
leaving out a case is a warning.

<!-- focus: 3 -->
`Circle(float64 r)` matches a circle and names its radius `r`.

<!-- focus: 4 -->
A rectangle brings two names.

<!-- focus: 5 -->
`Empty` carries nothing, so it needs no parentheses.

```drachm
float64 area(&Shape s) {
    match (s) {
        case Circle(float64 r)          { return 3.14159 * r * r; }
        case Rect(float64 w, float64 h) { return w * h; }
        case Empty                      { return 0.0; }
    }
}
```

### Your turn: a new variant

Add a `Square(float64 side)` variant and a case for it, so that all three lines
print.

```drachm title="starter"
in io;

enum Shape {
    Circle(float64 r),
    Rect(float64 w, float64 h),
    Empty,
}

float64 area(&Shape s) {
    match (s) {
        case Circle(float64 r)          { return 3.14159 * r * r; }
        case Rect(float64 w, float64 h) { return w * h; }
        case Empty                      { return 0.0; }
    }
}

none main() {
    Shape a = Rect(2.0, 3.0);
    Shape b = Square(4.0);
    Shape c = Shape::Empty;
    io::outln(area(&a)::text());
    io::outln(area(&b)::text());
    io::outln(area(&c)::text());
}
```

```text title="intended output"
6.0
16.0
0.0
```

<!-- solution-of: solution-13-1 -->
```drachm solution
in io;

enum Shape {
    Circle(float64 r),
    Rect(float64 w, float64 h),
    Square(float64 side),
    Empty,
}

float64 area(&Shape s) {
    match (s) {
        case Circle(float64 r)          { return 3.14159 * r * r; }
        case Rect(float64 w, float64 h) { return w * h; }
        case Square(float64 side)       { return side * side; }
        case Empty                      { return 0.0; }
    }
}

none main() {
    Shape a = Rect(2.0, 3.0);
    Shape b = Square(4.0);
    Shape c = Shape::Empty;
    io::outln(area(&a)::text());
    io::outln(area(&b)::text());
    io::outln(area(&c)::text());
}
```

## Enums that own data

A variant can hold a `string`. Then the enum owns memory, so it moves like a
`string` does instead of copying. `match` looks at it where it is and does not
move it, so a `case` that ignores the data (`case Text(_)`) leaves the value
whole.

To use the data, the pattern has to say what happens to it. `take string body`
moves the text out of the value, and the value is used up afterwards.
`copy string body` makes a copy and leaves the value alone. A plain
`string body` is an error (E0528) that offers both. Numbers and bools bind
plainly, as in `Circle(float64 r)`.

A `case` can also carry a guard (`case Circle(float64 r) if r > 10.0`), several
patterns that bind the same names (`case Circle(float64 x), Rect(float64 x, _)`),
a constant inside the pattern (`case Rect(2.0, float64 h)`), and patterns inside
patterns. The [reference page on enums](reference-enums.md) has them all.

<!-- example: enum-owning -->

## A value that might be missing

`T?` is an optional: either a value of type `T` or `none`. It is an enum with two
variants, so everything above applies to it. The word `none` is both the "returns
nothing" type and the empty optional.

A variable of optional type with no value starts as `none`. A `T` goes to a `T?`
by itself: `uint32? age = 42;` needs no wrapping.

`age` has no value, so it is `none`, and `maybe` unwraps an optional and gives a
fallback when it is `none`. `has value` and `is empty` ask the same question in
words. Or `match` on it, as with any enum: a case with a typed name for the value,
and `case none` for the empty one.

<!-- example: option-basic -->

`x == none` is not how you test for it: the compiler says to write `is empty`.
The [optionals part of the reference](reference-enums.md#optionals) lists the rest.

### Your turn: a fallback

`pick` returns its number when it is even and `none` otherwise. Print both
results, using `maybe` so that a missing value prints 0.

```drachm title="starter"
in io;

uint32? pick(uint32 n) {
    if (n % 2 == 0) { return n; }
    return none;
}

none main() {
    uint32? a = pick(8);
    uint32? b = pick(7);
    // your code here: print a, then b, with 0 as the fallback
}
```

```text title="intended output"
8
0
```

```drachm solution
in io;

uint32? pick(uint32 n) {
    if (n % 2 == 0) { return n; }
    return none;
}

none main() {
    uint32? a = pick(8);
    uint32? b = pick(7);
    io::outln((a maybe 0)::text());
    io::outln((b maybe 0)::text());
}
```

The next lesson is about functions that can fail:
[Lesson 14: Errors](lesson-14-errors.md).
