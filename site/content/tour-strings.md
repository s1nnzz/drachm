---
title: Strings
description: String literals, interpolation, joining, methods and io::outf.
---

# Strings

<!-- example: strings -->

- `"..."` is a string. `\n`, `\t`, `\"` and `\\` are escapes, and `\u{1F600}`
  is any character by its code.
- `f"..."` puts values inside the text with `${...}`.
- `"""..."""` spans several lines.
- `+` and `+=` join strings. Both sides must be strings, so use `::text()` on
  numbers first.
- A method call with `::` comes first. Each call after it uses `->`.
- `io::outf` formats values. `%` prints a value, `%6` pads it to six
  characters, `%-6` pads on the right, `%06` pads with zeros, `%.1` sets one
  decimal place, and `%x` prints in hex.

Next: [Structs and methods](tour-structs.md).
