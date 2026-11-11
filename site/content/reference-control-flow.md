---
title: Control flow
description: The rules for if, while, for, break, continue, skip and match.
---

# Control flow

The full program is in the [tour](tour-control.md). This page lists the rules.

## if, else if, else

```drachm
if (n < 0) {
    return -1;
} else if (n == 0) {
    return 0;
} else {
    return 1;
}
```

The condition must be `bool`, and it is always in parentheses.

## while and for

```drachm
while (n > 0) {
    n--;
}

for (mut uint32 i = 0; i < 5; i++) {
    sum += i;
}

for (uint32 n of 1..3) {        // 1, 2 and 3
    io::out(n::text());
}
```

- In a C-style `for`, the first part runs once, the condition is tested before
  each round, and the last part runs after each round. Each part may be left
  out: `for (;;)` runs until a `break`.
- The first part may declare a variable. That variable exists only inside the
  loop.
- `for (T x of a..b)` goes over the inclusive range from `a` to `b`. If `a` is
  above `b`, it runs zero times. A changed loop variable (`mut`) does not change
  the range.

## break and continue

- `break;` leaves the innermost loop. `continue;` starts its next round.
- A number counts loops outward: `break 2;` leaves the innermost loop and the
  one around it. `continue 2;` leaves the innermost loop and goes on with the
  outer one. The number must be at least 1
  and no bigger than the number of loops around the statement (E0011).
- `continue` in a C-style `for` still runs the step.

## skip

`skip` leaves `if` blocks, as `break` leaves loops.

- `skip;` leaves the outermost `if` level that encloses it, up to the loop
  around it.
- `skip N;` leaves N levels, counting outward from the innermost.
- `skip to next;` leaves the current branch and goes on to the next `else if`
  or `else` of the same chain.

A whole `if` / `else if` / `else` chain counts as one level. Each `case` of a
`match` is also one level. Loops are walls: a `skip` cannot leave the loop
body it is in. A `skip` with no level to leave is error E0012.

## match

```drachm
match (score) {
    case 0 { io::outln("zero"); }
    case 1, 2 { io::outln("one or two"); }
    case 13..19 { io::outln("teen"); }
    case _ { io::outln("other"); }
}
```

- `match` works on whole numbers, `bool` and floats.
- Each `case` gives one value, a list of values separated by commas, or an
  inclusive range `a..b`. `case _` matches anything.
- The first case that matches runs. Cases never fall through.
- A value may be a constant expression, such as `case 2 + 1`.
- A `match` must handle every value. If it does not, the compiler gives
  warning W0110. Add `case _` to catch the rest. On floats, only `case _`
  covers every value.
- A case after one that already matches every value it could match is
  warning W0111.
- The matched value is evaluated once.
- `break`, `continue` and `skip` inside a case act on the enclosing loop or
  `if`. A `skip` inside a `match` leaves the `match` and not a loop.
- A `match` on `string` is not built yet (see [Coming later](coming-later.md)).

## Returns and definite values

A function that declares a return type must return a value on every path, or
the compiler reports E0105. A loop with no `break`, such as `for (;;)`, never
falls out of its bottom, so no `return` is needed after it.
