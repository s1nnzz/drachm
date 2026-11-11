---
title: Moves and copies
description: When a value moves, how to keep it with copy, taking a field out, and the errors the compiler reports.
---

# Moves and copies

The [ownership lesson](lesson-8-ownership.md) introduces these rules with
examples. This page states them in full.

## What moves

A `string`, a struct that is too big or that holds a string, and a `&mut` borrow
do not copy by themselves. Assigning, passing or returning one moves it: the new
place owns the value, and the old name has none until it is given a new one.
Using a name after its value moved is error E0301.

Numbers, bools and small plain structs copy by themselves, so they never need
`copy`. Borrow parameters such as `&string` are designed but this compiler cannot
build them yet (see [Coming later](coming-later.md)).

```drachm
none greet(string s) { }

none main() {
    string name = "Illy";
    greet(name);
}
```

## Keeping the value with copy

`copy name` makes a separate value and leaves `name` as it is. It works on a
variable, a field (`copy p.name`) or any other expression that gives a value.

<!-- example: ownership -->

## Taking one field out

Moving a field out of a struct you own must say so, and a plain
`string m = p.name;` is error E0324 with `take p.name` and `copy p.name` as
fix-its. `take p.name` moves the field out and leaves it with no value until
it is assigned again, and the other fields stay usable. `take p.title leaving "none"`
puts a new value back in the same statement, so the field stays usable.

A struct with a `drop()` method must stay whole, so a plain `take` out of it is
E0325, while `take f.path leaving ""` is allowed. `drachm check` accepts `take`
today; programs that move a field out cannot be built yet.

```drachm
mut Player p = "Illy", 10;
string n = take p.name leaving "nobody";
```

## Other move errors

E0301 is also reported when a value moved in a loop is used again in the next
round, when it moved on only some paths, and when `p` is used as a whole after
`p.name` was taken. E0310 means a function that only borrows a value cannot move
out of it, `self` included, so return a copy: `return copy self.name;`.

E0313 and E0332 say a field cannot be given a value while the struct holding it
has moved (E0313) or has no value yet (E0332), so give the whole struct a value
first. E0315 means a struct holding a `&mut` cannot be copied.

Two neighbours are checked in the same pass. E0207 means a binding declared
without a value was read before every path gave it one, and E0105 means a
function that promises a value can reach its end without a `return`. See the
[error index](errors.md) for every code.
