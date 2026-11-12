---
title: Coming from Python, JavaScript or Java
description: Everyday code side by side, what feels different, and the five rules that trip newcomers up.
---

# Coming from Python, JavaScript or Java

You already know how to program, so this guide skips the basics and shows how
everyday code looks in Drachm. Where a feature is designed but the compiler does
not accept it yet, the section carries a badge and its code is not runnable. The
[lessons](lesson-1-first-program.md) teach the same ground from the start.

## The big picture

Drachm is compiled ahead of time to a native program, like C, with no virtual
machine and no garbage collector. Three things follow from that and shape
everything below.

- Every variable has a type you write down, and the compiler checks all of them.
- Memory is freed when its owner goes out of scope, so the compiler tracks who
  owns what. That is the largest difference from the languages you know.
- Mistakes that Python or JavaScript find while running (a name that moved, a
  string added to a number) are compile errors here.

## Variables and types

Python and JavaScript let a name hold anything. Java makes you write the type
but lets you reassign freely. In Drachm a binding is **immutable unless you
say `mut`**, and the type is always written.

<!-- focus: 10 -->
Python `count = 3`, Java `final int count = 3;`. This binding cannot change.

<!-- focus: 11 -->
Python `total = 0` and later `total += i`. Here the word `mut` allows that.

<!-- focus: 14-16 -->
Python `for i in range(1, count + 1)`. A range with `of` includes both ends.

<!-- focus: 20-22 -->
`while` works as you expect, but the condition is always in parentheses.

<!-- example: guide-py-basics -->

Whole numbers come in fixed sizes (`uint8` to `uint128`, `sint8` to `sint128`),
and decimals are `float32` and `float64`. There is no arbitrary-size `int`
like Python's: arithmetic that overflows wraps around. See
[Types](reference-types.md).

| Python / JavaScript / Java | Drachm |
|---|---|
| `x = 5` / `let x = 5` / `final int x = 5;` | `uint32 x = 5;` |
| `x = 5` then `x += 1` / `let` / `int x = 5;` | `mut uint32 x = 5;` |
| `7 // 2` / `Math.floor(7 / 2)` / `7 / 2` | `7 / 2` on integers is `3` |
| `7 / 2` giving `3.5` | `7.0 / 2.0`, or `a::to<float64>() / 2` |
| `True`, `true` | `true` |
| `None`, `null` | no null; see [Optionals](#null-becomes-an-optional) |

Mixing sizes converts automatically when nothing can be lost. A conversion that
can lose information, such as `uint32` into `uint8`, compiles with warning
W0102 until you write `big::to<uint8>()` to say you meant it.

## Functions

The return type comes first and there is no `def`, `function` or `fn`. A
function that returns nothing returns `none`. See
[lesson 4](lesson-4-functions.md).

```drachm
// Python: def average(a, b): return (a + b) / 2
float64 average(float64 a, float64 b) {
    return (a + b) / 2;
}
```

A parameter you change inside the function must be `mut`, and the change
affects the function's own copy only.

## Strings

A `string` is UTF-8 text, and a character is a whole grapheme, so an emoji or a
letter with an accent counts as one. `f"..."` interpolates `${expr}` like a
Python f-string or a JavaScript template literal. `+` joins strings only: adding
a number is an error that suggests `::text()`.

```drachm
string name = "Illy";
io::outln(f"hello ${name}, you have ${name::len()} letters");
io::outln(name::upper() + "!");
```

Methods use `::` for the first call and `->` for each call after it:
`name::upper()->reverse()`. Strings are covered in
[lesson 5](lesson-5-strings.md) and [Strings](reference-strings.md).

## Classes become structs

There are no classes, inheritance or `this`. A `struct` holds fields and
methods, `self` is the value a method was called on, and a method needs no
declaration saying whether it changes `self`: the compiler works that out.

<!-- focus: 7 -->
A `mut` field can change even on a struct bound without `mut`.

<!-- focus: 9-11 -->
`self` is explicit. This method changes only a `mut` field, so it can be
called on an immutable `acct`.

<!-- focus: 20 -->
There is no `new` and no constructor to write. `"Illy", 10` fills the fields in
order.

<!-- focus: 26 -->
Python and JavaScript would make `other` a second name for the same object.
Here the struct holds a string, so `acct` moves, and `copy` is how you ask for
a separate one.

<!-- example: guide-py-structs -->

Sharing behaviour between types uses interfaces, designed in
[Generics and interfaces](design-generics.md), not inheritance.

## Null becomes an optional

<!-- status: not-implemented -->

There is no `null`, `None` or `undefined`. A value that may be missing has the
type `T?`, and you must say what to do when it is missing. `maybe` supplies a
fallback and `match` takes the two cases apart.

```drachm
// Python: age = ages.get("Bob", 0)
uint32? found = ages["Bob"];
uint32 age = found maybe 0;

match (found) {
    case uint32 a { io::outln(a::text()); }
    case none     { io::outln("no age"); }
}
```

See [Enums and optionals](design-enums.md).

## Exceptions become failures

<!-- status: not-implemented -->

Nothing is thrown through code that does not expect it. A function that can go
wrong says so in its signature with `fails`, and every call must say what
happens on failure: give a fallback with `maybe`, pass it on with `fails`, or
handle it with `fails with`. There is no `try` or `catch`, and no `Result`
wrapper to unpack.

```drachm
uint32 parse(string s) fails ParseError {
    if (s::isEmpty()) { fail Empty; }
    return 42;
}

uint32 a = parse(input) maybe 0;
uint32 b = parse(input) fails;
```

Faults that are bugs, such as dividing by zero or an index out of range, are
not failures: they stop the program with a message and exit code 101. Lesson 14
([Errors](lesson-14-errors.md)) and [Errors in the design](design-errors.md) have
the full design.

## Lists and maps

<!-- status: not-implemented -->

Lists, arrays and maps are designed for step 6, and the compiler does not
accept them yet. The code here shows the planned design.

| Python | JavaScript | Drachm |
|---|---|---|
| `names = ["Illy", "Sam"]` | `let names = ["Illy", "Sam"]` | `string[?] names = ["Illy", "Sam"];` |
| `ages = {"Illy": 18}` | `new Map([["Illy", 18]])` | `Map<string, uint32> ages = { "Illy": 18 };` |
| `for n in names:` | `for (const n of names)` | `for (string n of names) { }` |
| `names.append("Al")` | `names.push("Al")` | `names::push("Al");` on an `open` list |
| `len(names)` | `names.length` | `names::len()` |

An index outside a list stops the program with exit code 101, and a map lookup
with `[]` returns an optional. See [Collections](design-collections.md).

## What feels different

- **There is no garbage collector.** A string is freed at the end of its
  scope, and when you pass it somewhere else, that place frees it. The
  compiler proves it is freed exactly once, so there is no use-after-free and no
  leak to hunt.
- **Nothing is copied behind your back.** In Python, `b = a` makes a second name
  and in Java a second reference. In Drachm it moves or, for small plain
  values such as numbers, copies, and for anything expensive you write `copy`.
- **Output is not buffered by magic.** `io::out` and `io::outln` write what you
  give them, and `io::outf` formats with `%`, as in
  `io::outf("%-8|%6.2|\n", name, 3.14159)`.
- **Errors come early.** The compiler tells you about a mistake with a fix, and
  `drachm explain E0301` describes any code. See [Diagnostics](reference-diagnostics.md).

## The five rules that trip newcomers up

### 1. A binding does not change unless it says `mut`

Assigning to a name declared without `mut` is error E0205 with the fix "declare
it as `mut`". This catches mistakes, and it means a reader knows at a glance
what can change. Function parameters follow the same rule.

### 2. Values move

A `string` has one owner. Assigning it or passing it to a function hands it
over, and using the old name afterwards is error E0301:

```text title="drachm check"
error[E0301]: `a` is used after it moved
  --> c.drh:6:10
   |
 5 |     string b = a;
   |                - `a` moved here
 6 |     show(a);
   |          ^ used here, after the move
   |
 help: to keep both, write `string b = copy a;`
```

Fix it with `copy`, or lend the value with a `&` borrow
([lesson 9](lesson-9-borrowing.md)). Numbers, bools and small plain structs copy
by themselves.

<!-- example: guide-py-moves -->

### 3. Conditions are real booleans

`if (x)` with a number is error E0101: `bool` never converts to or from a
number, and there is no truthiness of empty strings or zero. Write
`if (x != 0)` or `if (!name::isEmpty())`. Conditions always sit in parentheses.

### 4. Text and numbers do not mix

`"a" + 1` is an error with the fix `1::text()`. Use an `f"..."` string when
you want to build a message. Equally, the first call in a chain is written `::`
and the rest `->`, so `n::text()->len()` is correct and `n::text()::len()` is
error E0007.

### 5. Missing and failing are in the type

You cannot forget to check for a missing value or a failing call, because the
program will not compile until you do ([null](#null-becomes-an-optional) and
[exceptions](#exceptions-become-failures) above). The cost is a few more words
at the call; the reward is that no code path can reach a `null` it did not
expect.

## Where next

Work through the [lessons](lesson-1-first-program.md) in order, or jump to
[Moves and copies](reference-ownership.md). If speed matters to you, read
[Performance](guide-performance.md), and to check your programs,
[Testing your programs](guide-testing.md).
