---
title: Reading compiler errors
description: How a Drachm error is laid out, what the codes mean, and a worked fix for each family of mistakes.
---

# Reading compiler errors

Everyone writes programs the compiler rejects, so reading its messages is a
skill worth learning early. Drachm messages are built to be read from the top:
the first line says what is wrong, the arrows say where, and the `help` line
says what to write. This guide takes one message apart, then walks through one
real mistake from each family and its fix.

You can follow along with `drachm check file.drh`, which reports problems and
builds nothing.

## The shape of a message

Every message has the same parts. This one comes from adding a number to a
string.

<!-- focus: 1 -->
The first line is the **headline**: a severity (`error` or `warning`), a
**code** in brackets, and a short title. The code never changes, so you can
search for it or ask the compiler about it.

<!-- focus: 2 -->
The **location** gives file, line and column. Many editors turn it into a link.

<!-- focus: 3-7 -->
The **source lines** quote your code. A label under `^^^` marks the exact place
that is wrong, and a label under `---` marks related code, here the left side
of the `+`. Labels say what the compiler knows about the spot, such as a type.

<!-- focus: 9-10 -->
A **note** explains a rule, and a **help** line proposes a fix. Not every
message has both. When `help` shows code in backticks, you can usually paste it.

<!-- focus: 12 -->
The last line counts the errors and warnings in the file. The compiler reports
everything it finds in one run, so fix from the top and recheck.

```drachm title="main.drh"
in io;

none main() {
    uint32 hp = 42;
    io::outln("hp: " + hp);
}
```

```text title="drachm check main.drh"
error[E0101]: `+` joins strings, but the right side is `uint32`
  --> main.drh:5:24
   |
 5 |     io::outln("hp: " + hp);
   |               ------ this is `string`
   |                        ^^ this is `uint32`
   |
 help: turn it into text: `hp::text()`

main.drh: 1 error
```

## Errors and warnings

The letter in the code is the severity. An **E** code is an error: the program
does not compile, and `drachm check` exits with code 1. A **W** code is a
warning: the program still compiles and runs, and the message is printed first.
Treat warnings as errors you have not been forced to fix yet. They point at
code that probably does not do what you meant.

This program has two warnings and no errors, so `check` exits with 0.

```drachm title="main.drh"
in io;

none main() {
    sint64 big = 1000;
    uint8 small = big;
    io::outln(small::text());
    uint8 n = 3;
    match (n) {
        case 0 { io::outln("zero"); }
    }
}
```

```text title="drachm check main.drh"
warning[W0102]: converting `sint64` to `uint8` can lose information
  --> main.drh:5:19
   |
 5 |     uint8 small = big;
   |                   ^^^ this is `sint64`
   |
 note: `sint64` holds -9223372036854775808 to 9223372036854775807; `uint8` holds 0 to 255; values outside that wrap around
 help: write `big::to<uint8>()` to say the conversion is intended

warning[W0110]: this `match` does not handle `1..255`
  --> main.drh:8:5
   |
 8 |     match (n) {
   |     ^^^^^^^^^ `n` is `uint8`
   |
 help: add a `case` for them, or `case _ { }` for everything else

main.drh: 2 warnings
```

## What the digits mean

The first two digits of a code name the family of the problem, and they tell
you where to look.

| Codes | Family |
|---|---|
| `00xx` | syntax: the text is not Drachm |
| `01xx` | types, conversions, `match`, control flow |
| `02xx` | names, declarations, `mut` |
| `03xx` | ownership: moves, copies, borrows |
| `04xx` | units of measure |
| `05xx` | enums |
| `09xx` | valid Drachm that this compiler cannot do yet |

The rest of this guide has one example from each of the main families.

## Asking for more with drachm explain

A message is short on purpose. When you want the whole story for a code, ask:

```sh
drachm explain E0301
```

You get the title, a longer description and a small example, as the output
below shows. The same text for every code is on the [error index](errors.md).
An unknown code is reported as such.

```text title="drachm explain E0301"
E0301: used after it moved

A value of a type that does not copy automatically (`string`, a big
struct, a `&mut` borrow) moves when it is assigned, passed or returned.
The old name cannot be used again until it is given a new value:

    string a = "x";
    string b = a;            // `a` moved
    io::outln(a);            // error

To keep both, copy: `string b = copy a;`. Moving one field out with
`take p.name` leaves `p.name` without a value until it is assigned
again; `p` as a whole cannot be used meanwhile.
```

## Syntax: the text is not Drachm

Syntax errors are `E00xx`. The compiler found something it could not read. The
one to know is `E0003`, "unexpected token". Here a `;` is missing at the end of
line 4.

The compiler only notices the problem when it reaches the next line, because
`uint32 hp = 3` could still continue. So the arrow points at the start of the
*next* statement, and the real mistake is at the end of the line before it.
When a syntax error looks wrong, read the line above.

```drachm title="main.drh"
in io;

none main() {
    uint32 hp = 3
    io::outln(hp::text());
}
```

```text title="drachm check main.drh"
error[E0003]: expected `;`, found name `io`
  --> main.drh:5:5
   |
 5 |     io::outln(hp::text());
   |     ^^

main.drh: 1 error
```

The fix is to end line 4 with `;`.

## Types: a value of the wrong kind

Type errors are `E01xx`. Drachm never converts between unrelated types for
you, so the compiler says what it expected and what it found. The common case
is a number where text is wanted, as in the first example above, and the
`help` line already has the fix.

Conditions are strict too. A number is not a `bool`, and the message says so.

```drachm title="main.drh"
in io;

none main() {
    uint8 a = 3;
    if (a) {
        io::outln("x");
    }
}
```

```text title="drachm check main.drh"
error[E0101]: expected `bool`, found `uint8`
  --> main.drh:5:9
   |
 5 |     if (a) {
   |         ^ this is `uint8`
   |
 help: `bool` is not a number and never converts to or from one

main.drh: 1 error
```

Say what you mean instead: `if (a != 0)`. Number mistakes are covered in
more depth in [Numbers without surprises](guide-numbers.md).

## Names: something does not exist

Name errors are `E02xx`. `E0201` means a name or function is not known at that
point. Usually it is a typo, a missing `in io;`, or a binding declared in a
different block. Two typos give two errors, because the compiler keeps going.

```drachm title="main.drh"
in io;

none main() {
    uint32 hp = 42;
    io::outln(hpp::text());
    io::outln(missing(3));
}
```

```text title="drachm check main.drh"
error[E0201]: unknown name `hpp`
  --> main.drh:5:15
   |
 5 |     io::outln(hpp::text());
   |               ^^^

error[E0201]: unknown function `missing`
  --> main.drh:6:15
   |
 6 |     io::outln(missing(3));
   |               ^^^^^^^^^^

main.drh: 2 errors
```

There is no `help` line here, because the compiler cannot know which name you
meant. Compare the spelling against the declaration, and check that the name
is in scope.

## Control flow: a path with no answer

Control flow errors live in the `01xx` group as well. A function that promises
a value must give one on every path. Here `sign` returns only when `n` is
negative, so for any other `n` the function would run off its end.

The label sits on the function's head, not on a line in the body, because the
problem is the whole function and not one statement.

```drachm title="main.drh"
uint32 sign(sint32 n) {
    if (n < 0) {
        return 1;
    }
}

none main() {
    uint32 x = sign(3);
}
```

```text title="drachm check main.drh"
error[E0105]: function `sign` can finish without returning a value
  --> main.drh:1:1
   |
 1 | uint32 sign(sint32 n) {
   | ^^^^^^^^^^^^^^^^^^^^^ promises to return `uint32`
   |
 help: add a `return` at the end, or to every branch

main.drh: 1 error
```

Add a `return 0;` after the `if`, or an `else` with its own `return`. The same
family has `E0012` when you write `skip;` with no `if` or `match` around it:

```text title="a skip outside any if"
error[E0012]: `skip` is not inside an `if` or a `match`
   --> main.drh:10:5
    |
 10 |     skip;
    |     ^^^^^ nothing to skip here
    |
 help: `skip` leaves `if` blocks; use `return` to leave the function
```

## Ownership and borrowing: who owns what

Ownership errors are `E03xx`, and they are the ones most new readers find odd,
because the rule is new. A `string` has one owner. Writing `string b = a;`
moves the text to `b`, and `a` has nothing in it afterwards. The labels show
the whole story in order: the move, then the later use.

The second error is a borrow. `&c` lends a read-only view of `c`, and while the
view is still going to be used, `c` must not change. The label on the last line
says *the borrow is still used here*, which is what makes the change illegal.

```drachm title="main.drh"
in io;

none main() {
    string a = "hi";
    string b = a;
    io::outln(a);
    mut string c = "x";
    &string q = &c;
    c = "y";
    io::outln(q);
}
```

```text title="drachm check main.drh"
error[E0301]: `a` is used after it moved
  --> main.drh:6:15
   |
 5 |     string b = a;
   |                - `a` moved here
 6 |     io::outln(a);
   |               ^ used here, after the move
   |
 help: to keep both, write `string b = copy a;`

error[E0302]: cannot change `c` while it is borrowed
   --> main.drh:9:5
    |
  8 |     &string q = &c;
    |                 -- `c` is borrowed here
  9 |     c = "y";
    |     ^ change attempted here
 10 |     io::outln(q);
    |               - the borrow is still used here
    |
 help: finish using `q` before changing `c`

main.drh: 2 errors
```

The first fix is the one in `help`: `string b = copy a;`. The second is to print
`q` before the assignment, so the borrow ends first. See
[Ownership](lesson-8-ownership.md) and [Borrowing](lesson-9-borrowing.md) for
the ideas behind these.

## Units: the dimensions do not match

Unit errors are `E04xx`. They are the compiler doing its job: you cannot add
metres to seconds, and a plain number has no unit. The first error shows two
labelled operands. The second tells you how to give the number a unit, and
shows the exact text to write.

```drachm title="main.drh"
in si;
in io;

none main() {
    float64<m> d = 5.0<m>;
    float64<s> t = 2.0<s>;
    float64<m> bad = d + t;
    float64<m> bad2 = d + 5.0;
    io::outln(bad::text());
}
```

```text title="drachm check main.drh"
error[E0401]: units do not match: `float64<m>` and `float64<s>`
  --> main.drh:7:26
   |
 7 |     float64<m> bad = d + t;
   |                      - this is `float64<m>`
   |                          ^ this is `float64<s>`
   |
 note: values can only be added or subtracted when they are the same kind of unit
 help: to relate different kinds, multiply or divide them: `distance / time`

error[E0408]: a plain number cannot stand for a measure
  --> main.drh:8:27
   |
 8 |     float64<m> bad2 = d + 5.0;
   |                           ^^^ this number has no unit
   |
 help: give it a unit: `5.0<m>`

main.drh: 2 errors
```

See [Units of measure in practice](guide-units.md) for more.

## A routine for fixing errors

1. Fix the **first** error first. Later errors are sometimes caused by it.
2. Read the headline, then the label under the `^^^`. Together they usually
   say what the compiler saw.
3. Read `note` for the rule and `help` for the fix, and try the fix.
4. Run `drachm check` again. It is quick, and it shows what is left.
5. If the message is still unclear, run `drachm explain` with the code.

This program is the corrected version of the ownership example above. It
compiles with no messages at all.

<!-- example: guide-errors-fixed -->

Next: [Numbers without surprises](guide-numbers.md).
