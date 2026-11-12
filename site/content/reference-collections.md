---
title: Arrays and lists
description: Array and list types, literals, indexing with bounds checks, changing a list, its methods, comparing, for loops, and splitting strings into lists. Maps are marked as not built yet.
---

# Arrays and lists

An array holds a fixed number of values side by side; a list holds any number
and grows. Both hold values of one type. The rules on this page are what the
compiler accepts today; the parts still to come are marked.

## Arrays and lists

`T[N]` is an array of `N` values (`N` is a whole number of 1 or more), and
`T[?]` is a list. They can be stacked (`uint32[?][?]` is a list of lists) and
used anywhere a type goes: locals, parameters, results, struct fields, enum
payloads and optionals (`uint32[?]?`). A literal is the values in brackets,
with a trailing comma allowed; its type comes from the place it is used. With
no type to take from, the first value decides and the result is a list.

<!-- example: list-basic -->

A list declared without a value starts empty and can be used at once
(`mut string[?] names;`). `[]` needs a type to say what it holds
(`string[?] xs = [];`), or it is E0624. An array literal must give exactly as
many values as the array has (E0625), and the values of one literal must be
of one type (E0623 when the list has no type of its own to convert to).

A list owns its memory, so it moves like a `string`: passing it, returning it
or assigning it hands it over, and it is freed when its owner ends. `copy xs`
makes an independent copy, items included. An array owns what its items own.
It copies by itself, like a small struct, when its items copy and it is at
most 32 bytes, and moves otherwise.

A list or array cannot hold borrows yet (E0901), `none` is not an item type
(E0104), an array cannot contain itself (E0210; a list can, since it holds
its items on the heap), and an array above 1 MiB is E0901.

## Indexing

`xs[i]` is the item at position `i`, counting from 0. The index is any
integer type of at most 64 bits, without a unit. A negative index counts from
the end, like text: `xs[-1]` is the last item. An index outside the list
stops the program with exit code 101 and says which index and what length. A
constant index into an array is checked while compiling (E0620).

<!-- example: list-index -->

```text
$ drachm run past-the-end.drh
before
drachm: panic: index 4 is outside the list (length 4)
```

```text
error[E0620]: index 5 is outside `uint8[4]`
  --> main.drh:4:20
   |
 4 |     uint8 a = rgba[5];
   |               ---- an array of 4 values
   |                    ^ this index is 5
   |
   help: valid indexes are 0 to 3, or -1 to -4 from the end
```

An item is a place: `xs[i] = v;`, `xs[i] += 1;`, `xs[i]++` and `xs[i].field = v;`
write to it, and `&xs[i]` borrows it. Reading an item whose type owns memory
(a `string`, a struct holding one) would leave a hole, so it is E0622: look
at it with `&xs[i]`, copy it with `copy xs[i]`, or call a method on it. Taking
an item out (`take xs[i]`) is E0901 for now.

<!-- example: list-owning -->

An index is evaluated once, so `xs[f()] += 1` calls `f` once. Two borrows of
items of one list count as borrows of the same place: `&mut xs[0]` and
`&xs[1]` conflict.

## Changing a list

A list changes through `xs::push(item)`, which moves the item in, and through
its items; both need a binding that allows changes (`mut`, `open`, or a
`&mut` borrow), or the compiler says so (E0205, E0311). `xs::len()` is the
number of items (a `uint64`) and works on arrays too. A method that pushes
onto a list in `self` makes the method one that changes `self`.

A borrow of an item keeps the list from changing: `push` while `&xs[0]` is
still used is E0302.

## Reading and removing items

`xs::pop()` takes the last item out, and `xs::remove(i)` takes the item at
`i` out (the items after it move down). Both give the item, `pop` as a `T?`
that is `none` for an empty list; `remove` stops the program when `i` is
outside the list. `xs::insert(i, item)` puts an item in at `i` (`i` may be
the length, to add at the end). `xs::clear()` drops every item and keeps the
list usable.

`xs::get(i)`, `xs::first()` and `xs::last()` give the item as a `T?`, which is
`none` when there is no such item, so they never stop the program. They copy
the item, so they are for items that copy (numbers, `bool`, Copy structs); a
list of `string`s is E0901 for now. `xs::join(sep)` puts the items of a
`string[?]` together with `sep` between them (`""` for an empty list).

<!-- example: list-methods -->

`xs::contains(item)` is true when some item is equal to `item`. Lists and
arrays compare with `==` and `!=`: they are equal when they have the same
length and each item is equal to the item at its place. Items compare with
their own `==` (numbers, `bool`, `string`, structs and lists of those; an
enum with values is E0527). `contains` needs items that copy: on a list of
`string`s it is E0901 for now.

<!-- example: list-compare -->

## Walking a list

`for (T x of xs)` goes over the items in order, with `&T x` for a view and
`&mut T x` for a changeable view; the [loops page](reference-control-flow.md#looping-over-a-list)
has the rules and the example.

## Splitting strings

`text::split(sep)` gives the pieces between the occurrences of `sep`, in order,
as a `string[?]`. Empty pieces are kept, so `",a,,b,"` gives five pieces and
`""` gives one empty piece. `text::lines()` gives the lines without their line
ends (`\n` or `\r\n`); a last line without a line end counts, and an empty text
has no lines. `text::chars()` gives the characters, one owned string each, counted
the way `len()` counts them. Every piece is a copy, so the list keeps working
after the text is gone. A separator that is empty stops the program.

<!-- example: list-split -->

`join` goes back: `parts::join(sep)` puts the items of a `string[?]` together.

## Not built yet

<!-- status: not-implemented -->

Maps (`Map<K, V>`, see [Collections](design-collections.md)) are designed and
not accepted yet.

Generic functions over lists already work:

```drachm
T add_first<T>(&T[?] xs) { return xs[0] + xs[1]; }
```
