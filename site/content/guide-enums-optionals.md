---
title: Designing with enums and optionals
description: Make invalid states unwritable with enums, take payloads apart safely, and use T? instead of sentinel values.
---

# Designing with enums and optionals

Enums, payloads, `match` and the optional type `T?` are built, and the examples
run. For the rules, see [Enums](reference-enums.md); this guide is about when to
reach for them.

## Make invalid states unwritable

A common first design is a struct with a flag for every situation:

```drachm
struct Connection {
    bool isOpen;
    bool hasFailed;
    string peer;       // only meaningful when open
    string reason;     // only meaningful when failed
}
```

Nothing stops `isOpen` and `hasFailed` both being true, and nothing says that
`peer` is garbage when the connection is closed. Every function that touches it
has to remember the rules.

An enum states the rules in the type. Each variant carries exactly the data that
makes sense for it, and no other combination can be written.

<!-- focus: 3-8 -->

```drachm
enum Connection {
    Closed,
    Connecting(uint32 attempt),
    Open(string peer, uint32 port),
    Failed(string reason),
}
```

A `Connection` is exactly one of the four, and the `reason` of a failed
connection cannot be read from an open one, because it is not there.

## Pick variants that carry data

Ask what is true in each situation, and give the variant that. A variant with
nothing to say is written bare (`Closed`). A variant may carry several named
values, and each has a type. Payloads can be strings and structs too; an enum
that holds one of them moves instead of copying, like a `string` does.

Keep the variants to the real states. A variant such as `Unknown` that means "I
could not be bothered" brings the problem back: every `match` has to handle it.

## Taking a value apart

`match` is how you reach the data, and the compiler warns when a variant is
missing, so adding a variant later points you to everything that needs
updating. The cases below use a guard (`if n > 3`) and a binding for each
payload.

A payload that owns memory, such as a `string`, must say what the pattern does
with it. `take string reason` moves it out and uses the matched value up;
`copy string peer` copies it and leaves the value whole. A bare `string peer`
is an error with both fixes offered. Numbers and other small values bind
plainly, as `uint32 n` does.

<!-- example: guide-enums-states -->

`describe` borrows the connection, so it has to `copy` the strings it reads.
`finish` takes the connection by value and moves the reason out of it.
Pick `copy` when the value is still needed afterwards and `take` when it is
being handed on.

## Comparing

`==` and `!=` work on enums whose variants carry nothing, like `Direction`
above. An enum with data in some variant is compared by taking it apart with
`match`, because "equal" would otherwise have to guess what you mean for each
payload. Behaviour on an enum is written as ordinary functions that take it,
not methods inside it.

## When a value might not be there

The most common "variant" of all is "there is no value". Many languages use a
sentinel for it: `-1` for "not found", an empty string for "no name", zero for
"no age". The sentinel is a legal value of the type, so nothing stops you using
it as a real one:

```drachm
sint32 findIndex(&string text, &string key) { /* ... */ }   // -1 when missing
```

`T?` says it in the type. The value is either a `T` or `none`, and the compiler
will not let you use it as a `T` until you have dealt with the `none`.

<!-- example: guide-enums-find -->

A `T` becomes a `T?` automatically wherever a `T?` is expected: in a
declaration (`uint32? x = 5;`), a `return`, an assignment or an argument. You
never write the wrapping.

## Asking whether there is a value

The test is written in words, after the optional: `age has value`, `age is
empty`, as the example above shows. `x == none` is not the test; `x is empty` is. `maybe` unwraps with a fallback,
the same word that gives a call a fallback in
[Error handling](guide-errors-handling.md). To use the value itself, `match` on
the optional, with a `case` for the value and one for `none` (also shown above).

Optionals also appear in the standard library: looking up a missing key in a
`Map` gives `none`, and a variable declared as `uint32? age;` without a value
starts as `none`.

## Choosing between them

| You have | Use |
|---|---|
| One of several kinds of thing, each with its own data | an `enum` |
| A value that may be missing, and nothing more to say | `T?` |
| A value that may be missing, and the caller should know why | a [failure](guide-errors-handling.md) |
| Several yes/no flags that depend on each other | an `enum` instead of the flags |
| A value that is always there | a plain `T` |

Rule of thumb: if the "missing" case has a reason worth reporting, it is a
failure. If it is just "nothing here", it is `T?`. If there are more than two
shapes, it is an enum. `T?` is an enum too, with the two variants `Some` and
`none`, so everything you know about `match` applies to it.
