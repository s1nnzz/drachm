---
title: Generics and interfaces
description: When to write code over any type, when to describe what a type can do with an interface, and how the two combine.
status: not-implemented
---

# Generics and interfaces

Generics and interfaces are designed for step 6; the compiler does not accept
them yet, so every block here shows the intended shape and none of it is
runnable. [Generics and interfaces](design-generics.md) lists the syntax; this
guide is about choosing between the tools.

## Two different questions

- "This code does not care what type it works on." That is a **generic**: a type
  parameter in `<>`.
- "This code needs the type to be able to do something." That is an
  **interface**: a list of what a type must offer, which a struct says it
  implements with `with`.

Most real code needs both, which is why `with` also appears inside the `<>`.

## Generic functions and structs

Write the type as a parameter and use it like any type. The compiler works the
parameters out from the arguments, so you write them yourself only when it
cannot.

<!-- focus: 1,3,5,6 -->

```drachm
T max<T>(T a, T b) { if (a > b) { return a; } return b; }

struct Stack<T> { T[?] items; none push(T item) { self.items::push(item); } }

uint32 m = max(a, b);               // T is uint32, from the arguments
uint32 n = max<uint32>(5, 9);       // written out
```

Reach for a generic when you are about to copy a function and change only the
type. When the body needs the type to offer something specific, say so with a
constraint, as in the next section.

## Constraints with with

`with` after a type parameter requires the type to implement an interface. Now
the body can use what the interface offers, and callers get an error if they
pass a type that does not have it.

```drachm
float64 total<T with Shape>(T[?] shapes) { /* ... */ }
```

Constrain by the least you need. A function that only calls `area()` should say
`with Shape`, not a larger interface that happens to include it.

## Interfaces

An interface lists method signatures, `static` methods and required fields. It
has no default method bodies. A struct says which interfaces it implements with
`with`, several joined by `and`.

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

Interfaces can require fields (`interface Positioned { float64 x; float64 y; }`)
and build on others (`interface Shape3D extends Shape { float64 volume(); }`).
Because there are no default bodies, an interface is only a promise, and what
each struct does is written in that struct.

## Interface as a type, or as a constraint

An interface name used as a type means "any struct with it", and which one is
decided while the program runs.

<!-- focus: 1,4 -->

```drachm
none report(&Shape s) { io::outln(s::area()->text()); }

float64 total<T with Shape>(T[?] shapes) { /* ... */ }
```

The two look alike and differ:

| | Interface as a type: `&Shape` | Constraint: `T with Shape` |
|---|---|---|
| Which struct | any, decided while running | one type, chosen per call |
| Use it when | the caller has a value and the kind does not matter | the code deals in one kind at a time, or must return that same kind |

In `total`, every element of `T[?]` is the same `T`. If the function gives back
something of the type it was given, it has to be a generic:
`T biggest<T with Shape>(T[?] shapes)` returns a `T`, whereas an interface type
only says "some `Shape`".

## Self

`Self` names the implementing type inside an interface, for things that return
or take the type itself.

```drachm
interface Zero { static Self zero(); }
```

## Methods that change self

Struct methods infer whether they only read `self` or change it. An interface
has no body to look at, so a method that may change `self` says so after its
parameters. A struct method may not change more than the interface allows.
`changes self` is written only in interfaces.

<!-- focus: 2 -->

```drachm
interface Counter {
    none bump() changes self;
    uint32 get();
}
```

Write `changes self` only for methods that need it. A method without it can be
called through a `&` borrow, and one with it needs `&mut`; so every `changes
self` you add makes the interface harder to use on borrowed values.

## Your own iterables

A type works with `for (T x of thing)` when it says `with Iterable<T>` and has a
method `T? at(uint64 i)`. The loop calls `at(0)`, `at(1)`, ... until one gives
`none`. The value is only read, so it can be walked again.

<!-- example: iterable -->

A type can be `Iterable` over several item types. The variable's type picks
which `at` the loop uses.

<!-- example: iterable-two -->

## Generic over units

A unit in a signature is fixed (see [Units of measure](reference-units.md)):
`float64<m>` is metres and nothing else. A function can instead work over any
unit, and the result keeps the unit it was given. You write `unit U` once and
use `U` where the unit goes:

```drachm
float64<U> twice<unit U>(float64<U> x) { return x * 2; }
```

You never say which unit `U` is: `twice(2.5<m>)` gives metres and
`twice(1.5<km>)` kilometres, because the compiler reads the unit from the
argument. The [reference](reference-generics.md#generic-over-units) says how
that works when no argument has a unit.

## Which tool when

| You want | Use |
|---|---|
| The same code for many types, no special needs | a generic: `T max<T>(...)` |
| A container of anything | a generic struct: `Stack<T>` |
| To use a method on the type | `T with Interface` |
| One function over values of mixed kinds | an interface type: `&Shape` |
| A function that returns the type it was given | a generic |
| Code that works for any unit | `<unit U>` |
