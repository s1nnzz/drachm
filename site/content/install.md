---
title: Install and run
description: Build the Drachm compiler and run, build and check a program.
---

# Install and run

## Build the compiler

You need Rust with `cargo` to build the compiler, and a C compiler on your `PATH`
(`cc`, from gcc or clang). The compiler uses the C compiler to link programs and
to build `-O` programs. From the root of the repository, build in release mode.
The compiler is then `target/release/drachm`, and the examples use that path.

```sh
cargo build --release
```

## Run a program

Save the program below as `hello.drh`. `run` compiles it and runs it in one step.
If you would rather keep the executable, `build` writes it and you start it
yourself.

<!-- example: hello -->

```sh
target/release/drachm run hello.drh
target/release/drachm build hello.drh -o hello
./hello
```

## Optimised builds

Add `-O` to any command. An `-O` build takes longer to compile, because the C
compiler does the work, and the program itself runs much faster. Both builds
print the same output.

```sh
target/release/drachm run hello.drh -O
target/release/drachm build hello.drh -O -o hello
```

## Check without building

`check` prints errors and warnings and creates no program. The lessons use it to
show compiler messages, and the full list of commands is on the
[Command line](reference-cli.md) page. When you are ready to write code,
start with [Lesson 1](lesson-1-first-program.md).

```sh
target/release/drachm check hello.drh
```
