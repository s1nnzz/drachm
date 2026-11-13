---
title: The showcase
description: One program that uses everything the compiler accepts today, beside its output.
---

# The showcase

This is the voyage of the survey ship Drachma: one program that uses everything
the compiler accepts today. Numbers of every width, units of measure, structs
and constructors, enums and `match`, optionals, failures with handlers and
causes, interfaces and generics, borrows and `lives`, lists, maps, lambdas.
Each part has a header comment saying what it shows off.

It is a real, tested example: the source is `examples/showcase.drh` and the
output is `tests/examples/showcase.out`. `cargo test` runs it on both backends and
checks the output, and this page reads both files when the site is built, so it
cannot go out of date. Run it yourself with `drachm run examples/showcase.drh`.

<!-- showcase -->
