---
title: Types and numbers
description: Built-in types, bindings, number literals, conversions, operators and the text of values.
---

# Types and numbers

## Built-in types

Whole numbers come in two families, `uint8` to `uint128` (zero and up) and
`sint8` to `sint128` (negative too), and decimals are `float32` and `float64`.
`bool` is `true` or `false`, `string` (also written `str`) is text, and
`none` is the type of a function that returns nothing. Every binding and
parameter has its type written first.

A binding cannot change after it has its value, unless it is declared `mut`.
It may also be declared without a value, as long as every path assigns it
exactly once before it is read. Reading it too early is error E0207.

```drachm
uint32 answer = 42;
mut sint8 cold = -12;

uint32 later;
if (answer > 40) {
    later = 1;
} else {
    later = 2;
}
```

<!-- example: mutable -->

## Enum types

An `enum` is a type with a name of its own, a value that is exactly one of its
variants. See [Enums](reference-enums.md).

## Generic types

A type name followed by `<` is read as a unit only after a number type
(`float64<m/s>`); after any other name it opens type arguments
(`Map<string, uint32>`), and a name that takes none is E0602. `[?]` and `[N]`
after a type write a list and an array. Type arguments work for generic
functions, structs and enums today, and `Optional<T>` is `T?` spelled out; see
[Generics](reference-generics.md).

## Optional types

`T?` is a `T` that may be missing: a value of type `T`, or `none`. See
[Optionals](reference-enums.md#optionals).

## Function types

`(uint32) => uint32` takes one `uint32` and returns another.
`() => none` takes no arguments and gives no result. See
[Function types and lambdas](reference-generics.md#function-types-and-lambdas).

## Borrow types

`&T` is a shared borrow of a `T`: it reads. `&mut T` is a unique borrow: it
reads and changes. A `&T` copies by itself, since it only points; a `&mut T`
moves, like a `string`. There is no borrow of a borrow, so a `&string` parameter
is passed on as it is.

A binding can hold a borrow: `&string first = &name;`. Put `mut` in front
(`mut &string cur = &a;`) to point it somewhere else later with `cur = &b;`.
A borrow lasts until its last use. The rules, and the errors E0302 to E0307 for
breaking them, are on the [Moves and copies](reference-ownership.md#borrowing)
page.

## Pointer types

`*T` is a raw pointer that can be written through, and `*fixed T` one that can
only be read. Using either needs the file flag and `unsafe { }` blocks; see
[Unsafe and pointers](reference-unsafe.md).

## Number literals

Decimal numbers may use `_` to group digits (`1_000_000`) and an exponent
(`1e21`). Other bases are `0xff` for hex, `0b101` for binary and `0o17` for octal.
A suffix picks the type: `255u8`, `-5s8`, `2.5f32`. The short forms `u`, `s`
and `f` mean `uint32`, `sint32` and `float64`.

A number in the source has no type until it reaches one, and arithmetic on
constants is exact. That is why `0.1 + 0.2` is `0.3` and `2 ** 100` can be
written without overflow, as long as the final value fits where it is stored.

<!-- example: literals -->

## Conversions

A number converts to a type that holds all its values with no warning, such as
a `uint8` into a `uint32`. In `a + b` the right side converts to the type
of `a`, so the left side leads.

A conversion that may lose value is warning W0102. The value wraps around, so
`1000` in a `uint8` becomes `232`. Writing `value::to<T>()` states that you meant
it and silences the warning. A float turned into a whole number drops its
fraction first, and `bool` never converts to or from a number.

<!-- example: conversions -->

## Operators

Arithmetic is `+ - * / % **`. Division of whole numbers drops the fraction,
overflow wraps around (255 plus 1 in a `uint8` is 0), and dividing by zero
panics at run time with exit code 101. A division by zero that the compiler can
see in constants is an error (E0107).

Comparisons are `== != < <= > >=`, logic is `&& || !` with short-circuiting,
and bits are `& | ^ ~ << >>`. A bit operator next to an arithmetic or
comparison operator needs parentheses: `((n & 1) == 0)`. `++`, `--` and the
compound forms like `+=` change a `mut` binding.

Take care with `::` and `->`. They bind tighter than any operator, so
`7 / 2::text()` divides 7 by a string. Write `(7 / 2)::text()`.

<!-- example: operators -->

## Text of a value

`::text()` turns a number or a `bool` into a `string`. Floats always keep a
decimal point, so `1e21` prints as `1000000000000000000000.0`. To round to a
number of places, use `io::outf` with `%.N` (see [Strings](reference-strings.md)).

```drachm
io::outln((10 / 4)::text());     // 2
io::outln(true::text());         // true
```

## Units

A number type can carry a unit, `float64<m/s>`, and the compiler checks that
units match. See [Units of measure](reference-units.md).
