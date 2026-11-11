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
another expression, call a function that returns it.

<!-- example: struct-build -->

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
