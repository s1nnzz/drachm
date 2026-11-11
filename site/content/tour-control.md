---
title: Control flow
description: if, while, for, ranges, break and continue, skip, and match.
---

# Control flow

<!-- example: control -->

- Conditions are always in parentheses: `if (n < 0)`, `while (n > 0)`.
- `if`, `else if` and `else` work as in C. Each branch can `return`.
- `while` runs while its condition is true.
- `for (mut uint32 i = 0; i < 5; i++)` is the C-style loop. The first part
  runs once, the test runs before each round, and the last part runs after
  each round.
- `for (uint32 n of 1..3)` goes over an inclusive range: 1, 2 and 3.
- `break` leaves a loop and `continue` starts its next round. A number after
  either one counts loops outward: `continue 2` goes on with the outer loop.
- `skip` leaves an `if` block. It is not `break`: `skip;` leaves the outermost
  `if`, `skip 2;` two levels, and `skip to next;` moves on to the next `else if`.
  Loops stop a `skip` from going out past them.
- `match` checks one value against `case` lines. The first case that matches
  runs, and there is no fall-through. `case _` matches anything.

Next: the [Reference](reference-types.md) pages, one for each area.
