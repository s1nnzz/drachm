---
title: Constructors and defaults
description: Constructors, new ... of, building from the declared type, fill blocks and def defaults.
status: not-implemented
---

# Constructors and defaults

This page shows how Drachm is designed to build struct values beyond listing
every field. The compiler does not accept any of it yet (it is planned for step
5). Today you build a struct by listing its fields, as in
[Structs and methods](reference-structs.md#building-a-value).

## Writing a constructor

A constructor is a method with the struct's own name and no return type. Inside
it, `self` is the value being built. A struct may have several constructors, and
the one that runs is chosen by the argument types, the same way a function
overload is.

```drachm
struct Circle {
    float64 r;

    Circle(float64 radius) { self.r = radius; }
    Circle(uint32 radius)  { self.r = radius; }
}
```

A struct with no constructor gets an automatic one that takes its fields in
order, which is what the current compiler's "list the fields" form does.

## Building from the declared type

When a declaration says the type, the values after `=` are the constructor
arguments, with no call to write. The compiler picks the constructor that
matches, exact types first, then lossless conversions, then lossy ones with a
warning.

```drachm
Circle a = 5;                 // Circle(uint32)
Circle b = 2.5;               // Circle(float64)
Person p = "Illy", 18;        // two arguments
Vector3 v = 1, 2, 3;
```

This shorthand works only in declarations, because only there is the type
known before the values. Elsewhere you say what you are building.

## new ... of

`new Type of (...)` builds a value inside any expression. The parentheses after
`of` are required.

<!-- focus: 3 -->

A static method that returns a new value of its own type is the usual home of
this form.

```drachm
struct Circle {
    float64 r;
    static Circle unit() { return new Circle of (1.0); }
}

Circle c = new Circle of (2.5);
```

## Naming the fields

Arguments may be named instead of positional, in any order, but never mixed
with positional ones.

```drachm
Vector3 p = z: 3, x: 1, y: 2;
Vector3 q = new Vector3 of (x: 1, y: 2, z: 3);
```

(Naming fields in a declaration already works today. What is planned is naming
them in `new ... of`, and naming the arguments of constructors.)

## A constructor that can fail

A constructor may declare `fails`, and then the declaration handles the failure
like any other failing call. See [Errors](design-errors.md).

```drachm
struct Port {
    uint32 n;
    Port(uint32 n) fails BadPort {
        if (n > 65535) { fail BadPort; }
        self.n = n;
    }
}

Port p = 8080 maybe Port::default();
```

## fill: one field at a time

A struct that is declared without a value has no fields yet. A `fill` block
gives it all of them, and the compiler checks at the closing brace that none
was missed. Until the struct is complete, `fill` is the only way to touch it.

<!-- focus: 2-3 -->

`fill partial` sets some fields and leaves the struct incomplete, to be filled
further on.

```drachm
Player p;
fill p {
    p.name = "Illy";
    p.hp = 5;
}

Player q;
fill partial q { q.name = "Sam"; }
```

## def: named defaults

A `def` is a template of field values that lives apart from the struct. A
struct may have several, and names the one it uses by default in its head.

<!-- focus: 1-4 -->

`fill p with defaults { }` starts from the struct's default `def` and the block
sets the rest. There is a shorthand when the `def` has the struct's own name:
`def Player { ... }` pairs with `struct default Player { ... }`.

```drachm
def PlayerStart of Player { hp = 100; title = "none"; }
struct Player defaults PlayerStart { string name; uint32 hp; string title; }

Player p;
fill p with defaults { p.name = "Illy"; }
```

<!-- status: tentative -->

Choosing a `def` other than the default, as in `fill p with Veteran { }`, is
the design owner's reading of the plan and is not confirmed.

```drachm
fill p with Veteran { p.name = "Sam"; }
```

## Keys for maps

A struct is a map key automatically, compared field by field. A `key()` method
says what a key is compared and hashed by instead.

<!-- status: tentative -->

What happens when a type has both a `key()` and its own `==` is still open.

```drachm
struct Word {
    string text;
    string key() { return self.text::lower(); }
}
```
