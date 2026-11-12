---
title: Moves and copies
description: When a value moves, how to keep it with copy, taking a field out, and the errors the compiler reports.
---

# Moves and copies

The [ownership lesson](lesson-8-ownership.md) introduces these rules with
examples, and the [ownership and borrowing guide](guide-ownership.md) explains
the thinking behind them. This page states them in full.

## What moves

A `string`, a struct that is too big or that holds a string, and a `&mut` borrow
do not copy by themselves. Assigning, passing or returning one moves it: the new
place owns the value, and the old name has none until it is given a new one.
Using a name after its value moved is error E0301.

Numbers, bools and small plain structs copy by themselves, so they never need
`copy`. A `&` borrow copies too, because it only points at something; see
[Borrowing](#borrowing) below.

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

## Enums

An enum is one value for these rules. It is Copy when every value in every variant is
and it is at most 32 bytes; otherwise it moves like a `string`. A `match` does not
move the value it looks at, and `take` and `copy` in a pattern say what happens to a
payload that owns memory: see [Enums](reference-enums.md#values-that-own-memory).

## Changing things

Three words say what may change, and the compiler checks every change against
them.

- `mut` on a binding lets it be given a new value, and lets its fields and its
  contents change.
- `open` on a binding (`open string xs`) lets its contents change: its fields,
  `+=`, and a `&mut` borrow. It is never given a new value. `open string xs;`
  without a value may be given its one value later.
- `mut` before a field lets that field change in a struct you own, even when
  the binding holding the struct is not `mut`.

A binding with none of these keeps the value it was given. The example shows
each rule, and it runs:

<!-- example: mutability -->

A method changes `self` when its body writes to `self` or calls a method that
does, and the compiler works that out itself. A method that changes only `mut`
fields, like `hit` above, may be called on any value you own, `mut` or not. Any
other method that changes `self` needs a `mut` or `open` binding.

Through a `&` borrow nothing changes, not even a `mut` field: changing it,
lending it `&mut`, or calling a method that changes `self` is error E0311. A
`&mut` borrow needs a `mut` or `open` binding, or it is error E0312. The
examples show each error:

```drachm
struct Player {
public:
    string title;
    mut uint32 hp;
}

none hurt(&Player p) {
    p.hp = p.hp - 1;
}

none main() { }
```

```text title="drachm check"
error[E0311]: cannot change `p.hp` through a `&` borrow
  --> main.drh:8:5
   |
 7 | none hurt(&Player p) {
   |           --------- `p` is a `&` borrow, which only reads
 8 |     p.hp = p.hp - 1;
   |     ^^^^ change attempted here
   |
 help: take a `&mut` borrow: declare it `&mut Player p`
```

```drachm
none grow(&mut string s) { }

none main() {
    string ys = "c";
    grow(&mut ys);
}
```

```text title="drachm check"
error[E0312]: cannot lend `ys` as `&mut`: it is neither `mut` nor `open`
  --> main.drh:5:10
   |
 4 |     string ys = "c";
   |            -- declared here without `mut` or `open`
 5 |     grow(&mut ys);
   |          ^^^^^^^ `&mut` borrow here
   |
 help: declare it `open string ys` (its contents may change) or `mut string ys` (it may also be reassigned)
```

An `open` binding is never given a new value once it has one:

```drachm
none main() {
    open string xs = "a";
    xs = "b";
}
```

```text title="drachm check"
error[E0205]: cannot reassign `xs`: it is `open`, not `mut`
  --> main.drh:3:5
   |
 2 |     open string xs = "a";
   |                 -- declared `open`: its contents may change, but it is never reassigned
 3 |     xs = "b";
   |     ^^ reassigned here
   |
 help: declare it `mut string xs` to allow reassigning
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

## Borrowing

A borrow lets a function use a value without owning it. `&string` is a shared
borrow: it reads. `&mut string` is a unique borrow: it reads and changes. The
call site says which one it passes, by writing `&` or `&mut` before the
argument, and leaving it off is error E0321.

<!-- example: borrow -->

Through a borrow you use the value as if you held it: `s::len()` and `p.hp`
read it, and `s = ...` or `p.hp = ...` on a `&mut` writes through to the
original, dropping the old value first. A borrow can be returned too, as
`name_of` does, as long as it points at something the function was given.

### lend

A `&mut` borrow is used up when you pass it on, like any other value that does
not copy. To pass it on and keep using it, write `lend`: `grow(lend s)` hands
`grow` a temporary borrow of what `s` points at, and `s` is yours again after
the call. Calling a method on a `&mut` binding does this by itself.

<!-- example: borrow-lend -->

Passing the binding itself (`grow(s)`) moves it, and using `s` afterwards is
E0301. `lend` only works on a `&mut` you hold; a `&` is shared already, so you
simply pass it again.

### The rules

At any moment a value has either many `&` borrows or one `&mut` borrow, never
both. A borrow lasts until its last use, not until the end of the block, so
this is fine:

```drachm
mut string name = "Illy";
&string first = &name;
io::outln(first);
name = "Sam";
```

While a borrow is going to be used again, the value it points at cannot be
changed (E0302, E0303), read another way when the borrow is `&mut` (E0304),
moved (E0305) or dropped (E0306), and a function cannot return a borrow of
something it owns (E0307).

A method call borrows its receiver. If the arguments only read the receiver,
that is fine, because the borrow is not used until the call itself:

<!-- example: borrow-two-phase -->

`xs::push(xs::first())` is E0302: the borrow `first` returns is still in use
when `push` needs to change `xs`.

### Changing a value while it is borrowed (E0302)

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

Passing `&mut p` while a `&` borrow of `p` is still going to be used is E0302
as well, and so is changing `name` when the later use is a `return` or comes
round again in a loop; the message then says where.

### Two `&mut` at once (E0303)

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

### Using a value that is borrowed with `&mut` (E0304)

While a `&mut` borrow is going to be used, the value itself cannot be read or
borrowed again. Read it through the borrow, or before.

```drachm
in io;
struct Player { public: mut uint32 hp; }
none heal(&mut Player p) { p.hp = p.hp + 1; }
none main() {
    mut Player p = 1;
    &mut Player a = &mut p;
    io::outln(p.hp::text());
    heal(a);
}
```

```text title="drachm check"
error[E0304]: cannot use `p.hp` while `p` is borrowed with `&mut`
  --> main.drh:7:15
   |
 6 |     &mut Player a = &mut p;
   |                     ------ `p` is borrowed with `&mut` here
 7 |     io::outln(p.hp::text());
   |               ^^^^ used here
 8 |     heal(a);
   |          - the `&mut` borrow is still used here
   |
 help: read `p.hp` before borrowing `p` with `&mut`, or read it through `a`: `a.hp`
```

### Moving a value that is borrowed (E0305)

```drachm
in io;
none consume(string s) { }
none main() {
    string name = "Illy";
    &string first = &name;
    consume(name);
    io::outln(first);
}
```

```text title="drachm check"
error[E0305]: cannot move `name` while it is borrowed
  --> main.drh:6:13
   |
 5 |     &string first = &name;
   |                     ----- `name` is borrowed here
 6 |     consume(name);
   |             ^^^^ moved here
 7 |     io::outln(first);
   |               ----- the borrow is still used here
   |
 help: finish using `first` before moving `name`, or move a copy: `copy name`
```

### A borrow that outlives its value (E0306)

```drachm
in io;
none main() {
    mut &string keep = &"a";
    if (true) {
        string local = "x";
        keep = &local;
    }
    io::outln(keep);
}
```

```text title="drachm check"
error[E0306]: `local` does not live long enough
  --> main.drh:7:5
   |
 6 |         keep = &local;
   |                ------ `local` is borrowed here
 7 |     }
   |     ^ `local` is dropped here, at the end of its block
 8 |     io::outln(keep);
   |               ---- the borrow is still used here
   |
 help: declare `local` outside the block, so it lives as long as `keep` is used
```

### Returning a borrow of a local (E0307)

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

A function that returns a borrow and has more than one borrowed parameter says
which of them the result points into, with `lives`:
`&string longer(&string a, &string b) lives a, b`.

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
