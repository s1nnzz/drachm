---
title: Generics
description: Generic functions, type arguments and how they are worked out, how a generic function is checked, and the type syntax Name<A, B>, T[?] and T[N].
---

# Generics

A generic function is written once and works for many types. The compiler
makes one real function for each set of types it is called with, so a generic
call is exactly as fast as one written out by hand. This page states the
rules for what builds today; generic structs and enums, constraints and
generic units are at the end, marked. The [lesson](lesson-15-generics.md) and
the [guide](guide-generics.md) introduce the ideas.

## Generic functions

Type parameters go in `<>` after the function's name, and can be used
anywhere in the signature and the body where a type goes. A function can have
several, separated by commas.

<!-- example: gen-max -->

A name for a type parameter is a name like any other, so it cannot be a
struct, enum, unit or built-in type name, and a list cannot name one twice
(E0605). A function with type parameters is not a function until it is
called, so its name alone is not a value (E0604), and it is defined once like
any other function (E0202).

## Working out the type parameters

Each parameter is worked out from the arguments, left to right. The first
argument that has a type decides, and a later argument for the same parameter
must have exactly that type. `T` in a parameter written `&T` or `&mut T` is
what the borrow points at, and in `T?` it is the type inside the optional.

<!-- example: gen-swap -->

A number written as a constant, such as `300` or `2.5`, has no type of its
own. It takes the type the first typed argument fixed, and fails the way
`uint8 x = 300;` does when it does not fit. When every argument that shows `T`
is a constant, `T` is the type a constant has by itself: `uint32` for a whole
number and `float64` for a decimal.

When the arguments cannot tell, write the types out after the name:
`max<uint8>(5, 200)`. A call writes all of the type arguments or none of them.
Two arguments that disagree are E0603, and a call where nothing shows a
parameter is E0601:

```text
error[E0601]: cannot tell what `T` is in this call of `make`
  --> make.drh:16:16
   |
 3 | T make<T>() {
   |   ---- `T` is declared here
...
16 |     uint32 a = make();
   |                ^^^^^^ no argument shows `T`
   |
 help: write it out: `make<uint32>()`
```

```text
error[E0603]: `T` is `uint8` from the first argument, but `string` from the second
  --> max.drh:20:16
   |
20 |     max(small, text);
   |         ----- this one is `uint8`
   |                ^^^^ this one is `string`
   |
 help: convert one of them, or write the type: `max<uint8>(...)`
```

More type arguments than parameters is E0602. Writing only some of them is
not supported yet (E0901), and a type argument cannot be a borrow yet (E0901):
write `&T` in the parameter instead.

## How a generic function is checked

A generic function has no constraints yet, so nothing is checked where it is
written. Each call makes an instance, a function for those types, and the
instance is checked as if the types had been written in the function by hand.
Two calls with the same types share one instance. Calls with different types
make different functions, named `max<uint32>` and `max<float64>` in messages
and dumps.

This means a body can use any operator or method that every type it is called
with has, and an error inside the body appears when a call asks for a type the
body does not work for. The error is shown at its own line in the function,
with the call that made the instance:

```text
error[E0112]: `Opaque` has no `>` operator
  --> in_instance.drh:7:9
   |
 7 |     if (a > b) { return a; }
   |         ^^^^^
   |         - this is `Opaque`
...
17 |     Opaque r = biggest(p, q);
   |                ------------- instantiated here
   |
 note: while checking `biggest<Opaque>`, instantiated here
```

A function nothing calls is never checked, and says so with warning W0600.

A generic function may call itself and other generic functions. A call with
the same types reuses the instance, so ordinary recursion works. A call that
asks for a bigger type each time (`grow<T>` calling `grow<T?>`) would never
stop, and is E0608 after 64 instances.

## Type syntax

A type name can be followed by type arguments in `<>`: `Map<string, uint32>`,
`Stack<Stack<uint32>>`. After a number type, `<` still opens a unit
(`float64<m/s>`), so `Vec<float64<m>>` is a `Vec` of metres. A type that takes
no arguments, such as `Point`, is E0602 when it is given some.

`[?]` after a type is a list that grows (`string[?]`), and `[4]` is an array of
four (`uint8[4]`); they can be stacked (`uint32[?][?]`) and followed by `?`
(`string[?]?`). A length of 0 is an error. Lists and arrays are described in
[Arrays and lists](reference-collections.md); a generic function can take
`&T[?]` and have `T` worked out from the list. An array length as a generic
value parameter (`struct Buffer<T, uint32 N>`) is not built yet (E0901).

## Generic structs and enums
<!-- status: not-implemented -->

`struct Stack<T> { ... }` and `enum Tree<T> { ... }` take type parameters the
same way, and `Stack<uint32>` names an instance. Each instance is its own
type with its own fields, methods and `drop()`. A generic struct's own
methods use only the struct's parameters.

```drachm
struct Cell<T> {
    public:
    T value;
}

Cell<string> c = new Cell<string> of ("held");
```

## Optional types
<!-- status: not-implemented -->

`Optional<uint32>` is another way to write `uint32?`; the two are the same
type.

## Constraints
<!-- status: not-implemented -->

`T with Shape and Drawable` requires the type to implement interfaces, and is
checked once at the definition. Today a `with` in a type parameter list is
E0901. See [Generics and interfaces](design-generics.md).

## Generic over units
<!-- status: tentative -->

`float64<U> twice<unit U>(float64<U> x)` would work with any unit and keep it.
Today `unit U` is E0901. The spelling is still open.
