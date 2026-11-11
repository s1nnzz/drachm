---
title: Enums and optionals
description: Enums that carry data, match on their variants, and the optional type T?.
status: not-implemented
---

# Enums and optionals

An enum is a value that is exactly one of several named variants, and a variant
can carry data. This is designed for step 5. The compiler does not accept an
`enum` yet and reports a syntax error where one starts.

## Declaring an enum

Each variant is a name, with the data it carries in parentheses if it has any.
A trailing comma is fine.

```drachm
enum Shape {
    Circle(float64 r),
    Rect(float64 w, float64 h),
    Empty,
}
```

## Making a value

When the type is known from the place, such as a declaration with a type, the
variant name alone is enough. The qualified form `Shape::Empty` always works.

<!-- focus: 1-2 -->

Bare variant names need the language server to complete them, because there is
no `Shape::` to type first.

```drachm
Shape s = Rect(2.0, 3.0);
Shape t = Shape::Empty;
```

## Matching on variants

`match` takes the variants apart. Each `case` names a variant and gives its data
typed names. As with every `match`, a missing case is a warning.

<!-- focus: 2-4 -->

```drachm
float64 area(&Shape s) {
    match (s) {
        case Circle(float64 r)          { return 3.14159 * r * r; }
        case Rect(float64 w, float64 h) { return w * h; }
        case Empty                      { return 0.0; }
    }
}
```

## Optionals

`T?` is the optional type `Optional<T>`, and it is an ordinary enum of the
standard library: either a value (`Some`) or `none`. The word `none` is both
the "returns nothing" type and the empty optional.

```drachm
enum Optional<T> { Some(T value), none, }
```

A variable of optional type with no value starts as `none`. A map lookup with a
missing key gives `none`. `maybe` unwraps an optional with a fallback, and
`match` can test it.

<!-- focus: 1,2,4,6 -->

```drachm
uint32? age;                           // none
uint32? found = ages["Bob"];
uint32 n = found maybe 0;

match (found) {
    case uint32 a { io::outln(a::text()); }
    case none     { io::outln("no age"); }
}
```
