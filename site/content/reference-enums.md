---
title: Enums
description: Declaring enums, making values, matching with bindings, guards and nested patterns, exhaustiveness, equality, and how enums move and copy.
---

# Enums

An enum is a value that is exactly one of several named variants, and a variant
can carry values. The [enums lesson](lesson-13-enums.md) introduces them with
exercises. This page states the rules. Optionals (`T?`) are enums too, and they
are [at the end of this page](#optionals).

## Declaring an enum

An enum lists its variants, each a name, with the values it carries in
parentheses if it has any. Every value has a type and a name, and a trailing
comma is fine. An enum needs at least one variant, two variants with one name
are E0520, and an enum has no methods (write a function that takes the enum).

A variant cannot hold a borrow or a pointer yet (E0901), and an enum cannot
contain itself: E0210 says `Tree` contains itself, because a variant cannot hold
its own enum by value. A struct and an enum cannot share a name (E0202).

<!-- example: enum-basic -->

## Making a value

`Shape::Rect(2.0, 3.0)` and `Shape::Empty` always work. The bare names `Rect(2.0, 3.0)`
and `Empty` work wherever the type is known from the place: a declaration with a
written type, an assignment, an argument, a `return`, a field of a struct built
with the declaration shorthand, and the right side of `==` against an enum. The
expected type decides first, so `Shape s = Circle(1.0);` is the variant even if a
`struct Circle` exists. Where nothing says which enum, the struct or function of
that name wins, and a name that only some enum has is E0523.

Each variant takes one value for every value it declares (E0522 otherwise), and
a value converts like an argument does: a lossless conversion is silent and a lossy
one warns.

## Matching on variants

`match (value)` with `case Variant` or `case Variant(Type name, ...)`. A case can
use `Enum::Variant` too. The names in the parentheses are typed, and the type must
be exactly the type of the value (E0526); `_` ignores a value, and `case Variant`
without parentheses ignores all of them. `case A, B` runs one body for two variants.
`case _` matches everything. A name that is not a variant of the matched enum is
E0525.

`match` looks at the value where it is. It does not move it, so a value
that is used afterwards is still whole. A `match` on something that is not a place,
such as a call, keeps the result in a hidden variable for the length of the
block.

### Values that own memory

A plain binding of a payload that owns memory (a `string`, or a struct holding one) is E0528:

```text title="drachm check"
error[E0528]: binding `body` would take memory out of the matched value
  --> bind.drh:6:26
   |
 6 |         case Text(string body) { }
   |                          ^^^^ this payload is a `string`, which owns memory
   |
 help: write `take string body` to move it out (the matched value is used up), or `copy string body` to copy it
```

`take string body` moves the payload out. Everything else the value holds that owns
memory goes with it and is dropped at the end of the case, and the value counts as
moved afterwards (using it again is E0301, as after any move). `copy string body`
copies the payload and the value stays whole. Numbers, bools and Copy structs bind plainly.

<!-- example: enum-owning -->

### Guards, alternatives, constants and nesting

A guard is `if` and a condition after the patterns. The condition can use the
names the patterns bind. A guarded case is only chosen when the condition holds,
and otherwise the next case is tried. A guard sees what the pattern copies, not
what it takes out: those names exist in the body only.

Several patterns can share a body, `case Circle(float64 x), Rect(float64 x, _)`,
as long as each binds the same names with the same types (E0529).

A constant in a pattern (`case Rect(2.0, float64 h)`) must be equal to the
value, and works for numbers and bools. A payload that is itself an enum is matched
with a pattern inside the pattern: `case Some(Circle(float64 r))`.

<!-- example: enum-match -->

## Missing and unreachable cases

If the cases leave out a variant, the compiler warns (W0110) and names the
variants that are missing. A guarded case never counts as covering its variant, and a
nested pattern only covers what it spells out, so `case Some(Circle(_))` leaves
out the other shapes inside `Some`.

```text title="drachm check"
warning[W0110]: this `match` does not handle `Rect` and `Empty`
  --> missing.drh:9:5
   |
 9 |     match (s) {
   |     ^^^^^ these variants are missing
   |
 help: add `case Rect(float64 w, float64 h) { }`, `case Empty { }`, or `case _ { }`
```

A case that can never run, because the cases before it handle everything it
matches or because it comes after `case _`, is warning W0111.

## Equality

`==` and `!=` work on enums whose variants carry no values, like `d == North`.
An enum with values in some variant, or a struct holding one, is E0527: take it
apart with `match` instead. An enum cannot be turned into text by itself either.

## Copying and moving

An enum copies when every value in every variant is Copy and the enum is at most
32 bytes, with the tag counted. Otherwise it moves, like a `string`, and `copy x`
makes a copy that owns its memory. An enum is one value for the move rules: moving
it moves all of it, and there is no `take` of a single payload outside of a `match`.

An enum is a tag (1 byte for up to 256 variants) followed by the values of the
variant it holds, laid out like a struct's fields. The values start at the largest
alignment among them, so the size of `Shape` above is 24 bytes.

## Optionals

`T?` is an optional `T`: a value of type `T`, or `none`. It is an enum with two
variants, `none` (no value) and `Some` (one `T`), so everything above applies to
it: layout, copying and dropping, `match`, and the warnings. Each `T` that you
write with a `?` gets its own optional type, and the compiler prints it as `uint32?`.

A `T` goes to a `T?` by itself wherever a `T?` is wanted: a declaration, an
assignment, an argument, a `return`, a field value. `none` is the empty value and
is only written where an optional type is expected (E0530 otherwise). A variable
of optional type with no value starts as `none`: `mut uint32? age;` is `none`.

<!-- example: option-basic -->

Two words test an optional without a `match`. `x has value` is true when there is a
value, and `x is empty` when it is `none`. `x == none` is an error (E0533) that
says to write `x is empty`, because the whole value is not the thing to compare.

`x maybe fallback` gives the value of `x`, or `fallback` when `x` is `none`. The
fallback is a single value or a parenthesised expression: `(x maybe 0) + 1` adds
one to the value, and `x maybe 0 + 1` is an error (E0534). A `maybe` on something
that is not optional is E0532.

<!-- example: option-maybe -->

A `match` on an optional has two cases: `case uint32 a` for the value (typed like
any binding in a `case`) and `case none`. Leaving one out is W0110, which says
which one is missing, and `case _` covers the rest.

```text title="drachm check"
warning[W0110]: this `match` does not handle a value
  --> age.drh:5:5
   |
 5 |     match (age) {
   |     ^^^^^ this case is missing
   |
 help: add `case uint32 value { }` or `case _ { }`
```

Rules to know:

- An optional that owns memory, such as `string?`, is matched like any owning
  enum: `case take string name` moves the text out, and `case copy string name`
  copies it. A `string?` in a variable cannot be read with `maybe` yet (E0901);
  one returned from a call can.
- Using an optional where its value is wanted is E0531: `uint32 n = age;` and
  `age + 1` both need `maybe` or a test first.
- `T??` is E0901, and `Optional<T>` spelled out is E0901: write `T?`.
- `none` as a constructor argument needs an optional type in the constructor's
  parameter list, which the compiler does not look at yet: give the field its
  value with an assignment (`s.item = none;`).
