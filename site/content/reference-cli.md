---
title: Command line
description: build, run, check and explain, the -O flag, and exit codes.
---

# Command line

The compiler has four commands. `-O` works with `build` and `run`, and nothing
else takes a flag.

```text
drachm build <file.drh> [-o <output>] [-O]   compile to a native executable
drachm run   <file.drh> [-O]                 compile and run
drachm check <file.drh>                      check for errors only
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
