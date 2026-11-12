---
title: Constructors and defaults
description: Constructors that can fail and keys for maps, the parts of constructors and defaults that are still designed.
---

# Constructors and defaults

Constructors, `new ... of`, the declaration shorthand, `fill` blocks and `def`
defaults are built. The [structs reference](reference-structs.md#constructors)
covers them with examples. This page keeps the parts that are still designed.

## A constructor that can fail

<!-- status: not-implemented -->

A constructor may declare `fails`, and then the declaration handles the failure
like any other failing call. See [Errors](design-errors.md).

```drachm
struct Port {
    uint32 n;
    Port(uint32 n) fails BadPort {
        if (n > 65535) { fail BadPort; }
        self.n = n;
    }
}

Port p = 8080 maybe Port::default();
```

## Choosing another def

<!-- status: tentative -->

Picking a `def` by name, as in `fill p with Veteran { }`, is built and described
in the [structs reference](reference-structs.md#filling-and-defaults). It stays
tentative here until the owner confirms this page's wording.

## Keys for maps

<!-- status: tentative -->

A struct is a map key automatically, compared field by field. A `key()` method
says what a key is compared and hashed by instead. What happens when a type
has both a `key()` and its own `==` is still open.

```drachm
struct Word {
    string text;
    string key() { return self.text::lower(); }
}
```
