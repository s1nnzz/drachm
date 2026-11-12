---
title: Thinking in match
description: Choosing by value with match, from numbers and text to enums with guards and nesting, and when match beats an if chain.
---

# Thinking in match

An `if` asks a yes/no question. A `match` asks "which of these is it?". Once
you start thinking in terms of the second question, a lot of branching code gets
shorter and safer, because the compiler can check that you have covered every
answer. This guide builds up from numbers and text to enums with payloads, then
shows `skip` inside a `match` and when to reach for `match` instead of `if`.
[Lesson 7](lesson-7-match-and-skip.md) and [Enums](reference-enums.md) have the
basics and the rules.

## Match on a value

A `match` looks at one value, tries each `case` from the top, and runs the first
one that fits. Cases never fall through into the next, so there is no `break`.

What a case can say depends on the type:

- a **number** can be one value, a list (`case 6, 7`), an inclusive range
  (`case 90..100`) or a constant expression (`case 10 + 20`);
- a **bool** is covered by `true` and `false`, with no `case _` needed;
- a **string** can be exact text, a list, or a pattern with `globbed`;
- `case _` matches anything that is left.

The matched value can be any expression, and it is worked out once.

<!-- focus: 4-9 -->
Ranges include both ends. A score of 100 is an A.

<!-- focus: 13-17 -->
A list shares one body between several values.

<!-- focus: 22-25 -->
A `bool` has two values, so two cases cover it.

<!-- focus: 29-34 -->
For text, a plain case is exact and `globbed` makes `*` a wildcard. See
[Working with text](guide-text.md#matching-text).

<!-- focus: 45-49 -->
The matched value here is `hp * 2`, and `10 + 20` is a constant expression.

<!-- example: guide-match-values -->

## Did you cover everything?

The compiler checks that a `match` handles every possible value. When it cannot
tell that, it warns, lists what is missing, and offers a fix. The warning does
not stop the build, but the unhandled values do nothing at run time, so treat
it as a bug report. Here a `uint8` is matched against only `0`:

```text title="match on a uint8 with only case 0"
warning[W0110]: this `match` does not handle `1..255`
  --> main.drh:8:5
   |
 8 |     match (n) {
   |     ^^^^^^^^^ `n` is `uint8`
   |
 help: add a `case` for them, or `case _ { }` for everything else
```

The same check works on the other types. A `bool` with only `true` says it is
missing `false`. A float can only be covered by `case _`, because NaN equals
nothing. Text can only be covered by `case _` too:

```text title="match on a float64 with only case 1.5"
warning[W0110]: this `match` on `float64` does not handle every value
   --> main.drh:19:5
    |
 19 |     match (f) {
    |     ^^^^^^^^^ there is no `case _`
    |
 note: on floats only `case _` handles every value: NaN is equal to nothing
 help: add `case _ { }` for the values no case handles
```

The opposite mistake is a case that can never run, because the cases above it
already take all of its values. That is W0111, and it is why **order matters**:
put the specific cases first.

```text title="case 0..9 above case 5"
warning[W0111]: this case can never run
  --> main.drh:7:9
   |
 7 |         case 5 { io::outln("five"); }
   |         ^^^^^^ every value it matches is handled by an earlier case
   |
 help: remove it, or move it above the case that handles its values
```

## Matching an enum

An enum is "one of these", which is exactly what `match` is for. Each `case`
names a variant, and the compiler lists the variants you forgot:

```text title="a Light enum with Red and Green handled"
warning[W0110]: this `match` does not handle `Yellow`
  --> main.drh:6:5
   |
 6 |     match (l) {
   |     ^^^^^ this variant is missing
   |
 help: add `case Yellow { }` or `case _ { }`
```

This is the real payoff. If you add a variant to the enum later, every `match`
on it without a `case _` starts warning until you handle it, so the compiler
finds the places you would otherwise forget. For that reason, prefer listing
variants over a catch-all `case _` when you want to be told about new ones.

A variant that carries values gives you a pattern: `Circle(float64 r)` matches
a circle and names its radius `r`. Use `_` for a value you do not need.

## Guards, lists, nesting and values

A pattern can say a lot more, and these four features combine freely. They all
appear in this program.

<!-- focus: 20-21 -->
A **guard** is an `if` after the pattern. The case runs only when the pattern
fits *and* the condition is true. A circle with a radius above 10 is "big", and
any other circle falls to the next case.

<!-- focus: 23 -->
A **value** inside a pattern must be equal. `Rect(2.0, float64 h)` matches only
rectangles that are exactly 2.0 wide, and binds the height.

<!-- focus: 24 -->
Guards can use any binding from the same pattern, here to find squares.

<!-- focus: 32-36 -->
Patterns **nest**. `Some(Circle(float64 r))` looks inside the box and then
inside the shape. A comma-separated list gives several patterns one body.

<!-- example: guide-match-enums -->

Two rules about guards. First, a case with a guard does not count as covering
its variant, because the guard may be false. Here the only `Circle` case has a
guard, so the compiler still reports the variant as missing:

```text title="case Circle(float64 r) if r > 1.0, with no plain Circle case"
warning[W0110]: this `match` does not handle `Circle`
   --> main.drh:16:5
    |
 16 |     match (s) {
    |     ^^^^^ this variant is missing
    |
 help: add `case Circle(float64 r) { }` or `case _ { }`
```

Second, put guarded cases before the plain case for the same variant, or the
plain one will catch everything first.

## take and copy: payloads that own memory

A number or `bool` payload binds plainly, because copying it is free. A payload
that owns memory, such as a `string`, needs you to decide what happens to it,
since `match` can either move it out of the enum or copy it:

- `take string body` moves the text out, and the matched value is used up.
- `copy string body` makes a copy, and the original stays whole.

Writing a bare `string body` is an error, and the message gives you both fixes:

```text title="case Text(string body)"
error[E0528]: binding `body` would take memory out of the matched value
   --> main.drh:10:26
    |
 10 |         case Text(string body) { io::outln(body); }
    |                          ^^^^ this payload is a `string`, which owns memory
    |
 help: write `take string body` to move it out (the matched value is used up), or `copy string body` to copy it
```

In the program above, `read` takes a `Msg` by value and uses `take`, so after
the call the caller no longer has the message. `peek` borrows it with `&Msg`
and uses `copy`, which is why `peek(&m)` can be called twice and `read(m)`
afterwards works. A pattern with `_` instead of a binding, like `Text(_)`, only
looks at the kind and leaves everything in place.

## skip inside a match

`skip` works in a `match` the way it works in an `if`: each `case` counts as
one level, and `match` is not a wall. Two forms are useful.

- `skip;` leaves the case early, so you can stop halfway without nesting the
  rest of the code in an `else`.
- `skip to next;` leaves the case and **tests the cases below it** for the same
  value. It is how a value takes part in more than one case, which `match`
  otherwise does not allow.

<!-- focus: 5-12 -->
When `verbose` is false, `skip;` leaves the `case 0` block, so the last line of
the case is not printed. The program continues after the `match`.

<!-- focus: 13-17 -->
`skip to next;` hands the value on. A 3 prints "small" and then lands in
`case _`. A 5 prints "small" and then matches `case 5, 10`, so "five or ten"
follows. A 10 never reaches `case 1..9` at all.

<!-- example: guide-match-skip -->

Be careful with the second form. The last case has no next one, and the
compiler says so. Inside an `if` that is itself inside a case, `skip to next;`
talks to the `if` chain instead, and gets the same kind of error when there is
no further branch:

```text title="skip to next inside an if, inside a case"
error[E0012]: `skip to next` is in the last branch, so there is no next one
   --> main.drh:15:17
    |
 14 |             if (n == 5) {
    |             ----------- this chain has no branch after this one
 15 |                 skip to next;
    |                 ^^^^^^^^^^^^^ in the last branch
    |
 help: add an `else` branch, or write `skip 1;` to leave the `if`
```

## When to prefer match

Use `match` when the decision is "which value is it?" and `if` when it is "is
this condition true?". The two functions below say the same thing. The `if`
chain repeats `code` and writes out each comparison. The `match` names the value
once, and ranges and lists replace the `||` and `&&` pairs.

<!-- focus: 4-14 -->
The `if` version needs three kinds of operator to say what the match says with
its patterns.

<!-- focus: 17-23 -->
The `match` has the same cases in one column, and the compiler can check that
nothing is missing.

<!-- focus: 35-39 -->
A condition that mixes several variables is not a value, so an `if` is still the
right tool.

<!-- example: guide-match-vs-if -->

A rule of thumb:

- One value, several possible answers: `match`.
- A choice among enum variants: `match`, and let the compiler find the gaps.
- Text compared against several literals or patterns: `match`.
- Two or three unrelated conditions, or a single yes/no: `if`.
- A comparison that is only "greater than", with no other branch: `if`.

Next: back to the [lessons](index.md), or read about [enums](reference-enums.md)
and [control flow](reference-control-flow.md) for the full rules.
