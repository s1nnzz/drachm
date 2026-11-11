---
title: Structs and methods
description: Struct sections, fields, building values, methods and operator methods.
---

# Structs and methods

A `struct` groups named fields. Methods are written inside it.

```drachm
struct Point {
public:
    sint32 x;
    sint32 y;

    sint32 sum() {
        return self.x + self.y;
    }

    static Point origin() {
        Point p = 0, 0;
        return p;
    }
}
```

The full program is in the [tour](tour-structs.md).

## Sections

- Before any `public:` or `private:` label, fields are private and methods are
  public. A label starts a new section for what follows it.
- Code outside a struct may use only its public members. Using a private one
  is error E0212.
- Methods inside the struct may use private members.

## Fields

- `mut` before a field type lets that field change, even when the binding
  holding the struct is not `mut`:

  ```drachm
  struct Counter {
  public:
      uint32 id;
      mut uint32 hits;
  }
  ```

- A field may be a `string`, or another struct.
- Changing a non-`mut` field needs a `mut` binding. For example, `p.id = 3;`
  needs `mut Counter p`.
- Each copy of a struct is a separate value. Copies of a struct that holds a
  string copy the text too.

## Building a value

A struct is built in a declaration, by listing values:

```drachm
Point a = 1, 2;                 // in field order
Point b = y: 20, x: 10;         // by name, in any order
```

Every field must be given exactly once. Mixing the two forms is an error
(E0114). Build a struct inside another expression by calling a function that
returns it.

## Methods

Call a method on a value with `::`, and each call after the first with `->`:

```drachm
io::outln(a::sum()->text());
Point c = a::moved(5, 5)->moved(1, 0);
```

Inside a method, `self` is the value the method was called on. A method that
changes a field of `self` needs a `mut` binding when it is called.

`static` methods belong to the struct. They have no `self`, and you call them
on the struct:

```drachm
Point origin = Point::origin();
```

A method that never uses `self` should be `static`; otherwise the compiler
gives warning W0208.

## Operator methods

Operators are methods named by the operator. They take one value, the right
side:

```drachm
struct Vector2 {
public:
    float64 x;
    float64 y;

    Vector2 +(Vector2 o) {
        Vector2 r = self.x + o.x, self.y + o.y;
        return r;
    }
}
```

The operators you can define are `+ - * / % **` and `== != < <= > >=`.
Comparisons return `bool`. `==` and `!=` take the struct's own type.
Compound assignment such as `c += b` uses the `+` method.

`==` and `!=` work on any struct without a method. They compare field by
field, and strings compare by their text. Define `bool ==(T o)` to replace
that rule.
