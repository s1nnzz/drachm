---
title: Projects and config
description: drachm.toml, dependencies, [behaviour], [warnings], file flags and tests.
status: not-implemented
---

# Projects and config

A project is a folder with a `drachm.toml`. Today the compiler takes one file
at a time and reads no config; this is the design for step 7.

## A new project

`drachm new mygame` makes `drachm.toml`, `src/main.drh` and a `tests/` folder.

## drachm.toml

<!-- focus: 1-3,5-7,9,10,12-13 -->

```toml
[project]
name = "mygame"
version = "0.1.0"

[dependencies]
json = "1.2"
sdl = { git = "https://github.com/someone/sdl-drh", tag = "v0.4" }
utils = { path = "../utils" }

[build]
target = "linux-x64"         # os-cpu
opt = true

[warnings]
as_errors = ["lossy-conversion", "missing-case"]
```

Dependencies come from the [Agora](design-tooling.md#the-agora), a git URL or a
local path.

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

`overflow` covers arithmetic, constants that do not fit, shifts and automatic
conversions. `unsigned_negate` decides what `-x` does on an unsigned value; with
`signed` the result becomes the next larger signed type. The compiler does not
read these settings yet: it always wraps.

## Warnings

`[warnings]` can make named warnings into errors, and can switch off lints such
as the one for a `copy` that does nothing on a number (W0322).

## File flags

A line `//--!$key: value` at the very top of a file overrides the project
setting for that file. The only flag the compiler reads today is
`//--!$unsafe: true`, which is checked in a file, and the rest are designed.

```drachm
//--!$unsafe: true
//--!$overflow: saturate
```

## Tests

A `test` block in any file is a test, and `drachm test` runs them all.

```drachm
test "adds" {
    assert(add(2, 3) == 5);
}
```
