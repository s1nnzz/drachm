---
title: Strings
description: String literals, joining, comparison, every string method and io::outf formats.
---

# Strings

A `string` is UTF-8 text. A character is a grapheme cluster: `e` followed by a
combining accent is one character, and so is a family emoji. Each string owns
its text and is freed at the end of its scope.

## Literals

- `"..."`: escapes are `\n \t \r \0 \\ \" \' \` \$ \%` and `\u{1F600}`
  (a character by its code). `${...}` is plain text here.
- `f"..."`: `${expr}` puts a value into the text. Use `::text()` for a number
  that is not already text.
- `` `...` ``: like `f"..."`, with `${expr}` put into the text.
- `"""..."""` and `f"""..."""`: several lines. Escapes work in both.

`str` is another name for `string`.

## Joining and comparing

- `a + b` and `a += b` join. Both sides must be strings, so a number needs
  `::text()` first: `"x" + n::text()`. Without it, the compiler reports an
  error.
- `==`, `!=`, `<`, `<=`, `>`, `>=` compare by bytes. So `"Z" < "a"` is true,
  and `"é"` is not equal to `"e\u{301}"`.

## Methods

Call with `::` for the first call and `->` for each call after it:

```drachm
io::outln("dcba"::reverse()->sub(0, 1));   // ab
```

| Method | What it does |
|---|---|
| `len()` | number of characters |
| `isEmpty()` | true when there is no text |
| `reverse()` | characters in reverse order |
| `sub(from, to)` | characters from `from` to `to`, both included. Negative numbers count from the end. Outside the string it panics |
| `upper()`, `lower()` | change case |
| `trim()` | remove spaces at both ends |
| `find(text)` | index of the first match, or -1 |
| `contains(text)`, `startsWith(text)`, `endsWith(text)` | true or false |
| `count(text)` | number of matches that do not overlap |
| `join(text)` | this string followed by `text` |
| `replace(from, to)` | replace every match |
| `replaceLimit(from, to, n)` | replace the first `n` matches |
| `insert(index, text)` | insert `text` before the character at `index` |
| `remove(from, to)` | remove characters from `from` to `to`, both included |
| `repeat(n)` | `n` copies |
| `padLeft(width)`, `padLeft(width, fill)` | pad on the left to `width` characters, with spaces or with `fill` repeated |
| `padRight(width)`, `padRight(width, fill)` | pad on the right |
| `text()` | the string itself |

## Formatting with io::outf

`io::outf` takes a format string and values. Each `%` takes the next value.

| Placeholder | Result |
|---|---|
| `%` | the value as text |
| `%8` / `%-8` | pad to 8 characters, on the left / on the right |
| `%08` | pad with zeros after the sign |
| `%.2` | two decimal places (floats only) |
| `%x`, `%X`, `%b`, `%o` | hex, capital hex, binary, octal (whole numbers only) |

Write `\%` for a percent sign. Options go between `%` and the value's
placeholder in the order shown, for example `%-10.2`.

```drachm
io::outf("[%6] [%-6] [%06] [%x]\n", 42, 42, 42, 255);
// [    42] [42    ] [000042] [ff]
```

## Panics

An index outside the string stops the program with exit code 101:

```text
drachm: panic: string index out of range
```
