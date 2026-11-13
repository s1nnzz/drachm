---
title: Modules
description: in and out between files, paths, renaming, importing everything, and C headers.
status: not-implemented
---

# Modules

A project is split into files with `in` and `out`. The one form that builds
today is `in io;` and `in si;` for the built-in modules. The compiler already
finds the files an `in` points at, reads each once and checks the paths and the
`out` lists (the errors below are real); what the names inside them mean across
files comes next.

## Exporting

Any file that exports something is a module. What other files may use is
whatever the file marks with `out`; everything else is private. There are two
ways to mark, and one file can mix them freely, anywhere in the file.

```drachm
// src/game/physics.drh
out Vector3 as Vec3, step;          // a list; it may rename

struct Vector3 { float64 x; float64 y; float64 z; }
none step() { }

out struct Particle { Vector3 at; } // or `out` on the declaration itself
```

A name exported twice is an error (E0233), and so is exporting a name the
file does not declare (E0234). Passing an imported name on is still open.

## Paths

A module is a file, and a path says where it is. Dots or words, the same thing:

<!-- focus: 1,2 -->

```drachm
in game.physics;                    // game/physics.drh
in physics inside game;             // the same file, in words
in audio inside game.sound;         // game/sound/audio.drh
```

A path is relative to the folder of the file that writes it. To start at the
project's source folder from anywhere, begin with the project's name from
`drachm.toml`:

```drachm
in mygame.game.physics;             // src/game/physics.drh, from any file
```

If a path matches a built-in module and a project file at the same time (a
file `math.drh` beside you and the standard `math`), it is an error (E0231):
rename the file. A path that matches nothing is E0230, and the message lists
the files it looked for.

## Importing

Nothing is imported automatically in a project, not even `io`. Path-like names
use dots; call-like names use `::`.

<!-- focus: 1,2,3,4,5 -->

```drachm
in io;                                   // a whole module
in game.physics as phys;                 // renamed
in Vector3 as V3, gravity from physics;  // items, each may be renamed
in * from game.audio as audio;           // everything under one name
in * from game.audio;                    // everything straight into scope

phys::Vector3 v = 1, 2, 3;
```

Two modules with the same file name (`game/audio.drh` and `engine/audio.drh`)
cannot both be called `audio` in one file: the second import is an error
(E0232) with the fix-it `in engine.audio as engineAudio;`.

A name you declare yourself that has the same name as an imported module
shadows it in its scope, with a warning. When `in *` brings in a name that
already exists, your own declaration wins, then the first import; with
`import_clash = warn` the warning is at the later import line.

## Cycles

Two files may import each other; their declarations see each other. What is
not allowed is a function in `a.drh` that calls into `b.drh` which calls back
into `a.drh` (recursion across files). Recursion inside one file is fine.

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
