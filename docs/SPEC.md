# Drachm Language Specification (design snapshot)

Snapshot of every design decision as of 2026-11-12. Things marked *(implementer's reading)* are
not the owner's explicit choice (see `docs/impl/owner-answers.md`). The living version is the
design doc: https://claude.ai/artifact/DxosrpAV2x2TzpfNbaFw8i. If the two disagree,
ask the language's owner which is current; update both when a decision changes.

Items marked **OPEN** are undecided. Do not invent answers for them: ask the owner
(see `CLAUDE.md` for how).

- Name: Drachm, pronounced /drɑxm/. Source extension: `.drh`.
- Goal: one language for every kind of project (games, CLI tools, servers,
  systems), that also works as a scripting language.
- Implementation: compiler in Rust. Plain builds and scripts go through Cranelift;
  `-O` builds generate C for the system C compiler. Linked with `cc`.
- Naming convention everywhere (stdlib and user code): **camelCase**.

---

## 1. Lexical

- Comments: `//` to end of line, `/* ... */` blocks (not nested).
- Statements end with `;` (required). Blocks use `{ }`.
- Conditions are always parenthesised: `if (x)`, `while (x)`, `match (x)`, `for (...)`.
- Keywords so far: `otherwise any failure because exit extends Self none mut if else while for of match case return skip to
  struct interface with static public private new in out from as fails fail maybe
  copy lives open break continue unsafe async await move test true false lend take destroy leaving address
  stays changes by asm fixed`. Always reserved: `lend`, `take`, `destroy`. Context-only (ordinary names elsewhere):
  `to` directly after `skip`; `by` directly after a range in `for (... of a..b by n)`;
  `unit` at the start of a top-level item, followed by a name and `;` or `=` (a unit
  declaration; elsewhere an ordinary name, so `static Circle unit()` is fine);
  `fixed` at the start of a top-level value (`fixed uint32 MAX = ...;`, and `*fixed T` pointers);
  `leaving` directly after `take <place>`; `address` directly before `of`; `stays` directly after a
  struct's name (and `lives` list); `changes` directly before `self` after an interface method's
  parameters; `asm` directly after an `unsafe` function's signature; `globbed` directly after `case`;
  `now` before a field read inside `fill` or a constructor; `fill`, `def`, `defaults`, `auto` in their
  declarations; `has value` / `is empty` after an optional; `ignore failure` after a call
  *(implementer's reading of which are context-only)*. `drop` is a reserved method name,
  not a keyword. **OPEN:** owner to confirm which of these are reserved everywhere.
- File flags: `//--!$key: value` lines, allowed only at the very top of a file
  (they override `drachm.toml` settings for that file).

## 2. Functions and bindings

```cpp
uint32 add(uint32 a, uint32 b) {
    return a + b;
}

none greet() { io::outln("hi"); }   // `none` = returns nothing (no `fn`, no `void`)

none main() { ... }
```

- Every function starts with its return type. There is no `fn` keyword.
- `none main()` exits with code 0; `sint32 main()` returns the exit code.
- Bindings are immutable by default:
  - `mut` = may be reassigned AND changed (mutating methods, `&mut` borrows, field writes).
  - `open` = contents may change (mutating methods, `&mut`), but never reassigned.
  - `mut open` is redundant (warning).

  ```cpp
  uint32 x = 5;
  mut uint32 y = 0;            // y = 1; allowed
  open string[?] xs = [];      // xs::push("a"); allowed, xs = [...] is not
  ```
- `uint32 later;` (no value) is allowed; reading it before every path assigns it
  is a compile error.
- `uint32 later;` on an immutable binding may be assigned exactly once, where
  it is certainly still unassigned (so not inside a loop it was declared outside of).
- Parameters are immutable unless written `mut uint32 n`.
- Operators:
  - arithmetic `+ - * / %`, power `**`; `/` on two integers is whole-number division (`7 / 2` = 3)
  - comparison `== != < <= > >=`; logic `&& || !` (short-circuit)
  - bitwise `& | ^ ~ << >>` (`^` is XOR, never power, outside unit expressions)
  - `++ --`, compound assignment `+= -= *= /= %= **= &= |= ^= <<= >>=`
- Operands are evaluated left to right.
- Grouping, tightest first: calls and postfix `++ --`; prefix `- ! ~ ++ --`;
  `**` (chains from the right); `* / %`; `+ -`; `<< >>`; `& | ^`; comparisons;
  `&&`; `||`.
  - Unary minus applies before `**`: `-2 ** 2` is `(-2) ** 2` = 4. `2 ** 3 ** 2` is 512.
  - **Bitwise operators need parentheses** next to arithmetic or comparisons:
    `a & 1 == 0` and `a + b << 2` are compile errors (E0006) with a fix-it;
    write `(a & 1) == 0`, `(a + b) << 2`.
  - Shifts may sit inside `& | ^` without parentheses (`r << 16 | g << 8 | b`),
    but `&`, `|`, `^` cannot be mixed with each other, nor `<<` with `>>`,
    without parentheses. A chain of one operator is fine: `a | b | c`.
- `++` / `--` work as prefix and postfix inside expressions: `i++` gives the old
  value, `++i` the new one. The operand must be a `mut` number binding.
- Assignment and compound assignment are statements, not expressions.
- `%` takes the sign of the left side (pairs with `/`): `-7 % 3` is `-1`,
  `7 % -3` is `1`. On floats it is the same rule (C `fmod`).
- Integer `**` with a negative exponent is whole-number division of 1 by the
  power: `2 ** -1` is 0, `1 ** -n` is 1, `(-1) ** -n` is 1 or -1, and
  `0 ** -n` panics like division by zero.
- Shifting by the type's width or more, or by a negative amount, follows
  `[behaviour] overflow`: **wrap** = the amount wraps around the width
  (`1u32 << 40` is `1 << 8`), saturate = the bits fall off, panic = exit 101.
  A constant amount that wraps warns (W0106).
- Division by zero: integers panic (exit 101); floats follow IEEE (infinity / NaN).
  The smallest signed value divided by -1 wraps (under overflow = wrap).
- Unary `-` on an unsigned value: config `unsigned_negate` = wrap | error | signed
  (signed = becomes the next larger signed type); default **wrap**.

## 3. Control flow

```cpp
if (a) { } else if (b) { } else { }
while (cond) { }
for (mut uint32 i = 0; i < 10; i++) { }     // C-style
for (string name of names) { }             // for-each uses `of`
for (string c of text) { }                 // text: an owned copy of each character
for (&string c of text) { }                // or a borrowed view (no copy)
for (uint32 i of 0..9) { }                 // ranges are inclusive: 0 to 9
for (uint32 i of 0..10 by 3) { }           // 0, 3, 6, 9
for (float64 x of 0.0..1.0 by 0.25) { }    // float ranges must say `by`

break;  break 2;  continue;  continue 2;   // N = how many loops; default 1

match (x) {
    case 0    { }
    case 1, 2 { }
    case _    { }                           // catch-all
}
```

- `by` sets a range loop's step: `0..10 by 3` is 0, 3, 6, 9. The loop stops at
  the last value not past the end; it never clamps to the end. Integer ranges
  may omit `by` (step 1); float ranges must give it.
  Built for integer and float ranges (the step is a value of the loop's type; integer
  steps wrap on overflow).
  - Sign: a negative step counts down (`10..0 by -2`); a step pointing away from the
    end gives an empty loop. A negative literal step also counts down over an
    unsigned loop variable and emits W0102. Both directions include the end
    when the step lands on it.
  - A literal zero step is E0115 (compile error). A step that is zero at run time
    stops the program with `range step is zero` (exit 101), like an out-of-range
    index. Built.
  - Floats: value k is `start + k * step` (whole-number k, no drift); the loop stops
    at the last value not past the end. A float range without `by` is E0101. Built.
- `match` arms are `case` blocks with no fallthrough. A missing case is a
  **warning** (config can make warnings errors).
- Patterns: values, `case 1, 2`, inclusive ranges `case 13..19`, strings (a
  plain string matches exactly, `*` included), and globs written
  `case globbed "/api/*"`, where each `*` is a wildcard. A literal `*` is matched
  by a plain string. `case _` = anything.

### skip (original to Drachm)

`skip` leaves `if` blocks the way `break` leaves loops.

```cpp
if (a) {                       // outermost
    if (b) {
        if (c) {               // innermost
            skip;              // -> (3): bare skip leaves the OUTERMOST if of the nest
            skip 1;            // -> (1)
            skip 2;            // -> (2)
            skip to next;      // -> tests `else if (d)`
        } else if (d) { }
        // (1)
    }
    // (2)
}
// (3)
```

- `skip N` leaves N levels, counting outward from the innermost.
- Leaving an `if` also skips its `else if` / `else` branches; `skip to next`
  instead jumps into the next branch of the same chain.
- Loops are a wall: only `if` levels inside the current loop body count, and an
  N larger than that is a compile error.
- `match` is not a wall: each `case` counts as one level; `skip to next` inside a
  case tests the next case.
- Lambdas are walls too.
- `skip to next` from the last branch (`else` or last `case`) is a compile error.

## 4. Types

### Numbers

| Kind | Types | Suffix |
|---|---|---|
| Unsigned | `uint8 uint16 uint32 uint64 uint128 uint256` | `5u8` … `5u256`, bare `5u` = uint32 |
| Signed | `sint8 sint16 sint32 sint64 sint128 sint256` | `5s8` … `5s256`, bare `5s` = sint32 |
| Float | `float8` (E4M3) `float16 float32 float64` | `2.5f8` … `2.5f64`, bare `5.0f` = float64 |
| Boolean | `bool` | `true`, `false` |

- No `int`/`float` names.
- A suffix is needed only when nothing else gives the type (`uint8 a = 5;` needs none).
- Literal forms: decimal with `_` separators (`1_000_000`); `0xFF`, `0b1010`,
  `0o17` (a plain leading zero stays decimal); exponents `1e9`, `2.5e-3` (always
  a decimal constant); and word bases `hex FF`, `bin 1010`, `oct 17` (`hex`,
  `bin`, `oct` are only special directly before digits of that base). Integer
  suffixes work on all of them (`0xFFu8`, `hex FFu8`).
- Untyped constant expressions are evaluated exactly at compile time, then typed
  by their result: whole ≥ 0 → `uint32`, negative → `sint32`, decimal → `float64`.
  `5 - 10` is `-5` as `sint32`. `0.1 + 0.2` is exactly `0.3`. A whole constant
  too big for 32 bits grows to the 64-bit, then the 128-bit type.
  `~` on a constant is exact too (`~0` is `-1`), then fits like any constant:
  `uint8 m = ~0;` is 255 with W0102; `~0u8` is silent.
- An untyped constant on the right of a typed value takes that value's type
  (`total + 3`). On the left it takes the right side's type if it fits there
  exactly (`3 + total`); if not, it gets its own default type and leads
  (`1.5 * hp` is `float64`, `300 + someUint8` is `uint32`).
- Dividing a constant by a constant zero is a compile error (E0107), for
  decimals too; so is a whole-number `/` or `%` by a constant zero.
- **A constant that doesn't fit** where it is going (`uint8 a = 300;`,
  `uint32 x = -1;`, `uint32 n = 2.5;`) follows `[behaviour] overflow`: wrap =
  W0102 and it wraps (300 → 44; decimals drop the fraction first), saturate =
  W0102 and it clamps (255), panic = compile error.
- All numeric types convert automatically. **Lossy** conversions print a big
  warning (W0102); lossless ones are silent. Explicit conversion silences it:
  `(damage * 1.5)::to<uint8>()`.
  - Lossless: integer to an integer type covering its whole range; integer to
    a float that represents every value exactly (up to 16 bits → `float32`, up
    to 32 bits → `float64`); `float32` → `float64`. Everything else is lossy.
  - A value that doesn't fit at runtime follows `[behaviour] overflow` as well,
    for automatic conversions and `::to<T>()` alike. wrap: integers keep their
    low bits, floats drop the fraction and then wrap. saturate: clamp to the
    nearest end. panic: exit 101. NaN and infinities become 0 (or panic).
- **Mixed types: the left side leads.** In `a + b`, `a < b` and every other
  two-sided operator, the right side converts to the left side's type (W0102 if
  lossy) and the operation is done in that type. No exceptions for floats:
  with `uint32 hp`, `hp * 1.5` is done in `uint32` (1.5 becomes 1, W0102);
  write `1.5 * hp` or `hp::to<float64>() * 1.5` for a decimal result.
  - Except counts: the right side of `<<`, `>>` and integer `**` keeps its own
    integer type; the result has the left side's type.
- `bool` is not a number: it never converts to or from one, and conditions
  must be `bool`.
- `::text()` on a number or bool gives its text. Floats print the shortest
  digits that read back as the same number, always with a decimal point and
  never in exponent form: `1.0` → `"1.0"`, `0.1 + 0.2` at runtime →
  `"0.30000000000000004"`, `1e21` → `"1000000000000000000000.0"`.
  The special values print as `inf`, `-inf` and `nan` (draft, not confirmed).
- Overflow: set by config, default **wrap** for signed and unsigned.

### Strings

- `string` and `str` are the same type. No char type: a character is a
  one-character string, and "character" means a **grapheme cluster**.
- Method calls: `::` makes the first call on a value, `->` chains each later one:
  `"dcba"::reverse()->sub(0, 1)` is `"ab"`.
- Methods return a **new** string; the original is never modified.
- Indexes are 0-based; negative indexes count from the end (`-1` = last).
- Joining: `+` or `::join` (`..` is only for ranges). A number joins only via `::text()`:
  `"hp: " + hp::text()`.
- Interpolation: JS-style `${expr}` (any expression), only in `f"..."` and
  backtick strings (both single-line): `f"hp: ${hp}"`, `` `hi ${name}` ``. Plain
  `"..."` never interpolates. Bare `{ }` are ordinary text everywhere.
- Multi-line: `"""..."""`; `f"""..."""` interpolates. The newline right after the
  opening `"""` is dropped. The column of the closing `"""` is the left edge: that much
  leading whitespace is removed from every line. A line indented less than that
  is not an error; it just loses all its leading whitespace.
- Escapes: `\n \t \r \0 \\ \" \' \` \$ \%`, `\u{1F600}` by code point, and
  `\u{GREEK SMALL LETTER ALPHA}` by Unicode name.
- `io::outf("% has % hp (%.1)\n", name, hp, pct)`: a bare `%` prints any value
  (its `::text()`). Options sit between the `%` and the value:
  - `%.N` decimal places; `%8` pad to 8 characters on the left, `%-8` on the
    right, `%08` pad with zeros;
  - `%x` hex, `%X` capital hex, `%b` binary, `%o` octal.
  - A literal percent sign is written `\%` (in any string, `\%` is `%`).
  - Every unescaped `%` is a placeholder. `%\` ends a placeholder's options, so
    what follows is text: `io::outf("%\5 apples!", "£")` prints `£5 apples!`,
    whereas `"%5 apples!"` is width 5. (Reading of the owner's note "`%\` should
    negate anything after the `%`"; awaiting confirmation.)

String methods (confirmed 2026-11-10):

| Method | Returns | Example |
|---|---|---|
| `len()` | `uint64` | `"Drachm"::len()` = 6 |
| `isEmpty()` | `bool` | |
| `reverse()` | `string` | |
| `sub(start, end)` | `string` | both inclusive; `"Drachm"::sub(-3, -1)` = `"chm"` |
| `upper()` / `lower()` / `trim()` | `string` | |
| `find(text)` | `sint64` | `-1` when missing |
| `contains` / `startsWith` / `endsWith` | `bool` | |
| `count(text)` | `uint64` | |
| `split(sep)` / `lines()` / `chars()` | `string[?]` | `chars` splits by grapheme |
| `join(text)` | `string` | also `list::join(", ")` |
| `replace(from, to)` | `string` | all matches |
| `replaceLimit(from, to, n)` | `string` | first n matches |
| `insert(index, text)` / `remove(start, end)` | `string` | `remove` both inclusive |
| `repeat(n)` | `string` | |
| `padLeft(w)` / `padRight(w)` | `string` | optional fill text: `"7"::padLeft(3, "0")` = `"007"` |

### Collections

```cpp
uint8[4] rgba = [255, 0, 0, 255];          // = Array<uint8, 4>
string[?] names = ["Illy", "Sam"];         // = List<string>
Map<string, uint32> ages = { "Illy": 18 };
uint32? age = ages["Bob"];                 // missing key -> none
```

- Out-of-bounds index: config, default **panic**.
- Map `[]` returns an optional by default (config can make a missing key panic);
  `::get` always returns an optional.
- A map declared without an initializer starts empty. `m[k] = v` adds or
  replaces an entry. A compound update such as `m[k] += v` stops the program
  if the key is missing. Map methods also support mutation (see Q9).
- Map iteration uses two bindings, for example
  `for (string name, uint32 age of ages)`, or an iteration method (see Q16).
  Map order is unspecified. An `OrderedMap` keeps insertion order.
- Your own types work with `for (T x of thing)` by implementing the built-in
  `interface Iterable<T> { T? at(uint64 i); }` (owner, 2026-11-13; no separate
  walker). The loop evaluates `thing` once and calls `at(0)`, `at(1)`, ... until
  `none`. The value is not spent and can be walked again; `at` takes `self`
  shared, so it cannot change during the loop. A type may be `Iterable` over
  several item types (`with Iterable<Node> and Iterable<Edge>`); the binding's
  type picks the `at` (methods may differ only in their result for this; calling
  one by name is an error). `T x` takes the item, `&T x` views it.
- Built (S6C, S6D; owner answers Q9-Q11, Q20, 2026-11-12): arrays and lists
  share one set of rules. `xs[-1]` is the last item, and an index outside the
  list stops the program (exit 101). Methods: `push(item)`, `pop()` (`T?`),
  `get(i)`, `first()`, `last()` (`T?`), `insert(i, item)`, `remove(i)` (stops when
  outside), `clear()`, `len()`, `contains(item)`, `join(sep)`, `each(f)`. A list
  or map declared without a value starts empty and is usable at once:
  `mut string[?] xs;`.
- Taking an item out by index (Q11): `take xs[i] reorders` is built: the item
  moves out, the last item fills its place, and the order of the others is not
  kept. `take xs[i] leaving v` (built) moves the item out and puts `v` in its place,
  so the length and the order are kept.
- Items that own memory (S6C.2): `get(i)`, `first()` and `last()` give a view,
  an optional that borrows the list (`&T?`); `copy xs::first()` gives an owned
  `T?`. `contains` takes the value written with `&`: `names::contains(&name)`.
- A list of borrows is `List<&T>` (Q3, built): the list becomes a borrower.
- Split edges (S6C.2): `""::split(",")` is `[""]`; `lines()` drops one trailing
  line end and `""::lines()` is empty; `split("")` gives the characters, as
  `chars()` does.
- Built (S6D; Q9, Q13, Q14, Q16): `m[k] = v` adds or replaces an entry;
  `m::set(k, v)`, `m::remove(k)`, `m::get(k)` (an optional) and `m::len()`
  work too; `m[k] += v` on a missing key stops the program. `m::each(f)` and
  `for (K k, V v of m)` walk the entries as copies. Keys: whole numbers, `bool`,
  `string`, floats (compared by bits: a `NaN` key finds itself, `-0.0` and `0.0`
  differ), and structs, enums, optionals and arrays of those. Lists, maps,
  borrows and pointers are not keys (E0626).
- Map order (owner, 2026-11-13): an ordinary `Map` has no promised order; an
  `OrderedMap` keeps insertion order. Replacing a value keeps the entry's place;
  removing a key and adding it again puts it last.
- Not built: views of map entries (`for (&K k, V v of m)`, E0901).
- Built (owner, Q3 follow-ups, 2026-11-13): `maybe &string`, `a same as b` and
  `xs::containsRef(&p)`; see Optionals and Equality below.

### Optionals

- `T?` = `Optional<T>`, an ordinary stdlib enum: `enum Optional<T> { Some(T value), none, }`.
- `none` is both the empty return type and the empty optional value.
- `uint32? age;` starts as `none`. `maybe` unwraps with a fallback: `ages["Bob"] maybe 0`.
- Built (S5A.2): `T?` is one enum per distinct `T` (variants `none`, `Some`). A `T`
  wraps to a `T?` wherever a `T?` is wanted. `x has value` / `x is empty` test it
  (`x == none` is E0533). The fallback of `maybe` is one value or a parenthesised
  expression (E0534). Matching: `case uint32 a` and `case none`.
- Built (Q3 follow-ups, 2026-11-13): an optional that holds a borrow is `maybe &T`
  (`maybe &string f = names::first();`), the same type as `Optional<&T>`; messages
  print `maybe &string`. `&T?` stays a borrow of an optional. `maybe` after a value
  is still the fallback operator.

### Generics

```cpp
T max<T>(T a, T b) { if (a > b) { return a; } return b; }
struct Stack<T> { T[?] items; none push(T item) { self.items::push(item); } }
float64 total<T with Shape>(T[?] shapes) { ... }

uint32 m = max(a, b);            // type args inferred from arguments
uint32 n = max<uint32>(5, 9);    // written when they can't be
```

Type parameters always use `<>`. `with` constrains them.

- Built (S6A; Q1, Q7): the first argument that has a type decides `T`, and a
  later argument must match it. Constants adapt to it, and a call made only of
  constants uses the default type of a constant. An unconstrained parameter is
  checked at each use, as if written out; a constrained one (`with`) is checked
  at its definition as well.
- Built (Q3): a type argument may be a borrow, or a type that holds one:
  `Stack<&string>`. The container becomes a borrower, and its lifetime is worked
  out from what is put in.
- Built (S6A.3; Q2): unit arguments are written with `unit`: `Vec3<float64, unit m/s>`
  and `twice<unit m>(3.0<m>)`. A parameter may put `U` inside a bigger unit
  (`float64<U/s>`), and `U` is worked out from the argument (`dist(3.0<m/s>, 2.0<s>)`
  gives `U = m`). When no argument carries a unit, the call writes one (E0601).
- Built (Q12): value parameters. A generic takes a whole number written like a
  variable, `struct Buffer<T, uint32 N> { T[N] items; }`; the type is any `uintN` or
  `sintN`. The argument is a constant expression (`Buffer<sint32, 16>`,
  `Buffer<sint32, 2 * 8>`) or a value parameter of the enclosing generic; the value
  is part of the type, so `Buffer<sint32, 4>` is not `Buffer<sint32, 8>`. Functions
  take them and work them out from arguments (`sint32 sum<uint32 N>(&sint32[N] a)`);
  the body uses `N` as a constant of its type (`return N;`), also when it is not a
  length (`struct Mod<uint64 P>`). Errors: E0634 (not a constant), E0635 (does not
  fit the parameter's type), E0636 (length 0), E0602 (a type for a value or the
  reverse), E0603 (two arguments disagree), E0609 (used as a type). E0901: a
  constraint on a value parameter, a value parameter on an interface, a value above
  the largest `sint128`. A name that is neither a value parameter nor a named
  constant is E0634.
- Built (Q12 follow-ups, 2026-11-13): named constants and constant arithmetic in
  types. A top-level non-`mut` number whose initializer is a compile-time constant
  (numbers, other constants, arithmetic; any order) is a named constant: usable as a
  value and in types (`uint32 SIZE = 8; uint8[SIZE] raw; Buffer<sint32, SIZE * 2> b;`).
  `fixed` in front (`fixed uint32 MAX = SIZE * 2;`) is optional and makes the
  compiler check the value is constant (E0637 otherwise). `fixed` is context-only: a
  word at the start of a top-level value (and still `*fixed T`). A local with the same
  name shadows the constant. Duplicate names are E0638. A top-level value that is not
  constant (a call, `io::inln()`), a top-level `mut`, and a top-level value that is not
  a number are E0901 until scripts (step 7, Q10); using such a name in a type is E0634
  with a note that it gets its value while the program runs. Any constant arithmetic
  on value parameters is allowed inside types and is checked per instance:
  `T[N + 1]`, `Buffer<T, N * 2>`; a result that is out of range (E0635) or a zero
  length (E0636) is reported with the instantiation. A function infers a value
  parameter only from a plain `T[N]`, or from `[N + k]`, `[N - k]`, `[k - N]`, `[N * k]`
  and `[k * N]` with a number `k` (my reading of "trivially solvable"); anything
  else is E0601 asking the call to write the argument.
- Not built: a constraint on a unit parameter (`unit U with Shape`, E0901); methods with type parameters of
  their own, allowed only through interface methods (Q15: `interface Mappable<T>
  { U[?] map<U>((T) => U f); }`, E0901 today).

### Enums

```cpp
enum Shape {
    Circle(float64 r),
    Rect(float64 w, float64 h),
    Empty,
}

Shape s = Rect(2.0, 3.0);        // bare variant when the type is known
Shape t = Shape::Empty;          // qualified also works

match (s) {
    case Circle(float64 r)          { }
    case Rect(float64 w, float64 h) { }   // typed bindings
    case Empty                      { }
}
```

Bare variants require the language server to complete them (`drachm lsp`, built).

```cpp
case Text(take string body)  { }    // moves the payload out; the matched value is used up
case Text(copy string body)  { }    // copies it
case Circle(float64 r) if r > 10.0 { }            // guard
case Rect(2.0, float64 h)    { }    // a value inside a pattern
case Some(Circle(float64 r)) { }    // nesting
case A(..), B(..)            { }    // several patterns, one body
```

- A payload that owns memory must say `take` or `copy` in the pattern; a bare `string body` is an
  error with both fix-its. Copy payloads (numbers, bools, Copy structs) bind plainly.
- All pattern forms work: guards, several patterns per body, nesting, values inside patterns.
- `==` / `!=` only for enums without payloads (`if (d == North)`). No methods inside enums:
  behaviour is plain functions taking the enum.
- Variant names: the expected type decides first; with no expected type the struct or function of
  that name wins; `Shape::Circle(1.0)` names the variant explicitly. A variant may share a name
  with a struct.

## 4b. Maths

- `math` module: `sqrt pow abs min max clamp floor ceil round sin cos tan log`,
  constants `pi`, `e`.
- Standard library also has `Vector2/3/4`, `Matrix3/4` and quaternions, plus
  exact numbers: big integers, fractions (1/3 stays exact) and decimal /
  fixed-point for money. **OPEN:** their exact type names and module.
- Units of measure (optional, user-definable; step 4.5, done):

```cpp
unit m;  unit s;  unit kg;            // base units
unit N = kg * m / s^2;                // derived (^ is power inside unit expressions)
unit km = 1000 m;                     // scaled
unit pct = 0.01;                      // a named ratio, no base unit (implementer's choice)
unit degC = K offset -273.15;         // a point on a scale

float64<m> dist = 100.0<m>;
float64<s> time = 9.6<s>;
float64<m/s> speed = dist / time;     // dist + time is a compile error
```

  - `unit` is a context word (section 1). Declarations may come in any order;
    a cycle, a redeclaration or a zero or negative factor is E0403.
  - Units go on every number type: `float64<m/s>`, `uint8<km>`, `9.6<s>`,
    `2.5f32<m/s>`. A literal's unit needs no space before `<`: `9.6 < s` is a
    comparison. Unit names are ASCII; prefixes are not automatic (`km` is its
    own declaration).
  - `in si;` loads the SI set as ordinary declarations: base `m s kg A K mol cd`,
    `rad`; derived `N Pa J W C V F ohm S Wb tesla H Hz`; `g = 0.001 kg` (kg is the
    base mass unit), `mm cm km mg ms us ns min h kHz MHz GHz kJ kW kN`, and the
    point `degC`. Without it nothing is predeclared (E0402). A file that declares
    a name the SI set has is E0403.
  - The SI unit for magnetic flux density is `tesla` (owner, 2026-11-13: it was `T`,
    which clashed with type parameters). `float64<tesla>`, `0.5<tesla>`.
  - `X as unit` names the unit X out in full, anywhere a unit goes
    (`float64<tesla as unit>`, `0.5<tesla as unit>`). Built (2026-11-13, owner).
  - A type parameter named like a unit (`K` with `in si;`) is a warning, W0618,
    not an error. Inside the parameter's scope the plain name is the type:
    `float64<K>` there is E0402 ("a type parameter here, so it is not a unit"),
    and `float64<K as unit>` is the unit. Built. A struct, enum or built-in type
    name as a parameter is still E0605.
  - Unit into a plain slot strips to the base amount: `uint8 s = 3<m>;` is 3,
    `uint32 t = 3<km>;` is 3000. Whole numbers truncate toward zero when a unit
    shrinks (W0102 is the implementer's reading).
  - Plain into a slot with a unit attaches it: declaring or assigning
    (`float64<m> d = 5;` is 5 m, `d = 0;` is 0 m), passing an argument, returning
    and fields. `d++` adds one of `d`'s own unit. Only operators are strict.
  - Operators: `*` and `/` take plain numbers (scales). `+ - %` and the
    comparisons need a unit on a plain number, the same for a variable as for a
    constant (`d + 5<m>`, E0408). Zero is the exception, so `d + 0` and
    `speed > 0` are fine; a point's zero is not (`t > 0<degC>`).
  - Results that have no unit left are plain numbers with the scale folded in:
    `4.0<km> / 2.0<m>` is `2000.0`, and `pct * pct` is plain.
  - Different scales of one kind: both sides go through a common base, and the
    result is in the left operand's unit. `10<km> + 100<m>` is `10.1<km>`,
    `10<m> + 1<km>` is `1010<m>`. The left side leads the number type. `a ** n`
    needs a constant whole `n`. Bit operators and shifts are E0406.
  - `x::in<U>()` changes the unit and keeps the number type; `x::to<T>()` changes
    the number type and keeps the unit (`T` is a bare number, E0407).
    `x::text()` is the number only and `x::unitText()` the display name as a
    `string`; `::to<string>()` is an error that points at `::text()`.
  - Integer scaling wraps on overflow exactly as `::to<T>()` does. The
    `[behaviour] overflow` setting is not read by the compiler yet, so no
    conversion path follows it.
  - Points and deltas: point `-` point is a delta (`delta degC`), point `+`
    delta is a point. Point `+` point, a point times anything and unary `-` on a
    point are E0409. `delta degC` is the same type as `K` (same size, converts
    freely), and `::unitText()` of a delta prints `delta degC`
    *(implementer's choice, as is a bare `0` not being unit-free beside a point: write `0<degC>`)*.
  - No unit-generic functions until generics (step 6); a unit in a signature is
    a fixed unit.
  - Diagnostics: E0401 to E0409 (section 10), and W0102 for lossy scaling.

## 5. Structs, interfaces, constructors

```cpp
interface Shape {
    float64 area();
}

struct Circle with Shape {
public:
    float64 r;

    Circle(float64 radius) { self.r = radius; }   // constructor = struct's name
    Circle(uint32 radius)  { self.r = radius; }   // overloads by argument type

    float64 area() { return 3.14159 * self.r * self.r; }
    static Circle unit() { return new Circle of (1.0); }
private:
    uint32 id;
}
```

- `self` is used but never declared as a parameter. Shared vs mutable borrow of
  `self` is **inferred** from the method body.
- Methods that don't use `self` are marked `static`.
- Visibility: C++-style `public:` / `private:` sections. Before any label,
  fields are private and methods public.
- A struct implements interfaces with `with`: `struct Circle with Shape`,
  several with `and`: `struct Circle with Shape and Drawable`.
- An interface name used as a type means "any struct with it", dispatched at
  runtime: `none report(&Shape s) { ... }`.
- Interfaces hold method signatures, `static` methods, and required fields
  (`interface Positioned { float64 x; float64 y; }`). No default method bodies.
  `Self` names the implementing type: `interface Zero { static Self zero(); }`.
- An interface method that may change `self` says `changes self` after its
  parameters: `interface Counter { none bump() changes self; uint32 get(); }`.
  Struct methods stay inferred and may not change more than the interface allows.
  `changes self` is written only in interfaces.
- Interfaces build on others with `extends`: `interface Shape3D extends Shape { float64 volume(); }`.
  Several parents are joined with `and`, as in `with` (implementer's reading, S6B):
  `interface Placed extends Shape and Positioned { ... }`. Equal-named members from two
  parents need equal signatures.
- Built (S6B, S6E): generic interfaces with static dispatch: `interface Read<T> { T get(); }`,
  `struct Cell<T> with Read<T>`, `interface ReadChild<T> extends Read<T>`, and a bound
  that names its instance, `R with Read<T>`. Borrowing one (`&Read<uint32>`) is not built
  (E0901).
- Built (S6B, owner 2026-11-12): `TYPE with I { }` sees only the type's public members;
  `TYPE with trusted I { }` sees private ones as well (`trusted` was the owner's pick).
- Built (S6G, owner 2026-11-13): a struct may have two methods with one name that differ
  only in their result (for two `Iterable<T>`s); calling one by name is E0632.
- A type can promise an interface from outside its declaration (owner, step 6 Q6):
  `uint32 with Zero { static uint32 zero() { return 0; } }`, anywhere in the project. Built in
  for numbers and `string`, without a declaration (owner's reading, tentative set chosen in
  S6B): `Zero` (`static Self zero()`), `Equal` (`bool ==(Self o)`), `Ordered extends Equal`
  (`<`, `<=`, `>`, `>=`), `Text` (`string text()`). A program that declares a type or interface
  of the same name uses its own.
- An interface is only ever borrowed (`&Shape`); `Shape s` and `Shape[?]` are errors (E0617).
  A constrained type parameter (`<T with Shape>`) is checked once at its definition, with only
  what its interfaces promise; unconstrained parameters are checked per instance.
- `&Shape` / `&mut Shape` (S6E): any interface can be borrowed (owner, step 6 Q5); the borrow is two
  words (address, vtable). `&Circle` converts to `&Shape` where one is wanted (argument, declaration,
  return) and `&Shape3D` converts to `&Shape` (upcast); a `static` method, a method with `Self` in its
  signature or an operator is not callable through it (E0619); no conversion is E0640. Required fields
  are read through the borrow. Owner Q5: upcasts are automatic, except that a parameter
  written `exclusive &Shape` accepts only a value whose interface is exactly `Shape`; `exclusive`
  is not built (E0901) and its details are still to design. Interim (E0901):
  writing a required field through `&mut Shape` (does the interface need to say `mut`?), a method that
  returns a borrow, a struct that holds borrows, `&Text` over a built-in number, lists of `&Shape` (Q3).
- `==` is automatic for structs (field by field), and structs with hashable
  fields can be map keys. A `bool ==(T o)` operator method overrides ordinary `==`.
- A `key()` method (e.g. `string key() { return self.text::lower(); }`) defines
  what map keys compare and hash by, including on types with a custom `==`.
  Owner (S6G/S6D follow-up, 2026-11-13): a key type with its own `==` and no `key()` is
  E0627, so add `key()`; a struct holding such a type is the same error.
- Float map keys hash and compare by bits. A NaN key finds itself and
  `-0.0` and `0.0` are distinct keys.
- Construction from the declared type: `Vector3 p = 1, 2, 3;`,
  `Person p = "Illy", 18;`, `Circle a = 5;` (picks `Circle(uint32)`). This
  shorthand works **only in declarations**.
- Elsewhere: `new Vector3 of (1, 2, 3)`. Parentheses after `of` are required.
- Field names may be given, in any order, but never mixed with positional
  arguments: `Vector3 p = z: 3, x: 1, y: 2;`, `new Vector3 of (x: 1, y: 2, z: 3)`.
- Operator overloading: a method named by the operator symbol:
  `Vector3 +(Vector3 o) { ... }`, `Vector3 *(float64 k) { ... }`. An operator
  can be overloaded by the right-hand type; the overload is chosen like a
  constructor's (exact, then lossless, then lossy with a warning). A dot
  product would be a named method, not `*`.
- A struct with no constructor gets an automatic one taking its fields in order. Writing any
  constructor removes it, unless the struct says `Person(auto);`, which keeps it alongside the
  written ones. *(Implementer's reading, unasked: named values match the parameter names of the
  chosen overload; one constructor calling another stays E0901.)*
- Field-by-field filling: a struct declared without a value is filled through `fill p { p.name = "a";
  p.hp = 5; }`, which must give every field a value by its `}`. `fill partial p { ... }` sets some
  fields and leaves `p` incomplete. Until `p` is complete, `fill` / `fill partial` are the only ways
  to touch it.
- Defaults: a `def` is a named template of field values, decoupled from the struct, and a struct may
  have several: `def PlayerStart of Player { hp = 100; title = "none"; }`. The struct names its
  default in its head: `struct Player defaults PlayerStart { ... }`. Shorthand: `def Player { ... }`
  and `struct default Player { ... }`. `fill p with defaults { }` starts from the default def;
  `fill p with Veteran { }` picks a named one. Def values are any expression that does not read
  fields, and a def may extend another: `def Veteran of Player from PlayerStart { hp = 200; }`.
- Reading a field set earlier in a `fill` or constructor needs `now`: `p.max = now p.hp * 2;`,
  `now self.r`. A plain read of an incomplete value is an error with the `now` fix-it; methods on an
  incomplete value are not allowed.
- Scheduled with step 5. In step 3, assigning a field of a struct with no value is E0332 and `fill`
  is E0901.
- `.new()` is not special; it can exist as an ordinary user function.

## 6. Memory: ownership and borrowing

- Strings, lists and most structs **move** on assignment or pass; the source is
  unusable afterwards (E0301 "used after it moved").
- **Copy** = numbers, bools, and structs whose fields are all Copy and whose
  memory size, padding included, is ≤ 32 bytes (so field order can change
  whether a struct is Copy). `struct Handle stays { ... }` opts out: it never
  copies automatically.
- Explicit copy: `string c = copy b;`. `copy r` on a borrow copies the pointed-at
  value (an owned `string`). `copy n` on a Copy value warns (W0322, "`copy` does
  nothing for uint32"); this redundancy lint can be disabled in `[warnings]`.
- No hidden copies anywhere. Returning a field of a borrowed `self` is an error:
  "self is borrowed; write `return copy self.name;`".

### Borrows

- `&T` (shared, many allowed) and `&mut T` (exclusive). Both are always written at
  the call site: `show(&p);`, `push(&mut xs, 1);`. Lending `&mut` needs a `mut` or
  `open` binding.
- A borrow always means its target. Reads are automatic (`q + 1`). `r = 5;` writes
  through `r`. `r = &mut other;` re-points `r`: allowed because the right side is
  a reference (a borrowed thing can never be assigned a pointer, so the right
  side's type decides).
- Passing on a borrow you already hold: `add(lend s);`. `&mut s` where `s` is already a `&mut`
  binding is an error with the fix-it "write `lend s`"; there are no `&&T` types.
- A `&mut` where `&` is expected is written `show(&m)` (an explicit shared borrow through the
  `&mut` you hold, the one allowed borrow of a borrow); bare `show(m)` is an error with the fix-it
  "write `&m`".
- `&T` copies. `&mut T` moves: `&mut string r2 = r;` uses `r` up.
- Built (Q3 follow-ups): `a same as b` compares addresses of two borrows (and of lists,
  optionals of borrows, item by item in order); `!(a same as b)` negates it, and so does `a not same as b` (built). Owned
  values are E0633. `xs::containsRef(&p)` is true when a list of borrows holds a borrow
  of exactly that place. Maps of borrows and structs or enums holding borrows are E0633
  (no single address; the borrows inside are compared one at a time). Address compare of
  `&Shape` (a borrow of an interface) is E0901 for now: compare the data address (owner's
  rule: two borrows are the same when they point at the same address).
- Two-phase receivers: `s::append(s::len()::text())` is fine. Only borrows still
  alive when the receiver changes are errors: `xs::push(xs::first())`.
- Borrowing a temporary in a declaration keeps the temporary alive to the end of
  the block.
- Borrow errors are technical and precise, e.g.:

  ```text
  error[E0302]: cannot change `xs` while it is borrowed
    --> main.drh:5:5
     |
   4 |     &uint32 first = &xs[0];
     |                     ------ `xs` is borrowed here
   5 |     xs::push(9);
     |     ^^ change attempted here
   6 |     io::outln(first::text());
     |               ----- the borrow is still used here
     |
   help: finish using `first` before changing `xs`
  ```

### Fields, `open`, `take`

- Struct fields are immutable unless marked `mut`. A `mut` field can be changed on
  a value you own, even through an immutable binding (like C++ `mutable`):
  `struct Player { string name; mut uint32 hp; }`. Through `&` everything is
  read-only, `mut` fields too; changing needs `&mut`.
- A method that changes only `mut` fields of `self` is a third kind, "changes
  only `mut` fields" (inferred, no syntax): it may be called on an owned value
  that is not `mut` (`Counter k = "page", 0; k::hit();` is fine), not through a
  `&` borrow (E0311), and fine through `&mut`. Any other changing method needs
  `mut` or `open`, else E0312.
- `take p.f leaving v` writes `p.f`, so it needs a `mut` binding or a `mut` field.
- `open` allows field writes, `open` parameters (`none fill(open string s)`), and
  declaring without a value and assigning once later.
- Moving a field out says `take`: `string n = take p.name;`. Plain
  `string m = p.name;` (moving type, owned struct) is an error with the fix-it
  "write `take p.name` or `copy p.name`".
- After `take`, `p.name` has no value (checked at compile time, like
  `uint32 x;`): reading it, or using `p` whole, is an error until it is assigned
  again. Fields still holding values are dropped at scope end.
- `take p.title leaving "none"` puts a value back in the same statement; the field
  stays usable.
- Methods only borrow `self`; there is no consuming `self`.

### Cleanup

- A method with the reserved name `none drop() { }` runs when the value is
  destroyed.
- On a struct with `drop()`, plain `take x.f` is an error; `take x.f leaving v` is
  allowed (the struct stays complete).
- Calling it yourself, `x::drop()`, is an error. `destroy x;` runs `drop()` now and ends `x` like a
  move (nothing runs at scope end); `destroy` is a keyword.

### Lifetimes

- Lifetimes are inferred. A function's `lives` is needed only when the returned
  borrow could come from several parameters (otherwise E0308); it names whole
  parameters, never paths:
  `&string longest(&string a, &string b) lives a, b { }`.
- A method names `self` with a from clause, sources joined by `or` *(implementer's reading)*:
  `&string pick(&string key) lives from self or key { }`. Plain `lives a, b`
  stays for parameters. `lives p` keeps the whole of parameter `p` borrowed
  while the result is used (owner, WP8 Q1).
- Struct fields may hold borrows; the struct's `lives` names the borrow fields
  themselves, each of which may borrow from a different place:
  `struct Pair lives a, b { &string a; &string b; }`,
  `struct Parser lives src { &string src; }`.
- `lives` and `fails` on a function, and `lives` and `with` on a struct, may come
  in either order; `drachm fmt` picks one.
- Structs holding raw pointers never copy automatically (explicit `copy` still works).

### Unsafe and raw pointers

- Pointer kinds: `*T` may be written through (no `*mut T`); `*fixed T` is read-only. `address of` a
  `mut`/`open` binding gives `*T`, `address of` an immutable binding gives `*fixed T`. Conversion is
  explicit both ways, inside `unsafe`: `p::freeze()` (`*T` to `*fixed T`), `p::thaw()` (back).
- `address of n` makes a pointer. Everything touching a pointer value (making,
  copying, comparing, dereferencing, moving, passing, returning, storing) must be inside
  `unsafe { }`, and the body of an `unsafe` function counts as one unsafe block; `*p = 7;`
  dereferences. Both the file flag `//--!$unsafe: true` and an `unsafe` block are
  needed.
- A pointer can be `none` (null), tested with `p == none`. Two pointers of one
  pointee type (`*T` and `*fixed T` may be mixed) compare addresses with `==` and
  `!=`, inside `unsafe`; no freeze or thaw is needed.
- `unsafe` functions: `unsafe` before the return type; calling one needs an
  `unsafe { }` block.
- `*T` moves (`copy p` duplicates it).
- Writing through a raw pointer field (`unsafe { *self.counter += 1; }`) does not change `self`: the
  method stays `&self`.
- No garbage collector.

## 7. Error handling (replaces Result/`?`; there is no Result type)

```cpp
uint32 parse(string s) fails ParseError {
    if (s::isEmpty()) { fail Empty; }
    return 42;
}

uint32 a = parse(input) maybe 0;                       // fallback value
uint32 b = parse(input) fails;                         // pass failure to caller
uint32 c = parse(input) fails with (ParseError e) => {
    io::outln(f"bad input: ${e}");
    maybe 0;                                           // replacement value
};

Save load(string p) fails FileMissing, ParseError { }  // several failure types
Save s = load(p) fails with (FileMissing e) => { maybe Save::empty(); }
                 fails with (ParseError e)  => { return; };   // return leaves the outer fn
```

- A bare `fails` passes the failure up; the caller must itself declare it.
- Calling a failing function with none of `maybe` / `fails` / `fails with`: config
  `unhandled_fail`, default compile **error**; `crash` mode (default in scripts)
  stops with a message on failure.
- Runtime faults (division by zero, bounds) panic: message + exit code 101.

Failure types are their own kind of declaration, listing variants with a
message and an optional exit code (used when the failure escapes `main`;
default 1):

```cpp
failure ToolError {
    NoInput              "no input file"          exit 2,
    BadConfig(string k)  f"bad config key ${k}"   exit 3,
    LoadFailed           "couldn't load save",
}

string t = fs::read(p) fails with (FileMissing e) => {
    fail LoadFailed because e;       // chain a cause
};
// prints: couldn't load save
//   because: no such file "save.dat"
```

- `fail Empty;` searches every failure type the function lists; if more than one
  has `Empty`, write `fail ParseError::Empty;`.
- `maybe` takes any expression and evaluates it only when needed:
  `load(p) maybe makeDefault()`.
- Failure messages are ordinary strings; write `f"..."` to interpolate.
- An escaping failure prints to stderr with an `error: ` prefix, then one `because:` line per cause;
  `${e}` prints the whole chain. Exit codes are 0-255; the failure's code wins over a number `main`
  returns.
- `::to<T>()` on text fails with the built-in `ConvertError` (variants `Empty`, `NotANumber`,
  `OutOfRange`).
- Handlers may name a single variant (`fails with (BadConfig e) => { }`); `any e` can be re-raised
  with `fail e;` when the enclosing function lists it; `e` has members `e::message()`, `e::exit()`,
  `e::cause()`.
- A `maybe` fallback is one value or a parenthesised expression; anything longer is an error asking
  for parentheses: `(parse(s) maybe 0) + 1`, `parse(s) maybe (n * 2)`.
- Ignoring a failure: `fs::close(f) fails with none;` or `fs::close(f) ignore failure;`.
- Failing functions may be anything, operator methods and `drop()` included (`bool ==(Port o) fails X`,
  then `(p == q) maybe false`). *(Implementer's note: a failure escaping an automatic `drop()` at scope
  end is reported like an escape from `main` until the owner says otherwise.)*
- Constructors can fail: `Port(uint32 n) fails BadPort { ... }`, then
  `Port p = 8080 maybe Port::default();`.
- Optionals in `match`: `case uint32 a { }` (has a value) and `case none { }`.
- Only `failure` types can follow `fails`.
- Catch-alls: `fails with (any e)` handles any failure; `otherwise (any e) => { }`
  after specific handlers handles whatever they didn't name:

  ```cpp
  load(p) fails with (FileMissing e) => { ... }
          otherwise (any e) => { io::outln(`load: ${e}`); return; };
  ```
- `${e}` on a failure prints its message.

## 8. Lambdas and concurrency

```cpp
names::each((string n) => io::outln(n));
thread::spawn(move () => { prepare(); work(); });

Channel<uint32> ch;  Mutex<uint32> score = 0;

async string fetch(string url) { ... }
async none main() {
    string page = fetch(url)::await;            // await is postfix, obeys the chain rule
    uint64 n = fetch(url)::await->len();
}
```

- Lambdas: `(params) => expr` or `(params) => { block }`. Captures borrow by
  default. The compiler infers a mutable borrow when the body changes a
  capture. `move` takes ownership.
- Function types mirror lambdas: `(string) => none`, `(uint32, uint32) => uint32`:

  ```cpp
  none each(string[?] items, (string) => none f) {
      for (string s of items) { f(s); }
  }
  ```
- Function values can be called, kept in locals and passed as parameters.
  This includes `move` lambdas. Returning or storing a function value
  inside another value is an error (E0652; owner Q17: a function value only
  goes down). Built (S6F; Q18): a lambda that changes a capture
  (`names::each((string n) => { count++; })`) takes a `&mut` borrow of it,
  inferred as for `self`; the captured variable is `mut`.
- `each` on lists and arrays takes a callback returning `none`. A parameter
  of type `T` gets the item by value, with a copy for an owning item. A
  parameter of type `&T` gets a shared view. **OPEN:** mutable item callbacks
  and their borrow rules.
- Both OS threads and async/await. The async runtime is built into the stdlib.
- Channels and Mutex in the stdlib. Data races are compile errors.
- **OPEN:** thread/async API details.

## 9. Modules

```cpp
// src/game/physics.drh
out Vector3 as Vec3, step;          // export list (anywhere in the file; may rename)
out struct Particle { Vector3 at; } // or `out` on the declaration
struct Vector3 { float64 x; float64 y; float64 z; }
none step() { }

// another file
in io;                                   // whole module
in game.physics as phys;                 // module, renamed
in physics inside game as phys;          // the same, in words
in Vector3 as V3, gravity from physics;  // items, each may be renamed
in * from game.audio;                    // everything straight into scope
in * from game.audio as audio;           // everything under audio:: only
in * from game.audio as audio also bare; // under audio:: and straight into scope
in mygame.game.physics;                  // from the source root: starts with the project name
in "../runtime/scheduler.drh";           // a file path in a string; may climb with ..

phys::Vector3 v = 1, 2, 3;
```

- Any file that exports something is a module.
- Dots for path-like things (module paths); `::` for call-like (`phys::Vector3`, `io::out`).
  Dotted and word forms (`game.physics`, `physics inside game`) are the same path.
- A path is relative to the importing file's folder; one that starts with the
  project's name from the config starts at the source folder. To climb a folder up,
  write the file path in a string, relative to the importing file
  (`"../runtime/scheduler.drh"`); it works wherever a module path does, and without
  `as` the module is named after the file. A standard module and a project file of
  one name is an error.
- `in * from m as y` puts the names under `y::` only; `also bare` also brings
  them bare (`play()` and `y::play()` are the same).
- Nothing is auto-imported in projects, not even `io`.
- Two modules of one file name in one file: an error at the second import, fix-it `as`.
- A local declaration with a module's name shadows the module, with a warning.
- `in *` name clashes (`import_clash` = warn | error): a local declaration wins,
  then the first import; the warning is at the later import.
- `X with I { }` blocks and `def`s follow the type.
- Import cycles are allowed; cross-file mutual recursion is an error.
- Built (step 7 WP1, WP2): paths, loading, the graph, `out` lists and `out` on
  declarations (functions, structs, enums, interfaces, failures, units, named
  constants), names across files (item and module imports, `*`, aliases,
  `module::name` in types, calls, constructors, enum variants, constants, array
  lengths), visibility (E0236, E0237), clashes (E0232, W0240, W0241) and the
  cross-file recursion check (E0238, direct calls of functions). Each file
  imports what it uses (`io` included). Names are joined into one program
  before the checker runs: a declaration of the entry file keeps its name, any
  other is `MODULE::name` (`game.physics::Vector3`), mangled for symbols.
  Not built: re-exporting an imported name, `out` on a `with` block or a `def`
  (defs follow their struct), a qualified unit (`lib::fathom` has no unit syntax: import units by name or bare).
- C headers (step 7.1): `in * from c."SDL2/SDL.h" as sdl;`. Imported C calls
  are unsafe. Drachm code can expose safe wrappers. Declare native libraries
  in the `[native]` table in `drachm.toml`, alongside include directories and
  definitions. The import exposes declarations from the named header and its
  transitive includes. Scripts can check headers with the default search path.
  Linking requires a project manifest.

## 10. Projects, config, scripts, tooling

- `drachm check` checks a library without requiring `main()`.
- `drachm new mygame` creates `drachm.toml`, `src/main.drh`, `tests/`.
- Tests: `test "name" { assert(...); }` blocks in any file; `drachm test`.
- `drachm fmt`: one official style with few knobs (indent width, line length,
  brace placement). It never aligns `=` or comments across lines.
- Registry: **the Agora** (agora.drachm.dev); `drachm add json`. Deps may also be
  git URLs or local paths.
- CLI: `new build run check test add fmt explain`, `-O` for optimised builds.
- Built (step 7 WP3): `drachm.toml` `[project]` (name, kind `program`|`library`,
  entry), `[dependencies]` by `path` or `git` with an exact `tag` (fetched into
  `deps/NAME/` of the project, pinned by `drachm.lock`; owner, 2026-11-13), the
  `[build]`, `[warnings]` and `[behaviour]` keys are read and checked (behaviour
  is applied by WP4), `drachm new NAME`. Registry versions are E0901. Errors
  E0800-E0814.

```toml
[project]
name = "mygame"
version = "0.1.0"

[dependencies]
json = "1.2"
sdl = { git = "https://github.com/someone/sdl-drh", tag = "v0.4" }
utils = { path = "../utils" }

[build]
target = "linux-x64"         # os-cpu: linux-x64, macos-arm64, windows-x64
opt = true

[warnings]
as_errors = ["lossy-conversion", "missing-case"]

[behaviour]
overflow = "wrap"          # wrap | saturate | panic
bounds = "panic"           # panic | unchecked
missing_key = "optional"   # optional | panic
unhandled_fail = "error"   # error | crash
import_clash = "warn"      # warn | error
unsigned_negate = "wrap"   # wrap | error | signed
```

Scripts:

| Nearby file | Treated as |
|---|---|
| `drachm.toml` | project (needs `main()`) |
| `srachm.toml` | script with config (same settings as drachm.toml) |
| neither | script with defaults |

Scripts allow top-level statements, auto-import core (`io`, `string`,
collections, `env`, `fs`), crash on unhandled failures, and hide warnings.
Scripts run through Cranelift's JIT in memory (nothing written to disk). They
may start with a shebang (`#!/usr/bin/env drachm`), import local files next to
them (`in helpers;` finds `helpers.drh`), and use Agora packages declared in
`srachm.toml` or inline: `//--!$dep json: 1.2`.

Diagnostics: detailed Rust/Elm style (labelled source lines, explanation,
`help:` fix). `E` codes for errors, `W` for warnings, `drachm explain E0301`.
Red errors, yellow warnings, colour auto-off when piped. Report all errors.
`E` and `W` share one number space: `00xx` syntax, `01xx` types and
conversions, `02xx` names and bindings, `03xx` ownership, `04xx` units, `05xx` constructors, enums, optionals and failures (enums built: E052x; optionals: E053x; failures: E054x, E0549, E0551, E0552, E0554), `06xx` generics, interfaces, collections and lambdas (step 6), `09xx` not supported
yet. The table is `src/diag/codes.rs`. W0102 looks like this:

```text
warning[W0102]: converting `sint64` to `uint8` can lose information
  --> main.drh:10:15
   |
10 |     uint8 a = big;
   |               ^^^ this is `sint64`
   |
 note: `sint64` holds -9223372036854775808 to 9223372036854775807; `uint8` holds 0 to 255; values outside that wrap around
 help: write `big::to<uint8>()` to say the conversion is intended
```

Standard library ("middle" size): core `io string collections math convert`;
system `fs time env process`; concurrency/network `thread async net http`.
`json random regex crypto` are official optional packages from the Agora.
`io::out` (no newline), `io::outln` (newline), `io::outf` (printf-style),
`io::inln()` (the next line of standard input as a `string`, without its line
ending; `""` at the end) and `io::ended()` (no input left). Text reads as a
number with `text::to<T>()`, forgivingly: surrounding spaces are ignored, a leading `+` is allowed,
`.5` and `5.` are fine for floats, out-of-range is still bad text. Bad text fails with `ConvertError`;
until failing calls exist it stops the program (exit 101) *(implementer's interim choice, not asked)*. Rules: `site/content/reference-io.md`.

## Later

- Whole functions written in assembly, never mixed inline: instructions one per
  line, and the body is the asm. Called like any `unsafe` function.

  ```cpp
  unsafe uint64 cycles() asm { rdtsc; shl rdx, 32; or rax, rdx }
  ```

  Scheduled after C header import. **OPEN:** register/ABI mapping, Intel vs AT&T
  syntax, other CPUs.
