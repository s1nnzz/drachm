---
title: Error index
description: Every error and warning code the compiler knows, with its explanation.
---

# Error index

Every error and warning code, generated from `src/diag/codes.rs` by
`site/gen-errors.sh`. The same text is printed by `drachm explain CODE`.
Do not edit this page by hand.

## E0001: unexpected character

The source contains a character that is not part of any Drachm token,
for example `#` or `@` outside a comment or string.

## E0002: invalid number

A number literal is malformed or too large: an unknown suffix (`5x`), a
whole-number suffix on a decimal (`2.5u8`), no digits after `0x`, or a
constant too large for any number type.

Suffixes are `u8 u16 u32 u64 u128`, `s8 s16 s32 s64 s128`, `f32 f64`,
and the bare `u` (uint32), `s` (sint32) and `f` (float64).

## E0003: unexpected token

The parser needed one thing and found another: a missing `;`, a missing
`)`, a statement that starts with something that cannot start one, and
so on. The label shows where, and the message says what was expected.

Conditions are always parenthesised: `if (x) { }`, `while (x) { }`.

## E0004: never closed

A `{`, a string or a `/*` comment was opened and never closed. The label
points at where it was opened.

## E0005: not assignable

Only a binding can be assigned to or changed with `++` / `--`.
`1 + 2 = x;` and `f()++` are not valid.

## E0006: parentheses needed

Bitwise operators (`& | ^ << >>`) have no fixed order against arithmetic
or comparison operators, so mixing them needs parentheses:

    if (a & 1 == 0) { }      // error
    if ((a & 1) == 0) { }    // ok

Shifts may sit inside `& | ^` (`r << 16 | g << 8 | b`), and a chain of
one operator needs none (`a | b | c`). Two different ones of `& | ^`,
or `<<` with `>>`, need parentheses.

## E0007: wrong call operator

`::` makes the first call on a value and `->` each call after it:

    x::to<uint8>()->text()

## E0008: invalid escape

A string contains a `\` followed by something that is not an escape.
The escapes are \n \t \r \0 \\ \" \' \` \$ \% and \u{1F600}.

## E0009: invalid format placeholder

An `io::outf` format has a `%` placeholder that cannot be read. A bare `%`
prints the next value as its `::text()`. Options go between the `%` and
the value, in this order:

    %8   %-8   %08      pad to 8 characters: on the left, on the right,
                        or with zeros after the sign
    %.2               two decimal places (floats)
    %x %X %b %o       hex, capital hex, binary, octal (whole numbers)

For example `%08x` or `%-10.2`. A letter other than x, X, b, o right after
a placeholder is an error, and so is `%%`: a percent sign is written `\%`.

## E0010: range out of place

An inclusive range `a..b` was used somewhere other than the two places
ranges can go so far: after `of` in a `for` loop and as a `case`
pattern.

    for (uint32 i of 0..9) { }     // ok: 0 to 9
    match (n) { case 1..5 { } }    // ok
    uint32 r = 0..9;               // error

## E0011: no loop to leave

`break` and `continue` work on loops. `break N;` and `continue N;` count
N loops outward (default 1), so N must be at least 1 and at most the
number of loops around the statement.

    while (a) {
        for (uint32 i of 0..9) {
            continue 2;            // next round of `while (a)`
            break 3;               // error: only 2 loops
        }
    }

To leave an `if`, use `skip`; to leave the function, `return`.

## E0012: nothing to skip

`skip` leaves `if` blocks the way `break` leaves loops. A whole
`if` / `else if` / `else` chain is one level, and so is a `match` (each
`case` is a branch of it). Bare `skip;` leaves the outermost level,
`skip N;` leaves N levels counting outward from the innermost, and
`skip to next;` goes on to the next branch of the innermost level.

Loops are walls: only the levels inside the current loop body count
(or inside the function body, outside any loop). This error means
there is no level to leave, N is 0 or larger than the number of
levels, or `skip to next` is in the last branch, which has no next one.

## E0013: invalid `for` header

The step of a `for (init; condition; step)` loop must be an assignment,
a compound assignment, `++` / `--`, or a call:

    for (mut uint32 i = 0; i < 10; i += 2) { }

## E0101: type mismatch

A value of one type was used where another is required.

    bool ok = 5;        // expected bool, found a number
    if (count) { }      // conditions must be bool

Number types convert to each other automatically, but `bool` is not a
number and never converts to or from one.

## W0102: lossy conversion

A number was converted to a type that cannot hold all of its values,
or a constant does not fit the type it is going into.

    sint64 big = 1000;
    uint8 small = big;             // W0102: 1000 becomes 232
    uint8 sure = big::to<uint8>(); // silent: you asked for it

Out-of-range values wrap around; converting a float to a whole number
drops the fraction first. In `a + b` the right side converts to the
left side's type, so this warning also appears on mixed arithmetic.

## E0103: wrong number of arguments

A function or method was called with more or fewer arguments (or type
arguments) than it takes.

## E0104: no value here

Something that produces no value was used where a value is needed: the
result of a `none` function, or a binding declared with type `none`.

## E0105: missing return

A function that declares a return type must return a value on every
path. Either a path reaches the end of the function without `return`,
or a `return;` gives no value.

## W0106: shift amount wraps

A value was shifted by a constant amount that is negative or not
smaller than its width. The amount wraps around the width: a `uint32`
shifted by 40 is shifted by 8.

## E0107: constant division by zero

A whole-number division or remainder has a constant zero on the right.
It would always panic, so it is rejected at compile time.

## E0108: format option does not suit the value

An `io::outf` placeholder asks for something its value cannot do:

    io::outf("%.2", count);    // decimal places need a float
    io::outf("%x", ratio);     // a base needs a whole number
    io::outf("%08", name);     // zero padding needs a number

## E0109: invalid pattern

A `case` pattern must be a constant of the matched type: a number or
constant expression (`case 3`, `case -1`, `case 2 ** 8`), `true` or
`false`, an inclusive range of numbers whose start is not above its
end (`case 13..19`), or `_` on its own (`case _`), which matches
anything.

## W0110: missing case

A `match` has values that no `case` handles, so for those nothing
runs. Add the missing cases, or `case _ { }` for everything else.

    match (flag) {
        case true { }
    }                             // W0110: `false` is not handled

On float values only `case _` handles everything (NaN equals nothing).

## W0111: unreachable case

A `case` can never run: it comes after `case _`, or every value it
matches is already handled by the cases before it. Cases are tested in
order and the first one that matches runs.

## E0112: no such operator

An operator was used on a struct that has no operator method for it.
The left side leads, so `a + b` calls the `+` method of `a`'s struct:

    struct Vector3 {
    public:
        float64 x; float64 y; float64 z;
        Vector3 +(Vector3 o) { Vector3 r = self.x + o.x, self.y + o.y, self.z + o.z; return r; }
    }

`==` and `!=` need no method: structs compare field by field (strings
by their text) unless they define `bool ==(T o)`.

## E0113: invalid operator method

An operator method has the wrong shape. Operator methods are named by
one of `+ - * / % **` or `== != < <= > >=`, take exactly one parameter
(the right-hand side), and are never `static`. Comparisons return
`bool`, and `==` / `!=` take a value of the struct's own type.

## E0114: struct construction

A struct built with the declaration shorthand must get every field
exactly once, either all in field order or all by name:

    Vector3 a = 1, 2, 3;
    Vector3 b = z: 3, x: 1, y: 2;
    Vector3 c = 1, y: 2, z: 3;     // error: mixed
    Vector3 d = 1, 2;              // error: `z` is missing

Only a struct can be built from a list of values.

## E0201: unknown name

A binding, function, method, module or type was used that does not
exist here. Check the spelling, and that the definition is visible.

## E0202: defined twice

Two things that need different names have the same one: two functions
or two structs in a file, two fields or two methods in a struct, a
field and a method of one struct, or a struct and a built-in type.

## E0204: missing or invalid `main`

A program needs exactly one `main` function that takes no parameters
and is either `none main()` or `sint32 main()` (which returns the exit
code).

## E0205: changing an immutable binding

Bindings and parameters cannot change unless declared `mut`.

    uint32 a = 1;
    a = 2;              // error
    mut uint32 b = 1;
    b = 2;              // ok

A binding declared without a value (`uint32 later;`) may be assigned
exactly once, where it certainly has no value yet.

A struct's fields can change when the binding is `mut` or the field
itself is `mut` (a `mut` field can always change):

    struct Player { public: uint32 level; mut uint32 hp; }
    Player p = 1, 100;
    p.hp -= 10;         // ok: `hp` is a `mut` field
    p.level = 2;        // error: neither `p` nor `level` is `mut`

A method that changes a non-`mut` field of `self` (or calls a method
that does) changes its value, so it can only be called on a `mut`
binding, never on a temporary value.

## E0206: module not imported

A module was used without importing it. Nothing is imported
automatically, not even `io`: add `in io;` at the top of the file.

## E0207: read before assignment

A binding declared without a value was read on a path where it may
not have been given one yet.

    uint32 x;
    if (ready) { x = 1; }
    io::outln(x::text());   // error: no value if `ready` is false

## W0208: method does not use `self`

A method that never uses `self` does not need a value to be called on.
Mark it `static` and call it on the struct instead:

    static Point origin() { Point p = 0, 0; return p; }
    Point o = Point::origin();

## E0209: no `self` here

`self` is the value a method was called on. It exists only inside a
struct's methods, and not in `static` ones, which are called on the
struct (`Point::origin()`) rather than on a value.

## E0210: struct contains itself

A struct has a field of its own type, directly or through other
structs (`A` holds a `B`, which holds an `A`). Its size would never
end, so this is not allowed.

## E0211: field of an unassigned struct

A single field was set on a struct binding that may not have a value
yet. Give the binding a whole value first:

    Point p;
    p.x = 1;            // error
    p = q;              // ok: the whole value

## E0212: private member

A field or method in a struct's private section was used outside that
struct's own methods. Before any `public:` or `private:` label, fields
are private and methods are public.

## E0213: static and non-static calls

A `static` method is called on the struct (`Point::origin()`); every
other method is called on a value (`p::length()`). This call used the
wrong one.

## E0309: `lives` does not match the body

A function's `lives` clause names whole parameters that hold a borrow,
and the result may borrow from those. Naming something that is not a
parameter holding a borrow is an error:

    &string pick(&string a, uint32 n) lives n { ... }   // `n` holds no borrow

## E0314: struct borrows without `lives`

A struct with a borrow field (`&string src;`) must say what it borrows
from, and `lives` must name exactly the borrow fields:

    struct Parser lives src { &string src; uint32 pos; }

## E0317: `unsafe` without the file flag

`unsafe { }`, `unsafe` functions, `*T` types and `address of` need the
file flag `//--!$unsafe: true` at the very top of the file.

## E0321: missing `&` or `&mut` at the call

A parameter of type `&T` or `&mut T` is passed by writing the borrow at
the call, so the call says what the callee may do:

    none grow(&mut string s) { ... }
    grow(&mut xs);          // not `grow(xs)`

## W0322: `copy` of a value that copies anyway

`copy` makes a deep copy, but numbers, bools, shared borrows and small
plain structs always copy when used, so `copy` changes nothing.

## W0323: `mut open` is redundant

`mut` already allows everything `open` does, so `mut open` says the same
as `mut`.

## E0326: `lend` needs a `&mut` you hold

`lend r` passes on a `&mut` borrow you hold and keeps it afterwards. The
operand must be a place of type `&mut T`; to borrow a plain value, write
`&mut x`.

## E0327: not a variable or field

`take`, `copy` and `address of` work on a place: a variable or a field
of one. A call result or other temporary value has nothing to take out
of; use it directly.

## E0328: `drop` has the wrong shape

A method named `drop` runs by itself when the value is destroyed, so it
must be written `none drop()`: not `static`, no parameters, not `unsafe`.

## E0329: borrow of a `&mut` you hold

`&mut s` where `s` is already a `&mut` binding would borrow the borrow.
To pass the borrow on and keep it, write `lend s`.

## E0330: `&mut` where `&` is wanted

A `&mut` binding passed bare where a `&T` is expected must be written
`&m`: an explicit shared borrow through the `&mut` you hold.

## E0331: calling `drop` yourself

`drop` runs by itself when a value is destroyed. To end a value now,
write `destroy x;`.

## E0324: field moved without `take`

Moving a field out of a struct you own leaves the struct without that
field, so it must be said: `take p.name` moves it out and leaves `p.name`
without a value until you give it one. To keep both, write `copy p.name`.

## E0325: `take` on a struct with `drop()`

A struct with a `drop()` method must stay whole, because `drop` runs when
it is destroyed. Take a field only with a replacement in the same
statement:

    string s = take f.path leaving "";

## E0301: used after it moved

A value of a type that does not copy automatically (`string`, a big
struct, a `&mut` borrow) moves when it is assigned, passed or returned.
The old name cannot be used again until it is given a new value:

    string a = "x";
    string b = a;            // `a` moved
    io::outln(a);            // error

To keep both, copy: `string b = copy a;`. Moving one field out with
`take p.name` leaves `p.name` without a value until it is assigned
again; `p` as a whole cannot be used meanwhile.

## E0310: cannot move out of a borrow

A function that only borrows a value does not own it, so it cannot
move anything out of it, `self` and raw pointers included. Return or
pass a copy instead:

    string nameOf(&Player p) { return copy p.name; }

## E0313: assigning part of a moved value

A field cannot be given a value while the struct holding it has moved
as a whole. Give the whole struct a new value first:

    consume(p);
    p = ...;
    p.hp = 5;

## E0315: cannot copy a value holding `&mut`

A copy of a struct with a `&mut` field would be a second `&mut` to the
same value. Borrow the struct instead: `&Writer v = &w;`.

## E0332: assigning part of a value that has none yet

A field cannot be given a value while the struct holding it has no
value at all yet. Give the whole struct a value first:

    mut Player p;
    p = ...;
    p.hp = 5;

## E0402: unknown unit

A unit was used that is not declared. Declare it with `unit NAME;` for a
new base unit, or `unit NAME = ...;` to give it a size in other units,
such as `unit km = 1000 m;`.

## E0403: bad unit declaration

A `unit` declaration is not valid: a name declared twice, a unit defined
in terms of itself, a factor that is not a number greater than zero, an
`offset` after anything but a single unit, or a name that is not plain
ASCII letters, digits and `_`.

## E0405: a unit cannot go here

A unit in angle brackets (`float64<m>`) goes only on a number type, and a
unit's power (`m^2`) must stay within the range of a 16-bit exponent.

## E0901: not supported yet

This is part of Drachm's design, but this version of the compiler does
not implement it yet.

## E0318: invalid file flag

A `//--!$` line at the top of a file sets a file flag. Flags are read
before the code, so they come first: above the first line of code,
after an optional `#!` line, with no blank line in between.

    //--!$unsafe: true

A flag is `key: value`. `unsafe` takes `true` or `false`. A line that
is not `key: value`, a value the key does not accept, or a flag below
the top block is an error.

## W0319: unknown file flag

A `//--!$` line names a key this compiler does not know. The line is
ignored, so the file still builds, but a misspelt key does nothing:
`//--!$unsfe: true` is not `unsafe`. New keys come with later versions
of the language, so older compilers warn instead of failing.

