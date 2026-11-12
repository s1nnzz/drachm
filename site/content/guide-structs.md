---
title: Structs, constructors and defaults
description: Designing a struct from its fields and methods to how its values are built, with constructors, fill blocks and defs.
---

# Structs, constructors and defaults

Fields, sections, methods, `mut` fields, constructors, `fill` and `def` are all
built, and every example on this page runs. The rules
are in [Structs and methods](reference-structs.md).

## Fields, sections and methods

A struct is its fields plus the methods that keep them consistent. Decide first
what outsiders may touch. Before any label, fields are private and methods are
public; `public:` and `private:` start a new section for what follows.

A method is called with `::`, and inside it `self` is the value. You never write
`self` as a parameter, and you never say whether a method only reads or also
changes `self`: the compiler works that out from the body. A method that never
uses `self` is `static` and is called on the type.

<!-- example: guide-structs-methods -->

`name` is private because it sits before any label. `hits` is a `mut` field, so
it can change even though `p` is not a `mut` binding, and `hit()` changes only
`mut` fields, so it can be called on `p` as it is. `score` and `hits` are
public, `label` reads `self`, and `startingScore` is `static` because it needs
no value.


## Choosing mut fields

Make a field `mut` when it is bookkeeping that changes during the value's life
and nothing else depends on it staying put: a hit counter, a cache, a "last
seen" time. Leave every other field plain, so a change to it needs a `mut`
binding and the code that does it is easy to spot. Through a `&` borrow nothing
changes, `mut` fields included; to change something you have to hold `&mut`.

## Building a value today

In a declaration you can list the values in field order, as the example does
with `Player p = "Illy", 0, ...;`, or name them in any order with `name: value`. This works
because the compiler gives every struct an automatic constructor that takes its
fields in order. Mixing positional and named values is an error.

## Writing constructors

The automatic constructor is fine until a value needs checking or computing.
Then write your own: a constructor is a member with the struct's own name and no
return type, and `self` is the value being built. Several are allowed, and the
one that runs is chosen by the arguments (an exact match first, then a lossless
conversion, then a lossy one with a warning).

<!-- example: guide-structs-ctor -->

Every field must have a value by the time the constructor ends, on every path;
the compiler checks it. A constructor that can fail says `fails`, and then every
construction has to handle it ([Error handling](guide-errors-handling.md)).

### Keeping the field-order form

Writing a constructor removes the automatic one. If you want both, ask for the
automatic one by name inside the struct with `Person(auto);`.

<!-- example: guide-structs-auto -->

## Building anywhere with new ... of

The `Type x = a, b;` shorthand works only in declarations, since only there is
the type known before the values. Elsewhere, say what you are building with
`new Type of (...)`. The parentheses are required. A `static` method that
returns a fresh value is the usual home for it.

<!-- example: guide-structs-new -->

## Filling a struct field by field

Sometimes a value is better built in steps. Declare it without a value and give
the fields in a `fill` block; the compiler checks at the closing brace that
none is missing. `fill partial` leaves some for later. Until a struct is
complete, `fill` is the only way to touch it: no reads, no borrows, no plain
field assignments.

<!-- example: guide-structs-fill -->

Reading a field you have just set needs the word `now`, which says "the value as
it is at this point". A plain read of an incomplete value is an error that
suggests it, and methods cannot be called on a value that is not complete. A
constructor uses the same word: `now self.r`.

## Defaults with def

A `def` is a named template of field values, kept apart from the struct. A
struct names the one it uses by default in its head, and `fill p with defaults`
starts from it, so the block only sets the rest. A def's values may be any
expression, as long as it does not read fields.

<!-- example: guide-structs-def -->

When the def has the struct's own name, `of` can be left out and the struct says
`default`: `def Player { ... }` with `struct default Player { ... }`.

### More than one def

A struct may have several defs. Pick one by name with `fill p with Veteran`,
and let a def build on another with `from`:

<!-- example: guide-structs-defs -->

## Which tool when

| You need | Use |
|---|---|
| A plain bundle of values | the automatic constructor: `Point p = 1, 2;` |
| To check or compute while building | a constructor |
| Both the plain form and your own | a constructor plus `Person(auto);` |
| To build inside an expression | `new Type of (...)` |
| A value built in steps, or from a template | `fill`, with a `def` |
| The same few starting setups, by name | several `def`s |
