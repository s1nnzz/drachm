---
title: Modules
description: in and out between files, paths, renaming, importing everything, and C headers.
---

# Modules

A project is split into files with `in` and `out`. Every file is a module.
Names work across files as described here; the C headers at the end are still
design.

## Dependency modules

A dependency publishes selected module paths in its manifest. Use the alias
from your own manifest to import them. The module's `out` declarations control
which items you can use. See [Dependencies](design-config.md#dependencies).

## Exporting

What other files may use is whatever the file marks with `out`; everything
else is private to the file. There are two ways to mark, and one file can mix
them freely, anywhere in the file: a list, which may rename with `as`, and
`out` in front of the declaration itself. Functions, structs, enums,
interfaces, failures, units and named constants can all be exported. A unit is
used by its bare name (`in fathom from lib;`), since a unit expression has no
`::`.

<!-- example: modules-basic -->

A name exported twice is an error (E0233), and so is exporting a name the file
does not declare (E0234). Using a name that is not exported is E0236, and
asking a module for a name it does not have is E0237. Passing an imported name
on (re-exporting) is not supported yet.

`X with I { }` blocks and `def`s follow the type: wherever the struct or the
interface is visible, so are the `with` blocks written for it, and importing a
struct brings the `def`s that fill it.

## Chains after a module call

A call through a module can be followed by either operator, because the module
call is not the first call on a value: `physics::gravity(2.0)::text()` and
`physics::gravity(2.0)->text()` are the same. A name that is a value, even one
named like a module, keeps the rule: `::` for the first call, `->` for each
call after it.

<!-- example: modules-chain -->

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

To climb out of a folder, write the file path in a string, relative to the
file that imports it. A string path works wherever a module path does; without
`as` the module is named after the file.

```drachm
in "../runtime/scheduler.drh";                       // named scheduler
in tick from "../runtime/scheduler.drh";             // one item
in * from "../runtime/scheduler.drh" as scheduler;   // everything, under scheduler::
```

If a path matches a built-in module and a project file at the same time (a
file `math.drh` beside you and the standard `math`), it is an error (E0231):
rename the file. A path that matches nothing is E0230, and the message lists
the files it looked for.

<!-- example: modules-paths -->

## Importing

Nothing is imported automatically in a project, not even `io`, and each file
imports what it uses: a file that uses `io::outln` without its own `in io;` is
E0206 even if another file imports `io`. Path-like names use dots; call-like
names use `::`, so a name from a module is `phys::Vector3` in types and
`phys::gravity(2.0)` in calls.

<!-- focus: 1,2,3,4,5,6 -->

```drachm
in io;                                   // a whole module
in game.physics as phys;                 // renamed
in Vector3 as V3, gravity from physics;  // items, each may be renamed
in * from game.audio;                    // everything straight into scope
in * from game.audio as audio;           // everything, under audio:: only
in * from game.audio as audio also bare; // under audio:: and straight into scope

phys::Vector3 v = 1, 2, 3;
```

`in * from m as y` puts everything under `y::` and nothing bare; add
`also bare` for both, so `play()` and `audio::play()` are the same function.

<!-- example: modules-glob -->

Two names that end up with the same spelling in one file are an error
(E0232) with a fix-it: two modules called `audio` (`game/audio.drh` and
`engine/audio.drh`) get `in engine.audio as engineAudio;`, and two items called
`Vector3` get `in Vector3 as VectorB from b;`.

A name you declare yourself that has the same name as an imported module
shadows it in that file, with a warning (W0241). When `in *` brings in a name
that already exists, your own declaration wins, then the first import, with a
warning at the later import line (W0240). The warning's help asks whether you
meant the module's own name, `m::play`.

## Cycles

Two files may import each other; their declarations see each other. What is
not allowed is a function in `a.drh` that calls into `b.drh` which calls back
into `a.drh` (recursion across files), and the error shows the chain (E0238).
Recursion inside one file is fine. The check follows direct calls of
functions; calls through methods are not followed yet.

## C headers

<!-- status: tentative -->

A C header can be imported the same way. The details of how C types map onto
Drachm types are open.

```drachm
in * from c."SDL2/SDL.h" as sdl;
```

## Local files in scripts
<!-- status: not-implemented -->

A script can import files next to it: `in helpers;` finds `helpers.drh`. See
[Scripts](design-scripts.md).
