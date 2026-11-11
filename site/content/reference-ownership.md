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
E0325, while `take f.path leaving ""` is allowed.

<!-- example: take -->

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

## When a value dies

A value that still has an owner when its scope ends is destroyed there. For a
`string` that frees its text. Several values dying at the same `}` go in the
reverse of the order they were declared in, so the last one made goes first.
A value that was moved away is not destroyed by its old name, because the new
owner does that: only what still holds a value when the scope ends is dropped,
and a struct with some fields taken out drops the ones that remain.

A temporary, such as the `"a" + "b"` in `("a" + "b")::upper()`, dies at the
end of the statement that made it. In the condition of a `while` that means
every time round; in the condition of an `if` it means before either branch runs.

### drop()

A struct can name what happens when it dies by giving it a method called `drop`
that returns `none`. It runs when a value of that struct is destroyed, before the
struct's fields are dropped, and the fields then go in the order they were
declared. You cannot call `drop` yourself: `x::drop()` is error E0331.

### destroy

`destroy x;` runs `drop()` now instead of at the end of the scope, and ends `x`
like a move does: reading it afterwards is error E0301, and nothing runs for it
at the end of the scope. It works on a variable whose struct has a `drop()`;
anything else is E0901 for now.

<!-- example: drop -->

If `destroy` is on only some paths, the compiler remembers at run time whether
the value is still there, so the scope end drops it on the paths that did not
destroy it and only on those.
