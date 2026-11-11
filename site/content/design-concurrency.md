---
title: Lambdas and concurrency
description: Lambdas, function types, threads, channels, mutexes and async functions.
status: tentative
---

# Lambdas and concurrency

Lambdas and concurrency are designed in outline, and the API details are still
open. None of it builds yet, and the names below may change.

## Lambdas and function types

A lambda is `(params) => expr` or `(params) => { block }`. It captures by
borrowing unless it is written `move`, which takes ownership. A function type
mirrors the lambda: `(string) => none`.

<!-- focus: 1,2,6 -->

```drachm
none each(string[?] items, (string) => none f) {
    for (string s of items) { f(s); }
}

each(names, (string n) => io::outln(n));
```

A lambda is a wall for `skip`, like a loop: `skip` inside one counts only the
`if` levels inside it.

## Threads

Both OS threads and async functions are planned. A thread is started with
`thread::spawn`, and `move` hands it what it uses.

```drachm
thread::spawn(move () => { prepare(); work(); });
```

## Channels and mutexes

The standard library has `Channel<T>` for sending values between threads and
`Mutex<T>` for sharing one. Data races are compile errors.

```drachm
Channel<uint32> ch;
Mutex<uint32> score = 0;
```

## Async

An `async` function is waited for with `await`, which is written after the
call like a method, so it follows the same `::` and `->` chain rule. The async
runtime is built into the standard library.

<!-- focus: 1,3,4 -->

```drachm
async string fetch(string url) { /* ... */ }
async none main() {
    string page = fetch(url)::await;
    uint64 n = fetch(url)::await->len();
}
```
