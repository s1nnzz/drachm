# Step 7: modules, config, scripts, stdlib (implementation spec)

Status: spec plus owner questions, written 2026-11-13. Nothing here is built.
Every language-behaviour point is a question in section 3; where a package
needs an answer before it can start, section 2 says which batch. Until an
answer exists a package uses the **interim** named for the question (an E0901
"not supported yet", or the narrowest reading that can be relaxed later).
**Do not decide a language question while implementing; stop at the interim and
report.**

## 0. What exists today

- One file, one `ast::Program` (`src/parser.rs`), one `hir::Program`.
  `lib::front` / `lib::check` take a single `&str`; `main.rs` reads one file.
- `diag::Span` is `{ start, end }` byte offsets into that one string, with no
  file id. Rendering (`diag::render_all`, `Source`) takes one `Source`.
- `ast::Import` is `{ module: String, span }`; the parser accepts only
  `in NAME;` (`Parser::import`). There is no `out` item and no `from`, `as` or
  `*` in the grammar. `in`/`out`/`from`/`as` are already keywords
  (`lexer.rs`); `in` and `out` are also parameter words, so `at_item_start`
  tells them apart by position.
- `typeck::check` (`src/typeck.rs` ~469): `in io` sets `Globals::io_imported`;
  `in si` parses `units::SI_PRELUDE` (`src/prelude/si.drh`, `include_str!`) and
  declares its units; anything else is E0201 "unknown module". `io::outln` etc.
  are special-cased in expression checking (`typeck.rs` ~2784 and ~3314-3390);
  using `io` without the import is E0206. `failures::PRELUDE` (core failures
  such as `ConvertError`) is parsed and merged with no import.
- Every declaration kind lives in a flat, program-wide table in `Globals`:
  `funcs_by_name`, `structs_by_name`, `enums_by_name` (failures and optionals
  are enums), the `interfaces::State` tables, `units::UnitEnv`, `defs`, and
  the generic template and instance tables in `generics::State`, keyed by
  plain name strings. `hir` and the MIR carry the same plain names, and the C
  backend emits them as symbols.
- Strings, lists, maps, `Optional` and `::to<T>()` are compiler built-ins
  (`typeck/strings.rs`, `lists.rs`, `maps.rs`), not library code. There is no
  `math`, `env`, `fs`, `time` or `process`, no `test` block, no `assert`, no
  config reading. File flags exist but `flags.rs` knows one key
  (`KNOWN_KEYS = ["unsafe"]`); it already skips a leading `#!` line.
- The runtime is C (`runtime/runtime.c`) plus a Rust staticlib
  (`runtime/strings`, built by `build.rs`, embedded with `include_bytes!`),
  linked with the system `cc` for both backends (`link.rs`). It cannot be
  loaded into the compiler's own process, which matters for the JIT (E6).
- `drachm run` builds an executable in the temp dir and runs it.

## 1. What SPEC and PLAN already decide

| Topic | Decided | Where |
|---|---|---|
| Module = one file; any file that exports something is a module | yes | PLAN step 7 item 1; SPEC 9 |
| Export list `out A, B;`, everything else private | yes (placement, renaming and re-export not stated) | SPEC 9 |
| Import forms `in io;`, `in physics from game as phys;`, `in Vector3 from phys as v3;`, `in * from game.audio as audio;`, `in * from game.audio;` | shown as examples | SPEC 9 |
| Dots for module paths, `::` for call-like (`phys::Vector3`, `io::out`) | yes | SPEC 9 |
| Nothing auto-imported in projects, not even `io` | yes | SPEC 9 |
| `in *` clash: config `import_clash` = warn or error | yes (which name wins under `warn`: not stated) | SPEC 9, 10 |
| C headers `in * from c."SDL2/SDL.h" as sdl;` | design only, not step 7 | SPEC 9 |
| Module graph, cycle detection | required; what a cycle means is not stated | PLAN step 7 item 1 |
| `drachm.toml` tables `[project] [dependencies] [build] [warnings] [behaviour]` | yes | SPEC 10 |
| `[behaviour]`: `overflow` wrap/saturate/panic, `bounds` panic/unchecked, `missing_key` optional/panic, `unhandled_fail` error/crash, `import_clash` warn/error, `unsigned_negate` wrap/error/signed; first value is the default | yes | SPEC 3, 4, 10 |
| `//--!$key: val` at the top of a file overrides per file | yes (which keys: not stated) | PLAN step 7 item 2 |
| Nearest `drachm.toml` = project (needs `main()`); nearest `srachm.toml` = script with config; neither = script with defaults | yes | SPEC 10 |
| Scripts: top-level statements lowered into a generated main; auto-import core (`io`, `string`, collections, `env`, `fs`); crash on unhandled failures; warnings hidden; shebang; local imports (`in helpers;` finds `helpers.drh`); deps in `srachm.toml` or `//--!$dep name: ver`; `cranelift-jit` in memory | yes | SPEC 10; PLAN "Decided (2026-11-10)" |
| `drachm new` makes `drachm.toml`, `src/main.drh`, `tests/` | yes | SPEC 10 |
| `test "name" { assert(...); }` in any file; `drachm test` | yes (what `assert` prints: not stated) | SPEC 10; PLAN item 4 |
| CLI `new build run check test explain` in step 7 (`add`, `fmt` listed in SPEC but not in PLAN's step 7) | yes | SPEC 10; PLAN |
| Dependencies: path first, then git; Agora registry later | yes | PLAN item 4 |
| Stdlib "middle" size; order `io` (files too), `string`, `convert`, `math`, `env`, `fs`, `time`, `process`; then `thread`, `net`/`http`; `json random regex crypto` are optional Agora packages | yes | PLAN item 5; SPEC 10 |
| `math`: `sqrt pow abs min max clamp floor ceil round sin cos tan log`, constants `pi`, `e` | yes | SPEC 4b |
| Vectors, matrices, quaternions, big integers, fractions, decimals: exact names and module | **open** | SPEC 4b |
| `io`: `out outln outf inln ended`, `text::to<T>()` | built | SPEC 10, `reference-io.md` |
| `unhandled_fail = crash`, `overflow` saturate/panic, `unsigned_negate`, `bounds`, `missing_key` | wait for step 7 | PLAN lines ~194, 205, 220, 604, 672 |
| Done when: a multi-file project with aliases builds; a script with no toml runs top-level code; behaviour settings visibly change overflow and bounds in golden tests | yes | PLAN step 7 |
| Open: the full `[warnings]` name list | grows with warnings | PLAN step 7 (Q15c) |

**Not decided anywhere** (the questions in section 3): how a module is named
in an `in` path and what it is relative to; where `out` goes, renaming and
re-export; multi-item imports; clashes; cycles; project layout and what a
dependency exports; script top-level scoping; which settings a file flag may
change and whose config a dependency obeys; the stdlib surface (module
functions vs methods, first cut, script auto-import names, `assert`, failure
types for files and system calls).

## 2. Package breakdown

Agent types follow HANDOFF: `drachm-architect` for phase restructuring and
both-backend work, `drachm-builder` for standard packages from a reviewed
brief, `drachm-writer` for docs, lessons and the showcase. **Every brief, for
every package below, must say** (CLAUDE.md):

- Update `site/` **in the same commit** the feature becomes usable: the tour
  and reference pages, a real example in `site/examples/NAME.drh` plus `.out`
  (checked by `cargo test`), the "Not implemented yet" / "Tentative" marker
  removed in that commit and never earlier, `site/gen-errors.sh` rerun when
  `src/diag/codes.rs` changes. Pages about things still unbuilt keep their
  markers.
- Extend `examples/showcase.drh` and `tests/examples/showcase.out` **in the
  same commit**, weaving the feature into the program rather than appending a
  toy.
- Commit after every step with short plain messages, no AI attribution, never
  push. CPU: `nice -n 19 env CARGO_BUILD_JOBS=3 DRACHM_TEST_JOBS=2`, targeted
  tests while working, the full `cargo test` once at the end. Both backends
  identical and leak-free. Never touch `examples/stupid.drh`. Do not decide
  language behaviour: stop at the interim and report.

Dependencies between packages:

```
WP0 (owner answers)
WP1 (multi-file front end) -> WP2 (scoped names) -> WP6 (stdlib core) -> WP7 (stdlib system)
WP3 (config, CLI)  -> WP4 (behaviour settings in codegen)
WP3 + WP2 -> WP5 (scripts) -> WP5b (JIT)
WP2 + WP3 -> WP9 (test blocks, drachm test)
all -> WP8 (docs sweep)
```

### WP0: owner answers (no code)
Batches A and B (section 3) block WP1 and WP2. Batch C blocks the second half
of WP3, WP4 and WP5. Batch D blocks WP6, WP7 and WP9. The first half of WP3
(reading toml, `drachm new`) needs only Q9.

### WP1: multi-file front end (drachm-architect)
Source map and module graph, no change to single-file behaviour.
- `diag::Span` must identify a file. Prefer a `SourceMap` with global byte
  offsets (**engineering choice**, E1): `Span` stays two words and every
  existing `Span::new(a, b)` keeps working; `diag::render_all` resolves a span
  to its file. Output text for single-file errors must not change.
- New `src/modules.rs`: `ModuleGraph` (resolve an `in` path to a file, load and
  parse each file once, import edges, cycle detection with the chain in the
  message). The stdlib registry (WP6; `io` and `si` for now) is consulted by
  the same lookup (E10).
- `ast::Import` gets real fields (path, items, alias, glob); `ast::Program`
  gets `exports`; `Parser::import` grows the forms and a new `out` item is
  added. Parser unit tests for each form, accepting and rejecting.
- `lib::front_project(root)` returns the ordered module ASTs; the string entry
  stays for tests as a one-module graph. `flags.rs` applied per file.
- New codes in `src/diag/codes.rs`: module not found, import cycle, duplicate
  export, exporting an unknown name, unusable file name.
- Files: `src/diag/mod.rs`, `src/ast.rs`, `src/parser.rs`, `src/lib.rs`,
  `src/main.rs`, `src/flags.rs`, `src/diag/codes.rs`, new `src/modules.rs`.
- Tests: parser unit tests; a new `tests/modules.rs` harness building temp
  project trees and asserting the graph, the cycle message and the not-found
  message; all existing goldens unchanged.
- Site and showcase: nothing is usable yet, so `design-modules.md` keeps its
  marker and the showcase is unchanged; regenerate the error list.
- Depends: Q1, Q2, Q3 (path form, root, export syntax); Q5 only for the
  multi-item grammar.

### WP2: scoped names across modules (drachm-architect, the hard one)
Everything that is a program-wide name today becomes module-scoped.
- Give every top-level entity an identity `(ModuleId, name)` and each module a
  scope table `visible name -> entity`, built from its own declarations, its
  imports (aliases, items, `*`) and the always-present core prelude
  (`Optional`, `ConvertError`, `List`, `Map`, the built-in interfaces). The
  flat maps in `Globals` (`funcs_by_name`, `structs_by_name`,
  `enums_by_name`, interface, unit, def and generic template tables) are
  re-keyed by id or by a mangled name (`game.physics.Vector3`); the architect
  writes a short design note for this at the top of the work and decides it
  (an engineering choice). The C backend needs valid symbols: mangle once, in
  hir construction (E5).
- Qualified names: `phys::Vector3` in types, struct construction, calls, enum
  variants (`Shape::Circle`), generic arguments; the order of resolving `a::b`
  per Q4.
- Visibility: only exported names are usable from another module (new error
  family beside E0212 for fields).
- Units: `in si;` becomes a normal import of the embedded `si` module; units
  must be exportable and importable (`units::UnitEnv` is flat today). `io`
  stops being special: the special cases at `typeck.rs` ~2784 and ~3314-3390
  are replaced by the embedded `io` module's functions (E4).
- `X with I { }` blocks and `def`s across modules (Q6b).
- Generic instance and interface tables key on the defining module.
- Files: `src/typeck.rs` (`check`, `Globals`, lookups),
  `src/typeck/{generics,interfaces,enums,failures,fill,units,overload,construct}.rs`,
  `src/hir.rs`, `src/mir/build.rs`, `src/cgen_common.rs`, `src/codegen_mir.rs`
  (symbol names), `src/modules.rs`.
- Tests: golden project trees (a `tests/projects/NAME/` folder with `src/` and
  `NAME.out`, run by `tests/run.rs` on both backends with the leak check); fail
  cases for private names, unknown module, clash, cycle; every existing
  single-file test must give byte-identical output and MIR dumps.
- Site: `design-modules.md` and `lesson-17-modules.md` lose their markers and
  become real, `guide-projects.md` updated. Multi-file examples need a folder
  form (`site/examples/NAME/src/main.drh` + `NAME.out`) and `site_examples`
  support for it (E11).
- Showcase: add the module feature via a small `examples/showcase_parts/` (or
  similar) folder imported by `showcase.drh`; whether the showcase itself
  becomes a project folder is the owner's call (E11).
- Depends: WP1, Q1 to Q8.

### WP3: config, project discovery, CLI (drachm-builder from a reviewed brief)
- Add the `toml` crate. New `src/config.rs`: `Config { project, dependencies,
  build, warnings, behaviour }`, the six behaviour enums with defaults, errors
  for unknown keys and bad values with spans into the toml file (new codes in a
  free range, named in the commit).
- Project discovery: nearest `drachm.toml` or `srachm.toml` walking up from the
  given file or folder; neither = script defaults. Pure functions, unit tests
  on temp dirs.
- CLI: `drachm new NAME` (creates `drachm.toml`, `src/main.drh` with a hello
  program, `tests/`); `build`/`run`/`check` accept a file or a project folder
  (default `.`); `test` is wired in WP9; `explain` unchanged. `main.rs` moves to
  a `Command` enum; the existing flags (`-o`, `-O`, `--dump-mir`,
  `--dump-borrowck`) keep working on a single file path.
- Dependencies: parse and validate the table; `path` ones are handed to WP2;
  git and registry entries are E0901 in this step unless Q12 says otherwise.
- Tests: toml unit tests; a harness case that runs `drachm new` in a temp dir
  then `drachm run`.
- Site: `design-config.md`, `reference-cli.md`, `install.md`; the `drachm new`
  walk-through is a text page (not a runnable example).
- Showcase: unchanged (config does not show in one program).
- Depends: Q9 (layout); Q11 and Q12 for the exact `[behaviour]`/`[dependencies]`
  rules. Can run in parallel with WP1.

### WP4: behaviour settings in codegen (drachm-architect; both backends and the MIR)
- Plumb `Config.behaviour` merged with file flags (Q11) into `hir` as a
  per-function `Settings`, so modules with different flags work in one build.
- `overflow = saturate | panic` on arithmetic, shifts, constants that do not
  fit and automatic conversions: MIR ops carry a mode; Cranelift
  (`codegen_mir.rs`) and C (`cgen_mir.rs`, `__builtin_*_overflow`) agree bit for
  bit; panic mode uses the existing panic path (message, exit 101).
- `unsigned_negate = error | signed`; `bounds = unchecked` drops index checks;
  `missing_key = panic` makes `m[k]` a `V` that stops the program on a miss;
  `unhandled_fail = crash` lets a call with no handler compile and stop with
  the failure's message and exit code.
- Tests: `tests/run/settings_*.drh` using file flags (so the harness needs no
  toml) plus a project case; `backends` and `leaks`; rerun `bench/run.sh` for
  `unchecked`.
- Site: `design-config.md` loses its marker for the keys that build,
  `reference-types.md` overflow text, one example per setting.
- Showcase: a section that uses `//--!$overflow: saturate` (after WP2 in a
  helper module).
- May split: 4a `overflow` and `unsigned_negate` (architect), 4b `bounds`,
  `missing_key`, `unhandled_fail` (builder).
- Depends: WP3 (Config), Q11.

### WP5: scripts (drachm-architect for lowering, builder for CLI plumbing)
- A script file allows statements at top level mixed with function, struct and
  other declarations; a lowering step before typeck builds the generated `main`.
  Shebang line recognised by the lexer as well as `flags.rs`.
- Auto-imports per Q14, crash on unhandled failures (shares WP4's
  `unhandled_fail = crash`), warnings hidden, scripts' top-level scoping per Q10.
- `srachm.toml` and the inline `//--!$dep name: ver` flag parsed (resolved per
  Q12).
- Interim execution: build to a temp executable like `drachm run` does today.
  The JIT is WP5b (E6).
- Tests: a new `tests/scripts/` set (with and without `srachm.toml`, shebang,
  crash with exit code, local import after WP2).
- Site: `design-scripts.md` loses its marker, a guide example.
- Showcase: scripts cannot be part of one program; extend the harness with
  `examples/scripts/*.drh` (tested like examples) and link one from the
  showcase page.
- Depends: WP3, WP2 for local imports, Q10, Q14.

### WP5b: JIT for scripts (drachm-architect, separable, can slip)
- `cranelift-jit` backend sharing `codegen_mir` (abstract over `Module`). The
  hard part is the runtime: see section 0 and E6. Options: link the Rust runtime
  into the compiler as an rlib and register its symbols; build `runtime.c` into
  the compiler through `build.rs`; or dlopen a shared build.
- Tests: the WP5 script suite through the JIT with the same expected output and
  the leak check.
- Depends: WP5.

### WP6: stdlib core as modules (drachm-architect for the mechanism, then builder)
- Mechanism (E3): modules are embedded `.drh` files (`src/stdlib/*.drh`,
  `include_str!`, like `si.drh`) with a compiler-only intrinsic form reaching
  the runtime. `io` moves first with unchanged behaviour and tests, then `math`
  (libm in both backends, `pi`, `e`), `convert`, `string` per Q13.
- Tests: each function has a `tests/run` case and a `site/examples` case; float
  edge cases (NaN, negative zero) identical on both backends.
- Site: one reference page per module (`reference-math.md`, ...), tour pages,
  `site/gen-errors.sh`.
- Showcase: use `math` with units, `convert`.
- Depends: WP2, Q13, Q14.

### WP7: stdlib system modules (drachm-builder, one architect review)
`env`, `fs`, `time`, `process`, in the order of Q13; file operations in `io`.
New runtime functions in the Rust staticlib and the C runtime. Handles are
structs whose `drop()` closes them (leak-free). Failure types per Q16. Tests use
the temp dir and only read the real environment. Depends: WP6, Q13, Q16, Q17.

### WP9: test blocks and `drachm test` (drachm-builder)
`test "name" { }` items, `assert`, a runner that builds one executable per
project and prints pass and fail counts; test code is dropped from normal
builds. Depends: WP2, WP3, Q15.

### WP8: docs sweep (drachm-writer)
`design-modules`, `design-scripts`, `design-config`, lesson 17,
`guide-projects`, `guide-testing`, `reference-cli`, SPEC sections 9 and 10, PLAN
progress and done-when check, HANDOFF, the living design doc, the showcase page.
Last.

### Order and parallelism
1. Batch A, then B answered. 2. WP1; WP3 in parallel once batch C is in.
3. WP2 alone (it touches most of typeck; no other typeck agent meanwhile).
4. WP4a and WP5 may run beside WP2 only if they avoid typeck files; merge
`hir.rs` carefully. 5. WP6, WP9, WP7, WP5b. At most 2-3 compiling agents at once.

## 3. Open questions for the owner

Ordered by what they block. All options are code; labels are mine and none is
marked recommended. "(original)" marks a wordy-English idea. The interim is
what the implementer builds until the owner answers.

### Batch A: blocks WP1 (names, paths, exports)

**Q1. How is a module named in an `in` path?**
SPEC 9 shows `in physics from game as phys;` and `in * from game.audio;`. Say
how a file `src/game/physics.drh` is written. 1b: may a file name be anything
(`my-file.drh`, `Physics.drh`), or must it be a camelCase identifier? 1c: if
the project has `src/math.drh` and the stdlib has `math`, which does
`in math;` mean?

```cpp
// Option 1: the SPEC form: module first, folder after `from` (dots for deeper folders)
in physics from game;
in audio from game.sound;
in helpers;                       // a file at the source root

// Option 2: one dotted path, the last part is the file
in game.physics;
in game.sound.audio as audio;

// Option 3 (original): the place in words
in physics inside game;
in audio inside game.sound;
in helpers beside this;           // the folder of this file

// Option 4: a string path, so odd file names work
in "game/physics";
```
Interim: Option 1; file names must be identifiers; a project file and a stdlib
module of the same name is an error.

**Q2. What is a path relative to?**

```cpp
// src/game/ai/brain.drh wants src/game/physics.drh and src/game/ai/senses.drh

// Option 1: always from the source root, the same in every file
in physics from game;
in senses from game.ai;

// Option 2: from the importing file's folder; `up` climbs one level
in senses;
in physics from up;

// Option 3: a plain name looks beside the file first, then from the root
in senses;
in physics from game;

// Option 4 (original): the path says where it starts
in physics from root game;
in senses from here;
in physics from above;            // one folder up
```
Interim: Option 1 for projects; a script's root is its own folder (SPEC).

**Q3. How does a file say what it exports?**
SPEC shows an export list first in the file. Not stated: whether it must be
first, renaming on the way out, re-exporting an import, and whether exporting
a type exports its methods (today `public:`/`private:` inside the struct
decide).

```cpp
// Option 1: the SPEC list
out Vector3, gravity;
struct Vector3 { float64 x; float64 y; float64 z; }

// Option 2: a word on the declaration
out struct Vector3 { float64 x; float64 y; float64 z; }
out float64 gravity = 9.81;

// Option 3: list anywhere, may rename and may pass an imported name on
out Vector3 as Vec3, gravity;
out audio;                        // an imported module, shown again

// Option 4 (original): everything shown unless marked
hidden float64 secret = 4.0;
struct Vector3 { float64 x; float64 y; float64 z; }
```
Interim: Option 1, anywhere in the file, no renaming, no re-export.

**Q4. A name two things want: module, variable, local declaration.**
`phys::Vector3` and `x::len()` both start `name::`.

```cpp
in physics from game as phys;
none main() {
    float64 phys = 1.0;           // a variable named like the module
    phys::Vector3 v = 1, 2, 3;    // which `phys`?
}

// Option 1: an error at the second declaration (like E0202)
// Option 2: the inner name shadows, with a warning
// Option 3: before `::` a module always wins, a variable wins everywhere else
// Option 4 (original): the import must be renamed first;
//           error with the fix-it `in physics from game as phys2;`
```
Same question for an import and a top-level declaration of the same name.
Interim: Option 1 for imports against imports or top-level declarations,
Option 3 for variables.

### Batch B: blocks WP2 (the rest of import semantics)

**Q5. Importing several items from one module.**

```cpp
// Option 1: one item per line (the SPEC forms only)
in Vector3 from physics;
in gravity from physics;

// Option 2: a comma list, each item may be renamed
in Vector3, gravity from physics;
in Vector3 as V3, gravity from physics;

// Option 3: a braced list
in physics { Vector3, gravity as g };

// Option 4 (original): spoken, with `and` like `with ... and ...`
in Vector3 and gravity from physics;
in Vector3 as V3 and gravity as g from physics;
```
Interim: Option 1.

**Q6. Two modules with the same file name.**
`game/audio.drh` and `engine/audio.drh`.

```cpp
in audio from game;
in audio from engine;             // same name

// Option 1: an error with the fix-it `as`
in audio from engine as engineAudio;

// Option 2: allowed; a bare `audio::play()` is an error ("ambiguous") only where used,
//           the folder-qualified forms game.audio::play() always work

// Option 3 (original): every import also opens its folder name
in audio from game;               // audio::play() and game::audio::play()

// Option 4: the later import must carry an alias; the first keeps the plain name
```
6b: do `X with I { }` blocks and `def`s apply everywhere once their module is
loaded, only in files that import their module, or only where the block's own
module is imported? Interim: Option 1; 6b everywhere once loaded.

**Q7. `in *` meeting a name that already exists.**
`import_clash = warn | error` is decided; with `warn`, which name is used and
where does it warn?

```cpp
in * from game.audio;             // exports play
in * from game.video;             // also exports play
none play() { }                   // and a local one

// Option 1: a local declaration beats imports; between imports the first wins; the warning is at the import
// Option 2: the clashing name cannot be used bare at all (using `play()` is an error);
//           `warn` only decides whether the import line itself is reported
// Option 3: the later import wins, warning at the import line
// Option 4 (original): the use names its source in words
play() from audio;                // needed only when the bare name is ambiguous
```
Interim: Option 2 (never silently picks one).

**Q8. Import cycles.**
`a.drh` imports `b.drh` and `b.drh` imports `a.drh`. Declaration order inside a
file already does not matter.

```cpp
// Option 1: always an error; the message shows the chain a -> b -> a
// Option 2: allowed; all declarations see each other across the cycle
//           (a struct that would contain itself is still E0210)
// Option 3: allowed for functions and borrowed types; an error when the cycle runs
//           through an owned field
// Option 4 (original): allowed when one side marks the loop-closing import
in billing from shop later;
```
Interim: Option 1.

### Batch C: blocks WP3 (second half), WP4, WP5

**Q9. Project layout: entry point and libraries.**

```cpp
// Option 1: src/main.drh is the program; every other file is a module;
//           no src/main.drh means a library
// Option 2: the toml says
//   [project]
//   name = "geometry"
//   kind = "library"            # "program" by default
//   entry = "src/main.drh"
// Option 3: every file in programs/ with a main() is its own executable
// Option 4 (original): the toml lists what the project makes, in words
//   makes = ["program mygame from src/main.drh", "library geometry from src/geometry.drh"]
```
9b: how a dependency's modules are named in the user's project:
Option 1 `in json;` takes the package's `src/lib.drh`, other exported files via
`in parser from json`; Option 2 `in parser from json` for any exported file, no
root file; Option 3 (original) the package's toml says
`shows = ["parser", "writer"]`. Interim: Q9 Option 1, 9b Option 1.

**Q10. Scripts: where do top-level variables live?**

```cpp
#!/usr/bin/env drachm
uint32 total = 0;
none addTo(uint32 n) { total += n; }   // can this see `total`?
addTo(5);

// Option 1: no. Top-level statements are the body of a generated main;
//           functions see only parameters and their own locals
// Option 2: yes. Top-level variables are script-wide state functions can read and change
// Option 3: functions can read them (shared borrow) but not change them
// Option 4 (original): a word marks the shared ones; unmarked ones stay in the script body
shared mut uint32 total = 0;
```
10b: may a script also define `main()`: called after the top-level statements,
instead of them, or an error? Interim: Option 1; `main()` in a script is an
error.

**Q11. Which settings can a file change, and whose settings does a dependency obey?**

```cpp
// Option 1: any behaviour key as a flag at the top of any file;
//           a dependency obeys its own drachm.toml
//   //--!$overflow: saturate

// Option 2: only the root project decides; flags work in the project's own files;
//           dependencies are compiled with the root's [behaviour]

// Option 3: flags may set only keys that change one function's code
//           (overflow, unsigned_negate, bounds); the other three are project-wide

// Option 4 (original): a block form for a few lines, beside the file flag
with overflow saturate {
    uint8 b = a + 200;
}
```
Interim: Option 3, a dependency obeys its own toml, no block form.

**Q12. Dependencies in step 7: how far, and how named?**

```cpp
// Option 1: path only now
//   utils = { path = "../utils" }
// Option 2: path and git (cloned into a cache folder, tag pinned, a lock file)
//   sdl = { git = "https://github.com/someone/sdl-drh", tag = "v0.4" }
// Option 3: path, git, and the inline script form
//   //--!$dep json: 1.2            (registry entries stay E0901)
// Option 4 (original): a dependency is given the local name used in `in`
//   maths = { path = "../utils", called = "geo" }     # in vector from geo;
```
Interim: Option 1; git and registry entries are E0901.

### Batch D: blocks WP6, WP7, WP9 (stdlib)

**Q13. What is in the first stdlib cut, in what order?**

```cpp
// Option 1: PLAN order: io(files) -> string -> convert -> math -> env -> fs -> time -> process
// Option 2: what scripts need first: io(files), env, fs, math, process; then the rest
// Option 3: what the test runner needs first: math, convert, string, assert; system modules after
// Option 4 (original): a small daily-use module that shows the common names in one import
in kit;                           // outln, inln, sqrt, pi, args, readFile ...
```
13b: is `string` a module at all? Text already has methods (`s::len()`,
`s::split(",")`); a module could hold what is not a method (`string::join`?).
13c: the open names for vectors, matrices, quaternions, big integers,
fractions, decimals (SPEC 4b): which module and which type names? Interim:
Option 1; `string` stays methods only; the numeric types are not started.

**Q14. Module functions, methods, and what scripts get bare.**

```cpp
float64 r = math::sqrt(2.0);              // Option 1: module functions
float64 r = 2.0::sqrt();                  // Option 2: methods on numbers
// Option 3: both spellings call the same function
float64 a = math::sqrt(2.0);
float64 b = 2.0::sqrt();

// Option 4 (original): scripts see the auto-imported functions bare; projects name the module
outln("hi");                              // in a script
io::outln("hi");                          // in a project
```
14b: constants `math::pi` or a bare `pi`. Interim: Option 1, `math::pi`,
`math::e`; scripts qualify like projects (the auto-import saves only the `in`
line).

**Q15. `assert` and a test failure.**

```cpp
test "adds" {
    assert(add(2, 3) == 5);               // Option 1: prints "assertion failed: add(2, 3) == 5" and the line
    assert(add(2, 3) == 5, "adding");     // Option 2: the same plus your message
}
// Option 3: a comparison form that shows both values
test "adds" { assert add(2, 3) is 5; }    // "left 4, right 5"

// Option 4 (original): tests can also expect a failure
test "bad port" {
    expect failure ConvertError from "x"::to<uint32>();
}
```
15b: may `assert` be used in normal code, and does it run in `-O` builds?
15c: the `[warnings] as_errors` names (PLAN open): may I derive kebab-case names
from the W codes in `src/diag/codes.rs` and list them in the docs? Interim:
Option 1, `assert` allowed in normal code and runs in every build; names
derived.

### Batch E: later, not blocking the first packages

**Q16. Failure types for `fs`, `env`, `process`, file `io`.**

```cpp
// Option 1: one shared failure
failure IoError { NotFound(string path), Denied(string path), Other(string why) }
// Option 2: one failure per module: FsError, EnvError, ProcessError
// Option 3: failures per cause, raised by any module: NotFound, Denied, BadPath
// Option 4 (original): the failure names the trouble with its own words and exit codes
failure FileTrouble {
    Missing(string path)  f"no such file ${path}"  exit 66,
    Locked(string path)   f"cannot open ${path}"   exit 77,
}
```
Interim: Option 1.

**Q17. `env` and `process`.**

```cpp
string[?] a = env::args();                // Option 1: a list, first user argument first
string[?] b = env::args();                // Option 2: item 0 is the program name
uint32 n = env::argCount();               // Option 3: count plus indexed `env::arg(i)`
string home = env::get("HOME") maybe "/"; // Option 4 (original): everything is looked up by name,
                                          //   `env::args` is `env::get("args")` split by the caller: no list function
process::exit(3);
string text = process::run("ls", ["-l"]) fails with (any e) => "";
```
Interim: Option 1; `env::get` gives `string?`.

**Q18. `drachm fmt` and the Agora.**
PLAN's step 7 CLI list leaves `fmt` and `add` out. Confirm both wait
(`fmt` with the language-server token work, `add` with the registry). Interim:
not built.

## 4. Engineering choices that change what users see

- **E1. Diagnostics name files.** Multi-file errors print
  `src/game/physics.drh:10:15` (relative to the project root). Single-file
  output is unchanged. Done with a global-offset `SourceMap`.
- **E2. New error codes** in the `02xx` names range (module not found, import
  cycle, name not exported, duplicate export, ambiguous name) plus a block for
  toml and config errors; wording is mine, for the owner's batch review;
  `site/gen-errors.sh` reruns.
- **E3. The stdlib is Drachm source embedded in the compiler** (`include_str!`,
  like `si.drh`), with a compiler-only intrinsic form for the few operations that
  must reach the runtime. Users see: stdlib source is real code with real
  diagnostics inside it, no install folder, the single binary still works.
- **E4. `io` and `si` become ordinary modules.** Behaviour is identical; E0206
  (missing import) stays; the message for a typo changes from "`io` and `si` are
  the only modules so far" to a module-not-found listing the stdlib modules.
- **E5. Module-level names are mangled in generated symbols.** Visible only in
  `--dump-mir` output (`game.physics.dist`) and in the C source of `-O` builds.
- **E6. Scripts first run as a normal build in the temp dir; the JIT is WP5b.**
  Visible difference: start-up of `drachm run script.drh` costs a compile and a
  link until then, not "nothing written to disk" as SPEC says. The runtime is not
  loadable into the compiler process today, so the JIT changes how the runtime is
  shipped.
- **E7. Project builds are whole-program.** All modules merge into one
  `hir::Program` and compile together: no per-module objects, no incremental
  cache. This suits step 7.5 (cross-module inlining) and keeps both backends
  identical, but a build recompiles everything.
- **E8. Unknown keys are errors.** An unknown `[behaviour]` key, toml key or file
  flag key is an error, not a warning, so a typo cannot silently keep a default.
- **E9. New crates:** `toml` (WP3) and `cranelift-jit` (WP5b).
- **E10. Search order for `in NAME`:** project files, then dependencies, then the
  stdlib, and an error if two match, so adding a stdlib module later can never
  silently change what a program means. `tests/` is not importable from `src/`.
- **E11. Folder forms for multi-file examples.** Site examples may be
  `site/examples/NAME/src/main.drh` with `NAME.out`, and the harness learns that
  shape; single files stay as they are. The showcase (one file, also read at site
  build time by `site/content/showcase.md`) either imports a folder of parts or
  becomes a project folder: the owner's call.
