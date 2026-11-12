---
title: "Lesson 14: Errors"
description: failure types, fail, fails, maybe and fails with, catch-alls and because.
status: not-implemented
---

# Lesson 14: Errors

The compiler can't run this lesson yet; the code shows the planned design.

**Goal:** write functions that can go wrong, and say at every call what happens
when they do.

## No Result type

Drachm has no `Result` type and no `?` operator. A function that can go wrong
says so in its signature with `fails`, and the caller has to say what happens
on failure. Faults such as dividing by zero are not failures: they stop the
program with a message and exit code 101.

## A failure type

A failure type is its own kind of declaration. Each variant has a message in
quotes, may carry data, and may have an exit code used when the failure escapes
`main`. The default exit code is 1.

<!-- focus: 2 -->
A plain variant with a message and an exit code.

<!-- focus: 3 -->
A variant with data, which the message can use as `${k}`.

<!-- focus: 4 -->
The exit code is optional.

```drachm
failure ToolError {
    NoInput              "no input file"          exit 2,
    BadConfig(string k)  "bad config key ${k}"    exit 3,
    LoadFailed           "couldn't load save",
}
```

Only `failure` types can follow `fails`.

## Raising a failure

`fails` after the parameters names the failure types a function may raise, and
`fail` raises one and leaves the function.

<!-- focus: 1 -->
`fails ParseError` is part of the signature.

<!-- focus: 2 -->
`fail Empty;` raises the `Empty` variant.

```drachm
uint32 parse(string s) fails ParseError {
    if (s::isEmpty()) { fail Empty; }
    return 42;
}
```

## Saying what happens at the call

A call to a failing function must say what to do. There are three ways.

<!-- focus: 1 -->
`maybe` gives a fallback value, any expression. It is evaluated only when needed.

<!-- focus: 2 -->
A bare `fails` passes the failure on to the caller, which must declare it too.

<!-- focus: 3-6 -->
`fails with` runs a handler. The handler's `maybe` gives the replacement value.

```drachm
uint32 a = parse(input) maybe 0;
uint32 b = parse(input) fails;
uint32 c = parse(input) fails with (ParseError e) => {
    io::outln(f"bad input: ${e}");
    maybe 0;
};
```

A call with none of the three is a compile error by default.

### Your turn: a fallback

`check` fails when the number is over 100. Add `fails TooBig` to its
signature, then call it so that a failure gives 0.

```drachm title="starter"
in io;

failure TooBig {
    Over "number is over 100",
}

uint32 check(uint32 n) {
    if (n > 100) { fail Over; }
    return n;
}

none main() {
    // your code here: print check(7), then check(500) with 0 as the fallback
}
```

```text title="intended output"
7
0
```

```drachm solution
in io;

failure TooBig {
    Over "number is over 100",
}

uint32 check(uint32 n) fails TooBig {
    if (n > 100) { fail Over; }
    return n;
}

none main() {
    io::outln((check(7) maybe 0)::text());
    io::outln((check(500) maybe 0)::text());
}
```

## Handlers, catch-alls and causes

A function may list several failure types, and each can have its own
`fails with`. A `return` inside a handler leaves the outer function, not the
handler. `fails with (any e)` handles any failure, and after specific handlers
`otherwise` handles whatever they did not name.

```drachm
Save s = load(p) fails with (FileMissing e) => { maybe Save::empty(); }
                 fails with (ParseError e)  => { return; };
```

`fail X because e` raises a new failure and keeps the one that caused it. When
it reaches the end of the program, both are printed.

<!-- focus: 2 -->
The handler raises `LoadFailed` and keeps the missing-file failure as its cause.

```drachm
string t = fs::read(p) fails with (FileMissing e) => {
    fail LoadFailed because e;
};
```

```text title="intended output"
couldn't load save
  because: no such file "save.dat"
```

### Your turn: report and recover

Using the same `check`, handle the failure with `fails with`: print the
message of the failure, then give 0.

```drachm title="starter"
in io;

failure TooBig {
    Over "number is over 100",
}

uint32 check(uint32 n) fails TooBig {
    if (n > 100) { fail Over; }
    return n;
}

none main() {
    // your code here: check(500) with a handler that prints ${e} and gives 0
}
```

```text title="intended output"
number is over 100
```

```drachm solution
in io;

failure TooBig {
    Over "number is over 100",
}

uint32 check(uint32 n) fails TooBig {
    if (n > 100) { fail Over; }
    return n;
}

none main() {
    uint32 n = check(500) fails with (TooBig e) => {
        io::outln(f"${e}");
        maybe 0;
    };
}
```

Settings such as `unhandled_fail` can relax the rule that every call must say
something; they live in [Config](design-config.md). The whole design is on
[Errors with fails and maybe](design-errors.md). Next:
[Lesson 15: Generics and interfaces](lesson-15-generics.md).
