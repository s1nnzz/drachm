---
title: "Lesson 3: Decisions and loops"
description: if and else, comparisons and bools, while, for, ranges, break and continue.
---

# Lesson 3: Decisions and loops

**Goal:** make a program choose between actions and repeat them.

## Asking a question with if

A comparison such as `hp < 10` produces a `bool`, which is `true` or `false`.
`if` runs its block only when the condition is true, and `else` runs when it
is not. The condition always sits in parentheses.

<!-- focus: 5 -->
The block runs only when the condition is true.

<!-- focus: 7 -->
Chain more questions with `else if`. The first condition that is true wins and
the rest are skipped.

<!-- focus: 9 -->
`else` catches everything that is left. Change `hp` and run the program again
to see each branch.

```drachm title="hp.drh"
in io;

none main() {
    uint32 hp = 4;
    if (hp == 0) {
        io::outln("down");
    } else if (hp < 10) {
        io::outln("careful");
    } else {
        io::outln("fine");
    }
}
```

## The condition must be a bool

Drachm does not treat numbers as true or false. If you write `if (n)` with a
number, the compiler stops and says so. Combine conditions with `&&` (and),
`||` (or) and `!` (not); the right side runs only when it is needed.

```drachm
uint32 n = 3;
if (n) {
    io::outln("yes");
}
```

```text title="drachm check"
error[E0101]: expected `bool`, found `uint32`
  --> main.drh:5:9
   |
 5 |     if (n) {
   |         ^ this is `uint32`
   |
 help: `bool` is not a number and never converts to or from one
```

## Repeating with while and for

`while` repeats its block as long as the condition is true. The counter has to
be `mut`, because the loop changes it.

<!-- focus: 5-8 -->
The `while` loop tests `n > 0` before every round, so the digits 3, 2 and 1 come out on one line.

<!-- focus: 11-13 -->
A C-style `for` has three parts. The first runs once, the second is tested
before each round, and the third runs after each round.

<!-- focus: 16-18 -->
`for (uint32 k of 5..8)` is the short way to go over a range. The range
includes both ends, so it counts 5, 6, 7, 8.

<!-- focus: 21-31 -->
A range that starts above its end runs zero times. `for (;;)` has no
condition, so it runs until a `break` leaves it.

<!-- example: loops -->

## Leaving a loop early

`break` leaves the innermost loop, and `continue` jumps to its next round.
Put a number after either to count loops outward: `break 2;` leaves the loop
and the one around it. Run this program and work out why each line prints
what it does.

<!-- example: break-continue -->

```text title="drachm check, when the number is too big"
error[E0011]: `break 2` counts 2 loops, but there is only 1 around it
  --> main.drh:5:15
   |
 4 |     for (uint32 i of 1..3) {
   |     ---------------------- the outermost loop
 5 |         break 2;
   |               ^ too many
   |
 help: `break 1;` leaves the outermost loop
```

### Your turn: add up the numbers 1 to 100

Use a `mut` total and a `for` over the range `1..100`, then print the total.
(Gauss knew the answer was 5050.)

```drachm title="starter"
in io;

none main() {
    mut uint32 sum = 0;
    // your code here
    io::outln(sum::text());
}
```

```text title="expected output"
5050
```

<!-- solution-of: solution-3-1 -->
```drachm solution
in io;

none main() {
    mut uint32 sum = 0;
    for (uint32 n of 1..100) {
        sum += n;
    }
    io::outln(sum::text());
}
```

### Your turn: FizzBuzz

For the numbers 1 to 15, print `FizzBuzz` when the number divides by 15,
`Fizz` when it divides by 3, `Buzz` when it divides by 5, and the number itself
otherwise. The remainder operator is `%`, and the order of your `else if`
chain matters.

```drachm title="starter"
in io;

none main() {
    for (uint32 n of 1..15) {
        // your code here
    }
}
```

```text title="expected output"
1
2
Fizz
4
Buzz
Fizz
7
8
Fizz
Buzz
11
Fizz
13
14
FizzBuzz
```

<!-- solution-of: solution-3-2 -->
```drachm solution
in io;

none main() {
    for (uint32 n of 1..15) {
        if (n % 15 == 0) {
            io::outln("FizzBuzz");
        } else if (n % 3 == 0) {
            io::outln("Fizz");
        } else if (n % 5 == 0) {
            io::outln("Buzz");
        } else {
            io::outln(n::text());
        }
    }
}
```

Next: [Lesson 4: Functions](lesson-4-functions.md).
