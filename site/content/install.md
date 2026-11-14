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

## Editor support

`drachm lsp` is the language server. It speaks the Language Server Protocol on
standard input and output, so any editor with a language client can use it. While
you type it checks the program you are editing (the open, unsaved text, not the
file on disk), and it understands a whole project folder, not only one file.

- **Errors and warnings** appear as you type, with the error code, and the notes,
  help lines and secondary labels as related information.
- **Hover** shows the type of a variable or expression, a field's type, and a
  function's or method's signature.
- **Go to definition** works for locals, functions, structs, fields, methods and
  enum variants, also across the files of a project.
- **Find references** lists every use of a local, function, method, field,
  struct, enum or variant, across the files of a project (the declaration too,
  if the editor asks).
- **Rename** changes a name and all its uses in one go, across the project's
  files. It refuses, with the reason shown: a keyword, a type name, `self`, a name
  from the standard library, a name from a C import, and a name from another
  package the project uses.
- **Completion** offers methods after `::` and `->` on a value of known type,
  fields after `.`, static methods after a struct name, variants after an enum
  name, bare enum variants where the enum is known from the context (a
  declaration, an assignment, a comparison, an argument, a `case`), the names in
  scope, and keywords.

Half-typed code still gives answers: the parser keeps what it could read, so a
function missing its closing `}` still has hover and completion inside it. (Type
errors are not shown while the file has syntax errors, so a half-typed line does
not fill the Problems panel.)

**Visual Studio Code:** the extension in `editors/vscode` starts the server by
itself. Run `npm install` and `npm run compile` in that folder, then link or copy it into
your extensions folder (see its README). It runs `drachm` from your `PATH`; the
setting `drachm.path` names another compiler, and `drachm.languageServer` turns the server
off, which brings back the older check on save.

**Other editors:** start `drachm lsp` as the server for `.drh` files, with no arguments.

## Check without building

`check` prints errors and warnings and creates no program. The lessons use it to
show compiler messages, and the full list of commands is on the
[Command line](reference-cli.md) page. When you are ready to write code,
start with [Lesson 1](lesson-1-first-program.md).

```sh
target/release/drachm check hello.drh
```
