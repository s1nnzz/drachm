---
title: "Lesson 18: Lambdas and concurrency"
description: Lambdas and function types, then threads, channels, mutexes and async functions.
status: tentative
---

# Lesson 18: Lambdas and concurrency

The compiler can't run this lesson yet, and the design is still in outline, so
the code shows the planned shape and the names may change.

**Goal:** pass a piece of code to a function, and meet the tools for running
code at the same time.

## Lambdas

A lambda is a function without a name, written `(params) => expr` or
`(params) => { block }`. A function type mirrors it: `(string) => none` is a
function that takes a string and returns nothing.

<!-- focus: 1 -->
`f` is a parameter whose type is a function.

<!-- focus: 2 -->
`each` calls it once per item.

<!-- focus: 5 -->
The lambda is passed in place.

```drachm
none each(string[?] items, (string) => none f) {
    for (string s of items) { f(s); }
}

each(names, (string n) => io::outln(n));
```

A lambda borrows what it uses from the code around it. Writing `move` before it
makes it take ownership instead. A lambda is a wall for `skip`, like a loop:
inside one, `skip` counts only the `if` levels inside the lambda.

### Your turn: a lambda

Call `each` with a lambda that prints each name followed by an exclamation
mark.

```drachm title="starter"
in io;

none each(string[?] items, (string) => none f) {
    for (string s of items) { f(s); }
}

none main() {
    string[?] names = ["Illy", "Sam"];
    // your code here
}
```

```text title="intended output"
Illy!
Sam!
```

```drachm solution
in io;

none each(string[?] items, (string) => none f) {
    for (string s of items) { f(s); }
}

none main() {
    string[?] names = ["Illy", "Sam"];
    each(names, (string n) => io::outln(n + "!"));
}
```

## Threads

Both OS threads and async functions are planned. A thread starts with
`thread::spawn`, and `move` hands it what it uses, so two threads never share a
value by accident.

```drachm
thread::spawn(move () => { prepare(); work(); });
```

## Channels and mutexes

The standard library has `Channel<T>` for sending values from one thread to
another, and `Mutex<T>` for sharing one value that threads take turns on. Data
races are meant to be compile errors. The methods of both types are not
designed yet.

```drachm
Channel<uint32> ch;
Mutex<uint32> score = 0;
```

## Async

An `async` function is waited for with `await`, written after the call like a
method, so it follows the same `::` and `->` rule as lesson 5. The async runtime
is part of the standard library.

<!-- focus: 1 -->
`async` goes before the return type.

<!-- focus: 3 -->
`::await` waits and gives the string.

<!-- focus: 4 -->
Chain further calls with `->`.

```drachm
async string fetch(string url) { /* ... */ }
async none main() {
    string page = fetch(url)::await;
    uint64 n = fetch(url)::await->len();
}
```

The page [Lambdas and concurrency](design-concurrency.md) lists what is still
open. That is the end of the lessons; the Reference pages cover what the
compiler builds today, starting with [Types and numbers](reference-types.md).
