---
title: Structs and methods
description: Structs, fields, methods, static methods, operators and copies.
---

# Structs and methods

<!-- example: structs -->

- A `struct` groups fields. `public:` starts the part that code outside the
  struct may use.
- `Point a = 1, 2;` builds a struct by listing its fields in order.
  `Point b = y: 20, x: 10;` names them, and can list them in any order.
- Inside a method, `self` is the value the method was called on.
- `static` methods belong to the struct. Call them as `Point::origin()`.
- `mut` on a field lets it change even when the struct binding is not `mut`.
  `Counter k = 7, 0;` cannot change `id`, but can change `hits`.
- Operator methods such as `+` are written like methods. Structs also compare
  with `==` field by field, unless you write your own `==`.
- Each copy of a struct is a separate value, so changing `c` leaves `a` alone.

Next: [Control flow](tour-control.md).
