---
title: Smaller decided details
description: 256-bit integers, float8 and float16, string patterns, targets and other small pieces of the design.
status: not-implemented
---

# Smaller decided details

Pieces of the design that are small enough to share a page.

## More number types

`uint256` and `sint256`, and the floats `float8` (E4M3) and `float16`, are in
the design with the suffixes `5u256`, `5s256`, `2.5f8` and `2.5f16`. The
compiler stops at 128 bits and `float32`.

```drachm
uint256 big = 1u256 << 200;
float16 half = 0.5f16;
```

## Strings in match

`match` on a `string` takes whole strings and `*` globs. Today it reports
E0901.

<!-- focus: 2-4 -->

```drachm
match (path) {
    case "/"        { home(); }
    case "/api/*"   { api(path); }
    case "*.png"    { image(path); }
    case _          { missing(); }
}
```

## Text to number

`"42"::to<uint32>()` is planned to be a failing call. Handle it with `maybe`,
`fails` or `fails with`; see [Errors](design-errors.md).

## Targets

Plain builds target x86-64 Linux and Windows. The `target` setting names an
operating system and a CPU such as `macos-arm64`, and macOS and AArch64 are
planned.

## Keywords still being settled

<!-- status: tentative -->

Which of the words that the compiler already knows are reserved everywhere,
and which only in their position, is open. `lend` and `take` are always
reserved. `to`, `by`, `unit`, `leaving`, `address`, `stays`, `changes` and
`asm` mean something only in their place, and are ordinary names elsewhere.
