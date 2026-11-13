---
title: Unsafe, pointers and assembly
description: unsafe blocks, raw pointers, address of, pointers to fixed values, freeze and thaw, assembly functions.
---

# Unsafe, pointers and assembly

Raw pointers are the one place where Drachm trusts you instead of checking.
Pointer programs build and run, and the operations that need `unsafe` are
checked; the accepted parts are on [Unsafe and pointers](reference-unsafe.md).
This page is the design, and parts still to come are marked below. There is no
garbage collector.

## Turning unsafe on

Two things are needed: the file flag `//--!$unsafe: true` at the top of the
file, and an `unsafe { }` block around the code that touches a pointer.

```drachm
//--!$unsafe: true
```

## Making and using a pointer

`*T` is a pointer. `address of n` makes one, and `*p = 7;` writes through it.
Everything that touches a pointer value (making, copying, comparing,
dereferencing, passing, storing) must be inside `unsafe { }`.

<!-- focus: 4,5,6 -->

```drachm
none main() {
    mut uint32 n = 1;
    unsafe {
        *uint32 p = address of n;
        *p = 7;
    }
    io::outln(n::text());     // 7
}
```

There is a single pointer kind that can be written through (no `*mut T`). A
pointer moves like a string does, and `copy p` duplicates it. A struct that
holds a raw pointer never copies automatically.

## Null

A pointer can be `none`, tested with `p == none`. Two pointers of one pointee type
compare with `==` and `!=` (addresses), inside `unsafe` too.

```drachm
unsafe { if (p == none) { return; } }
```

## Pointers to fixed values

`address of` an immutable binding gives a second kind, `*fixed T`, which cannot
be written through. `address of` a `mut` or `open` binding gives `*T`. Moving
between the two is explicit, both ways, inside `unsafe`.

```drachm
uint32 k = 5;
unsafe {
    *fixed uint32 f = address of k;
    *uint32 w = f::thaw();
    *fixed uint32 g = w::freeze();
}
```

## Unsafe functions

A function marked `unsafe` before its return type is called only inside an
`unsafe { }` block. Its own body counts as one unsafe block.

```drachm
unsafe uint32 peek(*uint32 p) { return *p; }

unsafe { uint32 v = peek(p); }
```

Writing through a pointer field does not change `self`: the pointer is
copied, not the value it points at, so such a method stays a read-only one.

## Assembly functions

<!-- status: tentative -->

A whole function may be written in assembly, never mixed into other code.
The instructions go one per line, and the function is called like any unsafe
function. The register and ABI mapping, the syntax flavour (Intel or AT&T) and
other CPUs are not designed yet, and this is scheduled after C header import, which is built.

```drachm
unsafe uint64 cycles() asm { rdtsc; shl rdx, 32; or rax, rdx }
```
