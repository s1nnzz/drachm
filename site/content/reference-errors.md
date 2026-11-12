---
title: Failures
description: failure types, fails, fail, maybe and a bare fails at the call, because, ConvertError, and what happens when a failure leaves main.
---

# Failures

A function that can go wrong says so with `fails`, and every call says what
happens when it does. There is no `Result` type and no unwinding: a failing
function returns its failure as an ordinary value, and every value alive in it is
dropped on the way out, exactly as for a `return`. The
[errors lesson](lesson-14-errors.md) introduces this with exercises, and
[Error handling without exceptions](guide-errors-handling.md) is about designing
with it. Handlers (`fails with`), `otherwise`, `any e` and `ignore failure` are
not built yet; they are marked on those pages.

## Failure types

A `failure` is its own kind of declaration. Each variant has a name, optional
values in parentheses, a message, and optionally `exit N`. The message is an
ordinary string, so one that uses the variant's values is an `f"..."` string,
and `${name}` must name one of the variant's own values (E0551). Exit codes run
from 0 to 255 (E0552), and 1 is used when none is written. A failure needs at
least one variant (E0554), and its variants have distinct names (E0520).

The values of a variant are numbers, bools and strings for now (E0901 for
anything else). A failure value is an ordinary value that moves and is dropped
like a struct that owns a `string`; it cannot be copied.

<!-- example: fails-basic -->

## fails and fail

`fails` after the parameters lists the failure types the function may end with,
in either order with `lives`. A name that is not a failure type is E0540, and a
type listed twice is E0554. Functions, methods and operator methods can fail;
a constructor or `drop()` that fails is E0901.

`fail X;` leaves the function with variant `X`; `fail X(values);` gives it
values. The name is searched in every listed failure type. If two of them have
a variant of that name it is E0543, and `fail ParseError::Empty;` names the type.
A variant none of the listed types has is E0541, and `fail` in a function with
no `fails` at all is E0542. `return` in a failing function is a success, and so
is reaching the end of a failing `none` function.

## Saying what happens at the call

A call of a function that can fail, with nothing after it, is an error (E0544):
the message shows the two ways out that are built today.

`call maybe value` is the call's result, or the value when it failed. The value
is converted to the result type, and it is evaluated only on failure. It is a
single value or a parenthesised expression: `parse(s) maybe 0 + 1` is E0534, and
`(parse(s) maybe 0) + 1` is what is usually meant. A call that returns `none` has
no value to fall back to.

<!-- example: fails-maybe -->

A bare `call fails` passes the failure up: the result is the call's result, or
the caller returns the failure. The caller must list every failure type the
callee lists (E0548 names the first missing one). Every value alive in the
caller, including the temporaries of the statement, is dropped as for a
`return`.

<!-- example: fails-propagate -->

A failing call can be an argument, an operand, an initialiser or a loop
condition, as long as it carries its own `maybe` or `fails`. For an operator
method that fails, put the operation in parentheses first:
`(a == b) maybe false`. `a != b` with an `==` that fails is not built yet (E0901);
write `!((a == b) maybe false)`.

## Reading text as a number

`text::to<T>()` fails with the built-in `ConvertError`, which has the variants
`Empty` (nothing but spaces), `NotANumber` and `OutOfRange` (a number that does
not fit `T`). The reading rules are the forgiving ones from
[Input and output](reference-io.md). So the call needs a `maybe` or a `fails`:

```drachm
uint32 n = line::to<uint32>() maybe 0;
```

You cannot declare another item called `ConvertError`.

## Leaving main

`main` may say `fails`. A failure that leaves it is printed to standard error
as `error: ` and the message, then one `  because: ` line for each cause, and
the program exits with the failure's code (1 by default). If `main` returns a
number and fails, the failure's code wins; on success the number is the exit
code. Everything alive is dropped before the program ends.

<!-- example: fails-main -->

```text title="a failure leaving main"
error: overslept by 60 minutes
```

## Because

`fail X because e;` raises `X` and keeps the failure `e` (a failure value, E0549
otherwise) as its cause: `e` is moved, and only its message is kept. The report
above lists it:

```text title="fail LoadFailed because cause;"
error: couldn't load save
  because: no such file save.dat
```

A cause is text, and it is not inspectable. Failure values can only be named
by a local variable until handlers are built, so the cause is a failure you made
yourself.

## What is not built yet

- Handlers: `fails with (E e) => { ... }`, several in a row, `otherwise`,
  `any e`, `${e}`, `e::message()`, `fail e;`.
- `ignore failure`, `fails with none`.
- `match` on a failure, `copy` of a failure, a constructor or `drop()` that fails.
- The `crash` setting for `unhandled_fail` (step 7).
