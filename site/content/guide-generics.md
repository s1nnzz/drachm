---
title: Generics and interfaces
description: When to write code over any type, when to describe what a type can do with an interface, and how the two combine.
---

# Generics and interfaces

Generics and interfaces work today. This guide is about choosing between the
tools; the [reference](reference-generics.md) has every rule and error, and the
[lesson](lesson-15-generics.md) walks through the first examples.

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

<!-- example: lesson-15-max -->

A generic struct works the same way. `Stack<string>` is a stack of strings:

<!-- example: lesson-15-stack -->

Reach for a generic when you are about to copy a function and change only the
type. When the body needs the type to offer something specific, say so with a
constraint, as in the next section.

## Constraints with with

`with` after a type parameter requires the type to implement an interface. Now
the body can use what the interface offers, and callers get an error if they
pass a type that does not have it.

<!-- example: iface-total -->

Constrain by the least you need. A function that only calls `area()` should say
`with Shape`, not a larger interface that happens to include it.

## Interfaces

An interface lists what a type must have: method signatures, `static` methods
and required fields. It has no method bodies. A struct says which interfaces it
implements with `with`, several joined by `and`, and the compiler checks that
it has every member.

<!-- example: lesson-15-shape -->

Interfaces can require fields (`interface Positioned { float64 x; float64 y; }`)
and build on others, several at once (`interface Placed extends Shape and
Positioned`, in the next example). Because there are no default bodies, an
interface is only a promise, and what each struct does is written in that struct.

<!-- example: iface-shape -->

## Interface as a type, or as a constraint

An interface name used as a type means "any struct with it", and which one is
decided while the program runs.

<!-- example: dyn-report -->

The two look alike and differ:

| | Interface as a type: `&Shape` | Constraint: `T with Shape` |
|---|---|---|
| Which struct | any, decided while running | one type, chosen per call |
| Use it when | the caller has a value and the kind does not matter | the code deals in one kind at a time, or must return that same kind |

In `total<T with Shape>(...)`, every element of the list is the same `T`. If the
function gives back something of the type it was given, it has to be a generic:
`T first<T with Shape>(T[?] shapes)` returns a `T`, whereas a `&Shape` result is
only a borrow of "some `Shape`".

## Self

`Self` names the implementing type inside an interface, for things that return
or take the type itself. In `static Self unit();` each struct that implements
the interface returns its own type.

<!-- example: iface-shape -->

## Methods that change self

Struct methods infer whether they only read `self` or change it. An interface
has no body to look at, so a method that may change `self` says so after its
parameters. A struct method may not change more than the interface allows.
`changes self` is written only in interfaces.

<!-- example: iface-counter -->

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

<!-- example: gen-twice-unit -->

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

## Still to come

Some generic and interface ideas are not built yet, and the design page marks
them: a constraint on a unit, methods with type parameters of their own, and
borrowing a generic interface. See [Generics and interfaces](design-generics.md).
