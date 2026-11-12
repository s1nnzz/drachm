---
title: Roadmap
description: What the compiler builds today, the steps still to come, and where each designed feature is described.
---

# Roadmap

Drachm is built in steps (`docs/PLAN.md` in the repository). This page mirrors
that plan. The pages under "Language design" describe the finished language as it
is designed, and carry a badge saying "Not implemented yet" or "Tentative" until
the compiler accepts what they show.

## Progress

| Step | What | Status |
|---|---|---|
| 0 | Foundations: lib split, test harness | done |
| 1 | Drachm syntax, numbers up to 128 bits | done (256-bit and `float8`/`float16` later, see [More details](design-more.md)) |
| 2 | Strings and structs | done (`split`, `lines`, `chars` later) |
| 3 | Ownership, borrow checking, pointers | mostly done: WP8 lives inference left; see [Borrows](design-lifetimes.md) and [Unsafe and pointers](reference-unsafe.md) |
| 4 | `for`, `skip`, `match` | done, including `match` on text with `globbed` patterns and `for` over characters (see [Control flow](reference-control-flow.md)) |
| 4.5 | Units of measure | done, see [Units](reference-units.md) |
| 4.6 | Input: `io::inln()`, `io::ended()`, text `::to<T>()` | done, see [Input and output](reference-io.md) |
| 5 | Constructors, enums, `fails` / `maybe` | done: enums with data and `match` on them ([Enums](reference-enums.md)), constructors, `new ... of`, `fill` and `def` ([Structs](reference-structs.md#constructors)) are done; optionals ([Optionals](reference-enums.md#optionals)) are done; [Failures](reference-errors.md) (`failure`, `fails`, `fail`, `maybe`, a bare `fails`, handlers, `because`, failing constructors and `drop()`) are done. Only the `crash` setting for `unhandled_fail` waits for step 7 |
| 6 | Generics, interfaces, collections | not started: [Generics](design-generics.md), [Collections](design-collections.md) |
| 7 | Modules, config, scripts, stdlib | not started: [Modules](design-modules.md), [Config](design-config.md), [Scripts](design-scripts.md) |
| 8 | Language server | not started, see [Tooling](design-tooling.md) |

## Later, not yet scheduled

- [Lambdas and concurrency](design-concurrency.md): threads, channels, async.
- [Maths and exact numbers](design-maths.md): vectors, matrices, fractions.
- Importing C headers, in [Modules](design-modules.md#c-headers).
- `drachm fmt` and the Agora package registry, in [Tooling](design-tooling.md).
- Assembly functions, in [Unsafe](design-unsafe.md#assembly-functions).
- Targets besides x86-64 Linux and Windows: macOS, and AArch64.
