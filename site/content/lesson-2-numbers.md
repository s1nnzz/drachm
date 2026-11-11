---
title: "Lesson 2: Numbers and variables"
description: Typed bindings, mut, whole and decimal numbers, bools, conversions, and printing numbers.
---

# Lesson 2: Numbers and variables

**Goal:** store values in named bindings, do arithmetic, and print the results.

## A binding has a type

A binding gives a value a name. In Drachm the type comes first, then the name,
then `=` and the value. Nothing is guessed for you, so you always see what a
name holds.

<!-- focus: 4 -->
`uint32` is an unsigned whole number that fits in 32 bits, so it holds 0 up to
about four billion. Sizes run from 8 to 128 bits, and `uint` becomes `sint`
when the number may be negative. The value on the right can be a calculation:
`40 + 2` is worked out before it is stored.

<!-- focus: 5 -->
`sint8` is a signed 8-bit number, which holds -128 to 127. The compiler checks
that the value fits the type.

<!-- focus: 6 -->
`float64` holds decimal numbers, with 64 bits of precision. `float32` is the
smaller one.

```drachm title="numbers.drh"
in io;

none main() {
    uint32 answer = 40 + 2;
    sint8 cold = -12;
    float64 pi = 3.14159;
}
```

## Printing a number

`io::outln` prints text, and a number is not text. The `::text()` call turns a
number (or a `bool`) into text. Try printing `answer` without it and the
compiler tells you what to write instead.

```drachm
io::outln(answer);
```

```text title="drachm check"
error[E0101]: `io::outln` prints text, found `uint32`
  --> main.drh:5:15
   |
 5 |     io::outln(answer);
   |               ^^^^^^ this is `uint32`
   |
 help: turn it into text: `io::outln(answer::text())`

main.drh: 1 error
```

## Bindings do not change unless you ask

Once a binding has a value, it keeps it. If you want to change it later, put
`mut` (short for mutable) in front of the type. The compiler explains
what you have to add when you forget.

This is how most mistakes look in Drachm: one message, the line it is about,
and a fix you can copy. The same program with `mut uint32 hp = 3;` works.

```drachm
uint32 hp = 3;
hp = 2;
```

```text title="drachm check"
error[E0205]: cannot change `hp`: it is not `mut`
  --> main.drh:5:5
   |
 4 |     uint32 hp = 3;
   |            -- declared here without `mut`
 5 |     hp = 2;
   |     ^^ assigned a second time here
   |
 help: declare it as `mut uint32 hp` to allow changes
```

## Arithmetic

The usual operators work: `+ - * /`, `%` for the remainder, and `**` for
powers. Dividing two whole numbers gives a whole number and drops the fraction,
so `42 / 4` is `10`. If you want `10.5`, use a decimal type.

Wrap a calculation in parentheses before calling `::text()` on it. The `::`
binds tighter than any operator, so `42 / 4::text()` would try to divide by
text. Run this program and read the output line by line.

<!-- example: operators -->

## Constants are exact

A number written in your source has no type yet, and arithmetic on such
constants is exact. That is why `0.1 + 0.2` prints `0.3` here, and why
`2 ** 100` does not overflow: the result only has to fit when it is stored in
a binding.

<!-- example: literals -->

## Mixing sizes

A smaller number goes into a bigger type by itself. Going the other way can
lose value, so the compiler warns you. A warning does not stop the program, but
it tells you exactly how to say that you meant it: `::to<T>()`. With it the
value wraps around, and `1000` becomes `232` in a `uint8`.

<!-- example: conversions -->

```text title="drachm check, without the to<uint8>() call"
warning[W0102]: converting `sint64` to `uint8` can lose information
  --> main.drh:5:19
   |
 5 |     uint8 small = big;
   |                   ^^^ this is `sint64`
   |
 note: `sint64` holds -9223372036854775808 to 9223372036854775807; `uint8` holds 0 to 255; values outside that wrap around
 help: write `big::to<uint8>()` to say the conversion is intended
```

### Your turn: the area of a rectangle

Store a width of 7 and a height of 6 in two `uint32` bindings, multiply them
into a third called `area`, and print `Area: 42`. Use `io::out` for the label
and `io::outln` for the number, as in lesson 1.

```drachm title="starter"
in io;

none main() {
    // your code here
}
```

```text title="expected output"
Area: 42
```

<!-- solution-of: solution-2-1 -->
```drachm solution
in io;

none main() {
    uint32 width = 7;
    uint32 height = 6;
    uint32 area = width * height;
    io::out("Area: ");
    io::outln(area::text());
}
```

### Your turn: Celsius to Fahrenheit

Convert 25 degrees Celsius with `fahrenheit = celsius * 9 / 5 + 32`. Use
`float64` for both bindings, and write the numbers with a decimal point
(`9.0`) so that every value is a decimal. Print the result.

```drachm title="starter"
in io;

none main() {
    float64 celsius = 25.0;
    // your code here
}
```

```text title="expected output"
77.0
```

<!-- solution-of: solution-2-2 -->
```drachm solution
in io;

none main() {
    float64 celsius = 25.0;
    float64 fahrenheit = celsius * 9.0 / 5.0 + 32.0;
    io::outln(fahrenheit::text());
}
```

Next: [Lesson 3: Decisions and loops](lesson-3-decisions-and-loops.md).
