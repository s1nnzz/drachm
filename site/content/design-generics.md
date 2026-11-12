---
title: Generics and interfaces
description: Type parameters, with constraints, interfaces, changes self, extends, Self and your own iterables.
status: not-implemented
---

# Generics and interfaces

Generic code works for many types, and an interface describes what a type must
be able to do. Generic functions, structs and enums build (see [Generics](reference-generics.md));
the rest of this page is designed for step 6 and the compiler does not accept
it yet.

## Generic functions and structs

Type parameters are written in `<>`. The compiler infers them from the
arguments, and you write them out only when it cannot. Generic functions and
structs work today and are described in [Generics](reference-generics.md).

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

An interface lists method signatures, `static` methods and required fields, a
struct says which it implements with `with`, `extends` builds one interface on
others, `Self` names the implementing type, and `changes self` marks an
interface method that may change `self`. These build today and are described in
[Generics](reference-generics.md).

An interface name used as a type means any struct that has it, dispatched while
the program runs: `none report(&Shape s) { ... }`. That part is not built yet.

## Your own iterables

<!-- status: tentative -->

A type works with `for (T x of thing)` when it implements `Iterable<T>`, whose
`T? next()` returns `none` when there is nothing left. The `Iterable` name and its exact shape are written down in the design but the
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

<!-- status: tentative -->

A unit in a signature is fixed today (see [Units of measure](reference-units.md)).
Once generics exist, a function can be generic over a unit. The `<unit U>`
spelling is the planned form, not a decided one.

```drachm
float64<U> twice<unit U>(float64<U> x) { return x * 2; }
```
