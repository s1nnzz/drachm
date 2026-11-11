---
title: Modules
description: in and out between files, renaming, importing everything, and C headers.
status: not-implemented
---

# Modules

A project is split into files with `in` and `out`. The one form that builds
today is `in io;` and `in si;` for the built-in modules. Imports between your
own files are designed for step 7.

## Exporting

Any file that exports something is a module. `out` lists what is visible to
other files, and everything else is private.

```drachm
// src/game/physics.drh
out Vector3, gravity;

struct Vector3 { float64 x; float64 y; float64 z; }
```

## Importing

Nothing is imported automatically in a project, not even `io`. Path-like names
use dots; call-like names use `::`.

<!-- focus: 1,2,3,4,5 -->

```drachm
in io;                              // a whole module
in physics from game as phys;       // renamed
in Vector3 from phys as v3;         // one item, renamed
in * from game.audio as audio;      // everything under one name
in * from game.audio;               // everything straight into scope

phys::Vector3 v = 1, 2, 3;
```

When `in *` brings in a name that already exists, the `import_clash` setting
chooses between a warning and an error.

## C headers

<!-- status: tentative -->

A C header can be imported the same way. The details of how C types map onto
Drachm types are open.

```drachm
in * from c."SDL2/SDL.h" as sdl;
```

## Local files in scripts

A script can import files next to it: `in helpers;` finds `helpers.drh`. See
[Scripts](design-scripts.md).
