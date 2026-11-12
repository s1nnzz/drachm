---
title: Errors and warnings
description: How errors and warnings look, their code groups, and run-time panics.
---

# Errors and warnings

The compiler reports every problem it finds in one run, not just the first.
Each message has a code, a short title, the lines it is about, and usually a
`help` line with the fix. A label with `^` marks the place that is wrong, and one
with `-` marks related code, such as where a name was declared.

An `error` means the program does not compile, and `drachm check` and `build`
exit with code 1. A `warning` means the program still compiles, so the warning is
printed and the build succeeds.

```text
error[E0205]: cannot change `x`: it is not `mut`
  --> q3.drh:5:5
   |
 4 |     uint32 x = 1;
   |            - declared here without `mut`
 5 |     x = 2;
   |     ^ assigned a second time here
   |
 help: declare it as `mut uint32 x` to allow changes
```

## Code groups

Each code has one letter and four digits, where `E` is an error and `W` a warning.
The first two digits say what kind of problem it is.

| Codes | Kind of problem |
|---|---|
| `00xx` | syntax: the text is not Drachm |
| `01xx` | types, conversions, operators, `match`, `skip` |
| `02xx` | names, declarations, bindings, structs |
| `03xx` | ownership: moves, copies and borrows |
| `05xx` | enums (and, as they arrive, constructors, optionals and failures) |
| `06xx` | generics, interfaces, collections and lambdas (as they arrive) |
| `09xx` | valid Drachm that this compiler cannot do yet |

## Explain a code

`drachm explain` prints the title and a longer description with examples for one
code. The [error index](errors.md) has every code on one page.

```sh
drachm explain E0101
```

## Panics at run time

Some mistakes are only found while the program runs. They stop it with exit
code 101 and a message on stderr. A program can also exit with a code of its own,
as [Command line](reference-cli.md) shows for `main`.

```text
drachm: panic: division by zero
drachm: panic: string index out of range
```

## Colour

Output is coloured when stderr is a terminal. Set `NO_COLOR` to turn colour off.
