---
title: Testing your programs
description: Using check, run and build, exit codes, writing expected-output tests the way the compiler's own repo does, and the planned test keyword.
---

# Testing your programs

Today you test a Drachm program by running it and comparing what it prints. This
guide shows the commands, what the exit codes mean, and the expected-output
technique the compiler's own repository uses for over two hundred programs. At
the end is the `test` keyword, which is designed and not built yet.

## Three commands

```sh
drachm check program.drh      # errors and warnings only; builds nothing
drachm run program.drh        # build and run; exits with the program's code
drachm build program.drh      # write an executable named "program"
```

- Use **`check`** while you edit. It is the fastest feedback and runs nothing.
- Use **`run`** to try the program. It passes your standard input through, so
  `printf '4\n5\n' | drachm run sum.drh` works.
- Use **`build`** (with `-o NAME` and `-O` if you want) when you need the
  executable, for example to run it many times in a script. Add `-O` to test the
  optimised build too: both builds must print the same, and a difference means a
  compiler bug worth reporting. See [Command line](reference-cli.md).

## Exit codes

Every step ends with an exit code, and 0 means success.

| Situation | Exit code |
|---|---|
| `check` finds no errors (warnings alone do not change it) | 0 |
| `check` or `build` finds an error | 1 |
| `none main()` finishes | 0 |
| `sint32 main()` returns `n` | `n` |
| a run-time fault: division by zero, string index out of range | 101 |
| a failure leaves `main` | the failure's exit code, 1 if it has none |

`run` exits with the code of your program, and with 1 if the program did not
compile. That makes shell checks easy:

```sh
drachm check program.drh && drachm run program.drh
```

A `sint32 main()` can report its own verdict, so a program may check itself:

```drachm
in io;

sint32 main() {
    io::outln("about to fail");
    return 3;
}
```

```text title="terminal"
$ drachm run program.drh
about to fail
$ echo $?
3
```

A run-time fault stops the program at once, prints a message on stderr and
exits with 101, so you can tell it from an exit code you chose:

```text title="terminal"
$ drachm run divzero.drh
before
drachm: panic: division by zero
$ echo $?
101
```

Warnings do not stop a build. Keep your programs free of them, because the
project's tests demand that a program compile in silence.

## Expected-output tests

A test is a program plus a file saying what it must print. The compiler's
repository tests itself this way: there is a folder of small programs, and for
each one `cargo test` builds it, runs it and compares the output.

You can do the same for your own code, with nothing but a shell. The program
under test prints what it computes:

<!-- focus: 4-10 -->
The function under test.

<!-- focus: 15-16 -->
Each line prints an input and the result, so a failure shows which case broke.

<!-- focus: 19-23 -->
Standard input is read until it ends. `io::ended()` is the end-of-input test.

<!-- example: guide-testing-sum -->

The output block above is the file `guide-testing-sum.out`. The input, one number
per line, is `guide-testing-sum.in`. This is the whole test: the program, its
`.in` and its `.out`.

To run it by hand:

```sh
drachm run guide-testing-sum.drh < guide-testing-sum.in > actual.txt
diff guide-testing-sum.out actual.txt && echo pass
```

A loop over a folder makes a test runner:

```sh
for t in tests/*.drh; do
    name=${t%.drh}
    input=/dev/null
    [ -f "$name.in" ] && input="$name.in"
    drachm run "$t" < "$input" | diff -q "$name.out" - > /dev/null \
        && echo "ok   $t" || echo "FAIL $t"
done
```

### The files this repo uses

The compiler's own test runner (`tests/run.rs`) follows the same idea and has a
few more files beside the program. Every one is optional except the `.out`.

| File | Meaning |
|---|---|
| `NAME.drh` | the program |
| `NAME.out` | what it must print on stdout, exactly |
| `NAME.in` | its standard input. Without it, input is empty, so a test never waits for a terminal |
| `NAME.code` | the exit code it must end with. Without it, 0 |
| `NAME.err` | lines that must each appear somewhere in the program's stderr |
| `NAME.warn` | lines that must each appear in the compiler's own output. Without it, the compiler must print nothing at all |

For a program that is expected to **fail to compile**, a different folder is
used: `tests/fail/NAME.drh` with a `NAME.err` listing the error text that must
appear. It matches the code and message, not the layout. For example,
`badmain.err` holds the error code line and the cause for a `main` of the wrong
shape.

A test of a run-time fault is the same as any other. The division by zero case
is a program, an `.out` of what printed first (`before`), a `.code` file
holding `101`, and an `.err` file with the panic message.

Two habits from the repo are worth copying:

- Every run test is built both **plain and `-O`**, and the outputs are compared.
- The tests run with leak checking on: a `string` that is never freed, or
  freed twice, fails the case.

### Updating expected output

When the output is supposed to change, regenerate the files and read the diff
before committing:

```sh
DRACHM_BLESS=1 cargo test
```

That is a feature of the compiler's own harness, and it overwrites `.out` and
`.code` from what the programs printed. Blessing without reading the diff
turns your tests into a rubber stamp.

### The examples on this site

Every program in these docs is a real test, with a `.drh` and a `.out` (and
sometimes an `.in`) in `site/examples/`. The harness builds it both ways, requires
that the compiler print nothing, and requires that the output match. That is why
the pages say "run this" only where it works.

## The `test` keyword

<!-- status: not-implemented -->

A `test` block in any file is a test, and `drachm test` will run them all.
Nothing here is accepted by the compiler today.

```drachm
test "adds" {
    assert(add(2, 3) == 5);
}
```

What the design says so far:

- `test "name" { ... }` can sit in any file, next to the code it checks.
- `assert(condition)` is the check.
- `drachm test` is a new command beside `new build run check add fmt explain`.
- `drachm new mygame` makes a `tests/` folder next to `src/`.

What it does not say yet, and the language's owner has not decided: what the
failure message looks like, whether a test sees private items of its file,
whether tests are compiled out of normal builds, and the exact exit code of
`drachm test`. Treat the details as open. Until then, the expected-output technique above
works with any compiler version and tests a program the way a user runs it.
See [Projects and config](design-config.md#tests).
