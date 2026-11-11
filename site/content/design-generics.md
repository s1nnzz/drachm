---
title: Generics and interfaces
description: Type parameters, with constraints, interfaces, changes self, extends, Self and your own iterables.
status: not-implemented
---

# Generics and interfaces

Generic code works for many types, and an interface describes what a type must
be able to do. Both are designed for step 6; the compiler does not accept them
yet.

## Generic functions and structs

Type parameters are written in `<>`. The compiler infers them from the
arguments, and you write them out only when it cannot.

<!-- focus: 1,5,6 -->

```drachm
T max<T>(T a, T b) { if (a > b) { return a; } return b; }

struct Stack<T> { T[?] items; none push(T item) { self.items::push(item); } }

uint32 m = max(a, b);
uint32 n = max<uint32>(5, 9);
```

## Constraints with with

`with` after a type parameter requires the type to have an interface.

```drachm
float64 total<T with Shape>(T[?] shapes) { /* ... */ }
```

## Interfaces

An interface lists method signatures, `static` methods and required fields. It
has no default method bodies. A struct says which it implements with `with`,
and several are joined with `and`.

<!-- focus: 1-3,5 -->

```drachm
interface Shape {
    float64 area();
}

struct Circle with Shape and Drawable {
    float64 r;
    float64 area() { return 3.14159 * self.r * self.r; }
}
```

An interface name used as a type means any struct that has it, dispatched while
the program runs: `none report(&Shape s) { ... }`. An interface can require
fields (`interface Positioned { float64 x; float64 y; }`) and can build on
others: `interface Shape3D extends Shape { float64 volume(); }`.

## Self

`Self` names the implementing type inside an interface.

```drachm
interface Zero { static Self zero(); }
```

## Methods that change self

Struct methods infer whether they only read `self` or change it. An interface
cannot see a body, so a method that may change `self` says so after its
parameters. A struct method may not change more than the interface allows.
`changes self` is written only in interfaces.

```drachm
interface Counter {
    none bump() changes self;
    uint32 get();
}
```

## Your own iterables

A type works with `for (T x of thing)` when it implements `Iterable<T>`, whose
`T? next()` returns `none` when there is nothing left.

<!-- status: tentative -->

The `Iterable` name and its exact shape are written down in the design but the
standard library's interfaces are not fixed yet.

```drachm
struct Countdown with Iterable<uint32> {
    mut uint32 n;
    uint32? next() {
        if (self.n == 0) { return none; }
        self.n--;
        return self.n + 1;
    }
}
```

## Units and generics

A unit in a signature is fixed today (see [Units of measure](reference-units.md)).
Once generics exist, a function can be generic over a unit:

<!-- status: tentative -->

The `<unit U>` spelling is the planned form, not a decided one.

```drachm
float64<U> twice<unit U>(float64<U> x) { return x * 2; }
```
