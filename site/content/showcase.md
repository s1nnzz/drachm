---
title: The showcase
description: One program that uses everything the compiler accepts today, beside its output.
---

# The showcase

This is the voyage of the survey ship Drachma: one program that uses everything
the compiler accepts today. Numbers of every width, units of measure carried through `math`, structs
and constructors, enums and `match`, optionals, failures with handlers and
causes, interfaces and generics, borrows and `lives`, lists, maps, lambdas, and a
C library (the harbours of part 6, `examples/showcase/c/`) called through a
safe wrapper.
Each part has a header comment saying what it shows off.

It is a real, tested example, and a project of several files: `examples/showcase/`
has a `drachm.toml` (with a `[native]` table for the C helper), `src/main.drh` and
a module for each chapter under `src/voyage/`, plus `src/rules.drh` for the
constants. `main.drh` imports them
in the different ways a file can (a module, a module under another name, every
name under a prefix, names bare as well, single items), and `systems.drh`
reaches `rules.drh` with a file path that climbs a folder. The expected output
is `examples/showcase/expect.out`. `cargo test` builds the project on both
backends and checks the output, and this page reads all the files when the site
is built, so it cannot go out of date. Run it yourself with
`drachm run examples/showcase`.

Select a source tab to switch files. Use Left or Right to move between tabs,
or Home and End to jump to the first or last file.

The block below can be resized. Drag the divider between the program and its
output (or press its arrow keys), drag the bar under the panes to make the block
taller or shorter, and press Expand for a full-screen view (Esc closes it). Each
pane scrolls on its own, and long lines scroll sideways.

<!-- showcase -->

The dispatch companion runs as a separate script. Run
`drachm run examples/showcase/dispatch/dispatch.drh` to send the survey report
to harbour through three relays. Its expected output is in
`examples/showcase/dispatch/dispatch.out`.
