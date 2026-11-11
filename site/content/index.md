---
title: Drachm
description: What Drachm is, what it can do today, and how this book is organised.
---

# Drachm

Drachm is a statically typed, natively compiled language. Its files end in
`.drh`, and it is pronounced /drɑxm/.

Drachm reads like C, with English words for some keywords: `with`, `of`,
`skip`, `fails with`, and so on. The compiler is written in Rust. It has two
ways to build a program:

- **Plain builds** go through Cranelift. They compile in a moment.
- **Optimised builds** (`-O`) write C and hand it to your C compiler. They run
  as fast as C.

Both ways must print the same thing. The compiler's tests check every example
both ways.

## Where Drachm is today

Drachm is early. A program can use:

- numbers of every size up to 128 bits, `bool`, and `none`
- `string`s, with interpolation and `io::outf` formatting
- structs, with methods, static methods and operator methods
- `if`, `while`, `for`, ranges, `break` and `continue` with counts, `skip`,
  and `match` on numbers and bools

Some parts of the design are not built yet, such as ownership rules, enums,
modules and error handling. The [Coming later](coming-later.md) page lists
them, with the step of the plan each belongs to. These pages describe only
what the compiler accepts today.

## How to read this book

Start with [Install and run](install.md), then go through the Tour. The
Reference has one page per area. Every program on these pages is a real file
that the test suite compiles and runs.
