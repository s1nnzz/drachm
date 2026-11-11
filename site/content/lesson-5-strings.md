---
title: "Lesson 5: Strings"
description: String literals, interpolation, joining, methods and chaining, comparison, and io::outf.
---

# Lesson 5: Strings

**Goal:** build, inspect and format text.

## Writing text

A `string` is UTF-8 text. An ordinary literal in double quotes understands
escapes: `\t` is a tab, `\"` a quote, `\\` a backslash and `\u{1F600}` any
character by its code.

<!-- focus: 5 -->
All of those escapes appear on this line.

<!-- focus: 6 -->
In a plain string `${...}` is just text, so nothing is looked up.

<!-- focus: 7 -->
Put an `f` in front and `${...}` puts a value into the text. Anything
that is already text or a number goes in as it is, and you can calculate
inside the braces.

<!-- focus: 8-12 -->
Backticks behave like `f"..."`, and triple quotes (with or without the `f`)
let the text run over several lines.

<!-- example: string-literals -->

## Joining text

`+` and `+=` join two strings. Both sides have to be strings, so a number needs
`::text()` first. The compiler points at the number and tells you the fix.
Alternatively an `f"..."` string does the conversion for you.

```drachm
uint32 hp = 42;
io::outln("hp: " + hp);
```

```text title="drachm check"
error[E0101]: `+` joins strings, but the right side is `uint32`
  --> main.drh:5:24
   |
 5 |     io::outln("hp: " + hp);
   |               ------ this is `string`
   |                        ^^ this is `uint32`
   |
 help: turn it into text: `hp::text()`
```

## Methods and chaining

A string has methods for the common jobs. Call the first one with `::`, and
every call after it with `->`, so a chain reads from left to right:
`s::trim()->upper()` trims, then changes the case. Write the first call with
`->` and the compiler corrects you.

<!-- focus: 5-7 -->
`trim` removes spaces at both ends, `upper` changes case, and `len` counts
characters, so the first three lines show the cleaned text.

<!-- focus: 8-9 -->
`sub(from, to)` takes characters from `from` to `to`, both included, and counts
from 0. A negative number counts from the end, so `-6, -1` is the last six
characters.

<!-- focus: 10-14 -->
`find` gives the index of the first match, or -1. `contains`, `startsWith` and
`endsWith` answer with a `bool`.

<!-- focus: 15-19 -->
`count`, `replace`, `replaceLimit`, `insert` and `remove` search and edit.
None of them changes the original, they give you a new string.

<!-- focus: 20-25 -->
`repeat`, `padLeft`, `padRight`, `reverse`, `isEmpty` and `join` finish the
set.

<!-- example: string-methods -->

## Comparing

`==` and `<` compare strings by their bytes, so `"Z" < "a"` is true because
capital letters come first. Two strings that look the same can differ. An `é`
written as one character and an `e` followed by a combining accent are not
equal, although the second one is still a single character.

<!-- example: string-compare -->

## Formatting with io::outf

`io::outf` takes a format string and then values. Each `%` takes the next value
and prints it as text. Options go between the `%` and the end of the
placeholder: a width pads to that many characters (`%8` on the left, `%-8` on
the right), a leading `0` pads with zeros, `.2` keeps two decimals, and `x`,
`X`, `b` and `o` print whole numbers in hex, capital hex, binary and octal.
For a real percent sign write `\%`.

<!-- example: outf -->

### Your turn: shout it

Make the text in `text` upper case, add an exclamation mark, print it, and then
print how many characters it has. Remember that the first method call in a
chain uses `::` and the next ones use `->`.

```drachm title="starter"
in io;

none main() {
    string text = "hello world";
    // your code here
}
```

```text title="expected output"
HELLO WORLD!
12
```

<!-- solution-of: solution-5-1 -->
```drachm solution
in io;

none main() {
    string text = "hello world";
    string loud = text::upper() + "!";
    io::outln(loud);
    io::outln(loud::len()->text());
}
```

### Your turn: a table row

With one `io::outf` call, print the name left-aligned in a field of 8
characters and the score right-aligned in a field of 4, each in square
brackets.

```drachm title="starter"
in io;

none main() {
    string name = "Illy";
    uint32 score = 7;
    // your code here
}
```

```text title="expected output"
[Illy    ] [   7]
```

<!-- solution-of: solution-5-2 -->
```drachm solution
in io;

none main() {
    string name = "Illy";
    uint32 score = 7;
    io::outf("[%-8] [%4]\n", name, score);
}
```

Next: [Lesson 6: Structs and methods](lesson-6-structs.md).
