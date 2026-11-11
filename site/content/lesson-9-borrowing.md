---
title: "Lesson 9: Borrowing"
description: Letting a function read or change a value without taking it, and the three errors you meet first.
---

# Lesson 9: Borrowing

**Goal:** pass values to functions without giving them away, and read the
errors the compiler reports when a borrow is used wrongly.

## A function that only looks

[Lesson 8](lesson-8-ownership.md) ended with a choice: copy the string or give
it away. Often a function only needs to look at a value. A `&string` parameter
borrows the string: the function may read it, and the caller keeps it. The call
site writes `&` too, so you can see where a borrow starts.

```drachm
in io;

uint64 size(&string s) {
    return s::len();
}

none main() {
    string name = "Illy";
    io::outln(size(&name)::text());
    io::outln(name);
}
```

`name` is still ours after the call, and nothing was copied.

## A function that changes

`&mut string` borrows the string so the function can change it, and the caller
writes `&mut` at the call. The binding being lent must be `mut` or `open`.
Inside the function, `s = ...` writes through to the caller's string.

<!-- example: borrow -->

A function can also return a borrow, like `name_of` above, when the borrow
points at something it was given. To pass a `&mut` you hold on to another
function and keep using it, write `lend`:

<!-- example: borrow-lend -->

## The rule, and the errors it gives

A value has either many `&` borrows or one `&mut` borrow at a time, and a
borrow lasts until its last use. The compiler's three most common complaints
are about breaking this.

### E0302: changing a value that is borrowed

```drachm
in io;
none main() {
    mut string name = "Illy";
    &string first = &name;
    name = "Sam";
    io::outln(first);
}
```

```text title="drachm check"
error[E0302]: cannot change `name` while it is borrowed
  --> main.drh:5:5
   |
 4 |     &string first = &name;
   |                     ----- `name` is borrowed here
 5 |     name = "Sam";
   |     ^^^^ change attempted here
 6 |     io::outln(first);
   |               ----- the borrow is still used here
   |
 help: finish using `first` before changing `name`
```

The fix is the help line: print `first` before changing `name`. Once `first` is
not used any more, the borrow is over and the change is allowed.

### E0303: two `&mut` at once

```drachm
struct Player { public: mut uint32 hp; }
none heal(&mut Player p) { p.hp = p.hp + 1; }
none main() {
    mut Player p = 1;
    &mut Player a = &mut p;
    &mut Player b = &mut p;
    heal(a);
    heal(b);
}
```

```text title="drachm check"
error[E0303]: `p` is already borrowed with `&mut`
  --> main.drh:6:21
   |
 5 |     &mut Player a = &mut p;
   |                     ------ first `&mut` borrow here
 6 |     &mut Player b = &mut p;
   |                     ^^^^^^ second `&mut` borrow here
 7 |     heal(a);
   |          - the first borrow is still used here
   |
 help: only one `&mut` borrow of `p` can be in use at a time; finish using `a` before borrowing `p` again
```

Call `heal(a)` before taking `b`, or use one borrow for both calls.

### E0307: returning a borrow of a local

```drachm
&string shout() {
    string s = "hi"::upper();
    return &s;
}
none main() { }
```

```text title="drachm check"
error[E0307]: cannot return a borrow of `s`, which belongs to this function
  --> main.drh:3:12
   |
 2 |     string s = "hi"::upper();
   |            - `s` is created here and dropped when `shout` returns
 3 |     return &s;
   |            ^^ the borrow is returned here
   |
 note: `shout` takes no borrows, so a borrow it returns can only point at something it owns
 help: return the string itself: change the return type to `string` and write `return s;`
```

`s` is dropped when `shout` returns, so a borrow of it would point at nothing.
Return the string itself. The other borrow errors, E0304 to E0306, are in the
[Borrowing](reference-ownership.md#borrowing) reference.

### Your turn: borrow instead of copy

`describe` takes ownership of the string, so the program copies it. Change
`describe` to borrow, and remove the `copy`, keeping the output.

```drachm title="starter"
in io;

none describe(string s) {
    io::outln(s + " has " + s::len()->text() + " letters");
}

none main() {
    string word = "borrow";
    describe(copy word);
    describe(copy word);
}
```

```text title="expected output"
borrow has 6 letters
borrow has 6 letters
```

<!-- solution-of: solution-9-1 -->
```drachm solution
in io;

none describe(&string s) {
    io::outln(s + " has " + s::len()->text() + " letters");
}

none main() {
    string word = "borrow";
    describe(&word);
    describe(&word);
}
```

### Your turn: a function that changes

Write `none shout(&mut string s)` that makes `s` upper case, and call it on
`word`.

```drachm title="starter"
in io;

// your code here

none main() {
    mut string word = "quiet";
    shout(&mut word);
    io::outln(word);
}
```

```text title="expected output"
QUIET
```

<!-- solution-of: solution-9-2 -->
```drachm solution
in io;

none shout(&mut string s) {
    s = s::upper();
}

none main() {
    mut string word = "quiet";
    shout(&mut word);
    io::outln(word);
}
```

### Your turn: fix the borrow error

This program has error E0302. Change it so it prints `Illy` and then `Sam`,
without removing either print.

```drachm title="starter"
in io;

none main() {
    mut string name = "Illy";
    &string first = &name;
    name = "Sam";
    io::outln(first);
    io::outln(name);
}
```

```text title="expected output"
Illy
Sam
```

<!-- solution-of: solution-9-3 -->
```drachm solution
in io;

none main() {
    mut string name = "Illy";
    &string first = &name;
    io::outln(first);
    name = "Sam";
    io::outln(name);
}
```

That is the end of the lessons. The Reference pages cover every rule in full,
starting with [Types and numbers](reference-types.md).
