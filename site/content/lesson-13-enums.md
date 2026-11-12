---
title: "Lesson 13: Enums and optionals"
description: Values that are one of several variants, taking them apart with match, and the optional type T?.
status: not-implemented
---

# Lesson 13: Enums and optionals

The compiler can't run this lesson yet; the code shows the planned design.

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

## A value that might be missing

`T?` is an optional: either a value of type `T` or `none`. It is an ordinary
enum of the standard library, `enum Optional<T> { Some(T value), none, }`, so
everything above applies to it. The word `none` is both the "returns nothing"
type and the empty optional.

A variable of optional type with no value starts as `none`. Looking up a key
that a map does not have also gives `none`.

<!-- focus: 1 -->
`age` has no value, so it is `none`.

<!-- focus: 2 -->
`maybe` unwraps an optional and gives a fallback when it is `none`.

<!-- focus: 4-7 -->
Or `match` on it, as with any enum: a case with a typed name for the value, and
`case none` for the empty one.

```drachm
uint32? age;
uint32 n = age maybe 0;

match (age) {
    case uint32 a { io::outln(a::text()); }
    case none     { io::outln("no age"); }
}
```

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

See [Enums and optionals](design-enums.md) for the whole design. The next lesson
is about functions that can fail: [Lesson 14: Errors](lesson-14-errors.md).
