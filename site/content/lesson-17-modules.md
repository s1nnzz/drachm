---
title: "Lesson 17: Modules and projects"
description: Splitting a program into files with in and out, renaming imports, and drachm.toml.
status: not-implemented
---

# Lesson 17: Modules and projects

The compiler can't run this lesson yet; the code shows the planned design.

**Goal:** split a program across files, choose what each file shows to the
others, and describe a project in `drachm.toml`.

## out: what a file shows

Any file that exports something is a module. An `out` line lists the names that
other files can use, and everything else stays private to the file.

<!-- focus: 1 -->
`Vector3` is visible to other files. The list can go anywhere in the file, and you can also write `out` in front of a declaration: `out struct Vector3 { }`.

Anything not listed, such as a helper function in the same file, is private.

```drachm title="src/game/physics.drh"
out Vector3;

struct Vector3 { float64 x; float64 y; float64 z; }
```

## in: what a file uses

You have written `in io;` since lesson 1. In a project nothing is imported
automatically, not even `io`. Names that are paths use dots, and names you call
use `::`.

<!-- focus: 1 -->
A whole module.

<!-- focus: 2 -->
The `physics` file of the `game` folder (next to this file), under the name `phys`.
`in physics inside game as phys;` means the same.

<!-- focus: 3 -->
Items, each one may be renamed.

<!-- focus: 4-5 -->
`*` takes everything: under one name, or straight into scope.

```drachm
in io;
in game.physics as phys;
in Vector3 as V3, gravity from game.physics;
in * from game.audio as audio;
in * from game.audio;
```

A path is relative to the folder of the file that writes it. To start from the
project's `src/` folder, begin with the project's name: `in mygame.game.physics;`.

When `in *` brings in a name that already exists, a setting in `drachm.toml`
chooses whether that is a warning or an error.

### Your turn: import and use

`src/game/physics.drh` exports `Vector3`. Write the imports at the top of
`src/main.drh` so that it prints the `x` field.

```drachm title="src/game/physics.drh"
out Vector3;

struct Vector3 {
public:
    float64 x;
    float64 y;
    float64 z;
}
```

```drachm title="src/main.drh (starter)"
// your code here

none main() {
    phys::Vector3 v = 1, 2, 3;
    io::outln(v.x::text());
}
```

```text title="intended output"
1.0
```

```drachm solution
in io;
in game.physics as phys;

none main() {
    phys::Vector3 v = 1, 2, 3;
    io::outln(v.x::text());
}
```

## A project

A project is a folder with a `drachm.toml`. `drachm new mygame` creates it,
along with `src/main.drh` and a `tests/` folder. The file names the project,
its dependencies and the settings of the build.

<!-- focus: 1-3 -->
The project's name and version.

<!-- focus: 5-8 -->
Dependencies come from the Agora, a git URL, or a local path.

<!-- focus: 10-11 -->
Build settings.

<!-- focus: 13-14 -->
Warnings you want treated as errors.

```toml title="drachm.toml"
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

[warnings]
as_errors = ["lossy-conversion", "missing-case"]
```

`drachm add json` adds a dependency from the registry. Settings that change what
a program does, such as `overflow` and `bounds`, go in a `[behaviour]` section.
A project needs a `main()`, and a single file with no `drachm.toml` nearby runs
as a script instead.

### Your turn: a dependency

Add a dependency called `json` at version `1.2` to this file.

```toml title="drachm.toml (starter)"
[project]
name = "notes"
version = "0.1.0"
```

```toml solution
[project]
name = "notes"
version = "0.1.0"

[dependencies]
json = "1.2"
```

## C headers

<!-- status: tentative -->

A C header may be imported the same way, as `in * from c."SDL2/SDL.h" as sdl;`.
How C types map onto Drachm types is still open.

See [Modules](design-modules.md) and [Projects and config](design-config.md).
The last lesson is about running things at the same time:
[Lesson 18: Lambdas and concurrency](lesson-18-concurrency.md).
