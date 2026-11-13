---
title: Generics and interfaces
description: The design of type parameters, constraints, interfaces, Self, changes self and iterables, and the parts that are still to come.
---

# Generics and interfaces

Generic functions and structs, constraints with `with`, interfaces, `Self`,
`changes self`, interfaces as `&Shape` borrows, generic interfaces and
`Iterable` all work. Their rules and examples are in
[Generics](reference-generics.md), and the [lesson](lesson-15-generics.md) and
[guide](guide-generics.md) introduce them. This page keeps the design
and marks the parts the compiler does not accept yet.

## Generic functions and structs

Type parameters are written in `<>`, inferred from the arguments, and each
call makes its own function. See [Generic functions](reference-generics.md#generic-functions),
[Working out the type parameters](reference-generics.md#working-out-the-type-parameters)
and [Generic structs and enums](reference-generics.md#generic-structs-and-enums).

## Constraints with with

`T with Shape` asks for a type that implements an interface, and `T with Shape
and Drawable` for several. See [Constraints](reference-generics.md#constraints).

## Interfaces

An interface lists method signatures, `static` methods and required fields.
A struct says which interfaces it implements with `with`, several joined by
`and`, and `extends` builds one interface on others. See
[Interfaces](reference-generics.md#interfaces) and
[Generic interfaces](reference-generics.md#generic-interfaces).

## Self, and methods that change self

`Self` names the implementing type inside an interface, and `changes self` marks
an interface method that may change `self`. See [Self](reference-generics.md#self)
and [Methods that change self](reference-generics.md#methods-that-change-self).

## Interfaces as types

An interface is borrowed, never held: `&Shape` accepts any struct that
implements it, and the method that runs is picked while the program runs. See
[Interfaces as types](reference-generics.md#interfaces-as-types).

## Iterables

`Iterable<T>` has one method, `T? at(uint64 i)`, and `for` walks it. See
[Iterable](reference-generics.md#iterable).

## Types you do not own

Numbers, `bool` and `string` promise the standard interfaces `Zero`, `Equal`,
`Ordered` and `Text` already. Any type can promise an interface from outside its
declaration. This is the first attempt and is tentative: see
[Types you do not own](reference-generics.md#types-you-do-not-own).

## Still to come

These parts are designed but not built. Each carries its own marker.

### Constraints on units

<!-- status: not-implemented -->

A unit parameter with a constraint, `unit U with Shape`, is E0901 today. The
spelling is open: an interface over units is not designed yet.

### Methods with type parameters of their own

<!-- status: not-implemented -->

A method may have type parameters only through an interface method, so that
every extra parameter is visible in the interface:

```drachm
interface Mappable<T> { U[?] map<U>((T) => U f); }
```

A struct's own methods use only the struct's parameters. The interface form is
E0901 today.

### Borrowing a generic interface

<!-- status: not-implemented -->

`&Read<uint32>` is E0901 today: a generic interface can be implemented and
used through a constraint, but not yet through a borrow.

### Array lengths as parameters

<!-- status: not-implemented -->

The length of an array could be a generic value parameter:

```drachm
struct Buffer<T, uint32 N> { T[N] items; }
```

Today this is a parse error (E0003), not the E0901 the design asks for. Array
lengths are written as numbers.

### Exclusive parameters

<!-- status: not-implemented -->

`none show(exclusive &Shape s)` would accept only a value whose interface is
exactly `Shape`, with no upcast from `Shape3D`. The word `exclusive` is E0901
today, and its details are still to design.

### Returning a function value

<!-- status: not-implemented -->

A function value only goes down: it can be a parameter, a local or be called.
Returning one, or storing it in a struct or list, is E0652 by design, and a
way to keep one (a `move` lambda, for example) is not designed yet. See
[Function types and lambdas](reference-generics.md#function-types-and-lambdas).
