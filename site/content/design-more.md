---
title: Smaller decided details
description: 256-bit integers, float8 and float16, string patterns, text to number, targets and other small pieces of the design.
---

# Smaller decided details

Pieces of the design that are small enough to share a page.

## More number types
<!-- status: not-implemented -->

`uint256` and `sint256`, and the floats `float8` (E4M3) and `float16`, are in
the design with the suffixes `5u256`, `5s256`, `2.5f8` and `2.5f16`. The
compiler stops at 128 bits and `float32`.

```drachm
uint256 big = 1u256 << 200;
float16 half = 0.5f16;
```

## Strings in match

`match` on a `string` takes whole strings. A plain string matches exactly, `*`
included. A pattern with wildcards is written `globbed`, where each `*` matches
any run of characters. The rules and the example are in
[Matching text](reference-control-flow.md#matching-text).

<!-- example: match-text -->

## Text to number

`"42"::to<uint32>()` is a failing call: it fails with the built-in
`ConvertError` when the text is empty, not a number, or out of range. Handle it
with `maybe`, `fails` or `fails with`; see [Errors](design-errors.md).

<!-- example: input-lines -->

## Targets
<!-- status: not-implemented -->

Plain builds target x86-64 Linux and Windows. The `target` setting names an
operating system and a CPU such as `macos-arm64`, and macOS and AArch64 are
planned.

## Keywords still being settled

<!-- status: tentative -->

Which of the words that the compiler already knows are reserved everywhere,
and which only in their position, is open. `lend` and `take` are always
reserved. `to`, `by`, `unit`, `leaving`, `address`, `stays`, `changes` and
`asm` mean something only in their place, and are ordinary names elsewhere.
