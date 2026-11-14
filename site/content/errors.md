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

## E0014: `expect failure` outside a test

`expect failure` checks that a call fails with a given failure. It only
means something in a `test` block, where a wrong outcome fails the test,
so anywhere else it is an error:

    test "bad text" {
        expect failure ConvertError from "x"::to<uint32>();
    }

Outside a test, handle the failure with `fails with` instead.

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

## E0116: constant does not fit (overflow = panic)

A constant was written where its type cannot hold it, in a function or
file whose `overflow` setting is `panic`. With `wrap` or `saturate` the
constant is changed to a value that fits (and W0102 says so); with
`panic` it is an error, since the program could never run it.

    //--!$overflow: panic
    uint8 a = 300;        // E0116

Use a value that fits, or a bigger type.

## E0115: a range step of zero

A range loop's `by` step is written as a constant zero, so the loop
would never move on. It is rejected at compile time. A step that is
zero only when the program runs stops the program at that point with
the run-time error `range step is zero` (exit 101).

    for (uint32 i of 0..10 by 0) { }    // E0115
    for (uint32 i of 0..10 by n) { }    // a zero n stops the program

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

## W0117: whole number given to a float function

`math::sqrt`, `math::log`, `math::log2`, `math::log10` and `math::pow`
have a whole-number version: given a whole number they give a whole
number, the true result rounded down (`math::sqrt(10u32)` is 3).
This warning is there in case a fractional result was wanted. Convert
the number first for that: `value::to<float64>()`.

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
code). A script can instead start with top-level statements. A script
with an explicit `main` cannot also have top-level statements.

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

## E0230: module not found

An `in` names a module that is neither a standard module (`io`, `si`, `math`, `os`, `fs`, `time`)
nor a file of the project. A path is relative to the folder of the file
that writes it: `in game.physics;` (or `in physics inside game;`) means
the file `game/physics.drh` beside this one. To start from the source
folder of the project, begin with the project's name from `drachm.toml`:

    in mygame.game.physics;

A module file name must be a plain name (letters, digits, `_`).

## E0231: module name is ambiguous

An `in` path matches two places: a standard module and a file of the
project (a file `math.drh` beside the importer and the standard `math`),
or the project's own name and a folder of that name. Drachm does not
pick one silently, so adding a standard module later can never change
what a program means. Rename the file or folder.

## E0232: two imports with the same name

Two `in` lines give different modules the same name in one file, for
example `game/audio.drh` and `engine/audio.drh` are both `audio`. Rename
one with `as`:

    in game.audio;
    in engine.audio as engineAudio;

The same goes for names imported one by one (`in Vector3 from a;` and
`in Vector3 from b;`, or an import with the name of a declaration in
the file): rename with `as`: `in Vector3 as VectorB from b;`.

## E0233: exported twice

A name is exported more than once from one file, with `out` before the
declaration and again in an `out` list, or in two lists. The name other
files see (after `as`) must be unique in the file.

## E0234: exporting a name that does not exist

An `out` list names something this file does not declare. Only the
functions, structs, enums, interfaces, failures and units written in the
file can be exported.

## E0235: cannot read a module file

The file an `in` points at exists but could not be read (permissions,
or it is not valid UTF-8 text).

## E0236: name is private

A name was imported or used through its module (`in Vector3 from physics;`,
`phys::gravity()`), but the module does not export it. Only what a file
marks with `out` can be used from another file:

    out struct Vector3 { ... }      // exported
    out gravity, Vector3 as Vec3;   // a list, which may rename

Everything else is private to its file.

## E0237: no such name in the module

An import or a qualified name asks a module for a name it does not have.
Check the spelling; the help lists what the module exports.

## E0239: two tests have the same name

Each `test` block needs its own name, because `drachm test` reports
a test by its name. Two blocks in one file with the same name are an
error; the second one is the one reported.

    test "adds" { assert(add(2, 3) == 5); }
    test "adds" { assert(add(1, 1) == 2); }   // E0239


## E0238: files call each other in a circle

A function of one file calls a function of another file that calls back
into the first (directly or through more functions). Files may import
each other, and types and constants may refer to each other across
files, but code that calls itself around a circle through several files
is rejected. The error shows the chain. Recursion inside one file is fine.

## W0240: imported name is hidden

An `in * from m;` brings a name that this file already has: its own
declaration wins over any import, and between two imports the first
one wins. The warning is at the later import, and its help asks whether
you meant the module's name, `m::play`. Import the name on its own with
`as` to use both: `in play as playAudio from audio;`.

## E0242: imported name is hidden (import_clash = error)

An `in * from m;` brings a name that this file already has, and the
project says `import_clash = "error"` in `[behaviour]`. It is the same
clash as W0240: a declaration of this file wins over any import, and
between two imports the first one wins. With the default,
`import_clash = "warn"`, it is the warning W0240 instead. Import the
name on its own with `as` to use both: `in play as playAudio from audio;`.

## E0243: `intrinsic` is for the standard library

A function written `intrinsic;` instead of a body is one the compiler itself
knows how to call: only the modules embedded in the compiler (`math`, `fs`,
...) have them. A function of your own needs a body.

    out float64 half(float64 x) { return x / 2.0; }   // yours

## W0241: name hides an imported module

A declaration in this file has the same name as a module this file
imports. The declaration wins inside the file, so the module cannot
be reached by that name. Rename the module with `as`.

## E0308: which parameter the result borrows from is ambiguous

A function that returns a borrow gets its `lives` from its body when only
one parameter can be the source. When the result may borrow from several
parameters, depending on the path taken, say which with `lives`:

    &string longest(&string a, &string b) lives a, b {
        if (a::len() > b::len()) { return a; }
        return b;
    }

A method that may return a borrow of `self` or of a parameter names `self`
after `from`: `lives from self or key`.

## E0309: `lives` does not match the body

A function's `lives` clause names whole parameters that hold a borrow,
and the result may borrow from those. Naming something that is not a
parameter holding a borrow is an error:

    &string pick(&string a, uint32 n) lives n { ... }   // `n` holds no borrow

It is also an error when the body returns a borrow of a parameter the
clause does not name:

    &string pick(&string a, &string b) lives a { return b; }   // add `b`

## E0320: a borrow stored where the caller cannot promise it

A parameter that holds a borrow, such as `&mut &string a`, can only be given
borrows that last as long as what the caller put there. Storing a borrow of
another parameter into it is not allowed, because the caller cannot promise
the other parameter lives that long:

    none keep(&mut &string a, &string b) {
        a = b;                   // error
    }

Store a copy of the value instead.

## E0314: struct borrows without `lives`

A struct with a borrow field (`&string src;`) must say what it borrows
from, and `lives` must name exactly the borrow fields:

    struct Parser lives src { &string src; uint32 pos; }

## E0317: `unsafe` without the file flag

`unsafe { }`, `unsafe` functions, `*T` types, C calls and `address of` need
the file flag `//--!$unsafe: true` at the very top of the file, or
`unsafe = true` under `[native]` in `drachm.toml`, which allows every
file of the project (and of a dependency that sets it).

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

## E0302: cannot change while borrowed

A value cannot be changed while a borrow of it is still going to be
used. The borrow ends after its last use, so use it first:

    mut string name = "Illy";
    &string first = &name;
    name = "Sam";             // error: `first` is used below
    io::outln(first);

Move `io::outln(first);` above the assignment, or do not hold the
borrow across it.

## E0303: two `&mut` at once

Only one `&mut` borrow of a value can be in use at a time, so that
nothing else can change it behind your back:

    &mut Player a = &mut p;
    &mut Player b = &mut p;     // error: `a` is used below
    heal(a);

Finish using the first borrow before taking the second.

## E0304: used while borrowed with `&mut`

While a `&mut` borrow of a value is going to be used, the value itself
cannot be read or borrowed again, only the borrow can:

    &mut Player a = &mut p;
    io::outln(p.hp::text());    // error: `a` is used below
    heal(a);

Read through the borrow (`a.hp`), or read before borrowing.

## E0305: moved while borrowed

A value cannot be moved away while a borrow of it is still going to be
used:

    string name = "Illy";
    &string first = &name;
    consume(name);              // error: `first` is used below
    io::outln(first);

Finish with the borrow first, or move a copy: `consume(copy name)`.

## E0306: does not live long enough

A value is dropped at the end of its block, or when its function
returns, but a borrow of it is still going to be used after that:

    &string keep;
    if (true) {
        string local = "x";
        keep = &local;
    }                           // `local` is dropped here
    io::outln(keep);            // error

Declare the value outside the block, so that it lives as long as the
borrow is used.

## E0307: returns a borrow of a local

A function cannot return a borrow of something it owns: the value is
dropped when the function returns, and the borrow would point at
nothing. This covers its own locals, its by-value parameters and
temporaries:

    &string shout() {
        string s = "hi"::upper();
        return &s;               // error
    }

Return the value itself (`string shout()` and `return s;`), or borrow
from a parameter that is already a borrow.

## E0310: cannot move out of a borrow

A function that only borrows a value does not own it, so it cannot
move anything out of it, `self` and raw pointers included. Return or
pass a copy instead:

    string nameOf(&Player p) { return copy p.name; }

## E0311: cannot change through a `&` borrow

A `&` borrow only reads what it points to, and that includes the `mut`
fields of a struct behind it. Changing it, lending it `&mut`, or calling
a method that changes `self` needs a `&mut` borrow:

    none hurt(&mut Player p) { p.hp = p.hp - 1; }

## E0312: cannot lend `&mut` from an immutable binding

A `&mut` borrow, and a method that changes `self`, need a binding that is
`mut` (it may also be reassigned) or `open` (its contents may change).
A temporary value has no name to make either, so it cannot be lent
`&mut` at all. A method that changes only `mut` fields is the exception:

    Counter k = "page", 0;
    k::hit();                 // fine if `hit` writes only `mut` fields
    open string xs = "a";
    grow(&mut xs);            // fine

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

## E0401: units do not match

Two values were combined, compared or assigned, and their units measure
different kinds of things: metres and seconds, say. Values add, subtract
and compare only when they are the same kind of unit (`10<km> + 100<m>`
is fine). To combine different kinds, multiply or divide: `dist / time`.

## E0402: unknown unit

A unit was used that is not declared. Declare it with `unit NAME;` for a
new base unit, or `unit NAME = ...;` to give it a size in other units,
such as `unit km = 1000 m;`. Inside a generic, a type parameter named
like a unit is the type, so a plain name there is not a unit: write
`T as unit` for the unit (W0618).

## E0403: bad unit declaration

A `unit` declaration is not valid: a name declared twice, a unit defined
in terms of itself, a factor that is not a number greater than zero, an
`offset` after anything but a single unit, or a name that is not plain
ASCII letters, digits and `_`.

## E0404: a unit can only be raised to a constant whole power

The unit of `side ** n` is the unit of `side` raised to `n`, so `n` must
be a whole number the compiler can see (`side ** 2`). Write `side * side`
or use a constant exponent.

## E0405: a unit cannot go here

A unit in angle brackets (`float64<m>`) goes only on a number type, and a
unit's power (`m^2`) must stay within the range of a 16-bit exponent.

## E0406: bit operators cannot be used on a value with a unit

`<<`, `>>`, `&`, `|`, `^` and `~` change a pattern of bits, not a measure.
For a value with a unit use arithmetic: `len * 2` instead of `len << 1`.

## E0407: cannot convert to this unit

`x::in<unit>()` changes the unit of `x` to another unit of the same kind
(`d::in<km>()`), and `x::to<T>()` changes only the number type, so `T` is
a plain number type (`d::to<float32>()`). A point such as `degC` cannot
be turned into a `delta`.

## E0408: a plain number cannot stand for a measure

A number without a unit was used where a measure is needed: `d + 5`,
`d < n`, or a plain number given to a variable with a unit. Write the
unit (`d + 5<m>`). Multiplying and dividing by plain numbers is fine, and
zero is unit-free in `+`, `-` and comparisons, except beside a point such
as `degC`.

## E0410: a unit has no square root

`math::sqrt` halves the powers of a unit: `math::sqrt(9.0<m^2>)` is `3.0<m>`.
A unit with an odd power (`m`, `m^3`) has no square root. A point (`degC`)
has none either: take the root of an amount.

## E0411: this math function needs a plain number

`math::sin`, `cos` and `tan` take a plain number (radians) or an angle (`rad`,
`deg`); `exp`, `log`, `log2`, `log10` and the exponent of `pow` take a plain
number. A value with another unit, such as metres, is not one. Divide by a
value of the same unit to get a plain number: `distance / 1.0<m>`.

## E0409: points and deltas cannot be mixed this way

A unit with an offset, such as `degC`, measures a point on a scale, not an
amount. Subtract two points to get a `delta`, and add a `delta` to a point
to get a point. Two points cannot be added, and a point cannot be scaled
or negated.

## E0333: cannot write through a `*fixed` pointer

A `*fixed T` pointer comes from `address of` an immutable binding, so it
can only be read. To write through a pointer, make the binding it
points to `mut` (or `open`) and take the address again:

    mut uint32 n = 5;
    unsafe { *uint32 p = address of n; *p = 6; }

## E0334: `*T` and `*fixed T` do not convert to each other

A `*T` and a `*fixed T` are different kinds of pointer, and an assignment,
argument or comparison does not change one into the other. To change
the kind, say so inside an `unsafe { }` block:

    unsafe { *fixed uint32 r = w::freeze(); }   // `*T` to `*fixed T`
    unsafe { *uint32 w2 = r::thaw(); }          // `*fixed T` to `*T`

## E0335: a pointer goes to C only with `as foreign`

A Drachm pointer (`*T`, `*fixed T`) and a C pointer (`foreign *T`) are
different types, and nothing changes one into the other by itself. The
hand-over to C is written out with `as`, inside `unsafe { }`:

    unsafe { sdl::SDL_free(buf as foreign *none); }    // any pointer to `void *`
    unsafe { foreign *uint8 p = text as foreign *uint8; }   // `*uint8` to `foreign *uint8`

## E0336: a union member needs `unsafe`

A C union holds the bytes of one member at a time, and nothing in the
program says which one. Reading or writing a member of a union, or of
a struct inside it (`ev.key.down`), must be inside an `unsafe { }`
block. Check the tag the library gives you (SDL's `ev.type`) first:

    unsafe {
        if (ev.type == sdl::SDL_EVENT_KEY_DOWN) { uint32 key = ev.key.key; }
    }

A union value starts as zero bytes (`new sdl::SDL_Event of ()`), and C fills
it through its address (`(address of ev) as foreign *sdl::SDL_Event`).

## E0316: an operation on a raw pointer needs `unsafe`

Making, reading, writing, copying, comparing, converting, moving, passing
or returning a raw pointer, and calling an `unsafe` function, must be
inside an `unsafe { }` block. The body of an `unsafe` function is one
such block:

    none read(*uint32 p) { unsafe { uint32 v = *p; } }
    unsafe none bump(*uint32 p) { *p = *p + 1; }

## E0520: duplicate variant

An enum has two variants with the same name. Each variant of an enum
needs a name of its own, because the name is how a value of that
variant is built and matched.

    enum Shape { Circle(float64 r), Circle }     // error

## E0521: unknown variant

The name after `Enum::` is not a variant of that enum. The message
lists the variants it does have.

    Shape s = Shape::Triangle(1.0);               // error: no `Triangle`

## E0522: wrong number of values for a variant

A variant is built with exactly one value for each value it declares,
in order: `Rect(float64 w, float64 h)` takes two.

    Shape s = Rect(2.0);                          // error: needs 2

## E0523: cannot tell which enum a variant belongs to

A bare variant name such as `Circle(1.0)` or `Empty` takes its enum
from the type the value is going to: a declaration with a written
type, an assignment, an argument or a `return`. Where nothing says,
write the enum: `Shape::Circle(1.0)`.

## E0525: pattern of another enum

A `case` names something that is not a variant of the enum being
matched. The message lists the variants that exist.

## E0527: enum with values compared with `==`

`==` and `!=` work on enums whose variants carry no values (`d == North`).
An enum with values is taken apart with `match`.

    if (shape == Empty) { }                      // error when some variant has values

## E0528: binding that owns memory needs take or copy

A payload that owns memory (a `string`, or a struct holding one) cannot
be bound by a plain `Type name` in a `case`: the pattern has to say
whether it moves the payload out or copies it.

    case Text(take string body) { }   // moves it; the matched value is used up
    case Text(copy string body) { }   // copies it; the matched value stays whole

Numbers, bools and Copy structs bind plainly: `case Circle(float64 r)`.

## E0529: inconsistent bindings in a case

A name is bound twice in one pattern, or the patterns of one `case`
(`case A(float64 x), B(float64 y)`) do not bind the same names with
the same types, so the body could not know which one it has.

## E0530: `none` where no optional type is wanted

`none` is the empty value of an optional type (`uint32? x = none;`).
Where no optional type is expected there is nothing for it to be empty
in, so the place needs an optional type.

    uint32? x = none;                           // fine
    uint32 y = none;                            // error: no optional type

## E0531: an optional used as its value

A `uint32?` may be `none`, so it is not a `uint32`. Arithmetic, a
plain `uint32` argument or a plain declaration cannot take it.
Give the empty case a value with `maybe`, or test it with `has value`
and `is empty`.

    uint32? age = 5;
    uint32 n = age maybe 0;                     // fine
    uint32 m = age;                             // error: may be `none`

## E0532: `maybe` or `has value` on a value that is not optional

`maybe`, `has value` and `is empty` work on an optional value (`T?`)
only: they tell whether there is a value to take.

    uint32 n = 5;
    uint32 m = n maybe 0;                       // error: `n` is not optional

## E0533: `none` compared with `==`

An optional is not compared with `none`. Ask whether it has a value
with `has value`, or whether it is empty with `is empty`.

    if (age is empty) { }                       // instead of `age == none`
    if (age has value) { }                      // instead of `age != none`

## E0534: the fallback of `maybe` is more than one value

The fallback after `maybe` is a single value, or a parenthesised
expression. A binary operator after it would read as part of the
fallback, so the fallback needs parentheses to say what it is.

    uint32 n = (age maybe 0) + 1;               // fine
    uint32 m = age maybe (n * 2);               // fine
    uint32 k = age maybe 0 + 1;                 // error

## E0526: wrong type in a pattern

A binding in a variant pattern must name exactly the type of the
value it binds (no conversion is applied in patterns), and must
bind as many values as the variant has.

    case Rect(string w, float64 h) { }            // error: `w` is a float64

## E0501: no constructor accepts these values

A struct with constructors was built from values that none of them takes:
the count is wrong, a name matches no parameter, or the types do not
convert. The labels list every constructor's parameters.

    Circle(float64 radius) { ... }
    Circle c = "five";          // error: no constructor takes a string

## E0502: more than one constructor fits equally well

Two constructors could take the values and neither is a better fit: for
example `(float64)` and `(uint32)` both take a `uint8` without losing
anything. Convert the value yourself so only one fits:

    Port p = 5::to<uint16>();

## E0503: two constructors take the same types

Two constructors of one struct have the same parameter types, so a call
could not tell them apart. Change the parameter types of one of them.

## E0504: constructor leaves a field without a value

A constructor returns before every field of the struct has a value. Each
field must be given one on every path to the end of the constructor:

    Player(string n) { self.name = n; }   // error: hp has no value

## E0505: fill leaves a field without a value

A `fill` block ends before every field has a value. Give the field a value
in the block, or end with `fill partial` and complete the value later.

## E0506: fill of a value that already has one

`fill` gives the fields of a value that has no value yet. A value that is
complete already cannot be filled; declare it without a value first
(`Player p;`), or change its fields with plain assignments if it is `mut`.

## E0507: value used before it is complete

A struct being filled, or a constructor's `self`, can only have its fields
given values until every field has one. Reading it, borrowing it or calling
a method on it is an error. A field that is already set can be read with
`now`: `now p.hp * 2`.

## E0508: no such def

A `def` name in `with NAME`, `from NAME` or `defaults NAME` names no def for
the struct. Defs are written `def NAME of STRUCT { field = value; }`.

## E0509: field or def given twice

A field gets two values in one `def` or `fill`, or two defs have the same
name. Keep one of them.

## E0510: no default def

`fill ... with defaults` needs the struct to name its default def, with
`struct Player defaults PlayerStart { ... }` or `struct default Player { ... }`.

## E0540: not a failure type

Only a `failure` can be listed after `fails`. The name is either not a
type at all, or it is a struct or an enum.

    failure ParseError { Empty "nothing to parse" }
    uint32 parse(string s) fails ParseError { ... }

## E0541: failure not in the list

`fail X;` names a failure variant that none of the failure types in the
function's `fails` list has. Add the failure type to the list, or fail
with one of the variants it does have.

## E0542: fail in a function that does not say fails

`fail` ends a function with a failure, so the function has to say so:
write `fails ParseError` after the parameters.

    uint32 parse(string s) fails ParseError { fail Empty; }

## E0543: ambiguous failure variant

Two of the failure types in the `fails` list have a variant with this
name. Write the failure type too: `fail ParseError::Empty;`.

## E0544: a call that can fail is not handled

A call to a function that can fail has to say what happens when it
does. Fall back to a value with `maybe`, or pass the failure up with
`fails` (the caller must then list the failure type itself).

    uint32 n = parse(line) maybe 0;
    uint32 m = parse(line) fails;

With `unhandled_fail = "crash"` in `drachm.toml` the call is allowed:
if it fails, the program prints the failure's message and ends with
the failure's exit code.

## E0545: handler for a failure the call cannot raise

A `fails with (Type e)` handler names a failure that the call cannot
fail with. Handle one of the failures the function lists after `fails`.

## E0546: failure handled twice

Each failure type gets one handler after a call.

## E0547: handler gives no value

A handler for a call that returns a value has to give one with
`maybe v;`, or leave the function (`return`, `fail`) or the loop
(`break`, `continue`). Ignoring a failure (`fails with none`) is only
for calls that return nothing.

    uint32 n = parse(line) fails with (ParseError e) => { maybe 0; };

## E0548: failure not declared by the caller

`call fails` passes the failures of the call up to the caller, so the
caller has to list every one of them after its own `fails`.

## E0549: because needs a failure

`fail X because e;` records the failure `e` as the cause of `X`. The value
after `because` has to be a failure value.

## E0550: maybe outside a handler

`maybe v;` gives the value of the call whose failure a handler is
dealing with, so it can only be written inside a `fails with` block.

## E0551: unknown value in a failure message

A failure message can use the values of its variant with `${name}`. The
name is not one of the variant's values.

## E0552: exit code out of range

The exit code of a failure is a whole number from 0 to 255.

## E0553: catch-all handler out of place

`otherwise (any e)` and `fails with (any e)` handle every failure the
handlers before them did not, so they come last, and there is only one.

## E0554: failure list problem

A failure type is listed twice after `fails`, or a `failure` has no
variants at all.

## E0601: cannot tell what a type parameter is

A generic function's type parameter (`T`) is worked out from the
arguments of the call. When no argument shows it, or the call only
passes values with no type of their own that leave it open, write it
out between `<` and `>`.

    T make<T>() { ... }
    make<uint32>();                             // fine
    make();                                     // error: what is `T`?

## E0602: wrong number of type arguments

A type is given type arguments, but it takes none, or a different
number. `Point` is not generic, so `Point<uint32>` is an error; a
`Stack<T>` takes exactly one.

## E0603: type arguments disagree

One type parameter is used for several arguments, and they do not have
the same type. Convert one of them, or write the type out so the others
are converted to it.

    T max<T>(T a, T b) { ... }
    max(a_u8, a_string);                        // error: `T` is two types
    max<uint32>(a, b);                          // fine

## E0604: type arguments missing

A generic function or type was named without its type arguments where
a type or a value is needed. A generic function used as a value, or a
generic type written alone, has to say what its type parameters are.

## E0605: bad type parameter list

A type parameter is declared twice in the same list, or has the name of
a struct, enum or type (`Point`, `uint32`). A type parameter takes no
other name's place. A unit's name is only a warning (W0618).

## E0608: a generic keeps instantiating itself

A generic function calls itself with a bigger type each time
(`grow<uint32>` calls `grow<uint32?>`, which calls `grow<uint32??>`, ...),
so the compiler would never stop making copies. Make the recursive call
use the same type. The same happens to a generic struct that holds an
instance of itself with a bigger type (`struct Bad<T> { Bad<Cell<T>> x; }`).

## E0609: a unit or value parameter used as a type

`unit U` declares a unit, so it can only be used inside the `<>` of a
number type: `float64<U>`. A type parameter would be written `<T>`.

A value parameter (`uint32 N`) stands for a number, so it can be used
as a value in the body and as an array length (`T[N]`), not as a type.

## E0606: beyond the constraint

Inside a generic function or struct whose type parameter has a
constraint (`T with Shape`), a value of type `T` can only do what the
interface promises: call its methods, read its required fields, call
its `static` methods as `T::zero()`. Anything else, such as another
method or an operator the interface does not declare, is an error in
the definition, even if every type used so far happens to allow it.

    float64 total<T with Shape>(T a) { return a::perimeter(); }   // error

## E0607: constraint not met

A generic function or struct was used with a type that does not
implement the interface its type parameter asks for (`T with Shape`).
A struct implements an interface by naming it: `struct Triangle with
Shape`, and giving it the methods the interface lists.

## E0610: interface method missing

A struct says `with Shape`, but has no public method for one of the
methods `Shape` lists. A method under `private:` does not count: an
interface's methods are public.

## E0611: interface method differs

A struct's method has the name of an interface method, but its
parameters, its result or its `static` do not match. `Self` in the
interface stands for the struct that implements it.

## E0612: changes more than the interface allows

A struct method changes `self`, but the interface method it
implements does not say `changes self`, so callers holding a shared
borrow would be promised something the method breaks. Either add
`changes self` to the interface method, or make the struct method stop
changing `self`. (A method that writes only `mut` fields counts as
changing.)

## E0613: changes self out of place

`changes self` is only written after an interface method's
parameters. A struct's methods and a function's mode are worked out
from their bodies.

## E0614: required field missing

An interface lists a field (`float64 x;`), so a struct that implements
it needs a `public` field of that name and type.

## E0615: interfaces clash

An interface extends itself (directly or through others), or extends
two interfaces that have a member of the same name with different
signatures, or lists the same member twice.

## E0616: Self out of place

`Self` stands for the implementing type, so it only means something
inside an interface. In a struct, write the struct's name.

## E0617: interface used as a value type

An interface is only ever borrowed: a value, a field, a list item or a
type argument cannot be `Shape` itself, because it could be any struct
that implements it. Write `&Shape`, or use an enum for a closed set of
choices.

## E0619: not callable through a borrow of an interface

A `&Shape` stands for any struct that implements `Shape`, and the
compiler does not know which one. A `static` method needs a type, not
a value, and a method that mentions `Self` (or is an operator) has
types that depend on the struct. Call these on the struct itself, or
in a function with a constraint (`<T with Shape>`).

## E0650: function parameters do not match

A callable value's parameter types and their order must match the function type.

## E0651: function result does not match

A callable value must give the result specified by its function type.

## E0652: function value would escape

Function values can be local bindings or passed to a function. They cannot be stored in another value or returned.

## E0654: value is not callable

Only functions and values with a function type can be called.

## E0640: no conversion to a borrow of an interface

A `&Shape` is wanted, but the borrow given is of a struct that does not
implement `Shape` (or of an interface that does not extend it). Add
`with Shape` to the struct, or `extends Shape` to the interface.

## E0620: index outside an array

An array has a fixed length, so an index written as a number can be
checked while the program is compiled. Valid indexes run from 0 to one
less than the length, or from -1 (the last item) back to minus the
length.

    uint8[4] rgba = [255, 0, 0, 255];
    uint8 a = rgba[5];                          // error: only 0 to 3

## E0622: an item cannot be moved out of a list by index

Reading `names[0]` where the item owns memory (a `string`, a struct
holding one) would leave a hole in the list. Borrow the item with
`&names[0]`, copy it with `copy names[0]`, or call a method on it.

## E0623: a list literal mixes types

The items of `[a, b, c]` must all have one type. When the list is not
given a type, the first item decides it.

## E0624: an empty list or map has no type

`[]` holds nothing to take a type from, so something has to say what the
list holds: `string[?] xs = [];`. A map literal `{}` needs a type the same
way: `Map<string, uint32> ages = {};`.

## E0625: wrong number of items for an array

An array has a fixed length, and its literal must give exactly that many
items: `uint8[4] a = [1, 2, 3, 4];`.

## E0626: this type cannot be a map key

A map finds an entry by hashing its key, so a key must be hashable: a whole
number, a float (compared by its bits), `bool` or `string`, or a struct,
enum, optional or array made only of those. Lists, maps, borrows and
pointers cannot be keys, and neither can a struct or enum that holds one:
give the struct a `key()` method that returns something hashable.

## E0627: a key type has `==` but no `key()`

A map hashes a struct field by field, and compares keys the same way. A
struct with its own `==` method may call two values equal that differ field
by field, so the fields would give equal keys different hashes. Give the
struct a `key()` method returning what makes two of them equal; the map then
hashes and compares that result instead, and `==` stays whatever it is.

## E0628: a `key()` method has the wrong shape

`key()` tells a map what to hash a struct by. It must take no parameters,
must not be `static`, must not fail, and must return a value a map can
hash: `string key() { return self.name; }`.

## E0629: a map literal gives the same key twice

Each key of a map literal is stored once: a second entry with the same
constant key would replace the first, so the literal would not say what it
means. Remove one of the two entries.

## E0630: the loop variable's type is not the items' type

A loop over a list or array gives each item to the variable, so the
variable must have the items' type: `for (uint32 n of xs)` over a
`uint32[?]`. To look at each item write `&uint32 n`; no conversion
happens inside a `for`.

## E0631: not iterable over this type

A `for` over a value of your own type needs the type to say `with
Iterable<T>` for the type `T` of the loop variable. A type that is
`Iterable` over several types (`Iterable<Node> and Iterable<Edge>`) is
walked once for each: the variable's type picks which. `&T` looks at
each item; `&mut T` is not available, because `at` hands out values.

## E0633: address compare of a value with no single address

`same as` and `containsRef` compare where values are, so their items must
be borrows (or lists, arrays or optionals of borrows). An owned value
such as `string` or a number has no address to compare: borrow both sides
with `&` to compare where they are, or use `==` or `contains` to compare the
values. A map, or a struct or enum that holds a borrow, has no single
address either: compare the borrows inside it one at a time.

    &a same as &b            // the same place
    a == b                  // the same text or number

## E0632: same method name, several results

A struct may have two methods with the same name and parameters when
only their results differ, so that it can be `Iterable` over two types
(`Node? at(uint64 i)` and `Edge? at(uint64 i)`). Calling `g::at(0)` by
name cannot choose between them; walk the value with `for` instead.

## E0634: a value argument is not a constant

A value parameter (`uint32 N`) is given its value when the program is
compiled, so the argument must be known then: a whole number, a
calculation on whole numbers, a named constant (a top-level value with
a constant initializer, `uint32 SIZE = 8;`) or a value parameter of the
generic that is being written. A variable or anything else that only
has a value while the program runs cannot be used.

    struct Buffer<T, uint32 N> { T[N] items; }
    Buffer<sint32, 16> a;                       // fine
    Buffer<sint32, 2 * 8> b;                    // fine
    Buffer<sint32, n> c;                        // error: `n` is not a constant

## E0635: a value does not fit its parameter

A value parameter has an integer type, and its argument must fit that
type: `uint8 N` takes 0 to 255, and no value parameter takes a negative
number unless its type is a `sint`.

    struct Mod<uint8 P> { }
    Mod<300> m;                                 // error: 300 does not fit `uint8`

## E0636: an array length of zero

An array holds at least one item, so a length parameter used as an array
length (`T[N]`) cannot be zero. Use a list (`T[?]`) for something that may
be empty.

## E0637: a fixed value is not a constant

A top-level value written with `fixed` must be a constant: its value is
worked out when the program is compiled, from numbers, other constants
and arithmetic. A function call, input or anything else that only has a
value while the program runs is an error. Without `fixed` a constant
value is still usable in types; `fixed` only makes the compiler check it.

    uint32 SIZE = 8;
    fixed uint32 MAX = SIZE * 2;               // fine: 16
    fixed uint32 N = io::inln().to<uint32>();  // error: known only when running

## E0638: constant defined twice

Two top-level values have the same name.

## W0600: generic never used

A generic function, struct or enum is only checked when something uses
it with particular types, so one nothing uses is not checked at all.

## W0618: type parameter named like a unit

A type parameter has the name of a unit, such as `K` (kelvin) or `N`
(newton) after `in si;`. Inside the parameter's scope the name is the
type, so the parameter works as written. The unit is still there: `X as
unit` names it, even inside the scope, as in `float64<K as unit>`.
Rename the parameter to remove the warning.

    in si;
    K max<K>(K a, K b) { ... }        // W0618: K is the type here
    float64<K as unit> t = 1.0<K>;   // the unit K

## E0800: project file is not valid

A project file cannot be read, or is not valid TOML: `drachm.toml`,
`srachm.toml` and `drachm.lock` are TOML files. The label points at
the first thing the TOML reader could not parse.

## E0801: unknown setting

A key or table in `drachm.toml` or `srachm.toml` is not one the
compiler knows, so it would do nothing. The sections are `[project]`,
`[dependencies]`, `[build]`, `[warnings]` and `[behaviour]`. A close
spelling is suggested.

## E0802: bad setting value

A setting has a value it does not accept: a word that is not one of
the choices (`overflow = "clamp"`), text where a number or a list
goes, a `kind` other than `program` or `library`, or an `entry` that is
not a `.drh` file inside the project.

## E0803: required setting missing

A setting that must be there is not: `[project] name` in a project, a
git dependency's `tag`, or the `path` or `git` of a dependency.

    [project]
    name = "mygame"

## E0804: bad dependency

A `[dependencies]` entry is not usable: its name is not a plain name,
it is not a table, it mixes `path` and `git`, it has a key that is
not `path`, `git`, `tag` or `version`, or two different packages of the
build would share one folder in `deps/`.

    sdl = { git = "https://example.org/sdl.git", tag = "v0.4" }
    utils = { path = "../utils" }

## E0805: invalid project name

The project's name is part of every path that starts at the source
folder, so it must be a plain name: letters, digits and `_`, starting
with a letter or `_`. `2fast` and `my-game` are not names.

## E0806: behaviour set in a file flag

A `//--!$` file flag sets a behaviour that must be the same across the
whole project: `missing_key`, `unhandled_fail` or `import_clash`. Set
those in `[behaviour]` of `drachm.toml`. Only `overflow`, `bounds` and
`unsigned_negate` may differ from file to file.

## E0807: project and script config both here

A folder has both `drachm.toml` (a project) and `srachm.toml` (a
script). A folder is one or the other. Keep the one that fits.

## E0808: cannot fetch a git dependency

A `git` dependency could not be fetched: `git` is not installed, the
address is wrong, or the tag does not exist. Dependencies are cloned
into `deps/NAME@VERSION` in the project folder. The message shows what git said.

## E0809: git tag moved

A git dependency is built from the commit `drachm.lock` records, even when
its tag moves later. When that commit is not in `deps/`, it has to be
fetched from the tag, and the tag now names another commit: the compiler
refuses rather than build something the lock file did not record. To take
the new commit, run `drachm update NAME`.

## E0810: dependency cycle

A dependency is part of a cycle: it depends on a project that depends on
it, directly or through other
dependencies. A project's dependencies must form a tree.

## E0811: library has no program

`drachm run` and `drachm build` make a program, and a project with
`kind = "library"` has no program to make: its code is used by other
projects. Change `kind` to `"program"` to build it, or run a program
project instead.

## E0812: entry file not found

The project's `entry` (`src/main.drh` by default) is not a file. Create
it, or change `entry` in `[project]`.

## E0813: cannot make a new project

`drachm new NAME` needs NAME to be a plain name and a folder that does
not exist yet in the current folder.

## E0814: no drachm.toml here

A project folder is given to `build`, `run` or `check`, or the current
folder is used by default, and it has no `drachm.toml`. A project is a
folder with a `drachm.toml` and the program in `src/main.drh`; `drachm new`
makes one.

## E0815: no such version

The tag or range of a git dependency matches nothing in the repository:
the tag does not exist, a `^` range matches no tag that reads as a
version (`v1.2.3`, `1.2`), or the repository has no version tags at all.
`git ls-remote --tags URL` lists the tags. `drachm add URL` with no
version takes the newest version tag.

## E0816: package differs from drachm.lock

A package in `deps/` does not hold what `drachm.lock` recorded: the sha256
hash over its files differs (a file was edited, added or deleted), or a
fresh fetch of the locked commit gave other contents. The hash covers every
file except `.git`: each file's path, size and bytes, in path order.
`drachm install` throws `deps/` away and fetches it again from the lock.
If you edited the lock file by hand, restore it: it is written by drachm.

## E0817: cannot change the dependencies

`drachm add`, `remove`, `update` or `install` could not do what was asked:
the folder has no `drachm.toml`, the dependency named is not in
`[dependencies]`, the address is not one `git` can use, or the file could
not be written. Nothing is changed when this happens.

## E0700: libclang cannot be loaded

`in * from c."header.h" as name;` reads the header with libclang, the C
parser of the LLVM project, which is loaded when a C import is checked.
It was not found. Install the Clang libraries (`libclang`), or set
`LIBCLANG_PATH` to the folder that holds `libclang.so` (`libclang.dylib`,
`libclang.dll`). Programs without C imports never need it.

## E0701: C header not found

The header named in `in * from c."header.h" as name;` is not on the
include path. Headers are found like a C compiler finds them: the system
folders, then the `include` folders of the `[native]` table in `drachm.toml`.
Install the library's development package, or add its header folder:

    [native]
    include = ["vendor/include"]

## E0702: C header has errors

libclang found the header but could not read it without errors: a type it
needs is missing, a definition has to come first, or a setting differs
from the one the library expects. The first messages are shown. Add the
library's `defines` and `include` folders to the `[native]` table.

## E0703: no such name in the C header

The name after `alias::` is not declared by the header or by any header it
includes. C names are written exactly as the header spells them
(`sdl::SDL_Init`). The message suggests a similar name when there is one.

## E0704: C types are their own types

The C number types (`c::int`, `c::long`, `c::size_t`, `c::char`, ...) are
types of their own, not synonyms of `sint32` or `uint64`. A value of one
does not become another number type by itself; convert it explicitly:
`n as sint32` or `n::to<sint32>()`. A plain constant (`5`, `FLAG_A`) fits any of them.

## E0705: a match on a C enum needs a catch-all

C code can send a value the header does not name, so a `match` on a C enum
type (for example `fx::fx_color`) always needs a `case _`, even when every
named value has a case:

    match (color) {
        case sdl::RED { }
        case _ { }
    }

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

