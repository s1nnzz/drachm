---
title: C imports
description: The rules for importing C headers, the C types and how they map, the [native] table, and what is not supported yet.
---

# C imports

The [guide](guide-c-imports.md) shows the steps. This page states the rules.

## The import

```drachm
in * from c."header.h" as alias;
```

The header is found like a C compiler finds it: the system folders, then the
`include` folders of the `[native]` table. The alias is required. It exposes
every declaration of the header and of the headers it includes, so an umbrella
header (`SDL.h`) offers the whole API under one name. Several declarations of
one C symbol are one declaration. Two imports of the same header (with the same
settings) share their types, whatever aliases they use, so a struct from one
file passes to a function imported in another.

Names live apart from your own: `alias::fx_add` and a Drachm `fx_add` do not
clash. Only the names a program uses are brought into it.

| Problem | Error |
|---|---|
| libclang cannot be loaded | E0700 |
| the header is not found | E0701 |
| the header has errors | E0702 |
| the name is not in the header | E0703 |
| a C and a Drachm number mixed | E0704 |
| `match` on a C enum without `case _` | E0705 |
| a declaration the compiler cannot use yet | E0901, with the reason |

## The `[native]` table

One table in `drachm.toml` (or `srachm.toml`):

| Key | Meaning |
|---|---|
| `include` | header folders, like `-I` |
| `defines` | preprocessor definitions, like `-D` (`NAME` or `NAME=value`) |
| `libraries` | libraries to link, like `-l` |
| `library_dirs` | where to look for them, like `-L` |
| `sources` | C files of the project, compiled and linked with it |

Relative paths are relative to the file that names them. Headers are read with
the same `include` and `defines` that the program is built with. A project that
imports a dependency also links what the dependency's own `[native]` table
lists, and a dependency's C imports use the dependency's table. The system
headers and the C and maths libraries need no table, so a script can use them.
A link failure names the missing library or symbol and points at the table.

`drachm check` reads the headers and checks the program without linking, so a
library project (`kind = "library"`, no `main()`) can be checked on a machine
that does not have the C library installed, as long as the headers are there.

## Calls need `unsafe`

A C function is declared `unsafe`: calling it needs an `unsafe { }` block
(E0316), and `unsafe` needs the file flag (E0317). A wrapper function that
contains the block has an ordinary safe signature. Everything a wrapper's
author must establish (valid pointers, lifetimes, who frees what) is theirs;
a C declaration proves none of it.

## Types

| C | Drachm |
|---|---|
| `char`, `signed char`, `unsigned char` | `c::char`, `c::schar`, `c::uchar` |
| `short`, `unsigned short` | `c::short`, `c::ushort` |
| `int`, `unsigned int` | `c::int`, `c::uint` |
| `long`, `unsigned long` | `c::long`, `c::ulong` |
| `long long`, `unsigned long long` | `c::longlong`, `c::ulonglong` |
| `size_t`, `ssize_t` | `c::size_t`, `c::ssize_t` |
| `float`, `double` | `c::float`, `c::double` |
| `_Bool`, `bool` | `c::bool` |
| `void` result | `none` |
| `T *` | `foreign *T` |
| `void *` | `foreign *none` |
| `char *`, `unsigned char *` | `foreign *uint8` |
| `signed char *` | `foreign *sint8` |
| `struct S`, `typedef struct { } S` | a struct `alias::S` with the C layout |
| `struct S *` where `S` is incomplete | `foreign *alias::S`, an opaque handle |
| `enum E` | an enum type `alias::E` that also holds unknown values |
| `int (*f)(int)` | `foreign *(c::int) => c::int` |
| typedefs | what they stand for (`uint32_t` is `c::uint`) |

The C number types have the size the C ABI gives them on the machine
(`c::long` is 64 bits on Linux and macOS, 32 on Windows). They are types of
their own: `c::int` does not become `sint32` or the reverse by itself (E0704).
Convert with `as` or `::to<T>()`. `as` is the conversion of C numbers; for
other numbers use `::to<T>()`. A constant fits any of them, and arithmetic and
comparisons work between values of one C type. `c::bool` and `bool` are
converted the same way: `b::to<bool>()`.

A struct that a header declares gets exactly the C layout: fields are public
and writable, and `fx::fx_point p = 3, 4;` builds one. A struct whose layout is
not the natural one (packed or aligned fields), that has bit-fields, an
anonymous member or a flexible array member cannot be used by value (E0901);
pointers to it still work, as opaque handles.

A `foreign` pointer copies freely, since C owns what it points at. A pointer
made on the Drachm side (`address of p`) goes where C wants a pointer to the
same type, and any pointer goes where C wants `void *`. The other direction,
from a `foreign *none` to a typed pointer, is not implicit.

A C enum has the type's name (`shape::shape_kind`), its named values are
constants (`shape::SHAPE_WIDE`), and a number becomes one with `n as
shape::shape_kind`. A `match` on one needs `case _` (E0705).

## Macros

- An object-like macro that evaluates to a number or a string is a constant of
  the program: `alias::NAME`. It has no type of its own, so it fits whichever
  type it is given.
- A function-like macro needs a declaration of its types before it can be
  called: `from c macro NAME(Type name, ...) gives Type;`. The compiler writes a
  small C function that calls the macro, compiled with the same `include` and
  `defines`, so the macro is expanded by the C compiler.
- A macro that is neither is reported (E0901).

## Text, memory and callbacks

- `p::text()` on a `foreign *uint8`, in `unsafe`: the text up to the NUL as a
  `string`. Fails with `InvalidText` for a null pointer or bytes that are not
  UTF-8.
- `&s as c string`, written as an argument of a C call: a `foreign *uint8` to a
  NUL-terminated copy of `s` that lives until the end of the statement. A NUL
  inside `s` is a run-time error (exit code 101).
- Memory returned by C is never released by the compiler, and no release
  function is inferred from a type. The wrapper calls it, typically in `drop()`.
- A callback is the name of a top-level function whose parameters and result
  have exactly the C types of the pointer, with no captured values. A
  non-matching function is E0101, anything else E0901. Callbacks must not be
  called from other threads or unwind through C frames.

## Not supported yet
<!-- status: not-implemented -->

These are named in the headers but report E0901 with the reason when used:

- variadic functions (`printf`)
- unions, and structs with bit-fields or flexible array members
- `static inline` functions (they have no symbol)
- global C variables
- structs larger than 16 bytes passed or returned by value, and passing structs
  by value on targets other than System V x86-64
- `long double` and 128-bit integers
- callbacks that take or return structs, and C pointers to arrays
- import forms without an alias, and importing single names
- cross-compiling

## Toolchain

Reading headers needs libclang at the time `drachm check`, `build` or `run`
compiles a program with a C import. If it cannot be loaded, E0700 says so; set
`LIBCLANG_PATH` to the folder that holds it. libclang also needs its own
headers (`stddef.h`, `stdint.h`): the compiler finds them through
`clang -print-resource-dir` or the usual install folders, or from
`DRACHM_CLANG_RESOURCE_DIR`. Linking uses the system C compiler (`cc`), as for
every program.
