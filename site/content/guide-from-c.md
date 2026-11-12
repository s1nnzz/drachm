---
title: Coming from C or C++
description: What the compiler does for you that you did by hand, how ownership replaces malloc and free, and what unsafe and raw pointers still give you.
---

# Coming from C or C++

Drachm compiles to native code and aims for C's speed, so a lot will feel
familiar: fixed-size integers, structs, functions that return their type first,
a `main`, and a compiler that does not hide what the machine does. What
differs is who looks after the dangerous parts. This guide lists what you stop
doing by hand, and what you keep when you really want it.

## What you no longer do by hand

| In C or C++ | In Drachm |
|---|---|
| `malloc`, `free`, `new`, `delete` | a `string` frees itself when its owner goes out of scope |
| matching every early `return` with cleanup | `drop()` runs on every path out of a scope |
| `if (p == NULL)` before using a pointer | there is no null reference; a pointer that may be null is raw and `unsafe` |
| header files and include guards | none: a file is the unit, see [Modules](design-modules.md) |
| `int` being whatever size the platform says | `uint32`, `sint64` and so on are always that size |
| signed overflow is undefined behaviour | every overflow wraps |
| `x / 0` is undefined | stops with exit code 101 and a message |
| reading an uninitialised variable | error E0207: it must certainly have a value |
| `const` as an afterthought | immutable by default, `mut` to change |

### Cleanup that cannot be forgotten

A value that owns something is cleaned up at the end of its scope, whichever way
the scope is left: running off the end, `return`, `break`, `continue` or
`skip`. If a struct has a method named `drop`, it runs then, like a C++
destructor, but there is no copy constructor to get wrong, because values are
moved or copied explicitly.

<!-- focus: 8-10 -->
The reserved name `drop()` runs when the value is destroyed.

<!-- focus: 19 -->
`scratch` is declared inside `find`.

<!-- focus: 23 -->
`return i` leaves the function in the middle of the loop. The output shows
`scratch` released before `found 8` is printed.

<!-- focus: 30-31 -->
At the end of `main`, values drop in reverse order of declaration, `b` and
then `a`, as C++ destructors do.

<!-- example: guide-c-cleanup -->

You can run the cleanup earlier with `destroy a;`, which runs `drop()` now and
ends `a`. Calling `a::drop()` yourself is an error. See
[Moves and copies](reference-ownership.md) for what the compiler does with a
struct that has a `drop`.

### Integers

Sizes are in the name, so there is no `int` to depend on. Overflow wraps, signed
and unsigned alike, so the compiler never assumes that an overflow cannot
happen. Division by zero
and an out-of-range string index are not undefined: the program stops with a
message and exit code 101.

A conversion that can lose information (`uint32` to `uint8`) compiles with
warning W0102 until you write `x::to<uint8>()`. Bitwise operators need
parentheses next to arithmetic and comparisons, which removes the classic
`a & b == c` surprise.

## Ownership replaces manual memory management

C++ has `unique_ptr` and RAII as conventions. Drachm makes the rule part of the
language and checks it at compile time, so no program can free something twice
or use it after the free.

- A value has exactly one owner. Assigning it or passing it **moves** it.
- Using the old name afterwards is error E0301.
- `copy x` makes a second value, and you always write it. Nothing copies
  silently except small plain values: numbers, bools and structs of them up to
  32 bytes.
- Lending, instead of moving, is a borrow: `&T` for reading (many at once) or
  `&mut T` for changing (only one, and nothing else reads meanwhile). Both are
  written at the call: `show(&p)`, `bump(&mut n)`.

A `&mut` parameter is the safe form of the `int *out` parameter. Reads need no
`*`, and assigning writes through to the caller's variable.

<!-- focus: 6-9 -->
Two out-parameters. `q = a / b;` writes through the borrow, since the right
side is not itself a reference.

<!-- focus: 14 -->
`&mut` is written at the call, and only a `mut` binding can be lent that way.

<!-- focus: 19-23 -->
Overflow wraps for both signed and unsigned numbers.

<!-- example: guide-c-swap -->

The compiler also stops the dangling pointer. A borrow cannot outlive the value
it points to, and a function that returns a borrow says which parameter it
comes from with `lives` when that is not obvious. See
[Borrowing](lesson-9-borrowing.md) and [Borrows and lifetimes](design-lifetimes.md).

## What `unsafe` and raw pointers still give you

Sometimes you need an address and nothing checks it for you: talking to
hardware, building a data structure the borrow checker cannot see, or porting C.
Raw pointers exist for that, and every use is fenced.

- A file that uses them starts with `//--!$unsafe: true`. Without it the
  compiler refuses (E0317).
- `*T` is a pointer you can write through, `*fixed T` one you can only read. Both
  are 8 bytes. `address of n` makes one.
- Making, copying, comparing, reading and writing through a pointer all need an
  `unsafe { }` block. A function marked `unsafe` counts as one block, and
  calling it needs a block.
- A pointer can be `none`, and `p == none` is the null check.
- The compiler does not track a pointer: it must not outlive the thing it
  points to, and that is your job again.

There is no pointer arithmetic or `malloc` in the compiler yet, so this is
for pointing at values you already have.

<!-- focus: 6-11 -->
An unsafe function. The body needs no inner block.

<!-- focus: 15-19 -->
The call needs `unsafe { }`. `none` is a null pointer.

<!-- example: guide-c-pointer -->

The full rules are in [Unsafe and pointers](reference-unsafe.md) and
[lesson 11](lesson-11-unsafe.md).

### Assembly

<!-- status: not-implemented -->

A whole function may be written in assembly, with no mixing of assembly into
other functions. The register mapping and syntax flavour are still open.

```drachm
unsafe uint64 cycles() asm { rdtsc; shl rdx, 32; or rax, rdx }
```

## What `-O` does

Drachm has two backends, and the one you pick changes speed, never meaning.

- A plain build goes through Cranelift and compiles in milliseconds.
- `-O` writes **C** and calls your system C compiler (`cc`, or `clang` on
  Windows), then links the result with the small runtime. Your program gets
  everything that compiler does at its best, which is why `-O` runs at C speed.

Tests build every program both ways and compare the output bit for bit, so
a program that behaves differently between them is a compiler bug. You need a
C compiler installed to link either way. [Performance](guide-performance.md) has
numbers.

```sh
drachm run program.drh -O
```

## Importing C headers

<!-- status: tentative -->

Calling existing C libraries is planned. The idea is that a header is imported
like a module, with every name behind a prefix:

```drachm
in * from c."SDL2/SDL.h" as sdl;
```

It would use libclang to read the header. How C types map onto Drachm types, how
`char *` becomes a string and who owns what a C function returns are all open,
and none of it is built. Until then there is no way to call C from Drachm.

## What is not here

- No preprocessor or macros. Constants and functions do that job.
- No implicit conversion between a number and a `bool`.
- No `goto`. `break 2;` leaves two loops, and `skip` leaves `if` blocks. See
  [Control flow](reference-control-flow.md).
- No header and source split. [Modules](design-modules.md) are designed for step 7.
- No inheritance or virtual tables. Interfaces and generics
  ([Generics](design-generics.md)) are designed.
- Templates become generics, which are designed but not built.
