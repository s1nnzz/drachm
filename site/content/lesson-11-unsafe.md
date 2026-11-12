---
title: "Lesson 11: Unsafe and pointers"
description: Raw pointers, address of, unsafe blocks and functions, and pointers that can only be read.
---

# Lesson 11: Unsafe and pointers

**Goal:** make and use a raw pointer inside `unsafe`, and know what the compiler
no longer checks there.

## Turning unsafe on

Borrows ([lesson 9](lesson-9-borrowing.md)) are checked: they cannot outlive
what they point at. A raw pointer is not. It is an address, and you promise it
points at something that is still alive. Because of that, a file that uses
pointers starts with a flag, and every operation on a pointer sits inside an
`unsafe { }` block.

<!-- focus: 1 -->
The flag at the top turns `unsafe` and pointers on for this file.

<!-- focus: 7 -->
`*uint32` is a pointer to a `uint32`. `address of score` makes one, and `score`
must be `mut` for the pointer to write.

<!-- focus: 8-9 -->
`*p = 7;` writes through the pointer, and `*p` on its own reads the value.

<!-- focus: 12 -->
`score` changed through `p`, so it prints 7.

<!-- example: lesson-11-pointer -->

A block is a scope, so what you declare inside is gone after the `}`. Declare
first and assign inside when you need a result outside.

## Unsafe functions

A function whose body works on pointers is declared `unsafe` before the return
type, and its body counts as one unsafe block. Calling one needs a block at the
call.

<!-- focus: 4-6 -->
`add_one` writes through the pointer it is given.

<!-- focus: 11-12 -->
Each call makes a fresh pointer with `address of count`.

<!-- example: lesson-11-unsafe-fn -->

## A pointer you can only read

`address of` an immutable binding gives a `*fixed uint32`, which can be read
through but not written. Writing through it is error E0333. A pointer can also
be `none`, the null pointer, and `p == none` tells you. Both need `unsafe`.

<!-- focus: 7-10 -->
Reading through `f` is fine. `f == none` is `false` because it points at `k`.

<!-- example: lesson-11-fixed -->

The two kinds convert only on request, inside `unsafe`: `p::freeze()` makes a
`*fixed` from a `*`, and `p::thaw()` goes the other way. Every rule is on the
[Unsafe and pointers](reference-unsafe.md) page.

### Your turn: write through a pointer

Make `p` point at `health` and set it to 9 through the pointer.

```drachm title="starter"
//--!$unsafe: true
in io;

none main() {
    mut uint32 health = 3;
    unsafe {
        // your code here
    }
    io::outln(health::text());
}
```

```text title="expected output"
9
```

<!-- solution-of: solution-11-1 -->
```drachm solution
//--!$unsafe: true
in io;

none main() {
    mut uint32 health = 3;
    unsafe {
        *uint32 p = address of health;
        *p = 9;
    }
    io::outln(health::text());
}
```

### Your turn: an unsafe function

Write `unsafe none twice(*uint32 p)` that doubles the number `p` points at, and
call it on `n`.

```drachm title="starter"
//--!$unsafe: true
in io;

// your code here

none main() {
    mut uint32 n = 21;
    unsafe {
        twice(address of n);
    }
    io::outln(n::text());
}
```

```text title="expected output"
42
```

<!-- solution-of: solution-11-2 -->
```drachm solution
//--!$unsafe: true
in io;

unsafe none twice(*uint32 p) {
    *p = *p * 2;
}

none main() {
    mut uint32 n = 21;
    unsafe {
        twice(address of n);
    }
    io::outln(n::text());
}
```

### Your turn: read through a fixed pointer

`k` is immutable, so its address is a `*fixed uint32`. Declare `f` that way,
read it into `v` and print it.

```drachm title="starter"
//--!$unsafe: true
in io;

none main() {
    uint32 k = 5;
    unsafe {
        // your code here: f, then v
        io::outln(v::text());
    }
}
```

```text title="expected output"
5
```

<!-- solution-of: solution-11-3 -->
```drachm solution
//--!$unsafe: true
in io;

none main() {
    uint32 k = 5;
    unsafe {
        *fixed uint32 f = address of k;
        uint32 v = *f;
        io::outln(v::text());
    }
}
```

That is the end of the lessons for the language the compiler builds today. The
lessons after this one show the planned design, and each says so at its start.
