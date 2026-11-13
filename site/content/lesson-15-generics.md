---
title: "Lesson 15: Generics and interfaces"
description: Functions and structs over any type, interfaces, with, changes self and Self.
---

# Lesson 15: Generics and interfaces

**Goal:** write code that works for many types, and describe what a type must
be able to do with an interface.

## A generic function

Type parameters go in `<>` after the name. The compiler works the type out from
the arguments, and you write it out only when it cannot.

<!-- focus: 4 -->
`T` stands for any type that can be compared with `>`.

<!-- focus: 12 -->
Both arguments are `uint32`, so `T` is `uint32` without being said.

<!-- focus: 13 -->
The numbers `5` and `9` have no type of their own, so `max<uint32>(5, 9)` writes it out.

<!-- example: lesson-15-max -->

A struct can take a type parameter too. `Stack<string>` is a stack of strings,
and `push` works for any `T`.

<!-- example: lesson-15-stack -->

### Your turn: a generic min

Write `T min<T>(T a, T b)` that returns the smaller of the two, and call it on
whole numbers and on decimals.

```drachm title="starter"
in io;

// your code here

none main() {
    io::outln(min(4, 9)::text());
    io::outln(min(2.5, 1.5)::text());
}
```

```text title="intended output"
4
1.5
```

<!-- solution-of: solution-15-1 -->
```drachm solution
in io;

T min<T>(T a, T b) {
    if (a < b) { return a; }
    return b;
}

none main() {
    io::outln(min(4, 9)::text());
    io::outln(min(2.5, 1.5)::text());
}
```

## Interfaces

A generic function that calls methods on its `T` needs to know they exist. An
interface lists the methods a type must have. It has signatures only, no bodies.
A struct says which interfaces it implements with `with`, and several are
joined with `and`.

<!-- focus: 3-4 -->
`Shape` asks for an `area` method that returns a `float64`.

<!-- focus: 7 -->
`Circle` says `with Shape`, so the compiler checks that it has `area`.

<!-- focus: 10 -->
The method is the ordinary one from [lesson 6](lesson-6-structs.md).

<!-- focus: 13-15 -->
A function that takes `&Shape` works with any struct that has the interface.

<!-- example: lesson-15-shape -->

An interface name used as a type means any struct that has it, and the right
method is picked while the program runs: `none report(&Shape s) { ... }`. A
type parameter can ask for an interface with `with` as well:
`float64 total<T with Shape>(T[?] shapes)`, which is in the
[reference](reference-generics.md#constraints).

## Methods that change self

A struct method is checked from its body: the compiler knows whether it only
reads `self` or changes it. An interface has no body to look at, so a method
that may change `self` says so after its parameters. A struct's method may not
change more than the interface allows. This is written only in interfaces.

<!-- focus: 4 -->
`bump` may change the counter.

<!-- focus: 5 -->
`get` only reads it.

<!-- example: lesson-15-counter -->

### Your turn: implement an interface

Make `Square` implement `Shape`, so the call through `&Shape` prints the area.

```drachm title="starter"
in io;

interface Shape {
    float64 area();
}

struct Square {
    public:
    float64 side;
    // your code here
}

none report(&Shape s) {
    io::outln(s::area()->text());
}

none main() {
    Square sq = 3.0;
    report(&sq);
}
```

```text title="intended output"
9.0
```

<!-- solution-of: solution-15-2 -->
```drachm solution
in io;

interface Shape {
    float64 area();
}

struct Square with Shape {
    public:
    float64 side;
    float64 area() { return self.side * self.side; }
}

none report(&Shape s) {
    io::outln(s::area()->text());
}

none main() {
    Square sq = 3.0;
    report(&sq);
}
```

## Your own iterables

A type works with `for (T x of thing)` when it says `with Iterable<T>` and has a
method `T? at(uint64 i)`. The loop calls `at(0)`, `at(1)`, ... until one gives
`none`. The value is only read, so it can be walked again.

<!-- example: iterable -->

A type can be `Iterable` over several item types. The variable's type picks
which `at` the loop uses.

<!-- example: iterable-two -->

The rules are on [Generics](reference-generics.md#iterable), and the design is on
[Generics and interfaces](design-generics.md). Next, the types that hold many
values: [Lesson 16: Collections](lesson-16-collections.md).
