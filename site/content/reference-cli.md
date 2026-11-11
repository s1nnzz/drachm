---
title: Command line
description: build, run, check and explain, the -O flag, and exit codes.
---

# Command line

```text
drachm build <file.drh> [-o <output>] [-O]   compile to a native executable
drachm run   <file.drh> [-O]                 compile and run
drachm check <file.drh>                      check for errors only
drachm explain <code>                        describe an error code, e.g. E0101

  -O   optimised build: slower to compile, much faster to run
```

## build

Writes an executable. Without `-o`, the file is named like the source, without
its `.drh` extension. `build hello.drh` writes `hello`.

## run

Builds the program in a temporary place, runs it, and removes the program
afterwards. The compiler exits with the program's exit code. Compiler
warnings are printed first.

A `main` that returns `sint32` sets the exit code:

```drachm
sint32 main() {
    return 3;
}
```

`drachm run` on that program exits with 3. A `none main()` exits with 0.

## check

Reports errors and warnings and writes nothing. Use it while you edit.

## explain

Prints the title and description of one code, for example
`drachm explain W0102`. An unknown code is an error.

## -O

`-O` gives an optimised build. The compiler writes C and calls your C compiler
(`cc`). It is slower to build and much faster to run. Both builds print the
same output, and the compiler's tests check that.

`-O` works with `build` and `run`. It does not change what the program means.

## Environment

- `NO_COLOR`: turn off coloured diagnostics.
