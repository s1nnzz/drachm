---
title: "Lesson 8: Ownership"
description: Why a string moves when you pass it on, how copy keeps it, and the move errors the compiler reports.
---

# Lesson 8: Ownership

**Goal:** understand moves, so you know why the compiler rejects some programs
and how to fix them.

## A value has one owner

A `string` owns its text, and the text is freed when its owner goes out of
scope. To make that safe without a garbage collector, Drachm lets only one name own
a string at a time. Assigning a string, passing it to a function or returning
it **moves** it: the new place owns the text, and the old name has nothing
until you give it a new value.

The program below does exactly that, and the compiler refuses it. After
`greet(name)` the function owns the text, so the last line has nothing to print.

```drachm
none greet(string s) {
    io::outln(s);
}

none main() {
    string name = "Illy";
    greet(name);
    io::outln(name);
}
```

```text title="drachm check"
error[E0301]: `name` is used after it moved
   --> main.drh:10:15
    |
  9 |     greet(name);
    |     ----------- `name` moved here
 10 |     io::outln(name);
    |               ^^^^ used here, after the move
    |
 help: if `greet` should not take ownership, give it a `&string` parameter; or pass a copy: `greet(copy name)`

main.drh: 1 error
```

## Keeping the value with copy

The error offers two fixes. The first is to pass a copy. `copy name` makes a
separate string and leaves `name` as it was, at the cost of copying the text.
The other fix, a `&string` parameter that only borrows, is the subject of
[lesson 9](lesson-9-borrowing.md).

<!-- focus: 13-14 -->
Both calls to `greet` get their own copy, so `name` is still valid.

<!-- focus: 15 -->
`wrap(name)` has no `copy`, so it takes the text. That is fine because `name` is
not used afterwards.

<!-- focus: 18-19 -->
A new binding takes over a moved value as well. `boxed` is spent after this
line, and `other` owns the text.

<!-- focus: 22-25 -->
Numbers, bools and small plain structs copy by themselves, so they never need
`copy`.

<!-- example: ownership -->

## Moves in loops and branches

The compiler follows every path through your code. A value moved in the body of a
loop is gone for the next round, so the program below fails on the second trip,
and the message says so. A value moved on only some paths counts as moved
afterwards.

```drachm
string name = "Illy";
mut uint32 i = 0;
while (i < 2) {
    string t = name;
    i++;
}
```

```text title="drachm check"
error[E0301]: `name` is used after it moved
  --> main.drh:7:20
   |
 7 |         string t = name;
   |                    ^^^^ used here, after the move
   |                    ---- `name` moved here, in the previous time round the loop
   |
 help: move it only once (for example after the loop), or pass `copy name`
```

## Fields need take or copy

A struct that holds a string owns it too. Reading `p.name` into a new binding
would leave `p` without a name, so Drachm makes you say which you mean.

```drachm
struct Player {
public:
    string name;
    uint32 hp;
}
// ... Player p = "Illy", 10;  string n = p.name;
```

```text title="drachm check"
error[E0324]: cannot move `p.name` out of `p` without `take`
   --> main.drh:11:16
    |
 11 |     string n = p.name;
    |                ^^^^^^ this would leave `p` without a name
    |
 help: write `take p.name` to move it out, or `copy p.name` to keep both
```

## take

`take p.name` moves the field out and leaves it empty until you give it a new
value. The rest of the struct stays usable, and reading the empty field is an
error. Add `leaving "nobody"` and the new value is put back in the same
statement, so the field is never empty.

Here are both forms in a program that runs. The first `take` puts a new value
back, and the second one leaves the field empty until `p.name = "Sam"` gives it
a new one.

<!-- example: take -->

```text title="drachm check, when the field is read after a plain take"
error[E0301]: `p.name` is used after it moved
   --> main.drh:12:15
    |
 11 |     string n = take p.name;
    |                ----------- `p.name` moved here
 12 |     io::outln(p.name);
    |               ^^^^^^ used here, after the move
    |
 help: give `p.name` a new value first, or move a copy: `copy p.name`
```

### Your turn: fix the move

This program does not compile because `name` is used after `greet(name)` took
it. Change it so that the output is two greetings, keeping the last use of
`name` as it is.

```drachm title="starter"
in io;

none greet(string s) {
    io::outln("Hello, " + s);
}

none main() {
    string name = "Illy";
    greet(name);
    greet(name);
}
```

```text title="expected output"
Hello, Illy
Hello, Illy
```

<!-- solution-of: solution-8-1 -->
```drachm solution
in io;

none greet(string s) {
    io::outln("Hello, " + s);
}

none main() {
    string name = "Illy";
    greet(copy name);
    greet(name);
}
```

### Your turn: a function that takes a string

Write `string twice(string s)` that returns the text twice. Call it with
`word` two times, copying the first time so that the second call still has a value.

```drachm title="starter"
in io;

// your code here

none main() {
    string word = "ha";
    io::outln(twice(copy word));
    io::outln(twice(word));
}
```

```text title="expected output"
haha
haha
```

<!-- solution-of: solution-8-2 -->
```drachm solution
in io;

string twice(string s) {
    return s + s;
}

none main() {
    string word = "ha";
    io::outln(twice(copy word));
    io::outln(twice(word));
}
```

For a slower, picture-based walk through the same ideas, read the
[ownership and borrowing guide](guide-ownership.md).

A value that still has an owner when its scope ends is destroyed there, and a
struct can run its own code at that moment with a `drop()` method. The
[Moves and copies](reference-ownership.md) reference has the rules for when a
value dies.

The next lesson shows how to let a function use a value without taking it:
[Lesson 9: Borrowing](lesson-9-borrowing.md). The Reference pages cover every rule in full,
starting with [Types and numbers](reference-types.md).
