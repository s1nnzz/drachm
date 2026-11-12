---
title: Concurrency
description: What the design says about threads, channels, mutexes and async functions, and how ownership is meant to prevent data races.
status: tentative
---

# Concurrency

Concurrency is designed in outline only. The compiler builds none of it, the
standard library API is not designed, and every name below may change. This guide
says what is decided, and stops where the design does. The code blocks show
the intended shape, not something you can run.

## Two tools

Drachm plans both **OS threads** and **async functions**.

- A thread is a separate flow of work that the operating system schedules. It
  suits work that keeps a core busy.
- An async function is one that can pause while it waits, which suits work that
  mostly waits, such as a network call. The async runtime is built into the
  standard library, and an `async` function becomes a state machine.

Which one a given program needs, and how they work together, is not written
down.

## Threads

A thread starts with `thread::spawn` and a lambda. A lambda borrows what it uses
unless it is written `move`, which gives it ownership. For a thread, `move` is
the form to reach for.

```drachm
thread::spawn(move () => { prepare(); work(); });
```

## Why ownership is the point

The language already says who owns every value: one place at a time, with many
shared borrows or one `&mut`, never both (see [Moves and copies](reference-ownership.md)
and [Where a borrow comes from](reference-lives.md)). The design is that **data races are
compile errors**, and that those same rules are what make it so:

- a value moved into a thread is no longer usable by the thread that sent it, so
  the two never touch it at once;
- two threads cannot both hold a `&mut` to the same value.

The PLAN notes that this needs some way to mark which types are safe to send
between threads. Whether that marker is visible in the language or hidden is an
open question for the owner, so this guide does not show one.

## Sharing deliberately

The standard library is to have two types for sharing:

- `Channel<T>` sends values from one thread to another.
- `Mutex<T>` holds one value that threads take turns on.

```drachm
Channel<uint32> ch;
Mutex<uint32> score = 0;
```

Their methods are not designed. Rule of thumb from the ownership model: prefer
sending a value over a channel, so one thread owns it at a time, and use a mutex
only for something that really has to be shared.

## Async

An `async` function goes before the return type, and `await` is written after
the call like a method, so it follows the same `::` and `->` chain rule as any
other call.

<!-- focus: 1,3,4 -->

```drachm
async string fetch(string url) { /* ... */ }
async none main() {
    string page = fetch(url)::await;
    uint64 n = fetch(url)::await->len();
}
```

## What is still open

- The API of `thread`, `Channel<T>`, `Mutex<T>` and the async runtime.
- How thread safety of types is expressed, or kept hidden.
- How threads and async functions mix.

See [Lambdas and concurrency](design-concurrency.md) for the syntax that is
written down, and [Lesson 18](lesson-18-concurrency.md) for lambdas.
