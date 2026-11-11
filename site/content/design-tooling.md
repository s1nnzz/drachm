---
title: Tooling
description: drachm fmt, the Agora package registry, the language server and the standard library.
status: not-implemented
---

# Tooling

The compiler is `drachm build`, `run`, `check` and `explain` today. The rest of
the toolchain is designed and not built yet.

## drachm fmt

One official style with few knobs: indent width, line length and brace
placement. It never lines up `=` signs or comments across lines. It also picks
the order of `lives`, `fails` and `with` clauses when you write them either
way.

```sh
drachm fmt
```

## The Agora

The package registry is the Agora, at agora.drachm.dev. `drachm add json`
adds a package to `drachm.toml`. Official optional packages include `json`,
`random`, `regex` and `crypto`.

```sh
drachm add json
```

## The language server

<!-- status: tentative -->

Editors get errors as you type, completion, go to definition and hover. Bare
enum variants such as `Rect(2.0, 3.0)` rely on completion, so the server is part
of the language's design. Highlighting already exists for VS Code.

## The standard library

The core is `io`, `string`, `collections`, `math` and `convert`. System
modules are `fs`, `time`, `env` and `process`. Concurrency and network modules
are `thread`, `async`, `net` and `http`. Only `io` builds today.

`io::inln()` returns the next line of standard input without its newline,
and `io::ended()` says whether input is finished. `line::to<uint32>()` turns
text into a number, and with errors it becomes a failing call you handle with
`maybe`.

```drachm
while (!io::ended()) {
    string line = io::inln();
    uint32 n = line::to<uint32>() maybe 0;
    io::outln(n::text());
}
```
