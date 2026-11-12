---
title: Numbers without surprises
description: Mixing number types, the left-side rule, lossy conversions, wrapping, exact constants and how decimals print.
---

# Numbers without surprises

Numbers are where programs quietly go wrong. A whole number overflows, a
decimal is cut off, or a calculation runs in a type you did not expect. Drachm
cannot stop you from writing such code, but it makes each of these moments
visible: it either does the obvious thing, or it warns you and shows how to
say what you meant. This guide covers those rules in the order you will meet
them. [Lesson 2](lesson-2-numbers.md) has the basics, and
[Types](reference-types.md) is the reference.

## The left side leads

When two numbers of different types meet in `a + b`, `a < b` or any other
two-sided operator, the **right side converts to the type of the left side**,
and the calculation is done in that type. Nothing else is guessed. The rule
has no exceptions for decimals, which is the part that surprises people.

With a `uint32` called `hp`, `hp * 1.5` is a whole-number calculation. The
`1.5` is turned into a `uint32` first, so it becomes `1`, and the compiler
warns you that it dropped the fraction:

```drachm
uint32 hp = 10;
uint32 a = hp * 1.5;
```

```text title="drachm check"
warning[W0102]: `1.5` does not fit `uint32`
  --> main.drh:5:21
   |
 5 |     uint32 a = hp * 1.5;
   |                     ^^^ becomes 1
   |
 note: `uint32` holds only whole numbers, from 0 to 4294967295; the fraction is dropped
 help: use a value that fits or a bigger type; `::to<uint32>()` converts without this warning
```

`a` is 10, not 15. The warning is the compiler telling you that the left side
led. To get what you wanted, put the decimal on the left, or convert `hp`
first.

<!-- focus: 7 -->
A decimal on the left leads, so the product is a `float64`.

<!-- focus: 11 -->
Or convert the whole-number variable first, and the rest of the line is a
decimal calculation.

<!-- focus: 15 -->
If you want a whole number back, ask for it with `::to<uint32>()`. The
fraction is dropped.

<!-- focus: 19 -->
You can also stay with whole numbers: `hp * 3 / 2` multiplies first, so no
fraction is lost until the very end.

<!-- focus: 23-24 -->
A constant on the left takes the type of the variable on the right if it fits
there. `300` does not fit a `uint8`, so it keeps a type of its own and leads
the sum.

<!-- example: guide-numbers-left -->

One more thing to notice: `(hp * 1.5)::to<uint8>()` still warns. The
`::to<uint8>()` applies to the *result*, and the `1.5` was already cut down
before that. Rewrite the calculation, then convert.

## Lossless and lossy conversions

A number moves between types automatically, but not always silently. The
compiler asks one question: can the target type hold **every** value of the
source type? If so, the conversion is **lossless** and nothing is printed.
If not, it is **lossy**, and you get warning W0102.

Lossless means:

- a whole number to a whole-number type that covers its whole range:
  `uint8` to `uint32`, `uint8` to `sint16`, `sint32` to `sint64`;
- a whole number to a decimal type that holds every such value exactly:
  up to 16 bits into `float32`, up to 32 bits into `float64`;
- `float32` to `float64`.

Everything else is lossy: a narrower type, a sign change, a decimal to a whole
number, or a wide integer into a decimal. This program converts six times, and
every one warns. Note how the `note:` line says what each type can hold, so you
can see which values would be lost.

```drachm
sint64 big = 3_000_000_000;
sint32 narrow = big;
uint8 u = narrow;
float64 f = 3.99;
uint32 t = f;
uint16 h = 7;
float32 ok = h;               // lossless: no warning
uint32 m = 7;
float32 lossy = m;
float64 fine = m;            // lossless: no warning
uint64 u64 = 5;
float64 lossy2 = u64;
sint32 neg = -5;
uint32 u32 = neg;
```

```text title="drachm check"
warning[W0102]: converting `sint64` to `sint32` can lose information
  --> main.drh:5:21
   |
 5 |     sint32 narrow = big;
   |                     ^^^ this is `sint64`
   |
 note: `sint64` holds -9223372036854775808 to 9223372036854775807; `sint32` holds -2147483648 to 2147483647; values outside that wrap around
 help: write `big::to<sint32>()` to say the conversion is intended

warning[W0102]: converting `float64` to `uint32` can lose information
  --> main.drh:8:16
   |
 8 |     uint32 t = f;
   |                ^ this is `float64`
   |
 note: `uint32` holds only whole numbers from 0 to 4294967295; the fraction is dropped and values outside that wrap around
 help: write `f::to<uint32>()` to say the conversion is intended

warning[W0102]: converting `uint32` to `float32` can lose information
   --> main.drh:12:21
    |
 12 |     float32 lossy = m;
    |                     ^ this is `uint32`
    |
 note: `float32` holds whole numbers exactly only up to 16777216; `uint32` goes up to 4294967295; larger values are rounded
 help: write `m::to<float32>()` to say the conversion is intended

warning[W0102]: converting `sint32` to `uint32` can lose information
   --> main.drh:17:18
    |
 17 |     uint32 u32 = neg;
    |                  ^^^ this is `sint32`
    |
 note: `sint32` holds -2147483648 to 2147483647; `uint32` holds 0 to 4294967295; values outside that wrap around
 help: write `neg::to<uint32>()` to say the conversion is intended
```

Only four of the six warnings are shown above; the other two (`sint32` to
`uint8`, and `uint64` to `float64`) look the same. A warning does not stop the
build, but treat it as a question: *did you mean to lose this?*

## Saying "I meant it" with ::to

`value::to<T>()` converts a value to the number type `T` and never warns,
because you asked. Use it when losing information is the point: cutting a
decimal to a whole number, packing a value into a byte, or reading the low
bits of a larger number.

The values that do not fit follow the same rule as everywhere else, which is
the next section. For decimals going to whole numbers the **fraction is
dropped first** (toward zero), so `3.99` becomes `3`. It does not round.

## Overflow wraps around

Arithmetic that goes past either end of a type **wraps**. A `uint8` has 256
values, so 250 plus 10 is 260, which is 4. A `sint8` after 127 goes to -128, and
a `uint32` below 0 goes to the top. The same holds for `::to<T>()`: whole
numbers keep their low bits.

Two details matter. The sum is made in the type of the **left side**, and only
then widened if it goes into a wider variable. `big + big` with two `uint8`
values of 200 is `144`, even when it is stored in a `uint32`. To get 400,
convert one side first.

<!-- focus: 5-9 -->
A `uint8` counter pushed past its end ends up at 4 after ten steps.

<!-- focus: 11-16 -->
A signed number wraps to the other end, and a `uint32` going below zero goes to
4294967295.

<!-- focus: 18-23 -->
The left side leads here too. `wrapped` is 144, while `wide` converts first.

<!-- focus: 25-29 -->
`::to<T>()` on a value that does not fit keeps the low bits, and `3.99` loses
its fraction.

<!-- example: guide-numbers-wrap -->

Dividing a whole number by zero is different: it stops the program with a
message on stderr and exit code 101.

```text title="uint32 zero = 0; 5 / zero"
drachm: panic: division by zero
```

If the zero is a constant written in the source, it is caught earlier. See
[the constants section](#exact-constants) below.

### Choosing another overflow behaviour
<!-- status: not-implemented -->

The language design lets a project choose what happens on overflow in a config
file: `overflow = "wrap"` (the default), `"saturate"` (clamp to the nearest
end) or `"panic"` (stop with exit code 101). The compiler does not read that
setting yet, so every build wraps. See [Configuration](design-config.md).

## Constants that do not fit

A number written in your source is a **constant**, and it has no type yet.
When it lands in a typed place, the compiler checks that it fits. If it does
not, you get the same W0102, but with the value it **becomes** shown under the
arrow. That is the line to read: it says exactly what your program will hold.

```drachm
uint8 a = 300;
uint32 x = -1;
uint32 n = 2.5;
sint8 s = 200;
```

```text title="drachm check"
warning[W0102]: `300` does not fit `uint8`
  --> main.drh:4:15
   |
 4 |     uint8 a = 300;
   |               ^^^ becomes 44
   |
 note: `uint8` holds 0 to 255; a constant outside that wraps around
 help: use a value that fits or a bigger type; `::to<uint8>()` converts without this warning

warning[W0102]: `-1` does not fit `uint32`
  --> main.drh:5:16
   |
 5 |     uint32 x = -1;
   |                ^^ becomes 4294967295
   |
 note: `uint32` holds 0 to 4294967295; a constant outside that wraps around
 help: use a value that fits or a bigger type; `::to<uint32>()` converts without this warning

warning[W0102]: `2.5` does not fit `uint32`
  --> main.drh:6:16
   |
 6 |     uint32 n = 2.5;
   |                ^^^ becomes 2
   |
 note: `uint32` holds only whole numbers, from 0 to 4294967295; the fraction is dropped
 help: use a value that fits or a bigger type; `::to<uint32>()` converts without this warning

warning[W0102]: `200` does not fit `sint8`
  --> main.drh:7:15
   |
 7 |     sint8 s = 200;
   |               ^^^ becomes -56
   |
 note: `sint8` holds -128 to 127; a constant outside that wraps around
 help: use a value that fits or a bigger type; `::to<sint8>()` converts without this warning
```

Most of the time the fix is to pick a bigger type, or a value that fits.
Shifting a constant by too much is a close relative, with its own warning:
`1u32 << 40` shifts by 8, because the amount wraps around the 32-bit width
(W0106).

## Exact constants

Arithmetic on constants is done exactly, with no limit on size, and only the
final result has to fit where it is stored. So `2 ** 100` does not overflow,
`2 ** 100 / 2 ** 95` is exactly 32, and `0.1 + 0.2` is exactly `0.3`. The
comparison `0.1 + 0.2 == 0.3` is true for constants.

Once the numbers are in `float64` variables, they are ordinary binary decimals,
and `a + b == 0.3` is false. That is how computers store decimals, and it is
why you rarely compare decimals with `==`.

A constant divided by a constant zero is an error at compile time, for whole
numbers and decimals alike (E0107):

```text title="drachm check, with uint32 a = 5 / 0;"
error[E0107]: this constant expression divides by zero
  --> main.drh:4:16
   |
 4 |     uint32 a = 5 / 0;
   |                ^^^^^
   |                    - this is zero

main.drh: 1 error
```

<!-- example: guide-numbers-exact -->

## How decimals print

`::text()` on a decimal prints the **shortest digits that read back as the same
number**, always with a decimal point and never in exponent form. So `1.0`
prints `1.0`, a third prints `0.3333333333333333`, and `1e21` prints as a one
followed by twenty-one zeros and `.0`.

<!-- focus: 5 -->
Constants are exact, so the sum is exactly `0.3`.

<!-- focus: 8-10 -->
The same sum in variables carries the usual binary error: `0.30000000000000004`.

<!-- focus: 19-22 -->
A `float32` has fewer digits than a `float64`. Widening one to `float64` shows
what it really stored, `0.10000000149011612`.

<!-- focus: 25-27 -->
Dividing whole numbers drops the fraction (`42 / 4` is `10`), so divide
decimals when you want one.

<!-- focus: 30 -->
For display, `io::outf` rounds: `%.2` is two decimal places, `%8.3` is a width
of 8 with three places, and `%-8.1` pads on the right.

<!-- focus: 33-35 -->
Decimal division by zero does not stop the program. It follows the usual
standard and gives `inf` or `nan`.

<!-- example: guide-numbers-floats -->

## A checklist

- Put the type you want the answer in on the **left**.
- Read every W0102. It tells you what the value will become.
- Use `::to<T>()` only when losing information is on purpose.
- Remember that whole numbers wrap and that decimals keep their fraction until
  you convert them.
- Print money, measurements and results with `io::outf` formats, not with long
  raw decimals.

Next: [Units of measure in practice](guide-units.md).
