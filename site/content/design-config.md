---
title: Projects and config
description: drachm.toml, dependencies, [behaviour], [warnings], file flags and tests.
---

# Projects and config

A project is a folder with a `drachm.toml`. `build`, `run` and `check` take the
folder, and the program starts in the entry file. The `[project]` section is
built; the other sections are the design, and each one is marked until it
builds.

## A new project

`drachm new mygame` makes `drachm.toml`, `src/main.drh` and a `tests/` folder.

## drachm.toml

<!-- focus: 1-4 -->

```toml
[project]
name = "mygame"              # starts paths like `in mygame.game.physics;`
version = "0.1.0"
kind = "program"             # program (the default) | library
entry = "src/main.drh"       # the file that starts the program (the default)
```

`name` is required. It must be a plain name (letters, digits and `_`). The
source root is the folder of the entry file, so a path that starts with the name
begins there. A library has no program to run: `drachm run` and `drachm build`
say so (E0811), while `drachm check` checks the library without requiring `main()`.

## Dependencies

A dependency is a folder on this machine (`path`) or a git repository at a
`tag`. Both are built: the compiler finds the folder, reads its own
`drachm.toml` and resolves its dependencies in turn. A registry version such as
`json = "1.2"` is not supported yet (E0901): the Agora comes later.

```toml
[dependencies]
sdl = { git = "https://github.com/someone/sdl-drh", tag = "v0.4" }
utils = { path = "../utils" }
```

A git dependency is cloned at its exact tag with `git`, which must be on the
`PATH`, into `deps/NAME` in the project's folder. Versions are tags, pinned
exactly: there are no ranges yet. The commit it came from is written to
`drachm.lock` beside `drachm.toml`, and the next build uses that commit even if
the tag moves later. If the tag moved and the commit is not in `deps/`, the
build stops (E0809); delete `drachm.lock` to take the new commit. Commit the
lock file; whether `deps/` is committed is up to you.

### Dependency imports

A dependency obeys its own `drachm.toml`. It publishes module paths with
`public_modules = ["survey.readings"]` under `[project]`. Import a listed module
through the alias declared under `[dependencies]`, for example
`in chart_depth from charts.survey.readings;`. Each module exports its own
items with `out`. The dependency can import its private modules internally.

<!-- example: dependency-modules -->

## Build settings
<!-- status: not-implemented -->

```toml
[build]
target = "linux-x64"         # os-cpu: linux-x64, macos-arm64, windows-x64
opt = true
```

`opt = true` makes `build` and `run` act as if `-O` was given. Only the machine
the compiler runs on can be the target.

## Behaviour settings

Settings that change what a program does live in `[behaviour]`. Each has a
default.

```toml
[behaviour]
overflow = "wrap"          # wrap | saturate | panic
bounds = "panic"           # panic | unchecked
missing_key = "optional"   # optional | panic
unhandled_fail = "error"   # error | crash
import_clash = "warn"      # warn | error
unsigned_negate = "wrap"   # wrap | error | signed
```

### Bounds

`bounds = "panic"` checks collection indexes at runtime. With `unchecked`,
the runtime check is omitted. A negative index still counts from the end.
The compiler still checks constant array indexes.

<!-- example: settings-bounds -->

### Other behaviour settings
<!-- status: not-implemented -->

`overflow` covers arithmetic, constants that do not fit, shifts and automatic
conversions. `unsigned_negate` decides what `-x` does on an unsigned value; with
`signed` the result becomes the next larger signed type.

## Warnings
<!-- status: not-implemented -->

`[warnings]` can make named warnings into errors, and can switch off lints such
as the one for a `copy` that does nothing on a number (W0322).

```toml
[warnings]
as_errors = ["lossy-conversion", "missing-case"]
```

## File flags

A line `//--!$key: value` at the very top of a file overrides the project
setting for that file. Use `//--!$unsafe: true` to allow unsafe code.

```drachm
//--!$unsafe: true
//--!$overflow: saturate
```

`bounds` takes effect per file. File flags for `overflow` and
`unsigned_negate` are accepted and await code generation. `missing_key`, `unhandled_fail` and `import_clash` are project-wide: a
file flag that sets one is an error (E0806).

## Tests
<!-- status: not-implemented -->

A `test` block in any file is a test, and `drachm test` runs them all.

```drachm
test "adds" {
    assert(add(2, 3) == 5);
}
```
