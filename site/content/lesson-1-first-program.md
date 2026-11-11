---
title: "Lesson 1: Your first program"
description: Write, run and check a Drachm program, and print text with io::out and io::outln.
---

# Lesson 1: Your first program

**Goal:** write a program, run it, and make it print what you want.

## A program is a main function

Every Drachm program starts in a function called `main`. Its return type comes
first, as everywhere in Drachm, and `none` means that `main` hands nothing back
to whoever started the program.

<!-- focus: 1 -->
Printing is not built into the language. It lives in the `io` module, and
nothing is imported for you, so the first line of the file asks for it with
`in io;`.

<!-- focus: 3-5 -->
The function body sits between `{` and `}`. Inside it, `io::outln` means
"the function `outln` from the module `io`". It prints its text and ends the
line. The statement ends with a `;`, as every statement does.

Save this as `hello.drh`, then run it with `drachm run hello.drh`. If you have
not built the compiler yet, [Install and run](install.md) shows how.

```drachm title="hello.drh"
in io;

none main() {
    io::outln("Hello, world!");
}
```

```text title="output"
Hello, world!
```

## Printing without a newline

`io::outln` is `io::out` plus a line break. Use `io::out` when you want to
build one line from several pieces, and `io::outln` for the last piece.

<!-- focus: 4-5 -->
The first two calls print on the same line because neither of them adds a
newline.

<!-- focus: 6-7 -->
The third call finishes the line, and the fourth prints a line of its own.

<!-- example: hello-out -->

## When something is missing

Drachm checks the whole program before it runs any of it, and its messages say
what is wrong, where, and usually how to fix it. To see one, take `in io;` out
of `hello.drh` and ask the compiler to look at the file without building it.

The compiler points at the first use of `io`, and the `help` line gives the
one line you forgot. Put the import back and the error goes away. You will
use `drachm check` a lot, because it is the quickest way to find mistakes.

```sh
drachm check hello.drh
```

```text title="output"
error[E0206]: module `io` is used but not imported
  --> hello.drh:2:5
   |
 2 |     io::outln("Hello, world!");
   |     ^^ not imported
   |
 help: add `in io;` at the top of the file

hello.drh: 1 error
```

### Your turn: build a line from pieces

Write a program that prints `Hello, Ada!` on a single line, using two
`io::out` calls and one `io::outln`. The starter below is the empty
`main`; fill in the hole, run it, and compare with the expected output.

```drachm title="starter"
in io;

none main() {
    // your code here
}
```

```text title="expected output"
Hello, Ada!
```

<!-- solution-of: solution-1-1 -->
```drachm solution
in io;

none main() {
    io::out("Hello, ");
    io::out("Ada");
    io::outln("!");
}
```

Next: [Lesson 2: Numbers and variables](lesson-2-numbers.md).
