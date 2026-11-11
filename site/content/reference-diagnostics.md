---
title: Errors and warnings
description: How errors and warnings look, their code groups, and run-time panics.
---

# Errors and warnings

The compiler reports every problem it finds in one run, not just the first.
Each message has a code, a short title, and the lines it is about.

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

- `error` means the program does not compile. `drachm check` and `build` exit
  with code 1.
- `warning` means the program still compiles. The warning is printed, and the
  build succeeds.
- Each code has one letter and four digits. `E` is an error and `W` a warning.
  The first two digits group the problem:

| Codes | Kind of problem |
|---|---|
| `00xx` | syntax: the text is not Drachm |
| `01xx` | types, conversions, operators, `match`, `skip` |
| `02xx` | names, declarations, bindings, structs |
| `03xx` | ownership and borrowing (not built yet) |
| `09xx` | valid Drachm that this compiler cannot do yet |

## Explain a code

```sh
drachm explain E0101
```

prints the title and a longer description with examples. The
[error index](errors.md) has every code in one page.

## Panics at run time

Some mistakes are only found while the program runs. These stop it with exit
code 101 and a message on stderr:

```text
drachm: panic: division by zero
drachm: panic: string index out of range
```

A program can also exit with a code of its own. See
[Command line](reference-cli.md) for `main`.

## Colour

Output is coloured when stderr is a terminal. Set `NO_COLOR` to turn colour
off.
