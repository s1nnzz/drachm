---
title: Command line
description: build, run, check and explain, the -O flag, and exit codes.
---

# Command line

The compiler has these commands: `new` makes a project, `add`, `remove`, `update`
and `install` manage its packages, `lsp` starts the language
server for editors, and the others work on a file or project folder. `-O` works with `build`, `run` and `test`;
`--locked` works with `build`, `run`, `check`, `test` and `install`; nothing
else takes a flag.

```text
drachm build [file.drh | folder] [-o <output>] [-O] [--locked]   compile to a native executable
drachm run   [file.drh | folder] [-O] [--locked]                 compile and run
drachm test  [file.drh | folder] [NAME] [-O] [--locked]          run the test blocks, each on its own (NAME: only those whose name contains it)
drachm check [file.drh | folder] [--locked]                      check for errors only
drachm new <name>                            make a project folder
drachm add ADDRESS[@TAG | @^RANGE] [--name NAME]   add a git package to drachm.toml and drachm.lock
drachm remove NAME                           remove a dependency
drachm update [NAME]                         take newer tags (inside the ranges) for one or all dependencies
drachm install [--locked]                    fetch deps/ again from drachm.lock
drachm lsp                                   run the language server on stdin and stdout
drachm explain <code>                        describe an error code, e.g. E0101

  -O         optimised build: slower to compile, much faster to run
  --locked   use exactly what drachm.lock lists (see below)
```

## build and run

`build` writes an executable. Without `-o` the file is named like the source,
without its `.drh` extension, so `build hello.drh` writes `hello`. `run` builds the
program in a temporary place, runs it and removes it afterwards. The program gets
your standard input, so `printf '1\n2\n' | drachm run sum.drh` feeds it two lines. Compiler warnings
are printed first, and the compiler exits with the program's exit code.

A `main` that returns `sint32` sets the exit code, and a `none main()` exits with 0.

```drachm
sint32 main() {
    return 3;
}
```

## test

`test` builds each `test "name" { }` block of the file or folder as a program of
its own and runs it, so a test that fails does not stop the others. It prints
one line per test, shows the output of the failing ones, and exits with 1 if
any test failed. See [The test keyword](guide-testing.md#the-test-keyword).

A name after the file runs only the tests whose name contains it. The others are
not built, and the summary says how many were filtered out:

```text
$ drachm test tests-demo.drh wrong
running 1 test
test a wrong sum ... FAILED
    drachm: panic: assertion failed: add(2, 3) == 6
      --> tests-demo.drh:13
    exit code 101

test result: FAILED. 0 passed; 1 failed; 2 filtered out
```

```text
$ drachm test tests-demo.drh
running 3 tests
test adds small numbers ... ok
test a wrong sum ... FAILED
    drachm: panic: assertion failed: add(2, 3) == 6
      --> tests-demo.drh:13
    exit code 101
test negative sums ... ok

test result: FAILED. 2 passed; 1 failed
```

## check

`check` reports errors and warnings and writes nothing. Use it while you edit, as
the lessons do, because it is the fastest way to see whether a change compiles.

```sh
drachm check hello.drh
```

## lsp

`lsp` runs the language server. An editor starts it and talks to it over standard
input and output; you do not run it by hand. See [Editor support](install.md#editor-support).

## new

`new` makes a project folder in the current folder: `drachm.toml`, `src/main.drh`
with a greeting, and an empty `tests/` folder. The name must be a plain name
(letters, digits and `_`, not starting with a digit), and the folder must not
exist yet. Then `drachm run NAME` runs it.

```sh
drachm new mygame
drachm run mygame
```

The folder also gets a `.gitignore` with `deps/` in it (fetched packages are not
committed).

## add, remove, update, install

These four work on the project in the current folder (the one with the
`drachm.toml`) and are described in [Packages](guide-packages.md).

```sh
drachm add github.com/someone/sdl-drh@^0.4   # added sdl_drh: https://github.com/someone/sdl-drh at v0.4.2
drachm add github.com/someone/sdl-drh@v0.4   # one exact tag
drachm add github.com/someone/sdl-drh        # the newest version tag, pinned as `tag`
drachm add ADDRESS --name sdl                # choose the dependency name
drachm update                                # every range and moved tag
drachm update sdl                            # one dependency
drachm install                               # deps/ again, from drachm.lock
drachm remove sdl
```

Each prints what it did, one line per change. A command that fails changes
nothing in `drachm.toml`. `NAME` may be written with `-` or `_`
(`sdl-drh` finds `sdl_drh`).

## --locked

`--locked` makes the dependencies come from `drachm.lock` alone. Nothing is
resolved anew, nothing new is fetched and the lock file is not written. If the
lock is missing or does not cover `drachm.toml` (a dependency was added,
removed or its tag changed), the command fails with E0818, and `drachm install`
(without `--locked`) or `drachm update` brings the lock up to date. A fetched
package whose files do not match the lock fails with E0816. See
[Packages](guide-packages.md#locked-installs).

```sh
drachm install --locked      # in CI: fetch exactly the lock, or fail
drachm check --locked        # the same check for the program
```

## explain

`explain` prints the title and description of one code. An unknown code is an
error.

```sh
drachm explain W0102
```

## -O

`-O` gives an optimised build. The compiler writes C and calls your C compiler
(`cc`, or `clang` on Windows), so the build is slower and the program much faster. Both builds print the
same output, and the compiler's tests check that. It does not change what the
program means. Set `NO_COLOR` to turn off coloured diagnostics.

```sh
drachm run hello.drh -O
```

## Project folders

Instead of a file, `build`, `run` and `check` take a project folder: one with a
`drachm.toml`. With no argument they use the current folder. The program starts
in the file `entry` names under `[project]` (`src/main.drh` by default), and the
folder of that file is the source root. A file inside a project belongs to it;
a file with no `drachm.toml` above it is a script and its own folder is its
root. The nearest `srachm.toml` also selects script mode. Scripts can start
with top-level statements and use local imports. See [Scripts](design-scripts.md).

A project with `kind = "library"` has no program: `build` and `run` say so
(E0811), and `check` checks it without requiring `main()`. A folder with no `drachm.toml` is an error
(E0814), and so is a missing entry file (E0812). The settings are described in
[Projects and config](design-config.md).
