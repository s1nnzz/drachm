---
title: Moves and copies
description: When a value moves, how to keep it with copy, taking a field out, and the errors the compiler reports.
---

# Moves and copies

A `string`, a struct that is too big or holds a string, and a `&mut` borrow
do not copy by themselves. Assigning, passing or returning one **moves** it:
the new place owns the value and the old name has none until it is given a
new one.

```text
in io;
none greet(string s) { io::outln(s); }
none main() {
    string name = "Illy";
    greet(name);
    io::outln(name);
}
```

`drachm check` reports the second use:

```text
error[E0301]: `name` is used after it moved
  --> main.drh:6:15
   |
 5 |     greet(name);
   |     ----------- `name` moved here
 6 |     io::outln(name);
   |               ^^^^ used here, after the move
   |
 help: if `greet` should not take ownership, give it a `&string` parameter; or pass a copy: `greet(copy name)`
```

## Keeping the value: `copy`

`copy name` makes a separate value and leaves `name` as it is:

```drachm
none greet(string s) { }

none main() {
    string name = "Illy";
    greet(copy name);
    greet(name);
}
```

Numbers, bools and small plain structs copy by themselves, so they never
need `copy`.

## Taking one field out: `take`

Moving a field out of a struct you own must say so. `take p.name` moves the
field out and leaves `p.name` with no value until it is assigned again;
the other fields stay usable. A plain `string m = p.name;` is error E0324,
with the fix-its `take p.name` and `copy p.name`.

`take p.title leaving "none"` puts a new value back in the same statement, so
the field stays usable.

A struct with a `drop()` method must stay whole, so a plain `take` out of it
is E0325; `take f.path leaving ""` is allowed.

## Other move errors

- **E0301** is also reported when a value moved in a loop and the next time
  round uses it again, when it moved on only some paths, and when `p` is used
  as a whole after `p.name` was taken.
- **E0310**: a function that only borrows a value cannot move out of it,
  `self` included. Return a copy: `return copy self.name;`.
- **E0313** and **E0332**: a field cannot be given a value while the struct
  holding it has moved (E0313) or has no value yet (E0332). Give the whole
  struct a value first.
- **E0315**: a struct holding a `&mut` cannot be copied.
- **E0207**: a binding declared without a value was read before it was given
  one on every path.
- **E0105**: a function that promises a value can reach its end without a
  `return`.

See the [error index](errors.md) for every code.
