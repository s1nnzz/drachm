---
title: Working with text
description: Characters versus bytes, string methods, interpolation and formatting, matching and looping over text, and reading input.
---

# Working with text

Text looks easy until it holds an accent, an emoji or a line typed by a person.
This guide covers how Drachm thinks about a `string`, the tools for cleaning
and formatting it, how to branch on it with `match`, how to walk over its
characters, and how to read it from the keyboard. [Lesson 5](lesson-5-strings.md)
is the gentle introduction and [Strings](reference-strings.md) lists every
method.

## Characters, not bytes

A `string` is UTF-8 text, and the unit you work with is a **character**, which
in Drachm means what a reader sees as one symbol (the technical name is a
grapheme cluster). `len()` counts characters, `sub()` slices by character, and
`reverse()` turns whole characters around. There is no separate `char` type.
A character is a one-character `string`.

That matters because one visible symbol can be built from several code points.
The accented e in "cafe" can be a single code point, `\u{e9}`, or a plain `e`
followed by a combining accent, `\u{301}`. A family emoji is three people glued
by invisible joiners. In all of these, you want "one symbol, length 1".

<!-- focus: 6-11 -->
Both spellings of "café" have 4 characters, even though one has 5 code points.

<!-- focus: 13-15 -->
The family emoji has five code points and is one character.

<!-- focus: 17-18 -->
Comparison is different: `==` and `<` look at the underlying **bytes**. The two
spellings of "café" look the same on screen but are not equal. If you compare
text from outside, keep that in mind.

<!-- focus: 20-23 -->
Editing keeps characters together. Reversing puts the accent with its `e`, and
`sub(3, 3)` takes the last whole character. Negative indexes count from the end,
so `sub(-4, -2)` is `caf`.

<!-- example: guide-text-chars -->

Asking for an index outside the string stops the program (see
[panics](reference-diagnostics.md#panics-at-run-time)). Both ends of `sub` are
included.

```text title="string::sub(1, 10) on "abc""
drachm: panic: string index out of range
```

## Cleaning text with methods

The string methods give you a toolbox. Call the first one with `::` and every
later one with `->`, so a chain reads left to right. **No method changes the
string it is called on.** Each returns a new one, so the original is still
there.

<!-- focus: 4-6 -->
A function can take `&string`, a borrowed look at the text, so the caller keeps
it. This turns a title into a web-friendly name with one chain: trim, lower the
case, then replace pieces.

<!-- focus: 12-17 -->
`find` returns the index of the first match, or -1 if there is none. `sub(from,
to)` takes both ends, so the part before `=` ends one before it, and `-1` as an
end means "to the last character".

<!-- focus: 19-21 -->
`trim` makes a new string, and the original title keeps its 21 characters.

<!-- focus: 23-26 -->
`padLeft`, `padRight` and `repeat` are the layout helpers.

<!-- example: guide-text-clean -->

The full list is `len`, `isEmpty`, `reverse`, `sub`, `upper`, `lower`, `trim`,
`find`, `contains`, `startsWith`, `endsWith`, `count`, `join`, `replace`,
`replaceLimit`, `insert`, `remove`, `repeat`, `padLeft`, `padRight`, `text` and
`to`. If you call a method that does not exist, the compiler lists them:

```text title="a.shout()"
error[E0201]: `string` has no method `shout`
   --> main.drh:10:18
    |
 10 |     io::outln(a::shout());
    |                  ^^^^^
    |
 help: string methods are `len`, `isEmpty`, `reverse`, `sub`, `upper`, `lower`, `trim`, `find`, `contains`, `startsWith`, `endsWith`, `count`, `join`, `replace`, `replaceLimit`, `insert`, `remove`, `repeat`, `padLeft`, `padRight`, `text` and `to`
```

## Joining and the number trap

`+` joins two strings, and both sides must be strings. Numbers do not become
text by themselves, and the compiler tells you the fix:

```text title="3 + a, where a is a string"
error[E0101]: `+` has `uint32` on the left but `string` on the right
  --> main.drh:5:20
   |
 5 |     string b = 3 + a;
   |                - this is `uint32`
   |                    ^ this is `string`
   |
 help: turn it into text: `3::text()`
```

You can write `3::text() + a`, but for mixed text and numbers the next section
is nicer.

## Putting values into text

Put an `f` in front of a string and `${...}` becomes a placeholder: the
expression inside is worked out and joined in as text. Anything is allowed
inside, including a calculation or a method call. A plain `"..."` string leaves
`${...}` alone, which is why you can write a dollar sign without thinking.
Backticks are another way to write an `f` string.

<!-- focus: 9-10 -->
Values go in as they print: `1.7`, `36`, and a calculation like `age + 1`.

<!-- focus: 13-14 -->
Backticks interpolate, and a plain string does not.

<!-- focus: 17 -->
`\$` and `\%` are a plain dollar and percent sign.

<!-- focus: 20-21 -->
Triple quotes run over several lines. Everything between them is kept as it is
written, including the line break.

<!-- focus: 24-28 -->
`io::outf` fills each `%` with the next value, in order. Between the `%` and
the value you can put a width (`%6`), `-` to pad on the right, `0` for zeros, a
number of decimals (`%.1`), or a base: `x`, `X`, `b` or `o`. Write `\%` for a
percent sign.

<!-- example: guide-text-format -->

The compiler counts placeholders for you. A missing or extra value is E0103,
and an unknown option such as `%d` is E0009 that says what is allowed:

```text title="io::outf("%d\n", 1)"
error[E0009]: `%d` is not a format placeholder
  --> main.drh:9:15
   |
 9 |     io::outf("%d\n", 1);
   |               ^^ in this placeholder
   |
 help: options are a width (`%8`, `%-8`, `%08`), decimals (`%.2`) and a base (`%x`, `%X`, `%b`, `%o`); to put a letter right after a value, use an `f"..."` string
```

### Indented blocks and stopping a placeholder
<!-- status: not-implemented -->

Two details are decided in the design but are not built yet. In a `"""` block,
the newline right after the opening quotes will be dropped, and the column of
the closing `"""` will be the left edge: that much leading space is removed from
every line, so a block can be indented with the code around it. A line indented
less than that loses all of its leading space. Today the text is kept exactly as
written.

The second is `%\`, which ends a placeholder's options so that a digit can
follow a value: `io::outf("%\5 apples!", "£")` will print `£5 apples!`, while
`"%5 apples!"` means a width of 5. Until then, use an `f"..."` string for that
case: `f"${price}5 apples!"`.

## Matching text

`match` works on strings. A plain `case "stop"` matches exactly that text. To
match a *pattern*, write `globbed` before the string: then each `*` stands for
any run of characters, and the whole text must fit. A plain `"a*b"` is just
three characters, a star included.

<!-- focus: 5 -->
A plain string matches the whole text exactly.

<!-- focus: 6 -->
`globbed` goes before each string it applies to, so a list repeats it. This line
matches pictures.

<!-- focus: 7 -->
`*` matches anything, slashes included, so `src/deep/main.drh` is source code too.

<!-- focus: 8 -->
Without `globbed`, the star is just a star. Only `a*b` itself matches.

<!-- focus: 9 -->
`case _` catches the rest.

<!-- example: guide-text-match -->

Cases are tried from the top and the first that fits wins. Text has too many
possible values for the compiler to list, so a `match` on text without
`case _` gets a warning:

```text title="a match on text with no case _"
warning[W0110]: this `match` on `string` does not handle every value
  --> main.drh:5:5
   |
 5 |     match (cmd) {
   |     ^^^^^^^^^^^ there is no `case _`
   |
 note: text has too many possible values to list
 help: add `case _ { }` to handle every other value
```

A case that an earlier one already covers is W0111. For example, `case "go"`
after `case globbed "g*"` can never run.

## Looping over characters

`for (string c of text)` visits the characters one at a time, in order. Each
`c` is a copy that is yours to keep or move on.
`for (&string c of text)` gives a borrowed view of each one instead, which
costs nothing but cannot be kept. To keep one, write `copy c`.

Choose `&string` when you only look at each character, as in counting vowels,
and `string` when you build something from them, like the reversed word
`olleh`. The loop borrows the text, so you cannot change the text inside the
loop. A `&string` loop variable cannot be moved out, either:

```text title="a &string c kept with string keep = c"
error[E0310]: cannot move `c` out of a borrow
  --> main.drh:8:23
   |
 7 |     for (&string c of "ab") {
   |                  - `c` is a `&` borrow
 8 |         string keep = c;
   |                       ^ this would move `c` out of a value `main` does not own
   |
 help: write `copy c` to keep the original in place
```

The rounds follow characters, not bytes. The last loop below runs 6 times for
"café" with a combining accent, a space and a two-person emoji.

<!-- focus: 4-9 -->
`string c` builds the word backwards, one copy at a time.

<!-- focus: 11-21 -->
`&string c` only looks. `match` on it counts the vowels, and `copy c` keeps the
last character past the loop.

<!-- focus: 23-27 -->
Six characters, no matter how many bytes.

<!-- example: guide-text-loop -->

## Reading input

`io::inln()` returns the next line of standard input as a `string`, without the
line ending. `io::ended()` says whether any input is left, so the usual loop is
`while (!io::ended())`. At the end of input `io::inln()` returns `""`. A blank
line is `""` too, so check `isEmpty()` if you need to tell blanks from the end.

Turn a line into a number with `line::to<T>()`, where `T` is any number type.
Spaces around the number and a leading `+` are fine, and a `-` is only for
signed types. This program reads a name, then a number on each line, skipping
blank ones. The input it was run with is this file,
`guide-text-input.in`:

```text title="input"
  Ada  
10
 +4

-3
   8   
```

<!-- focus: 5-6 -->
`trim()` removes the spaces around the name.

<!-- focus: 11-18 -->
Read until the input ends, skip blank lines, and add each parsed number.

<!-- focus: 20-22 -->
`io::outf` rounds the average to two decimals.

<!-- example: guide-text-input -->

### When the text is not a number

Text that is not a number of the type you asked for (empty, letters, out of
range for the type) stops the program for now. It prints the text and the type
to standard error and exits with code 101:

```text title="echo abc | ./program, with line::to<uint32>()"
drachm: panic: cannot read "abc" as uint32
```

### A failing conversion you handle
<!-- status: not-implemented -->

The design turns this into a failure the program can handle, so
`line::to<uint32>() maybe 0` gives a fallback, and the error says why it
failed: empty, not a number, or out of range. Failures and `maybe` are not
built yet, see [Coming later](coming-later.md). Until they are, validate the
text first, for example with `isEmpty()`, as the program above does.

Next: [Thinking in match](guide-match.md).
