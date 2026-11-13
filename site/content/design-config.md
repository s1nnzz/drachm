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

## Native libraries

The `[native]` table names what C imports need: `include` (header folders),
`defines`, `libraries`, `library_dirs` and `sources` (C files compiled with the
project). A project that imports a dependency links what the dependency's table
asks for. `unsafe = true` in the same table lets every file of the project use
`unsafe` (C calls and raw pointers) without the file flag; a dependency's own
table says the same for its files, so an app needs nothing for it. See
[C imports](reference-c-imports.md#the-native-table).

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

### Unsigned negation

`unsigned_negate = "wrap"` keeps the operand's unsigned type. With `error`,
negating a typed unsigned value is a compile error (E0101). With `signed`,
the result has the next wider signed type. Widening `uint128` requires
`sint256`, which is **not implemented yet** (E0901).

<!-- example: settings-unsigned-negate -->

### Missing map keys

`missing_key = "panic"` makes `map[key]` return the value directly and stops
the program with exit status 101 if the key is missing. The default,
`optional`, returns an optional. `map::get(key)` always returns an optional.

<!-- example: settings-missing-key -->

### Overflow

`overflow` says what happens when a whole number does not fit its type. It
covers `+ - *`, unary `-`, `/` (the smallest signed value divided by -1),
`**`, `++` and `--`, shifts, constants that do not fit, conversions
(automatic ones and `::to<T>()`), and the scaling of units: `x::in<km>()` and
mixed-unit arithmetic ([Units](reference-units.md#scaled-units)).

| Setting | Arithmetic | Shift outside the width | Conversion |
|---|---|---|---|
| `wrap` (default) | wraps around | the amount wraps around the width | keeps the low bits |
| `saturate` | stops at the nearest end of the type | the bits fall off | clamps to the nearest end |
| `panic` | stops the program (exit 101) | stops the program | stops the program |

Under `saturate`, a negative shift amount shifts the other way: `8 << -1` is 4
and `8 >> -1` is 16. The exact result then clamps to the type like any other
overflow, so `200 >> -1` in `uint8` is 255. Only the negative amount reverses;
a shift by a positive amount that loses bits still lets them fall off.

A float turned into a whole number drops its fraction first. NaN becomes 0
under `wrap` and `saturate`, and the infinities become 0 under `wrap` but clamp
to the type's largest or smallest value under `saturate` (`+inf` gives the
maximum, `-inf` the minimum). Under `panic` NaN and the infinities stop the
program.

A constant that does not fit is W0102 and is changed (`wrap` wraps it,
`saturate` clamps it); under `panic` it is an error (E0116), since the program
could never run it.

The setting applies per function: the project's `[behaviour]` merged with the
`//--!$overflow:` flag of the file the function is written in. Generic
functions and methods use the setting of the file that defines them.

<!-- example: settings-overflow -->

<!-- example: settings-overflow-panic -->

### Unhandled failures

`unhandled_fail = "error"` (the default in a project) makes a call to a failing
function without `maybe`, `fails` or `fails with` a compile error (E0544). With
`crash`, the call stands: if it fails, the program prints the failure's
message, as a failure that leaves `main` would, and ends with the failure's
exit code (the `exit` of its variant, or 1). A call that has handlers for only
some of the failures it can raise crashes on the others. Nothing in the
program is cleaned up on the way out, as with a failure that escapes `main`.

A lone `.drh` file with no `drachm.toml` beside it, and a script, use `crash`
by default.

<!-- example: settings-unhandled-crash -->

### Import clashes

`import_clash = "warn"` (the default) makes an `in *` import that brings a
name the file already has a warning (W0240): a declaration of the file wins
over any import, and between two imports the first one wins. With `error`
the same clash is an error (E0242), so a project can forbid hiding names
this way. A dependency follows its own `drachm.toml`. A name that hides a
module (W0241) and two modules of one name (E0232) are not affected.

<!-- example: settings-import-clash -->

The compiler says:

```text
error[E0242]: `tighten` from `spares` is hidden by the import on line 2
```

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
setting for that file. Use `//--!$unsafe: true` to allow unsafe code in that
file (or `unsafe = true` under `[native]` for the whole project).

```drachm
//--!$unsafe: true
//--!$overflow: saturate
```

`overflow`, `bounds` and `unsigned_negate` take effect per file.
`missing_key`, `unhandled_fail` and `import_clash` are project-wide: a
file flag that sets one is an error (E0806).

## Tests

A `test` block in any file is a test, and `drachm test` runs them all. Built:
see [The test keyword](guide-testing.md#the-test-keyword).

```drachm
test "adds" {
    assert(add(2, 3) == 5);
}
```
