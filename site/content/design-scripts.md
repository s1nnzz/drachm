---
title: Scripts
description: Running a single file as a script, srachm.toml, auto-imports, shebangs and inline dependencies.
---

# Scripts

The nearest `drachm.toml` or `srachm.toml` determines how Drachm treats a file.

## Which kind of file

| Nearby file | Treated as |
|---|---|
| `drachm.toml` | a project (needs `main()`) |
| `srachm.toml` | a script with config (same settings as `drachm.toml`) |
| neither | a script with defaults |

## Starting a script

Use either an explicit `main()` or top-level statements. The compiler reports
E0204 and points to both locations if you mix them. Statements run in source
order and may call functions, use named constants, and declare locals inside
loops and other statement bodies. A shebang can occupy the first line, before
any file flags.

<!-- example: script-entry -->

Run the file with `drachm run script-entry.drh` or
`drachm run script-entry.drh -O`. Both commands build and run a temporary
executable. An explicit `sint32 main()` can set the exit status.

## Local imports

A script imports files next to it. For example, `in helpers;` finds
`helpers.drh`. Import standard modules explicitly, as in `in io;` above.

## Shared declarations and execution defaults
<!-- status: not-implemented -->

Script-wide variables will be shared by functions. Writes require `mut`,
and declarations initialize before the first statement. Runtime top-level
bindings, returning or propagating a failure from the generated entry, and
unhandled failures in its statements currently report E0901.

The design calls for automatic core imports, hidden warnings, crash on
unhandled failure, and in-memory Cranelift execution.

## Inline dependencies
<!-- status: not-implemented -->

The planned inline syntax is `//--!$dep json: 1.2`. Registry dependencies
are pending.
