---
title: Calling C libraries
description: Import a C header, link the library, call it from unsafe code, and wrap it in a safe Drachm API.
---

# Calling C libraries

A C header is imported like a module. Everything the header declares, and
everything it includes, comes in under the name you give it:

```drachm
in * from c."SDL2/SDL.h" as sdl;
```

`sdl::SDL_Init(...)` then calls the C function, `sdl::SDL_Rect` is the C struct,
`sdl::SDL_INIT_VIDEO` is the C macro. C names are written exactly as the header
spells them. The compiler reads headers with libclang (the C parser of the LLVM
project), so a machine that builds programs with C imports needs the Clang
libraries installed. Programs without a C import never load it.

This guide builds up a safe wrapper in four steps. The
[reference](reference-c-imports.md) has the exact rules and the list of what is
not supported yet.

## Telling the compiler where the library is

A project says where the headers are and what to link in its `drachm.toml`, in
one `[native]` table:

```toml
[native]
include = ["vendor/include"]     # where headers are looked for (-I)
defines = ["SDL_MAIN_HANDLED"]   # preprocessor definitions (-D)
libraries = ["SDL2"]             # libraries to link (-l)
library_dirs = ["vendor/lib"]    # where the libraries are (-L)
sources = ["c/helpers.c"]        # C files of your own, compiled with the program
unsafe = true                    # every file of the project may use unsafe (no file flag)
```

Relative paths are relative to the `drachm.toml`. The system's own header
folders are always searched, so a script (a file with no `drachm.toml`) can
import `stdlib.h` or `math.h` and call what the C library and the maths library
provide. Linking anything else needs a project. A project that uses a
dependency also links what the dependency's `[native]` table asks for.

## A first call

<!-- example: c-first-call -->

Three things to notice:

- **C calls are `unsafe`.** A C declaration tells the compiler the types, not
  what the function needs from its caller. Calling one needs an `unsafe { }`
  block, and `unsafe` needs the file flag `//--!$unsafe: true` or the
  project's `unsafe = true` in the table above (see
  [Unsafe and pointers](reference-unsafe.md)).
- **C numbers are types of their own.** `c::int`, `c::long`, `c::uchar`,
  `c::size_t`, `c::double` and the rest are not synonyms of `sint32` or
  `uint64`: their size follows the C ABI of the machine. Convert with `as`
  (`n as sint32`) or `::to<sint32>()`. A plain constant fits any of them.
- **A wrapper hides the unsafe part.** `add` and `clamp` take and return Drachm
  numbers. Callers need no `unsafe`; the wrapper's author is the one who
  checked what the C function needs.

Macros come in two kinds. A macro that evaluates to a number or a string is a
constant (`tally::TALLY_LIMIT`). A function-like macro cannot be inspected, so
you declare its types once, and then it is callable like a function:

```drachm
from c macro TALLY_DOUBLE(c::int x) gives c::int;
```

A `static inline` function, which has no symbol to link to, is declared the
same way.

## Structs and enums

<!-- example: c-shapes -->

A C struct is a value struct with the exact C layout: fields are read and
written, and small structs go to C and come back by value. A C enum is a type of
its own that can also hold values the header does not list, because C can send
any number. So a `match` on it always needs a `case _`, even when every named
value has a case ([E0705](errors.md)).

`address of p` gives a `*shape_point`. C does not take it by itself: the
hand-over is written with `as`, `(address of p) as foreign *shape::shape_point`,
and any pointer goes to C's `void *` with `as foreign *none`. Without the `as`
the compiler says so (E0335) and shows what to write.

## Pointers, text and returned memory

<!-- example: c-handles -->

C pointers are `foreign *T`. A `char *` is a `foreign *uint8`, a `void *` is a
`foreign *none`, and `none` is the null pointer. Only `unsafe` code uses them;
the wrapper turns them into Drachm values:

- `p::text()` reads the bytes up to the NUL and checks that they are UTF-8. It
  gives a `string?`: `none` for a null pointer, which is an answer and not a
  failure, and it fails with `InvalidText` only for bad text. Because it is an
  optional that can fail, the wrapper unwraps it: `string? t = p::text() fails;`
  then `return t maybe "fallback";`, or `p::text() maybe none` where no
  failure is wanted.
- `&s as c string` passes a Drachm string where C wants a `const char *`. The
  text is a copy that ends in a NUL and lives for that one call; a NUL inside
  the string stops the program, since C would see a shorter string than the
  one you wrote.
- Memory a C function returns is released by a C function, and nothing in the
  compiler guesses which. The wrapper pairs them itself, usually in `drop()`,
  as `Notes` does above. `drop()` runs however the scope ends, so an early
  `return` does not leak.
- A C function pointer parameter takes the name of a plain top-level Drachm
  function, written with the C types and capturing nothing. `walk` above hands
  `show` to C. A closure cannot be passed.

C's error conventions (a negative result, a null pointer, an error code you
read afterwards) are ordinary wrapper code: check the result and `fail` as
usual.

## A script and the system libraries

<!-- example: c-libm -->

## Where the compiler stops

Not everything in a C header is usable yet. The compiler tells you which
declaration and why (E0901) when you name one it cannot use, and the rest of
the header stays available: variadic functions (`printf`), unions, bit-fields,
global variables, structs larger than 16 bytes passed
by value, and struct passing on machines other than x86-64 Linux and macOS. The
[reference](reference-c-imports.md#not-supported-yet) lists them.
