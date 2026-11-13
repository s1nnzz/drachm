---
title: Structs and methods
description: Struct sections, fields, building values, methods, static methods and operator methods.
---

# Structs and methods

A `struct` groups named fields, and its methods are written inside it. The
[lesson on structs](lesson-6-structs.md) walks through these rules step by step.

## Sections and privacy

Before any `public:` or `private:` label, fields are private and methods are
public. A label starts a new section for everything after it. Code outside the
struct may use only public members, and using a private one is error E0212.
Methods inside the struct may use private members.

<!-- example: struct-sections -->

## Fields

A field may be a `string` or another struct. `mut` before a field type lets
that field change even when the binding holding the struct is not `mut`. Any
other field needs a `mut` binding to change, for example `p.id = 3;` needs
`mut Counter p`, and without it the compiler reports E0205. Through a `&`
borrow no field changes, not even a `mut` one (E0311); see
[Changing things](reference-ownership.md#changing-things).

Each struct value is separate. Plain assignment of a struct that holds a
string moves it; write `copy a` to get a second value with its own text.

<!-- example: struct-copy -->

## Fields that borrow

A field may be a borrow, `&string src;`. A struct with such a field must say
which fields are borrows, with `lives` after its name, or the compiler reports
E0314:

<!-- example: struct-lives -->

A value of that struct cannot outlive what it borrows, and the compiler checks
that: `text` above has to stay alive for as long as `p` is used. A `mut &string
cur;` field can be pointed elsewhere later; a plain borrow field stays pointed
where it started. A `drop()` method on such a struct may still read the borrow,
so the borrowed value stays alive until the struct is dropped.

## Building a value

A struct is built in a declaration, by listing a value for each field in order,
or by naming them in any order with `name: value`. Every field must be given
exactly once, and mixing the two forms is error E0114. To build a struct inside
another expression, write `new Type of (...)`; see [Constructors](#constructors).

<!-- example: struct-build -->

## Constructors

A constructor is a method named after its struct, with no return type. Inside it,
`self` is the value being built. Every field must have a value by the end of the
constructor, and the constructor returns the value by itself.

<!-- example: ctor-overloads -->

A struct may have several constructors, and the argument types choose which one
runs: an exact match first, then a conversion that loses nothing, then one that
may lose information, which gives warning W0102. A whole number such as `5`
prefers `uint32` (its default type). If no constructor takes the values, the
compiler reports E0501 and lists each constructor's parameters. If two are equally
good, it reports E0502 and asks for a conversion. Two constructors with the same
parameter types are E0503.

```text title="drachm check"
error[E0501]: no constructor of `Circle` accepts these values
  --> main.drh:9:16
   |
 4 |     Circle(float64 radius) { self.r = radius; }
   |     ------ this constructor takes (float64)
 5 |     Circle(uint32 whole) { self.r = 0.0; }
   |     ------ this constructor takes (uint32)
  ...
 9 |     Circle c = "five";
   |                ^^^^^^
   |                ------ this is `string`
   |
 help: `Circle` takes (float64) or (uint32)
```

A constructor that leaves a field without a value is E0504, at the end of the
constructor (or at its `return;`):

```text title="drachm check"
error[E0504]: the constructor ends without giving `hp` a value
  --> main.drh:5:39
   |
 5 |     Player(string n) { self.name = n; }
   |                                       ^ `hp` has no value when the constructor ends
   |
 help: add `self.hp = ...;` on every path that reaches the end
```

In a declaration, the values after `=` are the constructor's arguments, so
`Circle a = 5.0;` needs no call. Anywhere else, say what you are building with
`new Type of (...)`. The parentheses after `of` are required. `Circle(5.0)` is
not a construction: it is E0201, and the help says to use `new`.

<!-- example: ctor-new -->

Arguments may be named, in any order, and never mixed with positional ones. With
no written constructor the names are the field names. With one, they are the
parameter names of the constructor that is chosen. The automatic constructor
(the one that takes the fields in order) is removed by writing a constructor,
unless the struct says `Person(auto);`, which keeps it next to the written ones.

<!-- example: ctor-fields -->

A constructor under `private:` can be used only by the struct's own methods, as
in `static Token make(...)` (otherwise E0212). A constructor cannot yet build its
value by calling another constructor (E0901).

### Constructors that fail

A constructor may declare `fails` after its parameters, like any function. On a
`fail`, the fields set so far are dropped and the constructor ends with the
failure; the completeness check (E0504) only applies to paths that finish. The
declaration shorthand and `new Type of (...)` are calls that can fail, so they
need what any call needs: `maybe`, `fails` or `fails with`, written after the
value (E0544 otherwise).

<!-- example: ctor-fails -->

With a list of several values, write `new Type of (a, b) maybe ...`; the
shorthand `Type x = a, b maybe ...` is not built.

## Filling and defaults

A struct declared without a value has no fields yet: `Player p;`. A `fill` block
gives it its fields one at a time. The closing brace checks that every field has
a value, and reports each one that does not (E0505):

<!-- example: fill-basic -->

```text title="drachm check"
error[E0505]: `fill` ends without giving `hp` a value
  --> main.drh:12:28
   |
12 |     fill p { p.name = "x"; }
   |                            ^ `p.hp` has no value here
   |
 help: give it a value in the block, or end with `fill partial p { ... }` and complete it later
```

Inside the block only `p.field = value;` may be written, with ordinary statements
that do not touch `p`. Reading `p`, borrowing it or calling a method on it is
E0507. A field that is already set can be read with `now`, as in
`p.max = now p.hp * 2;`, and in a constructor `now self.r`. A field that is not
`mut` can be given its value only once.

```text title="drachm check"
error[E0507]: `p` cannot be used until it is complete
  --> main.drh:14:18
   |
14 |         p.name = p.name;
   |                  ^ used here, before every field has a value
   |
 help: a field that is already set can be read with `now`: `now p.field`
```

`fill partial p { ... }` gives some fields and leaves the value incomplete. Any
later `fill p { ... }` completes it, so the fields can be given in several blocks.

<!-- example: fill-partial -->

A `def` is a named set of field values for one struct: `def NAME of STRUCT { field
= value; }`. Its values are expressions that do not read fields, and they are
used again each time the def is applied. A struct names its default def in its
head, with `struct Player defaults PlayerStart { ... }`, or with `struct default
Player { ... }` when the def has the struct's own name. `fill p with defaults { }`
starts from the default and the block sets the rest. A block may set a field the
def set only if that field is `mut`.

A def may extend another with `from`: `def Veteran of Player from PlayerStart { hp
= 200; }` takes every value of `PlayerStart` and overrides `hp`. A def is applied
by name with `fill p with Veteran { }`.

<!-- example: def-defaults -->

A field value in a `def` may be a bare variant when the field has an enum type,
as in an assignment: `rank = Cadet;` takes `Rank` from the field.

<!-- example: def-bare-variant -->

Missing defaults are reported too: `fill ... with defaults` needs the struct to
name a default (E0510), and a name that is not a def of that struct is E0508.

## Methods

A method is called with `::`, and every call after the first in a chain with
`->`. Inside a method, `self` is the value it was called on. A method that
changes a field of `self` needs a `mut` binding when it is called, unless the
field itself is `mut`. A method that changes only `mut` fields can be called on
any value you own, and `mut` on the binding is not needed for it.

A `static` method has no `self` and is called on the type, as in
`Temperature::boiling()`. A method that never uses `self` should be
`static`, and warning W0208 reminds you.

<!-- example: struct-static -->

## Operator methods

Operators are methods named by the operator, and they take one value, the right
side. You can define `+ - * / % **` and the comparisons `== != < <= > >=`.
Comparisons return `bool`, and `==` and `!=` take the struct's own type.
Compound assignment such as `z += y` uses the `+` method.

`==` and `!=` already work on any struct without a method. They compare field by
field, and strings compare by their text. Define `bool ==(T o)` to replace that
rule.

<!-- example: struct-ops -->
