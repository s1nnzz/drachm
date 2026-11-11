---
title: Coming later
description: Designed parts of Drachm that the compiler does not build yet, by plan step.
---

# Coming later

These parts of Drachm are designed, but this compiler does not build them yet.
Each one names the step of the build plan (`docs/PLAN.md`) that will add it.
Names and syntax can still change before then.

## Step 3: ownership and borrowing

Moves, `copy`, `take`, `drop()`, `destroy` and the checks for them work, and
programs that use them build and run. What is still to come:

- Borrows, with `&string` and `&mut` parameters and `lend`. Until then the
  compiler cannot build a program that uses one.
- Struct `open` fields, lifetimes, and `unsafe` with raw pointers.

## Step 4: the rest of control flow

- `match` on `string`. Today it reports E0901.
- `*` globs in a `case`, such as `case "/api/*"`.
- `for` over the characters of a string.

## Step 5: constructors, enums, and errors

- Constructors, and `new Type of (...)`.
- Enums that carry data, with `match` on their variants.
- Error handling with `fails with` and `maybe`, and `failure` types. There is no
  `Result` type.

## Step 6: generics, interfaces and collections

- Generic functions and structs, with constraints such as `<T with Shape>`.
- Interfaces, and `struct S with X`.
- Lists, maps and other collections, with `for` over them.
- Lambdas.

## Step 7: modules, projects and the standard library

- Modules: `in` and `out` between files of a project.
- `drachm.toml` for project settings, and scripts.
- The standard library beyond `io`.

## Step 8: language server

- Editor features beyond highlighting: errors as you type, completion, go to
  definition and hover.

## Later, not yet scheduled

- Maths: vector and matrix types.
- Concurrency: threads, channels, and async functions.
- Importing C headers.
- A formatter (`drachm fmt`) and a package registry.
- 256-bit integers, and the `float8` and `float16` types.
- Targets besides x86-64 Linux and Windows: macOS, and AArch64.
