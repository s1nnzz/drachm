---
title: Error handling without exceptions
description: How failures work in Drachm and when to use maybe, a bare fails, or a handler. No Result type, no unwinding.
---

# Error handling without exceptions

This guide is about designing code that can go wrong. `failure`, `fails`, `fail`,
`maybe` and a bare `fails` on calls, and `because`, are built; see
[Failures](reference-errors.md) for the rules. Handlers (`fails with`),
`otherwise`, `any e` and `ignore failure` are designed but not built, and the
sections about them are marked. The optional `T?` sibling of `maybe` is on
[Designing with enums and optionals](guide-enums-optionals.md).

Drachm has two kinds of things going wrong, and they are handled differently.

- A **fault** is a bug: dividing an integer by zero, an index past the end of a
  list. The program stops with a message and exit code 101. You do not handle
  faults.
- A **failure** is something that can happen in a correct program: a missing
  file, text that is not a number. A function says it can fail, and every call
  to it says what happens when it does.

## Declare the failures you can have

A failure type is its own kind of declaration. Each variant has a message, may
carry data, and may have an exit code that is used when the failure escapes
`main`. Messages are ordinary strings, so a message that uses the variant's
data is an `f"..."` string.

<!-- focus: 2,3,4 -->

```drachm
failure ToolError {
    NoInput              "no input file"            exit 2,
    BadConfig(string k)  f"bad config key ${k}"     exit 3,
    LoadFailed           "couldn't load save",
}
```

Only `failure` types can follow `fails`, and an `enum` cannot. Keep one failure
type per area of your program (files, parsing, the network) rather than one for
everything. The caller sees the type, and handles the kinds it cares about.

## Raise one with fail

`fails` after the parameters lists what the function may raise, and `fail`
raises one and leaves the function.

<!-- focus: 1,2,6 -->

```drachm
uint32 parse(string s) fails ParseError {
    if (s::isEmpty()) { fail Empty; }
    return 42;
}

Save load(string p) fails FileMissing, ParseError { /* ... */ }
```

`fail Empty;` looks for `Empty` in every failure type the function lists. If
two of them have a variant called `Empty`, name the type: `fail ParseError::Empty;`.

## Every call says what happens

<!-- example: fails-maybe -->

Calling a function that can fail, with nothing after the call, is a compile
error by default. There are three answers, from least to most work.

<!-- focus: 1,2,3 -->

```drachm
uint32 a = parse(input) maybe 0;
uint32 b = parse(input) fails;
uint32 c = parse(input) fails with (ParseError e) => {
    io::outln(f"bad input: ${e}");
    maybe 0;
};
```

- `maybe` gives a **fallback**. Use it when a sensible default exists and the
  reason does not matter.
- A bare `fails` **passes the failure up**. Use it when this function cannot do
  anything useful about it. The enclosing function must list the failure in its
  own `fails` (and so must `main`, if the failure reaches it).
- `fails with` **handles** it (not built yet). Use it when you can recover, log, or turn the
  failure into a different one.

The fallback after `maybe` is only run when the call fails. It is a single value
or a parenthesised expression; anything longer needs parentheses, and the
compiler says so.

<!-- focus: 1,2,3 -->

```drachm
uint32 n = (parse(s) maybe 0) + 1;     // the call and its fallback, then + 1
uint32 m = parse(s) maybe (n * 2);     // a longer fallback in parentheses
Save d = load(p) maybe makeDefault(); // a single call is fine too
```

`parse(s) maybe 0 + 1` is an error: it is not clear which you meant.

## Handling one kind of failure
<!-- status: not-implemented -->

Write the handler for the failure type, or for one variant of it. Several
handlers in a row each take their own kind, and a `return` inside a handler
leaves the outer function, not the handler.

<!-- focus: 1,2 -->

```drachm
Save s = load(p) fails with (FileMissing e) => { maybe Save::empty(); }
                 fails with (ParseError e)  => { return; };

uint32 v = read(p) fails with (BadConfig e) => { maybe 0; };
```

Inside a handler, `maybe value;` is the replacement for the call's result,
`return` leaves the function, and `fail ...;` raises a failure of the outer
function. For a call that returns a value, a handler ends one of those three
ways.

## Looking at the failure you caught
<!-- status: not-implemented -->

The name in a handler (`e` above) is the failure. It prints with `${e}`, which
gives the message and, for a failure with causes, the whole chain. It also has
members:

- `e::message()` is the message on its own.
- `e::exit()` is the exit code the failure carries.
- `e::cause()` is what this failure was caused by, if anything.

What exactly each member returns is not written down yet.

To handle everything, use `any e`. A catch-all can also be written after
specific handlers with `otherwise`, which takes whatever they did not name.
Re-raise what you caught with `fail e;`, as long as the enclosing function
lists it.

<!-- focus: 1,2,5 -->

```drachm
Save s = load(p) fails with (FileMissing e) => { maybe Save::empty(); }
                 otherwise (any e) => {
    io::outln(f"load: ${e::message()}");
    fail e;                      // pass it on; the function must list it
};
```

## Keeping the cause

`because` works today with a failure value you hold in a variable
(`fail LoadFailed because cause;`, see [Failures](reference-errors.md#because));
catching one with a handler and keeping it, as below, comes with handlers.

When you turn one failure into another, keep the first as the cause with
`because`. Nothing is lost on the way up, and the report at the end shows both.

<!-- focus: 1,2 -->

```drachm
string t = fs::read(p) fails with (FileMissing e) => {
    fail LoadFailed because e;
};
```

If a failure reaches the end of `main`, the program prints it to the error
stream with an `error:` prefix, then one `because:` line for each cause, and
exits with the failure's exit code.

```text title="intended output"
error: couldn't load save
  because: no such file "save.dat"
```

Prefer `because e` to dropping the cause: the message you raise should be about
what *this* function was doing, and the cause says what went wrong underneath.

## Exit codes

<!-- example: fails-main -->

Every failure has an exit code, 0 to 255, and 1 when none is written. A program
exits with it when a failure escapes `main`. If `main` returns a number as well
(`sint32 main()`), the failure's code wins on failure and the number is the exit
code on success.

```drachm
sint32 main() fails ToolError {
    string cfg = load(path) fails;     // BadConfig exits with 3
    return 0;
}
```

## Deliberately ignoring a failure
<!-- status: not-implemented -->

Sometimes a failure really does not matter, such as closing a file you are done
with. Say so out loud, in either spelling:

<!-- focus: 1,2 -->

```drachm
fs::close(f) fails with none;
fs::close(f) ignore failure;
```

The call is still marked. Nobody reading the code has to wonder whether the
failure was forgotten.

## Text to number can fail

Reading a number from text can fail, so `::to<T>()` on text is a failing call
with a built-in `ConvertError`. It has one variant for each cause, so a program
can tell them apart: `Empty`, `NotANumber` and `OutOfRange`. Today, bad text
stops the program with exit code 101 (see [Input and output](reference-io.md));
failures replace that.

<!-- focus: 1,2,3 -->

```drachm
uint32 a = line::to<uint32>() maybe 0;
uint32 b = line::to<uint32>() fails with (ConvertError e) => {
    io::outln(f"not a number: ${e}");
    maybe 0;
};
```

## Failing constructors and operators

A failing function can be anything with a name: a constructor, an operator
method, a `drop()`. A constructor that checks its argument is the usual case:

<!-- focus: 1,2 -->

```drachm
struct Port {
    uint32 n;
    Port(uint32 n) fails BadPort {
        if (n > 65535) { fail BadPort; }
        self.n = n;
    }
}

Port p = 8080 maybe Port::default();
```

An operator that fails is handled the same way, around the whole expression:
`(p == q) maybe false`. A failing `drop()` has no caller to hand the failure to
when it runs at the end of a scope, so avoid it. Until the owner decides
otherwise, a failure that escapes an automatic `drop()` is reported the way an
escape from `main` is.

## Why there is no Result type and no unwinding

Drachm could have copied other languages here, and chose not to.

**No `Result` and no `?`.** A `Result<T, E>` is a value you can store, pass
around and forget to look at. A failure is not a value you carry about. It is a
property of a call, and the call site is where the language insists you decide.
There is no wrapper to unwrap and no operator to sprinkle on every line; the
words `maybe`, `fails` and `fails with` are the whole vocabulary.

**No unwinding.** A failure does not jump through the stack looking for a
catcher. It is designed to be returned like a result: a failing function
reports success or failure on its normal way out, and the caller tests it right
after the call. Every local on the way is dropped by the ordinary return path,
there is no hidden state, and the cost on the success path is one test.

## Which answer to pick

| Situation | Use |
|---|---|
| A default makes sense, and the reason does not matter | `maybe` |
| This function cannot help, the caller might | bare `fails` |
| You can recover, or want to add context | `fails with` |
| Turning a low-level failure into your own | `fail X because e;` |
| It really does not matter | `ignore failure` |
| A bug, not a condition | no failure; let it fault |
