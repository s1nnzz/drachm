---
title: Organising a project
description: Splitting code into modules with in and out, describing the project in drachm.toml, tuning behaviour and warnings, scripts and file flags.
---

# Organising a project

The compiler builds a single file or a project folder with a `drachm.toml`:
modules between your own files work, the `[project]` name, kind and entry are
read, path and git dependencies are fetched, and `build`, `run` and `check` take
the folder. The `[behaviour]` settings and the other flags are designed,
and their sections are marked. The code blocks there show the intended shape,
not something you can run.

## Start small, split when it hurts

A program starts as one file, and one file is fine for a long time. Split when
a part has a name of its own (physics, config parsing, a data format) and could
be used or tested without the rest. A folder of files becomes a project.

```text
mygame/
  drachm.toml
  src/
    main.drh
    game/
      physics.drh
  tests/
```

`drachm new mygame` makes `drachm.toml`, `src/main.drh` and `tests/` for you.

## Modules: what a file shows

Any file that exports something is a module. An `out` line (or `out` before a
declaration) marks what other files may use, and everything else stays private
to the file. Export the
smallest set that other code needs, so you can change the rest without
breaking anyone.

<!-- focus: 1,2 -->

```drachm
// src/game/physics.drh
out Vector3, gravity;

struct Vector3 { float64 x; float64 y; float64 z; }
```

## Modules: what a file uses

In a project, nothing is imported automatically, not even `io`. Names that are
paths use dots, and names you call use `::`.

<!-- focus: 1-6,8 -->

```drachm
in io;                                   // a whole module
in game.physics as phys;                 // renamed
in Vector3 as V3, gravity from physics;  // items, each may be renamed
in * from game.audio as audio;           // everything under audio:: only
in * from game.audio;                    // everything straight into scope
in "../tools/clock.drh";                 // a file path may climb with ..

phys::Vector3 v = 1, 2, 3;
```

Prefer the first forms. A whole module or one renamed item keeps the origin of
every name visible where it is used. `in *` straight into scope is handy in a
small file, but it can bring in a name you already have; the `import_clash`
setting below chooses whether that is a warning or an error.

## The project file

`drachm.toml` names the project. The `[project]` section is built: `name` is
required, `kind = "library"` marks code that other projects use, and `entry`
moves the start of the program (`src/main.drh` by default). Dependencies from a
local folder (`path`) or a git tag (`git` with `tag`) are read and fetched; a
registry version such as `json = "1.2"` waits for the Agora. The build settings
below it are designed.

<!-- focus: 1-3,5-7,9,10 -->

```toml
[project]
name = "mygame"
version = "0.1.0"

[dependencies]
json = "1.2"
sdl = { git = "https://github.com/someone/sdl-drh", tag = "v0.4" }
utils = { path = "../utils" }

[build]
target = "linux-x64"
opt = true
```

## Native libraries

A project that calls C says where the headers are and what to link in a
`[native]` table:

```toml
[native]
include = ["vendor/include"]
libraries = ["SDL2"]
sources = ["c/helpers.c"]
unsafe = true               # every file may use unsafe, without a file flag
```

See [Calling C libraries](guide-c-imports.md).

## Behaviour settings

Settings that change what a program does live in `[behaviour]`, each with a
default. File flags can override settings that apply to one file.

<!-- focus: 2-7 -->

```toml
[behaviour]
overflow = "wrap"          # wrap | saturate | panic
bounds = "panic"           # panic | unchecked
missing_key = "optional"   # optional | panic
unhandled_fail = "error"   # error | crash
import_clash = "warn"      # warn | error
unsigned_negate = "wrap"   # wrap | error | signed
```

`bounds` controls runtime index checks. See [Bounds](design-config.md#bounds) and
[Unsigned negation](design-config.md#unsigned-negation).

### Other behaviour settings
<!-- status: not-implemented -->

`overflow` covers arithmetic, constants that do not fit, shifts and conversions.
A project that values catching mistakes picks `panic`; one that wants the
arithmetic of a hash function keeps `wrap`. `unhandled_fail = "crash"` lets a
call to a failing function stand without saying what to do, and stops the
program with a message if it fails; it is the default in scripts and a bad idea
in a long-lived program (see [Error handling](guide-errors-handling.md)).

## Warnings
<!-- status: not-implemented -->

`[warnings]` can make named warnings into errors, so a build fails on them, and
can switch off lints such as the one for a `copy` that does nothing on a number
(W0322).

```toml
[warnings]
as_errors = ["lossy-conversion", "missing-case"]
```

A good order: start with the defaults, turn on `as_errors` for the warnings that
have bitten you, and keep the lint switch-offs for the ones you find noisy. How
a lint is switched off is not written down yet.

## Tests live with the code
<!-- status: not-implemented -->

A `test` block in any file is a test, and `drachm test` runs them all. Put them
next to the code they check.

```drachm
test "adds" {
    assert(add(2, 3) == 5);
}
```

## File flags

A line `//--!$key: value` at the very top of a file overrides the project
setting for that file, and only for that file. The one flag the compiler reads
today is `//--!$unsafe: true`, which lets that file use `unsafe`. A project can
allow it for all its files with `unsafe = true` under `[native]`. Raw pointers need it, and also an `unsafe { }`
block at the point of use, so a file that never asks for it cannot touch a
pointer by accident. See [Unsafe](reference-unsafe.md).

<!-- focus: 1,6,7 -->

<!-- example: guide-projects-flag -->

Use a flag for a file with one odd need, such as the file that wraps a C
library. If most files need the same, change the project setting instead. The
flag `//--!$bounds: unchecked` disables runtime index checks for the file.
The `unsigned_negate` flag also takes effect per file. The `overflow` flag
is validated and awaits code generation. `missing_key`, `unhandled_fail` and `import_clash` are
for the whole project, so a file flag that sets one is E0806.

## Scripts

When the nearest configuration file is `srachm.toml`, or there is no
configuration file above the source, you can run a file as a script. Use
either top-level statements or an explicit `main()`. A script can import
local helper files and call them from its statements.

<!-- example: script-entry -->

To move statements into a project, put them inside `none main() { ... }`
and add a `drachm.toml`. See [Scripts](design-scripts.md) for current
limitations and the planned script defaults.
