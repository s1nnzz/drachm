---
title: Converting
description: The convert module: numbers in other bases, the bytes of a text, and JSON built from a struct's field names.
---

# Converting

The `convert` module changes a value from one representation into another.
Import it with `in convert;` and call a function as `convert::name(...)`. It
holds three kinds of functions: numbers as text in a base, the bytes of a text,
and JSON. Turning one number type into another is not here; that is
`value::to<T>()`, described in [Numbers](guide-numbers.md).

## Bases

<!-- example: convert-bases -->

`convert::hex(n)`, `convert::binary(n)` and `convert::octal(n)` take a whole
number of any width and give its digits as text, in lower case, with no prefix.
A negative number has a minus sign in front: `convert::hex(-255)` is `-ff`.

`convert::fromHex(text)`, `convert::fromBinary(text)` and `convert::fromOctal(text)`
read digits back into a `uint64`. The text holds digits only, in capitals or not,
with no prefix, no sign and no separators. Anything else fails with `ConvertError`,
the failure that `text::to<T>()` uses:

| Variant | When |
|---|---|
| `Empty` | the text is empty |
| `NotANumber` | a character is not a digit of the base, including `-`, `0x` and spaces |
| `OutOfRange` | the value is above the largest `uint64` |

The functions only read the text, so a variable passed to them can be used
afterwards. For a narrower type, convert the result: `mask::to<uint8>()`.

## Bytes

`convert::utf8(text)` gives the bytes of a text as a `uint8[?]`. A character
outside ASCII takes two to four bytes, so the list can be longer than
`text::len()`.

## JSON

<!-- example: convert-json -->

`convert::toJson(value)` gives the JSON text of a struct, and
`convert::fromJson<T>(text)` reads a struct or a list of them back. The compiler
builds both from the field names and types of `T`, so there is nothing to write
for a new struct. The value passed to `toJson` is only read, and the text passed
to `fromJson` is borrowed, so both can be used afterwards.

The output is compact, with the fields in the order they are declared and no
spaces: `{"name":"Ada","level":7}`. Every field is written, whether it is
`public` or not.

| Drachm | JSON |
|---|---|
| whole numbers | a number, written in full (`uint128` too) |
| `float32`, `float64` | a number; NaN and infinity are written as `null` |
| `bool` | `true` or `false` |
| `string` | a string, with `"`, `\` and control characters escaped |
| `T?` | `null` for `none`, otherwise the value |
| `T[?]` | an array |
| a struct | an object with one entry per field |

Reading is strict. The text has to be one JSON value, and it has to fit the type:

- a whole number is written without a fraction or exponent, and has to fit the
  field's type: `256` for a `uint8` is an error, not a wrap;
- an integer is accepted for a float field;
- an object has exactly the fields of the struct. A missing field is an error,
  except for a field that is an optional, which becomes `none` (and so does a
  `null`); a field the struct does not have is an error;
- the escapes of JSON strings are understood, including `é` and surrogate
  pairs for characters outside the first plane.

When the text does not fit, `fromJson<T>` fails with `convert::JsonError`. Each
variant is one cause, and the message names the place in the text, starting from
`$`:

| Variant | Message |
|---|---|
| `Syntax(detail)` | `invalid JSON: expected ... at line 1, column 2` |
| `WrongType(path, expected)` | `$.items[2].name should be text` |
| `MissingField(path)` | `$.home is missing` |
| `UnknownField(path)` | `$.extra is not a field of the type` |
| `OutOfRange(path)` | `$.level does not fit its type` |

The call needs `maybe`, `fails` or a handler like any call that can fail, see
[Errors](reference-errors.md). Name the failure as `convert::JsonError`, or import
it with `in JsonError from convert;`.

## Not built yet

JSON is built for structs, lists, optionals and the types in the table above.
These are stopped with E0901 when the compiler checks the call, for now: maps,
enums, fixed-size arrays, C unions and a struct that contains itself. Reading
a whole number or a string at the top level (`fromJson<uint32>(text)`) also
waits. The same holds for reading a list of bytes back into text, and for a
choice of integer type in `fromHex`.
