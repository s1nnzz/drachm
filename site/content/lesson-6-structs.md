---
title: "Lesson 6: Structs and methods"
description: Grouping values in structs, building them, methods, static methods, mut fields, operators and public and private.
---

# Lesson 6: Structs and methods

**Goal:** define your own types that group values and carry their own functions.

## Grouping values in a struct

A `struct` is a type made of named fields. Everything declared under a
`public:` label can be used by any code. A function written inside the struct
is a method, and inside it `self` is the value the method was called on.

<!-- focus: 3-11 -->
`Point` has two fields and one method. `show` reads the fields through `self`.

<!-- focus: 13-17 -->
A field can itself be a struct, or a `string`. `Named` holds a name and a
`Point`.

<!-- focus: 20 -->
A value is built in a declaration by listing the fields in order. `Point a = 1, 2;`
sets `x` to 1 and `y` to 2.

<!-- focus: 21 -->
Or name them, in any order: `y: 20, x: 10`. Every field must be given exactly
once, and mixing the two forms is an error.

<!-- focus: 23-26 -->
Methods are called with `::`, and fields are read with a dot.

<!-- example: struct-build -->

```text title="drachm check, with Point p = 1; and Point q = 1, y: 2;"
error[E0114]: `Point` has 2 fields, but 1 value is given
   --> main.drh:10:15
    |
  3 | struct Point {
    |        ----- `Point` is defined here
   ...
 10 |     Point p = 1;
    |               ^
    |
 help: give a value for each field, in order: `sint32 x` and `sint32 y`

error[E0114]: values for `Point` are partly named and partly not
   --> main.drh:11:21
    |
 11 |     Point q = 1, y: 2;
    |                     ^ named
    |
 help: name every value (`x: 1, y: 2`) or none (`1, 2`)
```

## Public and private

Fields and methods before any label are private fields and public methods.
A `public:` or `private:` label changes the rule for everything after it.
Private fields can be used only inside the struct's own methods. This is how a
struct keeps its secrets: `pin` below can be checked with `check` but never read.

<!-- focus: 4-9 -->
`secret` comes before any label, so it is private. `owner` and `balance` are
public, and the `private:` label makes `pin` private again.

<!-- focus: 11-13 -->
`check` is a method, so it may read `pin`.

<!-- example: struct-sections -->

```drachm
struct Player {
    string name;
public:
    uint32 hp;
}
// ... Player p = "Illy", 10;  io::outln(p.name);
```

```text title="drachm check"
error[E0212]: field `name` of `Player` is private
   --> main.drh:11:17
    |
  4 |     string name;
    |            ---- declared in a private section
   ...
 11 |     io::outln(p.name);
    |                 ^^^^ used outside the struct's methods
    |
 help: only `Player`'s own methods can use it; put it under `public:` to use it here
```

## Changing a field

A field cannot change unless something says it can. Either the binding that
holds the struct is `mut`, or the field is declared `mut`. A `mut` field is
for things that are meant to change, such as a balance. The compiler's message
names both options.

```drachm
struct Player {
public:
    string name;
    uint32 hp;
}
// ... Player p = "Illy", 10;  p.hp = 5;
```

```text title="drachm check"
error[E0205]: cannot change `p.hp`: `p` is not `mut`, and neither is the field
   --> main.drh:11:5
    |
 10 |     Player p = "Illy", 10;
    |            - declared here without `mut`
 11 |     p.hp = 5;
    |     ^^^^ changed here
    |
 help: declare it as `mut Player p`, or the field as `mut uint32 hp;` in `Player`
```

## Static methods

A `static` method has no `self`. It belongs to the struct itself and is called
on the type, as in `Temperature::boiling()`. It is the natural way to build a
value or to write a function that has nothing to do with one particular
value. A method that never uses `self` should be `static`, and the compiler
warns you if it is not.

<!-- example: struct-static -->

## Operators on your own types

An operator is a method named by the operator, and it takes the right-hand
value. `Money +(Money o)` lets you write `x + y`, `+=` uses it too, and a
`bool <(Money o)` method gives you `<`. You can define `+ - * / % **` and
the six comparisons. Without writing anything, `==` and `!=` already compare
two structs field by field.

<!-- example: struct-ops -->

### Your turn: a rectangle

Define a struct `Rectangle` with two `uint32` fields, `width` and `height`,
and a method `area` that returns their product. Build a 7 by 6 rectangle in
`main` and print its area.

```drachm title="starter"
in io;

// your code here

none main() {
    Rectangle r = 7, 6;
    io::outln(r::area()->text());
}
```

```text title="expected output"
42
```

<!-- solution-of: solution-6-1 -->
```drachm solution
in io;

struct Rectangle {
public:
    uint32 width;
    uint32 height;

    uint32 area() {
        return self.width * self.height;
    }
}

none main() {
    Rectangle r = 7, 6;
    io::outln(r::area()->text());
}
```

### Your turn: a counter

Write a struct `Counter` with a `mut uint32 count` field and a method `bump`
that adds one to it. The binding `c` in `main` is not `mut`, so the field has to
be. Call `bump` three times and print the count.

```drachm title="starter"
in io;

// your code here

none main() {
    Counter c = 0;
    c::bump();
    c::bump();
    c::bump();
    io::outln(c.count::text());
}
```

```text title="expected output"
3
```

<!-- solution-of: solution-6-2 -->
```drachm solution
in io;

struct Counter {
public:
    mut uint32 count;

    none bump() {
        self.count++;
    }
}

none main() {
    Counter c = 0;
    c::bump();
    c::bump();
    c::bump();
    io::outln(c.count::text());
}
```

Next: [Lesson 7: match and skip](lesson-7-match-and-skip.md).
