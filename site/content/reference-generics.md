---
title: Generics
description: Generic functions, type arguments and how they are worked out, how a generic function is checked, and the type syntax Name<A, B>, T[?] and T[N].
---

# Generics

A generic function is written once and works for many types. The compiler
makes one real function for each set of types it is called with, so a generic
call is exactly as fast as one written out by hand. This page states the
rules for what builds today, then interfaces and constraints; generic units
are at the end, marked. The [lesson](lesson-15-generics.md) and
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

A generic function with no constraint on its parameters is not checked where
it is written (see [Constraints](#constraints) for the ones that have one).
Each call makes an instance, a function for those types, and the
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

`struct Cell<T> { ... }` and `enum Maybe<T> { ... }` take type parameters the
same way, and `Cell<uint32>` names an instance. Each instance is its own
type with its own fields, methods, operators, constructors, `drop()`, `fill`
and `def`s, laid out and checked as if you had written `Cell_uint32` by hand.
`Cell<uint32>` written in two places is one type; `Cell<uint32>` and
`Cell<sint32>` are different types. A generic struct's own methods use only
the struct's parameters: a method with type parameters of its own is E0901.

<!-- example: gen-cell -->

Generic struct arguments are never worked out for you: a declaration, `new`
and a static call write the whole type (`new Cell<uint32> of (5)`,
`Cell<string>::make("held")`, `Maybe<uint32>::Nothing()`). Where the type is
known, a bare variant name works as for any enum (`Maybe<uint32> m = Just(5);`).
A generic function can take one, though, and works its parameters out from it:
`T unwrap<T>(Cell<T> c)` called with a `Cell<uint32>` has `T` = `uint32`.

<!-- example: gen-pair -->

An instance may be an argument of another (`Pair<Cell<uint32>, string>`).
Whether an instance is Copy follows the usual rule applied to that instance:
`Cell<uint32>` is Copy and `Cell<string>` is not. A `def` is for one
instance (`def Origin of Vec2<sint32> { ... }`), and `struct Cell<T> defaults
Start` looks for a def of that name for each instance.

As with generic functions, the instance is what gets checked. An error inside
a method shows at its own line with the instance that made it:

```text
error[E0112]: `Opaque` has no `>` operator
   --> in_instance.drh:7:28
    |
  7 |     bool bigger() { return self.a > self.b; }
    |                            ^^^^^^^^^^^^^^^
    |                            ------ this is `Opaque`
   ...
 13 |     Holder<Opaque> h = new Holder<Opaque> of (new Opaque of (1), new Opaque of (2));
    |     ------ instantiated here
    |
 note: while checking `Holder<Opaque>`, instantiated here
```

Writing a generic type without arguments (`Cell c;`) is E0604, the wrong
number of arguments or arguments on a type that takes none is E0602, a struct
that holds itself through an instance is E0210 as for any struct, and a type
that asks for ever bigger instances of itself is E0608. A type argument that
is a borrow (`Stack<&string>`) is not supported yet (E0901).

## Optional types

`Optional<uint32>` is another way to write `uint32?`; the two are the same
type, and can be mixed freely.

<!-- example: gen-optional -->

## Interfaces

An interface lists what a type must have: method signatures, `static`
signatures and required fields. It has no method bodies. A struct says which
interfaces it implements with `with`, several joined by `and`, and the
compiler checks it once every declaration has been read.

<!-- example: iface-shape -->

A struct implements an interface method with a method of the same name,
`public`, with the same parameters and result and the same `static`ness.
Operator methods can be listed too (`bool <(Self o);`). A struct's own `==`
and `!=` need no method: every struct has them.

- A method missing, or only written under `private:`, is E0610.
- A method with other parameters, another result or a different `static` is
  E0611.
- A required field missing, of another type, or not `public` is E0614.
- An unknown name after `with` or `extends` is E0201, and an interface name
  can not be used as the name of a struct, enum or other interface (E0202).

```text
error[E0610]: `Circle` says `with Shape`, but has no method `unit`
  --> iface_missing.drh:6:20
   |
 3 |     static Self unit();
   |            ----------- `Shape` promises `unit`
  ...
 6 | struct Circle with Shape {
   |                    ^^^^^ `Shape` needs a `static` method `unit`
   |
 help: add `static Circle unit() { ... }` to `Circle`
```

`extends` makes an interface include another's members: `interface Placed
extends Shape and Positioned`. A struct that implements `Placed` implements
`Shape` and `Positioned` too. Two parents with a member of the same name need
equal signatures, and an interface that extends itself, directly or through
others, is E0615.

## Self

Inside an interface, `Self` stands for the type that implements it, so
`static Self zero();` means `static Circle zero()` for `Circle` and `static
Cube zero()` for `Cube`. It is a reserved word with no meaning outside an
interface (E0616): in a struct, write its name.

## Methods that change self

A struct method's mode is worked out from its body: it either reads `self` or
changes it. An interface cannot see a body, so a method that may change
`self` says so after its parameters. A struct method may change `self` only
where the interface method says `changes self`; changing less is fine. A
method that writes only `mut` fields counts as changing. Writing `changes
self` anywhere but in an interface is E0613. (`changes` is an ordinary name
everywhere else.)

<!-- example: iface-counter -->

```text
error[E0612]: `Hits::get` changes `self`, but `Counter::get` does not say `changes self`
   --> iface_changes.drh:15:20
    |
  3 |     uint32 get();
    |     ------------ `Counter::get` promises not to change `self`
   ...
 15 |     uint32 get() { self.n += 1; return self.n; }
    |                    ^^^^^^ changes `self.n` here
    |
 help: add `changes self` to `Counter::get`: `uint32 get() changes self;`, or stop changing `self`
```

## Interfaces as types

An interface can only be borrowed, never held: `Shape s` and `Shape[?]` are
E0617. A borrow of one, `&Shape` or `&mut Shape`, accepts a borrow of any
struct that implements it, and a call through it runs the method of the real
struct while the program runs. Any interface can be borrowed this way.

<!-- example: dyn-report -->

Where a `&Shape` is wanted (an argument, a `&Shape s = &c;` declaration, a
return) a `&Circle` or a `&Square` is converted by itself. A `&mut Counter`
takes a `&mut` of a struct and can call the methods that say `changes self`;
through a plain `&Counter` they are E0311, like any change through a `&`.

<!-- example: dyn-mut -->

A required field is read through the borrow (`p.x`), and a `&Placed` becomes
a `&Shape` or a `&Positioned` where one of those is wanted, because `Placed`
extends them. The borrow of the struct lasts as long as the `&Shape` is
used, exactly as for a `&Circle`.

<!-- example: dyn-fields -->

What a `&Shape` cannot do:

- A `static` method, a method that has `Self` in its signature, or an
  operator method is not callable through it: the struct behind it is not
  known (E0619). Call it on the struct, or in a generic constrained with the
  interface.
- A struct that does not implement the interface does not convert (E0640).
- A method that returns a borrow, a required field written to, a `&Circle`
  where `Circle` itself holds borrows, and a list of `&Shape` are E0901 for now.

```text
error[E0640]: `Plain` does not implement `Shape`, so `&p` is not a `&Shape`
  --> dyn_errors.drh:63:10
   |
25 | struct Plain {
   |        ----- `struct Plain` is declared here
  ...
63 |     look(&p);
   |          ^^ this is `&Plain`
   |
 help: add `with Shape` to `struct Plain`
```

```text
error[E0619]: `same` cannot be called through a `&Shape`: it has `Self` in its signature
  --> dyn_errors.drh:44:8
   |
44 |     s::same(s);
   |        ^^^^ the type behind the borrow is not known here
   |
 help: call it on a value of the struct, or in a function constrained with `<T with ...>`
```

A generic function constrained by an interface (`T with Shape`) is the other
way to write code over several kinds: the kind is fixed for each call, and the
call is not looked up while the program runs.

## Constraints

`T with Shape` after a type parameter asks for a type that implements the
interface; `T with Shape and Drawable` asks for several. It works on generic
functions and on generic structs, and `T` can be given a constraint while
another parameter in the same list has none.

<!-- example: iface-total -->

A constrained generic is checked twice, in two different ways.

**Where it is written**, once, whether or not anything uses it. The body is
checked with `T` standing for a type that has only what the constraints
promise: the methods of the interfaces and of the interfaces they extend,
their required fields, and their `static` methods, called as `T::zero()`.
Anything else is E0606, even if every type used so far happens to allow it.
An operator on a `T` is E0606 unless an interface declares that operator
method. A `T` can be passed to another constrained generic only if its own
constraints include the other's; a method that says `changes self` needs a
`&mut T` or a `mut` binding (E0311, E0312), whatever the type that is used
later does. A constrained template nothing uses is therefore not reported as
unused (no W0600).

```text
error[E0606]: `T` has no method `perimeter` that its constraint `Shape` promises
  --> define.drh:9:53
   |
 9 | float64 wrong_method<T with Shape>(T v) { return v::perimeter(); }
   |                             ----- `T` is constrained here
   |                                                     ^^^^^^^^^ not offered by `Shape`
   |
 note: in the definition of `wrong_method`
 help: add it to `Shape`, or constrain `T` with an interface that has it
```

**At each use**, the type given for `T` must implement the interfaces
(E0607). The instance is then an ordinary function or struct, so a call of
`s::area()` is a direct call, with nothing looked up while the program runs.

```text
error[E0607]: `Triangle` does not implement `Shape`, which `total` asks of `T`
   --> unmet.drh:26:5
    |
 22 | float64 total<T with Shape>(T a) { return a::area(); }
    |                      ----- `T` must implement `Shape`
   ...
 26 |     total(t);
    |     ^^^^^^^^ `T` is `Triangle` here
    |
 help: write `struct Triangle with Shape` and give it what `Shape` lists
```

A parameter with no constraint is checked per use, as before: what the body
does with it is checked when a call gives it a type, and the error points at
the body and the call. In a list that mixes the two, what depends on the
unconstrained parameter waits for a use, and the rest is checked at the
definition.

## Types you do not own
<!-- status: tentative -->

Any type can promise an interface from outside its own declaration, by
writing its name, `with`, the interface and the methods:
`uint8 with Describe { string describe() { ... } }`. The declaration can be
anywhere in the project. For a number type, `bool` or `string` the methods
get the value as `self`; for a struct they join the struct's own methods
(including its private fields). Operators and constructors cannot be given
to a built-in type yet, and enums and generic instances cannot promise
interfaces this way yet (E0901).

The standard interfaces below are already built in for numbers and `string`,
so `T with Ordered` works with `uint32` and `float64` without a declaration.
A program that declares its own struct, enum or interface with one of these
names uses its own instead; `uint32 with Zero { ... }` then promises that
one.

| Interface | Members | Implemented by |
|---|---|---|
| `Zero` | `static Self zero();` | numbers, `string` (`""`) |
| `Equal` | `bool ==(Self o);` | numbers, `bool`, `string` |
| `Ordered` (extends `Equal`) | `bool <(Self o);` and `<=`, `>`, `>=` | numbers |
| `Text` | `string text();` | numbers, `bool` |

The set and members above are the first attempt and may change. A struct can
implement `Equal` without writing `==`, because every struct has one.

<!-- example: iface-zero -->

## Generic over units
<!-- status: tentative -->

`float64<U> twice<unit U>(float64<U> x)` would work with any unit and keep it.
Today `unit U` is E0901. The spelling is still open.
