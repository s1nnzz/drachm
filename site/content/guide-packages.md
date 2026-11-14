---
title: Packages
description: Add git packages with drachm add, version ranges, the lock file, deps/, several versions side by side, and packages that wrap C libraries.
---

# Packages

A package is a git repository with a `drachm.toml`: a library other projects
depend on. Until the Agora registry exists, packages are found by their git
address, and a version is a git tag. Drachm runs the `git` program for this, so
`git` must be on your `PATH`, and your own git configuration (credentials,
proxies) applies.

The examples on this page need a network, so they are not run by the site's
checks; the compiler's tests run every command here against local repositories.

## Adding a package

```sh
drachm add github.com/someone/sdl-drh@v0.4      # one exact tag
drachm add github.com/someone/sdl-drh@^0.4      # the newest tag that fits ^0.4
drachm add github.com/someone/sdl-drh           # the newest version tag, pinned
```

An address without `://` is taken as `https://`. `drachm add` edits your
`drachm.toml` (your comments and layout stay), fetches the package into
`deps/` and writes `drachm.lock`:

```toml
[dependencies]
sdl_drh = { git = "https://github.com/someone/sdl-drh", version = "^0.4" }
```

The dependency is named after the repository, with `-` turned into `_` so it
can start an import path (`in sdl_drh.window;`). `--name sdl` chooses another
name. If the fetch fails, `drachm.toml` is left as it was.

You can also write the entry by hand: `tag = "v0.4"` names one exact tag,
`version = "^0.4"` a range. A build fetches whatever `deps/` and the lock file
are missing.

## Versions

A tag is a version when it reads `1.2.3`, with an optional `v` in front and as
few as one number (`v0.4` is 0.4.0). Tags like `nightly` or `v1.0.0-rc1` are
never picked by a range, but `tag = "..."` can name them.

`^` means the newest version that does not change the leftmost non-zero number:

| range | takes |
| --- | --- |
| `^1.2.3` | from 1.2.3 up to, not including, 2.0.0 |
| `^0.4` | from 0.4.0 up to, not including, 0.5.0 |
| `^0.0.3` | exactly 0.0.3 |

## The lock file

`drachm.lock` records what was fetched, so every machine and every build gets
the same files. Commit it. It is written by drachm; do not edit it.

```toml
# written by drachm, do not edit

[[package]]
name = "sdl-drh"
source = "https://github.com/someone/sdl-drh"
tag = "v0.4.2"
commit = "9f2c0a3d..."
hash = "sha256:5b7e..."
```

It lists every package of the build, including the packages your packages
depend on. A build uses the locked tag, even when a newer one fits the range,
and the locked commit, even when the tag has moved. The `hash` is the SHA-256
of the package's files: for every file except anything named `.git`, in the
order of their `/`-separated paths (compared as bytes), the path, a zero byte,
`f` (`l` for a symbolic link, whose contents are its target), the length of the
contents in decimal, a zero byte and the contents. Folders, permissions and
times do not count. A package in `deps/` whose files do not hash as recorded is
refused (E0816), so an edited or tampered package never builds silently.

## Keeping up

```sh
drachm update            # newer tags inside every range
drachm update sdl_drh    # one dependency
drachm install           # throw deps/ away, fetch it again from the lock
drachm install --locked  # CI: exactly the lock, or fail (see below)
drachm remove sdl_drh    # drop it from drachm.toml, the lock and deps/
```

`update` asks the repositories again. It moves a `^` range to the newest tag
that fits and takes the current commit of an exact tag that was moved, then
rewrites the lock and prints what changed. `install` is for a fresh checkout
and for repairing `deps/`: it fetches the commits the lock names and checks
their hashes.

## Locked installs

`drachm install --locked` is for CI and for checking a commit. It installs
exactly what `drachm.lock` lists and resolves nothing: it never looks for a
tag the lock does not name, and never writes the lock file. It fails when:

- the lock file is missing and `drachm.toml` has dependencies (E0818);
- `drachm.toml` needs a git dependency the lock has no entry for, because one
  was added, its tag was changed, or no locked tag fits its range (E0818);
- the lock lists a package the manifests no longer need, because one was
  removed or its tag changed (E0818);
- a fetched package does not hash as the lock records (E0816).

A range is checked only as far as the lock goes: the locked tag must fit it.
The lock does not record the range itself, so a range edited to a new one that
still contains the locked tag is not noticed.

The same flag works on `build`, `run`, `check` and `test`, with the same
meaning: the program is built from the lock as it is, and a mismatch is E0818.

To fix E0818, run `drachm install` without `--locked` on your machine, look at
the new `drachm.lock` and commit it. `drachm update` takes newer tags inside
the ranges.

## deps/

Packages live in the `deps/` folder of the project, one folder per package and
version: `deps/sdl-drh@0.4.2/`. Nothing is shared between projects or kept in a
user-wide cache. `deps/` is not committed: `drachm new` writes a `.gitignore`
with `deps/` in it, and `drachm install` makes the folder again from the lock.

## Several versions side by side

Two packages may need different versions of a third. Both are fetched, into
`deps/json-drh@1.4.0/` and `deps/json-drh@2.1.0/`, and each package's imports
resolve to the version it asked for. A range reuses a version the build already
holds when it fits, so you only get two copies when the ranges cannot agree.
Two different repositories that would share one folder name and version are
E0804.

## Packages that wrap C

A package can carry C source and headers, and say how to build them in its own
`drachm.toml`:

```toml
[project]
name = "triple"
kind = "library"
public_modules = ["triple"]

[native]
include = ["c/include"]     # header folders
sources = ["c/triple.c"]    # compiled with the system C compiler
link = ["m"]                # libraries to link (the same as `libraries`)
unsafe = true               # the wrappers may call C
```

Drachm compiles `sources` with the system C compiler and links them into
programs that use the package, on both builds. The header folders in `include`
are searched by `in * from c."triple.h" as t;` in the package and also in every
project that depends on it. There are no build scripts: a package that needs
more than a list of C files and libraries asks the user to install the library.
See [Calling C libraries](guide-c-imports.md).

## Not yet

Registry packages (`drachm add json`, the Agora) come later; a bare name is
E0901 for now. Signatures and mirrors are not
designed.
