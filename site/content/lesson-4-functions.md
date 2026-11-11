---
title: "Lesson 4: Functions"
description: Writing functions with parameters and return values, none, mut parameters, and recursion.
---

# Lesson 4: Functions

**Goal:** split a program into named pieces that take values and give one back.

## Writing a function

You have been writing one function already: `main`. Any other function looks the
same. The return type comes first, then the name, then the parameters in
parentheses, each with its own type. `return` hands a value back and ends the
function.

<!-- focus: 3-5 -->
`add` takes two `uint32` values and returns a `uint32`. Because the return type
is written first, you can see what a function gives you before you read how.

<!-- focus: 7-9 -->
A function that returns `bool` is a good way to give a question a name, which
makes the `if` that uses it easy to read.

<!-- focus: 11-13 -->
A function that returns `none` gives nothing back, so it is called for what it
does, here printing. Functions can be written in any order, and `main` can call
all of them.

<!-- focus: 15-23 -->
Parameters are fixed inside the function unless you write `mut`. A `mut`
parameter is the function's own copy, so `countDown` can change `n` without
touching the caller's value.

<!-- example: functions -->

## Every path must return

If a function promises a value, every way through it has to produce one. In
`sign` below, the case where `n` is 0 falls out of the bottom with nothing to
return, and the compiler refuses the program. Add a final `return` or an
`else` and it passes. A loop that never ends, such as `for (;;)`, needs no
`return` after it.

```drachm
sint32 sign(sint32 n) {
    if (n < 0) {
        return -1;
    } else if (n > 0) {
        return 1;
    }
}
```

```text title="drachm check"
error[E0105]: function `sign` can finish without returning a value
  --> main.drh:3:1
   |
 3 | sint32 sign(sint32 n) {
   | ^^^^^^^^^^^^^^^^^^^^^ promises to return `sint32`
   |
 help: add a `return` at the end, or to every branch
```

## A function can call itself

A function may call itself, as long as something stops it. The factorial of
`n` is `n` times the factorial of `n - 1`, and the factorial of 1 is 1. Use
`uint64` for the numbers, because the answers grow quickly. Mixing in a
`uint32` would give you the lossy-conversion warning from lesson 2.

```drachm
uint64 factorial(uint64 n) {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}
```

### Your turn: square it

Write `uint32 square(uint32 n)` so that the two calls in `main` print 81 and
144.

```drachm title="starter"
in io;

// your code here

none main() {
    io::outln(square(9)::text());
    io::outln(square(12)::text());
}
```

```text title="expected output"
81
144
```

<!-- solution-of: solution-4-1 -->
```drachm solution
in io;

uint32 square(uint32 n) {
    return n * n;
}

none main() {
    io::outln(square(9)::text());
    io::outln(square(12)::text());
}
```

### Your turn: factorial

Write the recursive `factorial` from above yourself, using `uint64` for both
the parameter and the result, and print the factorials of 5 and 10.

```drachm title="starter"
in io;

// your code here

none main() {
    io::outln(factorial(5)::text());
    io::outln(factorial(10)::text());
}
```

```text title="expected output"
120
3628800
```

<!-- solution-of: solution-4-2 -->
```drachm solution
in io;

uint64 factorial(uint64 n) {
    if (n <= 1) {
        return 1;
    }
    return n * factorial(n - 1);
}

none main() {
    io::outln(factorial(5)::text());
    io::outln(factorial(10)::text());
}
```

Next: [Lesson 5: Strings](lesson-5-strings.md).
