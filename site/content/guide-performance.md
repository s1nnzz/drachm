---
title: Performance
description: Plain and -O builds, what costs nothing, when copies happen, how to benchmark, current numbers, and the planned optimiser.
---

# Performance

Drachm is designed to run as fast as C or Rust, with no garbage collector and no
hidden work. This guide explains what you control, what the two build modes do,
and how fast the compiler is today. It is honest about the gaps.

## Two builds

```sh
drachm run program.drh        # plain build
drachm run program.drh -O     # optimised build
```

- A **plain build** compiles through Cranelift. It compiles in milliseconds and
  the program is reasonably fast. Use it while you write and test.
- An **`-O` build** generates C and hands it to your system C compiler, which
  optimises it. The build takes longer, and the program runs at roughly C
  speed. Use it for releases and benchmarks.

Both builds must print exactly the same output. The repo's tests build every
program both ways and compare, so `-O` only changes speed and never what your
program means.

## What costs nothing

Many things you write exist for the compiler, and are gone by the time the
program runs.

- **Units.** `float64<m/s>` is a `float64`. The compiler checks the units and
  erases them, so `speed` below does the same work as a function on plain
  numbers. A conversion between scaled units, such as `trip::in<m>()`, is a
  multiplication by a constant.

<!-- example: guide-perf-units -->

- **Moves.** Passing a `string` to a function, or assigning it, hands over
  ownership. The text is not copied, and the old name cannot be used again, so
  there is nothing to track at run time.
- **Borrows.** `&T` and `&mut T` are checked entirely at compile time. A borrow
  is passed as an address and costs the same as a C pointer parameter.
- **Inferred `self`.** You never write whether a method reads or changes
  `self`. The compiler works it out, and it is checked, so there is no `const`
  to add and nothing to pay for.
- **Enum layout.** An enum is a one-byte tag followed by the values of the
  variant it holds, laid out like a struct. The `Shape` enum in
  [Enums](reference-enums.md) is 24 bytes. There is no heap allocation per
  variant and no vtable, and `match` is a test on the tag.
- **Overflow.** Arithmetic wraps, so addition is one instruction with no
  check. Division by zero and a string index out of range are checked, and
  stop the program with exit code 101.

## When copies happen

Drachm never copies behind your back, with one exception: small plain values.

- Numbers, bools, and structs whose fields are all copyable and that are at most
  **32 bytes** (padding counted) copy by themselves.
- Everything else moves. To make a second one you write `copy x`, so every
  allocation and every memcpy of a large value is visible in the source.
- Field order can change the size, because of alignment padding. If a struct is
  just over 32 bytes and you want it to copy, reorder its fields. If you never
  want it to copy, write `struct Handle stays { ... }`.

<!-- focus: 6-9,40 -->
Four 8-byte fields: 32 bytes, so `Vec4` copies. `Vec4 w = v;` leaves both usable.

<!-- focus: 19-23 -->
One more field and `Big` is 40 bytes, so it moves.

<!-- focus: 43-46 -->
`&big` lends without copying, as many times as you like. `copy big` is the
only line here that copies 40 bytes, and it says so.

<!-- example: guide-perf-copies -->

The compiler also warns when `copy` is pointless (W0322 for a number) and
explains how to fix a use-after-move, so you can follow its hints without guessing.

### Lend instead of copying

For anything that is not tiny, a function that only reads should take `&T`. The
call site writes `&`, and a reader of the code sees that no copy is made:

```drachm
float64 total(&Big b) {
    return b::sum();
}
```

## Drops

A value is destroyed at the end of its scope, and the compiler knows the
points where that happens. For a `string` that is one free. For a struct with a
`drop()` method, your method runs at those points, in the reverse order of
declaration. The result is the same cost as a manual `free` in a correct C
program, with no tracing and no pauses. `destroy x;` runs it earlier.
See [the C guide](guide-from-c.md) for an example.

When your program is slow, look for loops that create a `string` per
iteration. Each is an allocation and a free. Building one `mut string` with
`+=` is cheaper than creating many.

## How to benchmark

The repo has a benchmark script. It builds Drachm, C and Rust versions of each
program in `bench/`, checks that they print the same result, and reports the best
of five runs.

```sh
bench/run.sh
```

It needs `cc` and `rustc` on your `PATH`. A benchmark is three files with one
name: `bench/NAME.drh`, `bench/NAME.c` and `bench/NAME.rs`. Each prints the
same answer, because the script compares them. The Drachm side is a plain
program:

```drachm
// Call-heavy: naive recursive Fibonacci.
in io;

uint64 fib(uint64 n) {
    if (n < 2) { return n; }
    return fib(n - 1) + fib(n - 2);
}

none main() {
    io::outln(fib(40)::text());
}
```

Tips for your own measurements:

- Always benchmark an `-O` build against C built with `-O3`. A plain build is for
  quick checks.
- Print a result that depends on all the work. Otherwise the C compiler is free
  to delete the loop, in your program or in the one you compare with.
- Run each program several times and take the best. The first run pays for cold
  caches.

## Current numbers

These come from the project's README, measured on one machine (Linux x86-64),
best of five runs, in milliseconds. They show the shape, not a promise. Your
machine will give different times.

| bench | Drachm `-O` | C `-O3` | Rust `-O3` | Drachm plain |
|---|---|---|---|---|
| collatz (integer loop) | 328 | 331 | 418 | 897 |
| fib (recursive calls) | 119 | 109 | 228 | 401 |
| mandel (floating point) | 167 | 177 | 185 | 247 |
| primes (trial division) | 860 | 1509 | 860 | 1443 |
| strings (build and scan text) | 254 | 95 | 145 | 244 |

What to take from it:

- `-O` matches or beats C on four of the five benchmarks, because it is C
  code under the hood. Small differences come from how each program is spelled
  out.
- Plain builds are roughly 1.5 to 3.5 times slower than `-O` on the first four,
  since Cranelift does no inlining or loop optimisation yet.
- The string benchmark is the weak spot. It is allocation-bound, and `-O`
  takes 2.7 times as long as C. The README expects moves to remove most of the
  copies it makes.

## The planned optimiser

<!-- status: not-implemented -->

Step 7.5 of the plan adds an optimiser of Drachm's own, working on the compiler's
mid-level form (the MIR) before either backend sees the program. The aim is
that plain builds get close to `-O`, and that `-O` can one day stop depending
on a C compiler. Planned passes, roughly in order of payoff, each measured with
`bench/run.sh`:

1. SSA form for the MIR.
2. Inlining of small and single-use functions, which is most of the `fib` gap.
3. Scalar replacement of structs: fields kept in registers.
4. Constant propagation and folding, dead code removal.
5. Loop-invariant code motion, common subexpressions, induction variables.
6. Removing checks and drops and moves that are provably unneeded.
7. Alias information from the borrow checker: a `&mut` never aliases anything
   else, which C can only assume with `restrict`.

Whether `-O` keeps going through C afterwards is an open question for the
language's owner. None of this changes what a program means, so nothing you
write today needs to change. See the [Roadmap](coming-later.md).
