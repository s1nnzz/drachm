---
title: Control flow
description: The rules for if, while, for, break, continue, skip and match.
---

# Control flow

The lessons cover [decisions and loops](lesson-3-decisions-and-loops.md) and
[match and skip](lesson-7-match-and-skip.md). This page gives the rules.

## if, else if, else

The condition must be a `bool`, and it is always in parentheses. A number is not
a condition (E0101). A function that returns a value must return on every path,
or the compiler reports E0105. A loop with no `break`, such as `for (;;)`, never
falls out of its bottom, so no `return` is needed after it.

```drachm
if (n < 0) {
    return -1;
} else if (n == 0) {
    return 0;
} else {
    return 1;
}
```

## while and for

In a C-style `for`, the first part runs once, the condition is tested before each
round, and the last part runs after each round. Each part may be left out, and
the first part may declare a variable that exists only inside the loop.
`for (T x of a..b)` goes over the inclusive range from `a` to `b`, and runs zero
times if `a` is above `b`. The loop variable is not `mut` unless you say so, and
changing it does not change the range.

<!-- example: loops -->

### Looping over characters

`for (string c of text)` goes over the characters of a `string`, one at a time, in
order. A character is a grapheme cluster, so an accent written as two code points,
a `\r\n` pair and an emoji with joiners each count once; the number of rounds is
the `len()` of the text, and an empty text runs zero times. Each `c` is a copy that
can be kept or moved on. `for (&string c of text)` gives a borrowed view of each
character instead, which cannot be kept: write `copy c` to keep one. The loop
borrows the text, so the body cannot change it. `break`, `continue` and the rest
work as in any loop.

<!-- example: for-chars -->

## break and continue

`break;` leaves the innermost loop and `continue;` starts its next round. A number
counts loops outward, so `break 2;` leaves the innermost loop and the one around
it, and `continue 2;` goes on with the outer loop. The number must be at least 1
and no bigger than the number of loops around the statement (E0011).
`continue` in a C-style `for` still runs the step.

<!-- example: break-continue -->

## skip

`skip` leaves `if` blocks, as `break` leaves loops. `skip;` leaves the outermost
`if` level that encloses it, up to the loop around it. `skip N;` leaves N levels,
counting outward from the innermost, and `skip to next;` leaves the current
branch and goes on to the next `else if` or `else` of the same chain.

A whole `if` / `else if` / `else` chain counts as one level, and each `case` of a
`match` is also one level. Loops are walls, so a `skip` cannot leave the loop body
it is in. A `skip` with no level to leave is error E0012.

<!-- example: skip -->

## match

`match` works on whole numbers, `bool`, floats and `string`. Each `case` gives one value, a
list separated by commas, or an inclusive range `a..b`, and `case _` matches
anything. The first case that matches runs and cases never fall through. A value
may be a constant expression such as `case 2 + 1`, and the matched value is
evaluated once.

A `match` must handle every value, or the compiler gives warning W0110; add
`case _` to catch the rest. On floats only `case _` covers every value. A case
after one that already matches every value it could match is warning W0111.
`break`, `continue` and `skip` inside a case act on the enclosing loop or `if`,
and a `skip` inside a `match` leaves the `match`, not a loop.

<!-- example: match-basic -->

### Matching text

On a `string`, each `case` gives plain strings, a list of them, or `case _`. A
plain string matches the text exactly, `*` included: `case "a*b"` matches only
`a*b`. To match a pattern, write `globbed` before the string: in
`case globbed "*.png"`, each `*` matches any run of characters, none included,
and the whole text must match. `globbed` applies to the one string after it, so a list writes
it on each: `case globbed "*.png", globbed "*.jpg"`. Without a `*` a `globbed`
string is an exact match too. The cases are tried in order, and the first one that
matches runs.

The text is borrowed by the `match`, not moved, so a variable is still usable after
it. `string` cases are checked the same way as the others: only `case _` (or a
`case globbed "*"`) makes the `match` handle every text, so any other `match` on
`string` gets W0110 with the note that text has too many values to list. A text
case that can never run is W0111, but only when an earlier case clearly covers it:
a repeated text, or a text that an earlier `globbed` case matches. Other overlaps
are not reported.

<!-- example: match-text -->
