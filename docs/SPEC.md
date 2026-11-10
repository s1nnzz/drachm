# Drachm Language Specification (design snapshot)

Snapshot of every design decision as of 2026-11-10. The living version is the
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
  copy lives open break continue unsafe async await move test unit true false`. `to` is a keyword only
  directly after `skip`.
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
for (uint32 i of 0..9) { }                 // ranges are inclusive: 0 to 9

break;  break 2;  continue;  continue 2;   // N = how many loops; default 1

match (x) {
    case 0    { }
    case 1, 2 { }
    case _    { }                           // catch-all
}
```

- `match` arms are `case` blocks with no fallthrough. A missing case is a
  **warning** (config can make warnings errors).
- Patterns: values, `case 1, 2`, inclusive ranges `case 13..19`, strings, and
  string globs with `*`: `case "/api/*"`, `case "*.png"`. `case _` = anything.

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
- Multi-line: `"""..."""`; `f"""..."""` interpolates.
- Escapes: `\n \t \r \0 \\ \" \' \` \$`, `\u{1F600}` by code point, and
  `\u{GREEK SMALL LETTER ALPHA}` by Unicode name.
- `io::outf("% has % hp (%.1)\n", name, hp, pct)`: a bare `%` prints any value
  (its `::text()`); `%.N` sets decimal places. **OPEN:** literal percent sign
  (assumed `%%`) and other format options (width, padding, hex).

String methods (signatures are a draft the owner should confirm):

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
| `repeat(n)` / `padLeft(w)` / `padRight(w)` | `string` | |

### Collections

```cpp
uint8[4] rgba = [255, 0, 0, 255];          // = Array<uint8, 4>
string[?] names = ["Illy", "Sam"];         // = List<string>
Map<string, uint32> ages = { "Illy": 18 };
uint32? age = ages["Bob"];                 // missing key -> none
```

- Out-of-bounds index: config, default **panic**.
- Map `[]` returns an optional by default (config can make a missing key panic);
  `::get` always exists. Map iteration order is unspecified; an `OrderedMap`
  keeps insertion order.
- Your own types work with `for (T x of thing)` by implementing `Iterable<T>`,
  whose `T? next()` returns `none` when done.

### Optionals

- `T?` = `Optional<T>`, an ordinary stdlib enum: `enum Optional<T> { Some(T value), none, }`.
- `none` is both the empty return type and the empty optional value.
- `uint32? age;` starts as `none`. `maybe` unwraps with a fallback: `ages["Bob"] maybe 0`.

### Generics

```cpp
T max<T>(T a, T b) { if (a > b) { return a; } return b; }
struct Stack<T> { T[?] items; none push(T item) { self.items::push(item); } }
float64 total<T with Shape>(T[?] shapes) { ... }

uint32 m = max(a, b);            // type args inferred from arguments
uint32 n = max<uint32>(5, 9);    // written when they can't be
```

Type parameters always use `<>`. `with` constrains them.

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

Bare variants require the language server to complete them.

## 4b. Maths

- `math` module: `sqrt pow abs min max clamp floor ceil round sin cos tan log`,
  constants `pi`, `e`.
- Standard library also has `Vector2/3/4`, `Matrix3/4` and quaternions, plus
  exact numbers: big integers, fractions (1/3 stays exact) and decimal /
  fixed-point for money. **OPEN:** their exact type names and module.
- Units of measure (optional, user-definable):

```cpp
unit m;  unit s;  unit kg;            // base units
unit N = kg * m / s^2;                // derived (^ is power inside unit expressions)
unit km = 1000 m;                     // scaled

float64<m> dist = 100.0<m>;
float64<s> time = 9.6<s>;
float64<m/s> speed = dist / time;     // dist + time is a compile error
```

- Scaled units of the same kind convert automatically, to the unit of the
  first operand: `10<km> + 100<m>` = `10.1<km>`, `10<m> + 1<km>` = `1010<m>`.
- The stdlib ships SI units (m, s, kg, N, J, W, …) ready to use.

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
- Interfaces build on others with `extends`: `interface Shape3D extends Shape { float64 volume(); }`.
- `==` is automatic for structs (field by field), and structs can be map keys
  automatically. A `bool ==(T o)` operator method overrides `==`.
- A `key()` method (e.g. `string key() { return self.text::lower(); }`) defines
  what map keys compare and hash by. **OPEN:** what happens when a type has both
  `key()` and a custom `==`.
- Construction from the declared type: `Vector3 p = 1, 2, 3;`,
  `Person p = "Illy", 18;`, `Circle a = 5;` (picks `Circle(uint32)`). This
  shorthand works **only in declarations**.
- Elsewhere: `new Vector3 of (1, 2, 3)`. Parentheses after `of` are required.
- Field names may be given, in any order, but never mixed with positional
  arguments: `Vector3 p = z: 3, x: 1, y: 2;`, `new Vector3 of (x: 1, y: 2, z: 3)`.
- Operator overloading: a method named by the operator symbol:
  `Vector3 +(Vector3 o) { ... }`, `Vector3 *(float64 k) { ... }`.
- A struct with no constructor gets an automatic one taking its fields in order.
- `.new()` is not special; it can exist as an ordinary user function.

## 6. Memory: ownership and borrowing

- Strings, lists and most structs **move** on assignment or pass; the source is
  unusable afterwards (E0301 "used after it moved").
- Numbers, bools, and structs whose fields are all copyable and whose total size
  is ≤ 32 bytes **copy** automatically.
- Explicit copy: `string c = copy b;`
- Borrows: `&T` (shared, many allowed) and `&mut T` (exclusive). The call site
  writes it: `push(&mut xs, 1)`. Lending `&mut` needs a `mut` or `open` binding.
- Struct fields are immutable unless marked `mut`. A `mut` field can always be
  changed, even on an immutable value (like C++ `mutable`):
  `struct Player { string name; mut uint32 hp; }`
- Struct fields may hold borrows; the struct names what it borrows from:
  `struct Parser lives src { &string src; }`
- Structs holding raw pointers never copy automatically (explicit `copy` still works).
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
- Lifetimes are inferred. When needed, `lives` names the parameters a returned
  borrow comes from: `&string longest(&string a, &string b) lives a, b { }`.
- Unsafe: raw pointers `*T` need BOTH a file flag `//--!$unsafe: true` AND an
  `unsafe { }` block. `*p = 7;` dereferences.
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
    BadConfig(string k)  "bad config key ${k}"    exit 3,
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
  default; `move` takes ownership.
- Function types mirror lambdas: `(string) => none`, `(uint32, uint32) => uint32`:

  ```cpp
  none each(string[?] items, (string) => none f) {
      for (string s of items) { f(s); }
  }
  ```
- Both OS threads and async/await. The async runtime is built into the stdlib.
- Channels and Mutex in the stdlib. Data races are compile errors.
- **OPEN:** thread/async API details.

## 9. Modules

```cpp
// src/game/physics.drh
out Vector3, gravity;               // export list; everything else is private
struct Vector3 { float64 x; float64 y; float64 z; }

// another file
in io;                              // whole module
in physics from game as phys;       // renamed
in Vector3 from phys as v3;         // one item, renamed
in * from game.audio as audio;      // everything under one name
in * from game.audio;               // everything straight into scope

phys::Vector3 v = 1, 2, 3;
```

- Any file that exports something is a module.
- Dots for path-like things (module paths); `::` for call-like (`phys::Vector3`, `io::out`).
- Nothing is auto-imported in projects, not even `io`.
- `in *` name clashes: config `import_clash` = warn | error.
- C headers: `in * from c."SDL2/SDL.h" as sdl;`

## 10. Projects, config, scripts, tooling

- `drachm new mygame` creates `drachm.toml`, `src/main.drh`, `tests/`.
- Tests: `test "name" { assert(...); }` blocks in any file; `drachm test`.
- `drachm fmt`: one official style with few knobs (indent width, line length,
  brace placement). It never aligns `=` or comments across lines.
- Registry: **the Agora** (agora.drachm.dev); `drachm add json`. Deps may also be
  git URLs or local paths.
- CLI: `new build run check test add fmt explain`, `-O` for optimised builds.

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
conversions, `02xx` names and bindings, `03xx` ownership, `09xx` not supported
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
`io::out` (no newline), `io::outln` (newline), `io::outf` (printf-style).
