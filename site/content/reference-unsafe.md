---
title: Unsafe and pointers
description: The unsafe file flag, raw pointers, address of, pointers to fixed values, freeze and thaw, unsafe functions, and the errors they give.
---

# Unsafe and pointers

Raw pointers are the one place where Drachm trusts you instead of checking. This
page states what the compiler accepts today. The [design](design-unsafe.md)
page has the rest, including assembly functions, which are not built yet. The
[ownership and borrowing guide](guide-ownership.md) explains why you rarely need
any of this.

## The file flag

A file that uses `unsafe` or pointers starts with this line:

```drachm
//--!$unsafe: true
```

Without it, `unsafe`, `*T` and `address of` give error E0317.

## Pointer types

`*T` is a pointer to a `T` that can be written through. `*fixed T` is a pointer
that can only be read. Both are 8 bytes. Pointers are not dropped or tracked: a
pointer must not outlive the value it points to.

`address of` makes a pointer. Applied to a `mut` or `open` binding it gives
`*T`; applied to an immutable binding it gives `*fixed T`.

<!-- example: unsafe-pointer -->

`n` changed through `p`, so the program prints `7`. Then `none` is tested: a
pointer can be `none`, which is the null pointer, and `p == none` tells them
apart. `p == q` compares the two addresses: `q` points at `n` too, so it
prints `true`. Comparing does not use either pointer up.

A pointer moves like a string does: assigning it or passing it on leaves the old
name with no value. `copy p` makes a second pointer to the same place. A pointer
variable that is assigned again must be declared `mut *uint32 p;`.

## Using a pointer needs `unsafe { }`

These operations must be inside an `unsafe { }` block:

- making a pointer with `address of`,
- reading through it (`*p`),
- writing through it (`*p = v;`),
- copying it with `copy`,
- comparing it with `==` or `!=`, with another pointer or with `none`,
- writing `none` as a pointer's value,
- calling an `unsafe` function.

The block is a scope. A variable declared inside it is not visible after it, so
declare first and assign inside:

```drachm
uint32 v = 0;
unsafe { v = *p; }
```

The body of an `unsafe` function counts as one unsafe block, so it needs no
inner block. Declaring such a function takes `unsafe` before the return type:

```drachm
unsafe none swap(*uint32 a, *uint32 b) {
    uint32 t = *a;
    *a = *b;
    *b = t;
}
```

Calling it needs a block:

<!-- example: unsafe-fn -->

## Pointers from C

A pointer that comes from a C header is a `foreign *T` (`foreign *uint8` for a
`char *`, `foreign *none` for `void *`). It is used like `*T` in `unsafe`
code, with two differences: it copies freely, since C owns what it points at,
and nothing turns a Drachm pointer into one by itself. The hand-over is
written with `as`: `(address of n) as foreign *c::int` for the same type, or
`buf as foreign *none` for `void *` (any pointer). Without the `as`, the
compiler says E0335 and shows the conversion to write. See
[Calling C libraries](guide-c-imports.md).

## Pointers to fixed values

`address of` an immutable binding gives a `*fixed T`. It can be read through but
not written through: writing is error E0333.

A `*T` and a `*fixed T` do not convert to each other by assignment. Comparing
them with `==` or `!=` does not need a conversion: it compares the addresses,
and the two pointers must have the same pointee type. Inside
`unsafe { }` the two conversions are explicit:

- `p::freeze()` gives a `*fixed T` from a `*T`,
- `p::thaw()` gives a `*T` from a `*fixed T`.

Assigning one kind where the other is wanted is error E0334.

<!-- example: unsafe-fixed -->

## Errors

Each operation from the list above, used outside `unsafe { }`, is error E0316.
The message names the operation and the help shows the block. For example,
reading through a pointer:

```text
error[E0316]: reading through a raw pointer needs an `unsafe { }` block
 --> main.drh:6:16
  |
6 |     uint32 v = *p;
  |                ^^ the pointer is read here
  |
help: do it inside a block: declare the result first, then `unsafe { v = *p; }`
```

Writing through a `*fixed` pointer gives E0333:

```text
error[E0333]: cannot write through `*p`: it is a `*fixed` pointer, which can only be read
 --> main.drh:6:9
  |
6 |         *p = 6;
  |         ^^ written through a `*fixed` pointer here
  |
help: only a pointer made from a `mut` or `open` binding (`*T`) can be written through
```

Calling an `unsafe` function outside a block gives E0316 too, with a second
label at the function's declaration. The full texts of E0316, E0333, E0334 and
E0317 are in the [error index](errors.md).

Moving or passing a pointer outside `unsafe`, and whether an `unsafe` function's
body needs a block, are not decided yet. The compiler reports E0901 for them
rather than guessing.
