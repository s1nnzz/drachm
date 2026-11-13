---
title: Testing your programs
description: Using check, run and build, exit codes, writing expected-output tests the way the compiler's own repo does, and the test keyword with drachm test.
---

# Testing your programs

Today you test a Drachm program by running it and comparing what it prints. This
guide shows the commands, what the exit codes mean, and the expected-output
technique the compiler's own repository uses for over two hundred programs. At
the end is the `test` keyword, which runs with `drachm test`.

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

A `test` block can sit in any file, next to the code it checks. Its body is
ordinary code, and `assert(condition);` stops the test when the condition is
false (see [assert](reference-control-flow.md#checking-a-condition-assert)):

<!-- example: test-blocks -->

`drachm test` runs every test of a file, or of a project folder. Run on the
file above it prints:

```text title="terminal"
$ drachm test test-blocks.drh
running 3 tests
test adds small numbers ... ok
test negative sums ... ok
test a text that is not a number ... ok

test result: ok. 3 passed; 0 failed
$ echo $?
0
```

A failing `assert` shows its condition and line, and the other tests still run:

```text title="terminal"
$ drachm test tests-demo.drh
running 3 tests
test adds small numbers ... ok
test a wrong sum ... FAILED
    this line prints before the check fails
    drachm: panic: assertion failed: add(2, 3) == 6
      --> tests-demo.drh:13
    exit code 101
test negative sums ... ok

test result: FAILED. 2 passed; 1 failed
$ echo $?
1
```

What the rules are:

- Each test is built and run as a program of its own. A test passes when its
  program exits with 0. The output of a test is shown only when it fails.
- Test names are unique within a file (E0239).
- `expect failure` checks a failing call inside a test (see below).
- A normal `build` or `run` checks the test blocks too, so a broken test is an
  error in every build. It leaves their code out of the executable: only `main`
  is built. `drachm test` builds each test.
- `drachm test` works on a library as well as a program, and `-O` runs the
  tests on the optimised build.
- The exit code is 0 when every test passes and 1 otherwise.
- A test block in a script that also has top-level statements is not supported
  yet (E0901). Put the tests in a file with `main`, or in a project.

An `assert` can name what it checks, after the condition:
`assert(add(2, 3) == 6, "the sum of two and three");`. The message is plain text,
and a failing check prints it after the condition, for example
`assertion failed: add(2, 3) == 6: the sum of two and three`.

### Expecting a failure

Some code must fail, and a test should check that it does. `expect failure`
names the failure a call must end with:

```drachm
test "a text that is not a number" {
    expect failure ConvertError from "x"::to<uint32>();
}
```

The test passes when the call fails with `ConvertError`. Three other outcomes
stop the test with a message:

- the call does not fail at all;
- the call fails with another failure of the same call;
- the call can never fail with the named failure (a compile error, E0545).

```text title="terminal"
$ drachm test expect.drh
running 2 tests
test a text that is not a number ... ok
test no failure ... FAILED
    drachm: panic: expected a `ConvertError` failure from `"7"::to<uint32>()`, but the call did not fail
      --> expect.drh:6
    exit code 101

test result: FAILED. 1 passed; 1 failed
```

`expect failure` is only allowed in a `test` block (E0014). The call must be
one that can fail (E0003 otherwise), and the failure must be a `failure` type
(E0540).

To run some tests, give a name after the file. Only the tests whose name
contains it run, and the summary says how many were filtered out:

```text title="terminal"
$ drachm test test-blocks.drh negative
running 1 test
test negative sums ... ok

test result: ok. 1 passed; 0 failed; 2 filtered out
```
