---
title: Borrows and lifetimes
description: Borrow parameters, lend, lives on functions and structs, open and destroy.
status: not-implemented
---

# Borrows and lifetimes

The ownership checks for moves work today (see [Ownership](reference-ownership.md)).
Borrows are designed, and the compiler already parses and checks parts of
them, but it cannot build a program that uses one yet. This page is the
design.

## Borrowing a value

`&T` is a shared borrow, and any number may exist. `&mut T` is an exclusive
borrow that may change the value. Both are always written at the call.
Lending `&mut` needs a `mut` or `open` binding.

<!-- focus: 1,5,6 -->

```drachm
none show(&Player p) { io::outln(p.name); }
none heal(&mut Player p) { p.hp = 100; }

none main() {
    mut Player p = "Illy", 5;
    show(&p);
    heal(&mut p);
}
```

A borrow always means what it points at. Reading is automatic (`q + 1`) and
`r = 5;` writes through. `r = &mut other;` points `r` somewhere else, which
is allowed because the right side is itself a reference.

## Passing on a borrow

To hand on a borrow you already hold, write `lend`. `&T` copies; `&mut T`
moves, so after `&mut string r2 = r;` the name `r` is used up.

```drachm
none add(&mut string s) { s::append("!"); }
none twice(&mut string s) { add(lend s); add(lend s); }
```

## What a borrow protects

While a borrow is alive the borrowed value cannot change. The compiler
reports the earlier borrow and the later change together.

```text title="intended output"
error[E0302]: cannot change `xs` while it is borrowed
  --> main.drh:5:5
   |
 4 |     &uint32 first = &xs[0];
   |                     ------ `xs` is borrowed here
 5 |     xs::push(9);
   |     ^^ change attempted here
 6 |     io::outln(first::text());
   |               ----- the borrow is still used here
```

A call such as `s::append(s::len()::text())` is fine, because the inner borrow
ends before the change. `xs::push(xs::first())` is not.

## lives on functions

A function that returns a borrow must say where it comes from when that is not
clear. With one candidate parameter the compiler works it out, and with several
it asks (E0308). `lives` names whole parameters, never parts of them.

<!-- focus: 1 -->

```drachm
&string longest(&string a, &string b) lives a, b {
    if (a::len() > b::len()) { return a; }
    return b;
}
```

### Naming self as a source

<!-- status: tentative -->

To name `self` as a source, the form is `lives from`, with sources joined by
`or`. This spelling is an unconfirmed reading.

```drachm
&string pick(&string key) lives from self or key { /* ... */ }
```

## lives on structs

A struct that holds borrows lists them in its head. Each may borrow from a
different place.

```drachm
struct Pair lives a, b { &string a; &string b; }
struct Parser lives src { &string src; }
```

`lives` and `fails` on a function, and `lives` and `with` on a struct, may come
in either order; `drachm fmt` picks one.

## open and the other binding words

`open` lets a binding's contents change without letting it be reassigned. It
allows field writes, `open` parameters (`none fill(open string s)`) and
declaring without a value and assigning once later.

## Ending a value early

A method named `drop()` runs when its value is destroyed, in a visible order.
Calling `x::drop()` yourself is an error; the statement `destroy x;` runs it
now and ends `x`, like a move.

```drachm
none drop() { io::outln("closing"); }
destroy conn;
```
