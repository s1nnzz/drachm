---
title: Strings
description: String literals, joining, comparison, every string method and io::outf formats.
---

# Strings

A `string` is UTF-8 text. A character is a grapheme cluster, so `e` followed by
a combining accent is one character and so is a family emoji. A string owns
its text and frees it at the end of its scope; see [Moves and copies](reference-ownership.md).

## Literals

`"..."` understands the escapes `\n \t \r \0 \\ \" \' \` \$ \%` and `\u{1F600}`
(a character by its code), and `${...}` is plain text in it. An `f"..."`
literal puts the value of `${expr}` into the text.
Backticks work like `f"..."`, and
`"""..."""` and `f"""..."""` run over several lines. `str` is another name
for `string`.

<!-- example: string-literals -->

## Joining and comparing

`a + b` and `a += b` join, and both sides must be strings, so a number needs
`::text()` first or the compiler reports E0101. The comparison operators go by
bytes: `"Z" < "a"` is true, and `"é"` is not equal to `"e\u{301}"`
even though both are one character.

<!-- example: string-compare -->

## Methods

Call the first method with `::` and each later one with `->`. `len()` counts
characters, `isEmpty()` tests for no text, and `reverse()`, `upper()`,
`lower()` and `trim()` give changed copies. `sub(from, to)` takes the
characters between two indexes, both included; negative numbers count from the
end and an index outside the string panics.

Searching uses `find(text)` (the index, or -1), `contains`, `startsWith`,
`endsWith` and `count(text)` (matches that do not overlap). Editing uses
`replace(from, to)`, `replaceLimit(from, to, n)`, `insert(index, text)`,
`remove(from, to)`, `repeat(n)` and `join(text)`. For layout there are
`padLeft(width)` and `padRight(width)`, each with an optional fill string.
`text()` returns the string itself. `to<T>()` reads the text as a number of type
`T`, and [Input and output](reference-io.md) has the rules. To go through the
characters, use [`for` over text](reference-control-flow.md#looping-over-characters),
and to match a string against cases, see [matching text](reference-control-flow.md#matching-text).

<!-- example: string-methods -->

## Formatting with io::outf

`io::outf` takes a format string and values, and each `%` takes the next value.
Between the `%` and the value's end, options appear in a fixed order: `-` pads
on the right instead of the left, a leading `0` pads with zeros after the sign,
a number is the width, and `.N` is the decimal places (floats only). The
letters `x`, `X`, `b` and `o` print whole numbers in hex, capital hex, binary and
octal. Write `\%` for a percent sign.

<!-- example: outf -->

## Panics

An index outside the string stops the program with exit code 101 and a message
on stderr.

```text
drachm: panic: string index out of range
```
