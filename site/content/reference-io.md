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

The text must hold one number. Spaces, tabs and line breaks around it are
ignored, so `" 42 "::to<uint32>()` is 42.

- An integer is an optional sign, then digits. A `-` is allowed only for a
  signed type; a `+` is allowed for any type. No spaces inside the number, no
  `_` and no `0x`: `"4 2"`, `"1_000"` and `"0x10"` are bad.
- A number out of range for its type is bad text: `"256"::to<uint8>()` is bad,
  and so is `"-1"::to<uint8>()`.
- A float is an optional sign (`-` or `+`), digits, an optional `.` with digits
  after it, and an optional exponent such as `e3` or `E-3`. So `.5` and `5.` are
  fine, but `.` alone, `inf` and `nan` are bad.
- A float too large for its type is bad text.
- Empty text, or text with only spaces, is bad text.

## Bad text

Reading text can fail, so `::to<T>()` on text needs a `maybe` or a `fails` (see
[Failures](reference-errors.md)). The failure is the built-in `ConvertError`:
`Empty`, `NotANumber` or `OutOfRange`. `maybe` gives a fallback:

```drachm
in io;

none main() {
    uint32 n = io::inln()->to<uint32>() maybe 0;
    io::outln(n::text());
}
```

A bare `fails` passes it up. If it leaves `main`, the program prints this to
standard error and exits with code 1. For the input `abc`:

```text title="output"
error: the text is not a number
```

## Files and the running environment

Import `fs` to read or replace whole UTF-8 text files. Paths are relative
to the running program's working directory unless they are absolute.

- `fs::read(path)` returns a `string`. It can fail with `FileMissing`,
  `AccessDenied`, `InvalidPath`, `InvalidFileText` or `FileIoError`.
- `fs::write(path, text)` replaces the contents of an existing file. It can
  fail with `FileMissing`, `AccessDenied`, `InvalidPath` or `FileIoError`.

Import `os` to inspect the running environment. `os::platform()` returns the
target OS name, such as `linux`, `windows` or `macos`.
`os::getEnv(name)` returns `string?`. A missing variable gives `none`, and a
present empty value gives an empty string.

```drachm
in fs;
in io;
in os;

none main() {
    string source = fs::read("Cargo.toml") maybe "";
    if (source != "") {
        io::outln("read project text");
    }
    string platform = os::platform();
    if (platform != "") {
        io::outln("platform available");
    }
    string value = os::getEnv("DRACHM_CHECK_LEAKS") maybe "unset";
    io::outln(value);
}
```

**Not implemented yet** — Exclusive `fs::create` with a shared `File` result,
`fs::open`, line I/O and `os::cwd`.
