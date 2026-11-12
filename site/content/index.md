---
title: Drachm
description: What Drachm is, what it can do today, and how to learn it.
---

# Drachm

Drachm is a statically typed, natively compiled language. Its files end in
`.drh`, and it is pronounced /drɑxm/. It reads like C, with English words for
some keywords: `with`, `of`, `skip`, `fails with` and so on.

The compiler is written in Rust and builds a program in two ways. A plain build
goes through Cranelift and compiles in a moment. An optimised build (`-O`)
writes C and hands it to your C compiler, so the program runs as fast as C. Both
must print the same thing, and the compiler's tests check every example both ways.

## What works today

Drachm is early. A program can already use numbers of every size up to 128 bits,
`bool` and `none`, strings with interpolation and `io::outf` formatting, and structs
with methods, static methods and operator methods. Control flow has `if`, `while`,
`for`, ranges, `break` and `continue` with counts, `skip`, and `match` on numbers
and bools. The compiler also tracks ownership and borrows, so it rejects use after a
move, a change to a value that is borrowed, and a borrow that outlives its value,
and offers `copy` as a fix for moves. Units of measure, input, enums and
`match` on them, optionals, constructors, `fill` and `def`, failures with
`fails` and `maybe`, and raw pointers in `unsafe` are built too.

Other parts of the design, such as generics, collections, modules, config and
concurrency, are not built yet. The "Language design" pages show them as designed,
each with a badge saying "Not implemented yet" or "Tentative". The [Roadmap](coming-later.md)
lists them with the step of the plan each belongs to. The lessons and the other
Reference pages describe only what the compiler accepts.

## How to learn it

Start with [Install and run](install.md), then work through the eight lessons in
order. Each one explains a few ideas beside the code, shows the compiler's real
messages, and ends with exercises that you solve by writing and running a
program. The Reference pages cover each area in full, once you know your way
around.
