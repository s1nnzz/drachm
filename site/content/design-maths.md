---
title: Maths and exact numbers
description: The math module, vectors, matrices, quaternions and exact number types.
---

# Maths and exact numbers

The `math` module and the number constants build today. Vectors, matrices,
quaternions and exact numbers are designed and still wait.

## The math module

`sqrt pow abs min max clamp floor ceil round sin cos tan log` and a few more
(`exp log2 log10 atan2 hypot`) are built, with units carried through. The
constants belong to the number type: `float64::pi`, `float32::pi`, `float64::e`.
See [Maths](reference-math.md) for every function, the unit rules and the
edge cases.

```drachm
in math;

float64 r = math::sqrt(2.0);
float64 c = 2.0 * float64::pi * r;
```

## Vectors and matrices

<!-- status: tentative -->

The standard library has `Vector2`, `Vector3` and `Vector4`, `Matrix3` and
`Matrix4`, and quaternions. Operators are ordinary methods, so `a + b` works
on vectors. A dot product is a named method, because `*` is not a good name for
it.

```drachm
Vector3 a = 1, 2, 3;
Vector3 b = 4, 5, 6;
Vector3 sum = a + b;
float64 d = a::dot(b);
```

## Exact numbers

<!-- status: tentative -->

Big integers, fractions and fixed-point decimals for money are planned. A
fraction such as 1/3 stays exact.

```drachm
Fraction third = 1 / 3;
Decimal price = 19.99;
```
