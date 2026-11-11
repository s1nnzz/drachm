---
title: Input and output
description: io::out, io::outln, io::outf, io::inln, io::ended, reading numbers with ::to<T>(), and what bad text does.
---

# Input and output

The `io` functions need `in io;` at the top of the file, and are called as
`io::name(...)`. Output goes to standard output and input comes from standard
input.

## Output

- `io::out(text)` writes text with no newline.
- `io::outln(text)` writes text and then a newline.
- `io::outf(format, values...)` writes values under a format. See
  [Strings](reference-strings.md) for the format options.

`out` and `outln` take a `string`, so turn a number into one with `::text()`.

## Reading a line

`io::inln()` returns the next line of standard input as a `string`, without
its line ending.

- The line ends at a newline. A `\r` just before the newline is removed as well,
  so a file with Windows line endings reads the same.
- A last line with no newline at the end is still a line.
- A blank line is a line, and reads as `""`.
- A newline at the very end does not start another line.
- A byte that is not valid UTF-8 becomes U+FFFD (the replacement character).

`io::ended()` is `true` when no input is left. At the end `io::inln()` returns
`""`, and it keeps doing that when called again. So the usual loop is:

```drachm
while (!io::ended()) {
    string line = io::inln();
    // ...
}
```

When standard input is a terminal, text written with `io::out` shows before the
program waits for you to type a line.

<!-- example: input-echo -->

## Reading numbers

`::to<T>()` turns text into a number of type `T`. Every number type works:
`uint8` to `uint128`, `sint8` to `sint128`, `float32` and `float64`. Reading
text as `bool` or as a number with a unit is not supported yet.

<!-- example: input-sum -->

The text must be the whole number, and the rules are strict for now:

- An integer is digits, with a `-` before them only for a signed type. No spaces,
  no `+`, no `_` and no `0x`.
- A number out of range for its type is bad text: `"256"::to<uint8>()` is bad.
- A float is an optional `-`, digits, an optional `.` followed by digits, and an
  optional exponent such as `e3` or `E-3`. So `.5`, `5.`, `inf` and `nan` are bad.
- A float too large for its type is bad text.

## Bad text

Until failing calls exist, bad text stops the program. The program below reads
`abc` as a number:

```drachm
in io;

none main() {
    io::outln("start");
    uint32 n = io::inln()->to<uint32>();
    io::outln(n::text());
}
```

It prints this to standard error and exits with code 101:

```text title="output"
start
drachm: panic: cannot read "abc" as uint32
```

The message shows the text, up to 40 bytes of it, and the type. A later step
turns this into an error the program handles, with `maybe`; see
[Coming later](coming-later.md).
