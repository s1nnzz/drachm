---
title: Tooling
description: drachm fmt, the Agora package registry, the language server and the standard library.
---

# Tooling

The compiler is `drachm build`, `run`, `check` and `explain` today. The rest of
the toolchain is designed and not built yet.

## drachm fmt
<!-- status: not-implemented -->

One official style with few knobs: indent width, line length and brace
placement. It never lines up `=` signs or comments across lines. It also picks
the order of `lives`, `fails` and `with` clauses when you write them either
way.

```sh
drachm fmt
```

## The Agora
<!-- status: not-implemented -->

The package registry is the Agora, at agora.drachm.dev. `drachm add json`
adds a package to `drachm.toml`. Official optional packages include `json`,
`random`, `regex` and `crypto`. Not built: today `drachm add` takes a git
address (`drachm add github.com/someone/sdl-drh@^0.4`); see
[Packages](guide-packages.md).

```sh
drachm add json
```

## The language server

Editors get errors as you type, completion, go to definition and hover from
`drachm lsp`; see [Editor support](install.md#editor-support). Bare enum variants
such as `Rect(2.0, 3.0)` rely on completion, so the server is part of the
language's design. Highlighting and the server's client exist for VS Code.

Not built yet: the inferred `self` mode as an inlay hint, completion of imports,
rename, find references and formatting.

## The standard library
<!-- status: not-implemented -->

The core is `io`, `string`, `collections`, `math` and `convert`. System
modules are `fs`, `time`, `env` and `process`. Concurrency and network modules
are `thread`, `async`, `net` and `http`. Only `io` builds today.

The input part is built: `io::inln()` returns the next line of standard input without its newline,
and `io::ended()` says whether input is finished. `line::to<uint32>()` turns
text into a number; it is a failing call you handle with `maybe`, `fails` or
`fails with`.

<!-- example: input-lines -->
