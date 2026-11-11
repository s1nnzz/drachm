---
title: Install and run
description: Build the Drachm compiler and run, build and check a program.
---

# Install and run

## You need

- Rust, with `cargo`, to build the compiler.
- A C compiler on your `PATH` (`cc`, from gcc or clang). The compiler uses it
  to link programs, and to build `-O` programs.

## Build the compiler

From the root of the repository:

```sh
cargo build --release
```

The compiler is `target/release/drachm`. The examples below use that path.

## Run a program

Save this as `hello.drh`:

<!-- example: hello -->

Compile and run it in one step:

```sh
target/release/drachm run hello.drh
```

Or build an executable and run it yourself:

```sh
target/release/drachm build hello.drh -o hello
./hello
```

## Optimised builds

Add `-O` to any command:

```sh
target/release/drachm run hello.drh -O
target/release/drachm build hello.drh -O -o hello
```

An `-O` build takes longer to compile, because the C compiler does the work.
The program itself runs much faster. Both builds print the same output.

## Check without building

```sh
target/release/drachm check hello.drh
```

This prints errors and warnings and creates no program. The full list of
commands is on the [Command line](reference-cli.md) page.
