---
title: Constructors and defaults
description: Keys for maps, the part of constructors and defaults that is still designed.
---

# Constructors and defaults

Constructors, `new ... of`, the declaration shorthand, `fill` blocks and `def`
defaults are built. The [structs reference](reference-structs.md#constructors)
covers them with examples. Constructors that can fail are in the
[structs reference](reference-structs.md#constructors-that-fail). This page keeps
the part that is still designed.

## Keys for maps

<!-- status: not-implemented -->

A struct is a map key automatically, compared field by field. A `key()` method
says what a key is compared and hashed by instead. A type that has both a
`key()` and its own `==` uses `key()` for maps, and `==` stays what `==` is.
Maps arrive with step 6, so this part is not built.

```drachm
struct Word {
    string text;
    string key() { return self.text::lower(); }
}
```
