---
title: "Lesson 12: Constructors and defaults"
description: Building structs with constructors, new ... of, fill blocks and def defaults.
status: not-implemented
---

# Lesson 12: Constructors and defaults

The compiler can't run this lesson yet; the code shows the planned design.

**Goal:** control how a struct is built, and fill in a struct one field at a
time, with defaults for the fields you skip.

## A constructor

[Lesson 6](lesson-6-structs.md) built a struct by listing its fields. A
constructor does the same job with code of your own. It is a method with the
struct's own name and no return type, and inside it `self` is the value being
built. A struct may have several constructors, and the argument types choose
which one runs.

<!-- focus: 5-6 -->
`Circle` has two constructors. One takes a `float64` and one takes a `uint32`.

<!-- focus: 9-10 -->
In a declaration the values after `=` are the constructor arguments. `5` picks
`Circle(uint32)`, and `2.5` picks `Circle(float64)`.

```drachm
in io;

struct Circle {
    float64 r;
    Circle(float64 radius) { self.r = radius; }
    Circle(uint32 radius)  { self.r = radius; }
}

Circle a = 5;
Circle b = 2.5;
```

A struct with no constructor gets an automatic one that takes its fields in
order, which is what lesson 6 used.

## new ... of

Outside a declaration the type is not known before the value, so you say what
you are building: `new Circle of (2.5)`. The parentheses are required. A
`static` method that returns a new value of its own type is the usual home for
it.

```drachm
struct Circle {
    float64 r;
    static Circle unit() { return new Circle of (1.0); }
}
```

### Your turn: two constructors

Give `Temperature` a constructor from a `float64` and one from a `uint32`, so
both declarations in `main` build a value. Each stores its number in `degrees`.

```drachm title="starter"
in io;

struct Temperature {
public:
    float64 degrees;
    // your code here
}

none main() {
    Temperature a = 20;
    Temperature b = 21.5;
    io::outln(a.degrees::text());
    io::outln(b.degrees::text());
}
```

```text title="intended output"
20.0
21.5
```

```drachm solution
in io;

struct Temperature {
public:
    float64 degrees;
    Temperature(float64 d) { self.degrees = d; }
    Temperature(uint32 d)  { self.degrees = d; }
}

none main() {
    Temperature a = 20;
    Temperature b = 21.5;
    io::outln(a.degrees::text());
    io::outln(b.degrees::text());
}
```

## fill: one field at a time

A struct that is declared without a value has no fields yet. A `fill` block
gives it all of them, and the compiler checks at the closing brace that none
was missed. Until the struct is complete, `fill` is the only way to touch it.

<!-- focus: 2-5 -->
Every field must have a value by the `}`, or the compiler reports the one that
is missing.

<!-- focus: 7-8 -->
`fill partial` sets some fields and leaves the struct incomplete, to be filled
further on.

```drachm
Player p;
fill p {
    p.name = "Illy";
    p.hp = 5;
}

Player q;
fill partial q { q.name = "Sam"; }
```

## def: named defaults

Often most fields start the same. A `def` is a template of field values that
lives apart from the struct. The struct names its default one in its head, and
`fill p with defaults { }` starts from it, so the block only sets the rest.

<!-- focus: 1 -->
The `def` sets two fields.

<!-- focus: 2 -->
`defaults PlayerStart` makes it the struct's default.

<!-- focus: 5 -->
`name` is the only field the block has to set.

```drachm
def PlayerStart of Player { hp = 100; title = "none"; }
struct Player defaults PlayerStart { string name; uint32 hp; string title; }

Player p;
fill p with defaults { p.name = "Illy"; }
```

When the `def` has the struct's own name there is a shorthand: write
`def Player { ... }` and then `struct default Player { ... }`.

### Your turn: fill with defaults

Declare the `def` so that a new `Monster` has 10 `hp` and the title `"grunt"`,
and let `main` set only the name.

```drachm title="starter"
in io;

// your def here

struct Monster defaults MonsterStart {
public:
    string name;
    uint32 hp;
    string title;
}

none main() {
    Monster m;
    fill m with defaults { m.name = "Bob"; }
    io::outln(m.name + " " + m.hp::text() + " " + m.title);
}
```

```text title="intended output"
Bob 10 grunt
```

```drachm solution
in io;

def MonsterStart of Monster { hp = 10; title = "grunt"; }

struct Monster defaults MonsterStart {
public:
    string name;
    uint32 hp;
    string title;
}

none main() {
    Monster m;
    fill m with defaults { m.name = "Bob"; }
    io::outln(m.name + " " + m.hp::text() + " " + m.title);
}
```

## Choosing another def

<!-- status: tentative -->

A struct can have several `def`s. Picking one other than the default, as in
`fill p with Veteran { }`, is the design's most likely reading but is not
confirmed.

```drachm
fill p with Veteran { p.name = "Sam"; }
```

The full page for this part of the design is
[Constructors and defaults](design-constructors.md). The next lesson covers
values that are one of several shapes:
[Lesson 13: Enums and optionals](lesson-13-enums.md).
