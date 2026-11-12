---
title: Errors with fails and maybe
description: failure types, fail, maybe, fails with, catch-alls and chained causes. There is no Result type.
---

# Errors with fails and maybe

Drachm has no `Result` type and no `?` operator. A function that can go wrong
says so with `fails`, and the caller must say what happens on failure. The
rules, with examples that run, are on [Failures](reference-errors.md). Only the
`crash` setting at the end is not built yet.

## A function that can fail

`fails` after the parameters names the failure types the function may raise.
`fail` raises one and leaves the function. Runtime faults such as division by
zero are not failures: they panic with a message and exit code 101.

<!-- focus: 1,2 -->

```drachm
uint32 parse(string s) fails ParseError {
    if (s::isEmpty()) { fail Empty; }
    return 42;
}
```

A function may list several failure types: `Save load(string p) fails FileMissing, ParseError`.
If more than one of them has a variant called `Empty`, write
`fail ParseError::Empty;`.

## Failure types

A failure type is its own kind of declaration. Each variant has a message in
quotes, may carry data, and may have an exit code used when the failure escapes
`main` (the default is 1).

<!-- focus: 2,3 -->

```drachm
failure ToolError {
    NoInput              "no input file"          exit 2,
    BadConfig(string k)  "bad config key ${k}"    exit 3,
    LoadFailed           "couldn't load save",
}
```

Only `failure` types can follow `fails`. Printing a failure with `${e}` gives
its message.

## Handling a failure at the call

A call to a failing function must say what to do. The three forms:

<!-- focus: 1,2,3 -->

- `maybe` gives a fallback value. The fallback is any expression, and it is
  evaluated only when needed.
- A bare `fails` passes the failure on to the caller, which must declare it too.
- `fails with` runs a handler.

```drachm
uint32 a = parse(input) maybe 0;
uint32 b = parse(input) fails;
uint32 c = parse(input) fails with (ParseError e) => {
    io::outln(f"bad input: ${e}");
    maybe 0;                         // the replacement value
};
```

## Several handlers

One `fails with` per failure type, written one after the other. A `return`
inside a handler leaves the outer function, not the handler.

<!-- example: fails-handlers -->

## Catch-alls

`fails with (any e)` handles any failure. After specific handlers, `otherwise
(any e)` handles whatever they did not name; the example above uses both.

## Chaining a cause

`fail X because e` raises a new failure and keeps the one that caused it. When
it reaches the end of the program, both are printed.

<!-- example: fails-cause -->

```text title="if LoadFailed leaves main"
error: couldn't load save
  because: no such file save.dat
```

## When nothing is said
<!-- status: not-implemented -->

A call to a failing function with none of `maybe`, `fails` or `fails with` is a
compile error by default (that part is built: E0544). The `unhandled_fail` setting in
[Config](design-config.md) can change that to `crash`, which stops the program
with a message when the call fails. Scripts use `crash` by default.
