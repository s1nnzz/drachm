---
title: Command line
description: build, run, check and explain, the -O flag, and exit codes.
---

# Command line

The compiler has six commands: `new` makes a project, `lsp` starts the language
server for editors, and the others work on a file or project folder. `-O` works with `build` and `run`, and nothing
else takes a flag.

```text
drachm build [file.drh | folder] [-o <output>] [-O]   compile to a native executable
drachm run   [file.drh | folder] [-O]                 compile and run
drachm check [file.drh | folder]                      check for errors only
drachm new <name>                            make a project folder
drachm lsp                                   run the language server on stdin and stdout
drachm explain <code>                        describe an error code, e.g. E0101

  -O   optimised build: slower to compile, much faster to run
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
root. Files the program imports are found and read, but a program of several
files cannot be built yet (error E0901): naming things across files is still to
come.

A project with `kind = "library"` has no program: `build` and `run` say so
(E0811), and `check` checks it without requiring `main()`. A folder with no `drachm.toml` is an error
(E0814), and so is a missing entry file (E0812). The settings are described in
[Projects and config](design-config.md).
