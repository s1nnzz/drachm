---
title: Install and run
description: Build the Drachm compiler and run, build and check a program.
---

# Install and run

## Build the compiler

You need Rust with `cargo` to build the compiler, and a C compiler on your `PATH`.
The compiler uses the C compiler to link programs and to build `-O` programs.

- **Linux (x86-64):** `cc`, from gcc or clang.
- **Windows (x86-64):** `clang` from LLVM, plus Visual Studio or its Build Tools
  with the C++ workload, which Rust needs anyway. Use the MSVC Rust toolchain
  (the default). clang finds Visual Studio's linker and libraries by itself.

For projects that use git dependencies, `git` must be on the `PATH` as well: the
compiler clones each one into the project's `deps/` folder. Programs with only
`path` dependencies or no dependencies do not need it.

From the root of the repository, build in release mode.
The compiler is then `target/release/drachm`, and the examples use that path.

```sh
cargo build --release
```

## Run a program

Save the program below as `hello.drh`. `run` compiles it and runs it in one step.
If you would rather keep the executable, `build` writes it and you start it
yourself. On Windows the executable gets `.exe` added (`hello.exe`), and you
start it with `.\hello`.

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

## Start a project

For more than one file, make a project folder. `drachm new` writes
`drachm.toml`, a `src/main.drh` with a greeting and an empty `tests/` folder:

```sh
target/release/drachm new mygame
target/release/drachm run mygame
```

`run` and `check` also take the folder itself, or nothing at all from inside
it. The [project settings](design-config.md) say what else `drachm.toml` holds.

## Check without building

`check` prints errors and warnings and creates no program. The lessons use it to
show compiler messages, and the full list of commands is on the
[Command line](reference-cli.md) page. When you are ready to write code,
start with [Lesson 1](lesson-1-first-program.md).

```sh
target/release/drachm check hello.drh
```
