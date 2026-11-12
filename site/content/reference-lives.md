---
title: Where a borrow comes from
description: When a function needs lives, how the compiler works it out, lives on structs, and the errors E0308, E0309 and E0320.
---

# Where a borrow comes from

A function that returns a borrow must give the compiler a way to know which of
its arguments the result points into. A caller needs that to keep those
arguments alive while it uses the result. Mostly the compiler works it out by
reading the body, and you write `lives` only when the answer is ambiguous.
The [ownership and borrowing guide](guide-ownership.md) puts this in context.

`lives` names whole parameters, never parts of them.

## When you write nothing

If a function could only return a borrow of one parameter, there is nothing to
say. The compiler reads the body, finds that parameter, and callers use that.
This also works through calls, and through functions that call each other.

A method that returns a borrow of `self`, such as a getter
(`&string title() { return &self.name; }`), needs nothing either.

## When you write lives

If the result may come from several parameters, depending on the path taken,
the compiler asks you to list them:

<!-- example: lives -->

`longest` lists both, so a caller keeps both `x` and `y` alive while it uses
the result. A method names `self` with `lives from`, joining the sources with
`or`: `lives from self or key`. Plain `lives a, b` lists parameters.

You may list more than the body needs. The extra names are accepted, and only
make a caller keep more things alive.

### Which parameter? (E0308)

```drachm
&string longest(&string a, &string b) {
    if (a::len() > b::len()) { return a; }
    return b;
}
none main() { }
```

```text title="drachm check"
error[E0308]: `longest` returns a borrow, but which parameter it borrows from depends on the path
  --> main.drh:1:9
   |
 1 | &string longest(&string a, &string b) {
   |         ^^^^^^^ the result borrows from `a` or `b`
 2 |     if (a::len() > b::len()) { return a; }
   |                                       - returns a borrow of `a` here
 3 |     return b;
   |            - returns a borrow of `b` here
   |
 help: write `lives a, b` after the parameters
```

The labels show each place the result is returned from, and the help is the
fix. For a method that may return `self` or a parameter, the help reads
`write `lives from self or key` after the parameters`.

### A lives that is too short (E0309)

```drachm
&string pick(&string a, &string b) lives a { return b; }
none main() { }
```

```text title="drachm check"
error[E0309]: `pick` returns a borrow of `b`, but its `lives` names only `a`
  --> main.drh:1:53
   |
 1 | &string pick(&string a, &string b) lives a { return b; }
   |                                    ------- `lives` names only `a` here
   |                                                     ^ returns a borrow of `b` here
   |
 help: add `b` to the list: `lives a, b`, or return a borrow of `a`
```

E0309 is also the error for a `lives` that names something that is not a
parameter holding a borrow.

### Storing a borrow where the caller cannot promise it (E0320)

A parameter that holds a borrow can only be given borrows that last as long as
what the caller put there. A borrow of another parameter does not:

```drachm
struct Parser lives src { public: mut &string src; uint32 pos; }
none keep(&mut Parser a, &string b) { a.src = b; }
none main() { }
```

```text title="drachm check"
error[E0320]: cannot store a borrow of `b` inside `a`
  --> main.drh:2:47
   |
 2 | none keep(&mut Parser a, &string b) { a.src = b; }
   |           ------------- `a` can only hold borrows that last as long as the caller's value
   |                                               ^ the borrow of `b` is stored in `a` here
   |
 help: the caller cannot promise that `b` lives as long as what `a` points to; store a copy: `copy b`
```

## lives on structs

A struct that holds borrows lists the borrow fields in its head. Each field may
borrow from a different place, so a `Pair` can hold borrows of two different
values. The struct is only valid while everything it borrows from is.

<!-- example: lives-struct -->

Here `p` is finished before `y` goes away, so this is accepted. Using `p.b`
after the block would be E0306, and the message says the borrow of `y` is
"kept in `p.b`".

A function that returns such a struct follows the same rules as one that
returns a `&string`: with several candidate parameters you write `lives`.

## The conservative limit

`lives p` keeps the whole parameter `p` borrowed while the result is in use,
even when the result only points at something `p` holds:

```drachm
in io;
struct Parser lives src { public: &string src; mut uint32 pos; }
&string rest(&Parser p) { return p.src; }
none main() {
    string text = "hello";
    mut Parser p = &text, 0;
    &string r = rest(&p);
    p.pos = 3;
    io::outln(r);
}
```

This is rejected with E0302 (`cannot change `p.pos` while it is borrowed`).
Finish with `r` before changing `p`.
