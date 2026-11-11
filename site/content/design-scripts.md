---
title: Scripts
description: Running a single file as a script, srachm.toml, auto-imports, shebangs and inline dependencies.
status: not-implemented
---

# Scripts

Drachm is meant to double as a scripting language. A file is a script when no
`drachm.toml` is near it. Scripts are designed for step 7.

## Which kind of file

| Nearby file | Treated as |
|---|---|
| `drachm.toml` | a project (needs `main()`) |
| `srachm.toml` | a script with config (same settings as `drachm.toml`) |
| neither | a script with defaults |

## What a script gets

A script may have top-level statements and no `main`. The core modules (`io`,
`string`, collections, `env`, `fs`) are imported for you, an unhandled failure
stops it with a message, and warnings are hidden. Scripts run through
Cranelift's JIT in memory, so nothing is written to disk.

```drachm
#!/usr/bin/env drachm
io::outln("hello from a script");
string name = env::get("USER") maybe "friend";
io::outln(f"hi ${name}");
```

## Using other files and packages

A script imports local files that sit next to it: `in helpers;` finds
`helpers.drh`. Packages come from `srachm.toml`, or inline with a flag:

```drachm
//--!$dep json: 1.2
```
