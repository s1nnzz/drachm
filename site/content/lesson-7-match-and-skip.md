---
title: "Lesson 7: match and skip"
description: Choosing between many values with match, ranges and lists in cases, exhaustiveness warnings, and leaving if blocks with skip.
---

# Lesson 7: match and skip

**Goal:** choose between many values without a long `else if` chain, and leave
a block early.

## Choosing with match

When one value decides what to do, `match` is shorter than a chain of `if`s.
It looks at the value once, tries each `case` from the top, and runs the block
of the first one that fits. There is no fall-through, so you never write
`break` to end a case.

<!-- focus: 5 -->
A case can be a single value.

<!-- focus: 6 -->
A list of values, separated by commas, matches any of them.

<!-- focus: 7 -->
`a..b` is an inclusive range, here 4 up to 9.


<!-- focus: 8 -->
A value may be a constant expression such as `5 + 5`.

<!-- focus: 9 -->
`case _` matches anything that is left.

<!-- example: match-basic -->

## The compiler checks you covered everything

A `match` must handle every possible value. If it cannot tell that it does,
the compiler warns you with the values that are missing and offers the
`case _` fix. The warning does not stop the program, but leaving it in
means some values do nothing. A case that can never be reached, because the
cases above it already match everything it would, gets a warning of its own
(W0111).

```drachm
uint8 n = 3;
match (n) {
    case 0 { io::outln("zero"); }
    case 1 { io::outln("one"); }
}
```

```text title="drachm check"
warning[W0110]: this `match` does not handle `2..255`
  --> main.drh:5:5
   |
 5 |     match (n) {
   |     ^^^^^^^^^ `n` is `uint8`
   |
 help: add a `case` for them, or `case _ { }` for everything else

main.drh: 1 warning
```

## Leaving an if with skip

`break` leaves loops. `skip` is the same idea for `if` blocks: it jumps out of
the `if` it is in, so you do not have to nest the rest of the code in an `else`.
`skip;` leaves the outermost `if` around it and `skip 2;` leaves two levels.
Loops act as walls, and a `skip` never leaves the loop body it is in.

<!-- focus: 7-14 -->
With `hp` at 5, the inner `if` is true, so `skip;` leaves the outer `if` as well.
"healthy" never prints, and the program continues after the outer block.

<!-- focus: 17-25 -->
`skip to next;` leaves only the current branch and goes on to the next
`else if` or `else` of the same chain. Here that prints "b" even though
`hp > 3` was already true.

<!-- example: skip -->

```text title="drachm check, with a skip outside any if"
error[E0012]: `skip` is not inside an `if` or a `match`
  --> main.drh:5:5
   |
 5 |     skip;
   |     ^^^^^ nothing to skip here
   |
 help: `skip` leaves `if` blocks; use `return` to leave the function
```

### Your turn: letter grades

Write `string grade(uint32 score)` with a `match`: 90 to 100 is "A", 80 to 89 is
"B", 70 to 79 is "C", and anything else is "F". The `case _` is how you avoid
the warning.

```drachm title="starter"
in io;

// your code here

none main() {
    io::outln(grade(95));
    io::outln(grade(82));
    io::outln(grade(40));
}
```

```text title="expected output"
A
B
F
```

<!-- solution-of: solution-7-1 -->
```drachm solution
in io;

string grade(uint32 score) {
    match (score) {
        case 90..100 { return "A"; }
        case 80..89 { return "B"; }
        case 70..79 { return "C"; }
        case _ { return "F"; }
    }
}

none main() {
    io::outln(grade(95));
    io::outln(grade(82));
    io::outln(grade(40));
}
```

### Your turn: weekdays

Loop over the days 1 to 7. Use a `match` with a list so that days 6 and 7 print
`weekend` and every other day prints `weekday`, each as `day: kind` using an
`f"..."` string.

```drachm title="starter"
in io;

none main() {
    for (uint32 day of 1..7) {
        // your code here
    }
}
```

```text title="expected output"
1: weekday
2: weekday
3: weekday
4: weekday
5: weekday
6: weekend
7: weekend
```

<!-- solution-of: solution-7-2 -->
```drachm solution
in io;

none main() {
    for (uint32 day of 1..7) {
        match (day) {
            case 6, 7 { io::outln(f"${day}: weekend"); }
            case _ { io::outln(f"${day}: weekday"); }
        }
    }
}
```

Next: [Lesson 8: Ownership](lesson-8-ownership.md).
