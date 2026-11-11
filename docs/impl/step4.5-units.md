# Step 4.5: units of measure (implementation spec and work-package briefs)

Status: **the owner answered Q1-Q11 on 2026-11-11** (`owner-answers.md`, "Units
of measure"). Three small follow-up questions are open (section 3); each has a
conservative behaviour marked `// OPEN(unitsQ12)` and so on, and none blocks
U1-U4. Units work may start now: WP2, WP3 and WP4 are merged, and WP5/WP6 run
in parallel (section 4.1 says how to avoid conflicts).

Read first: `docs/SPEC.md` section 4 (Numbers) and 4b (Units of measure),
`docs/PLAN.md` "Step 4.5", `CLAUDE.md` ("Working with the owner", "Keeping the
docs site current"), `docs/impl/owner-answers.md` (the whole units section and
the range/`by` notes), `src/types.rs`, `src/consts.rs`, `src/typeck.rs` (`Val`,
`binary_vals`, `into`, `fit_const`, the `"text"`/`"to"` methods), `src/lexer.rs`
(`number`, `KEYWORDS`), `src/parser.rs` (`ty`, `at_binding`, the method-call
loop), `docs/impl/wp2.md` (brief format).

Rules for the implementer:
- **The owner makes the language decisions.** Section 2 is decided; section 3
  lists what is not. For an open item implement exactly the conservative
  behaviour named there (an error, never a guess) and mark it `// OPEN(unitsQn)`.
  Do not choose among options yourself.
- Units are compile-time only: zero runtime cost, nothing unit-shaped reaches
  the HIR consumers (MIR, ownership, backends). Both backends must behave
  identically; `cargo test` runs every `tests/run` case both ways.
- Every user-visible change updates the docs site in the same commit
  (`site/content/*.md`, `site/examples/NAME.drh` + `.out`, `coming-later.md`,
  `site/gen-errors.sh` after touching `src/diag/codes.rs`). Only document what
  the compiler accepts.

---

## 1. What SPEC already decides

SPEC 4b, "Maths":

> - Units of measure (optional, user-definable):
>
> ```cpp
> unit m;  unit s;  unit kg;            // base units
> unit N = kg * m / s^2;                // derived (^ is power inside unit expressions)
> unit km = 1000 m;                     // scaled
>
> float64<m> dist = 100.0<m>;
> float64<s> time = 9.6<s>;
> float64<m/s> speed = dist / time;     // dist + time is a compile error
> ```
>
> - Scaled units of the same kind convert automatically, to the unit of the
>   first operand: `10<km> + 100<m>` = `10.1<km>`, `10<m> + 1<km>` = `1010<m>`.
> - The stdlib ships SI units (m, s, kg, N, J, W, ...) ready to use.

The last bullet is replaced by the owner's Q9 (below): `in si;`, with no
automatic prefixes. SPEC 1 lists `unit` as a keyword: replaced by Q7 (context
word). SPEC 4's rules that units must cooperate with: the left side leads in
mixed number types (the right converts to the left's type, W0102 if lossy);
an untyped constant takes the other side's type; `::to<T>()` silences W0102.
SPEC 10: this step adds `04xx` (units); update that sentence when U6 lands.

### The owner's scratch file

`examples/stupid.drh` (untracked, the owner's; **never edit or commit it**):

```cpp
in io;
none main() {
    uint8 s = 3<m>;
    io::outln(s::to<string>());
}
```

With the answers it becomes valid once `in si;` is added and `::text()` is used
(`::to<string>` is a separate strings-area question, E0101 today, not a units
item). U5 adds the corrected program as a golden test.

---

## 2. The design (decided)

### 2.1 Decisions, in one table

| Q | Decision (owner, 2026-11-11) | Consequence in the compiler |
|---|---|---|
| 1 | Unit value into a plain number slot is allowed and **strips to the base unit amount**: `uint8 s = 3<m>;` is 3, `uint32 t = 3<km>;` is 3000. "Units need syntax; they don't just work magically." | Plain slot = `T` with no unit. Value is multiplied by the unit's scale (and offset corrected for points, 2.8). Silent when exact; W0102 when it truncates or can overflow, like any lossy conversion. Constants fold. The reverse (plain into unit) is **not** decided: Q12. |
| 2 | `::text()` prints the number only. `::unitText()` gives the unit's text (`"km"`); the program lays them out. | `text` ignores the unit. New method `unitText` on a value with a unit: a `string` constant, the display name (2.2). On a plain number: E0101. |
| 3 | Dimensionless results are always a plain number with the scale folded in: `4.0<km> / 2.0<m>` is `2000.0`. | No dimensionless `Type::Unit` ever exists (`with_unit` returns the bare type). `*`, `/`, `**` that cancel every dimension multiply by the exact scale. A **declared** dimensionless unit (`unit pct = 0.01 1;`) is not decided: Q13. |
| 4 | Units on every number type; integers cannot hold fractions: `uint8<km> s = n;` with `n` = 300 m gives 0. | No restriction at declaration. |
| 5 | Integer scaled conversion truncates toward zero (my reading: with W0102 like any lossy conversion). | `x * p / q` in the target number type, `/` truncating; W0102 when `q != 1` for integers. |
| 6 | No unit-generic functions until step 6 generics (`float64<U> twice<unit U>(float64<U> x)`). | A unit in a signature is a fixed unit. Nothing else in this step; do not reserve syntax. |
| 7 | `unit` is a **context word**: it declares only at the start of a top-level item; elsewhere an ordinary name (`static Circle unit()` is fine). | Remove `"unit"` from `lexer::KEYWORDS`. The parser treats `Ident("unit")` as a declaration only at item start when followed by `NAME` and then `;` or `=`. |
| 8 | Offset units are **points** and **deltas**: `float64<degC>` is a point, `b - a` is `float64<delta degC>`, point + delta is a point, point + point is an error. Declared `unit degC = K offset -273.15;`. | 2.8. |
| 9 | SI units come with `in si;` (built into the compiler until modules exist). | `in si;` loads an embedded prelude of ordinary `unit` declarations (2.9). Without it nothing is predeclared. |
| 10 | `*` and `/` take plain numbers (they are scales); **zero is unit-free**; any other plain constant in `+ - %` or a comparison needs its unit (`d + 5<m>`). | 2.5. |
| 11 | `::in<m>()` changes the unit and keeps the number type; `::to<T>()` changes the number type. | 2.6. |
| prefixes | **Not automatic.** Scaled units are ordinary declarations (`unit g; unit kg = 1000 g; unit l; unit ml = 0.001 l; unit ft; unit mi = 5280 ft;`); `in si;` ships the common ones declared that way. Arithmetic on scaled units goes through the base and back: `2<mi> + 2<mi>` = `(5280*2 + 5280*2) ft / 5280`. Unit names are ASCII only (no `µ`). | No generated `k`, `m`, `u` prefixes. Unit names `[A-Za-z_][A-Za-z0-9_]*` only. Arithmetic rule 2.5 (common-base evaluation). |

Note the owner's prefix example declares `g` as the base mass unit (`unit g;
unit kg = 1000 g;`). SI's `kg` as the base is the compiler's choice for the
prelude only (2.9), and the owner may prefer `g`; either is ordinary
declarations. See Q14 (kilogram or gram as the base in the shipped set).

### 2.2 Unit representation (`src/units.rs`)

A unit is its **canonical form**: a base-unit exponent vector, an exact rational
scale, and an exact rational offset.

```rust
/// Interned, leaked: `&'static Unit` is Copy, Eq and Hash by content.
pub struct Unit {
    /// Sorted by base name, no zero exponents. `m/s^2` is [("m",1),("s",-2)].
    pub dim: Vec<(&'static str, i32)>,
    /// How many of the base units (in `dim`) one of this unit is. `km` is 1000,
    /// `g` with `unit kg; unit g = 0.001 kg;` is 1/1000. `BigRational`
    /// (`num_rational` is already a dependency, used by consts.rs).
    pub scale: BigRational,
    /// 0 for every ordinary unit and every delta. Nonzero only for a point
    /// unit such as `degC` (2.8): a reading `x` in this unit is the base
    /// reading `(x - offset) * scale`.
    pub offset: BigRational,
}
```

- Two units are the same *type* exactly when `dim`, `scale` and `offset` are
  equal, so `unit N = kg * m / s^2;` and the written `kg*m/s^2` are one type.
- Base units are the ones declared `unit m;`, identified by name; `scale` 1,
  `offset` 0. Every other unit is derived.
- **Display name.** A side table in `UnitTable` maps a canonical `Unit` to the
  name used in messages and by `::unitText()`: the first `unit NAME ...;` that
  declared it, else the expression as first written, normalised (`m/s`,
  `kg*m/s^2`); a product made by arithmetic is formatted from its operands'
  display names at creation (`km*m`). A delta prints `delta degC`. Not part of
  identity.
- Arithmetic (all exact): `mul`, `div`, `pow(n: i32)`, `ratio(from, to)` (only
  when `dim` equal), `is_dimensionless()`. Exponents are `i32`; past the `i16`
  range is E0405. A scale of zero or less is impossible by construction (E0403).
- `delta_of(point) -> Unit`: same `dim` and `scale`, `offset` 0, display
  `delta NAME`. By content that is the same type as the ordinary unit with that
  dimension and scale (`delta degC` and `K` are one type). That identity is my
  reading, not the owner's: Q14.

### 2.3 Declaring units

Grammar (top-level item, anywhere in the file; order does not matter, cycles are
an error):

```text
unit-item  := "unit" NAME ";"                              base unit
            | "unit" NAME "=" unit-def [ "offset" SIGNED ] ";"
unit-def   := [NUMBER] unit-expr | NUMBER "/" unit-expr
unit-expr  := unit-term { ("*" | "/") unit-term }
unit-term  := NAME [ "^" ["-"] INTEGER ] | "(" unit-expr ")" | "1"
SIGNED     := ["-"] NUMBER
```

- `NAME` is ASCII: `[A-Za-z_][A-Za-z0-9_]*`. A non-ASCII character in a unit
  name or expression is E0403 ("unit names are plain ASCII letters, digits and
  `_`").
- `NUMBER` is a decimal or whole literal read exactly; it must be positive
  (E0403). `1000 m` means multiply; the number is allowed only at the very front
  of `unit-def`. `1 / s` is `NUMBER "/" unit-expr` (Hz); inside `unit-expr` the
  only number is the dimensionless `1`.
- `offset c` (Q8): only after a unit-def that is a single unit name with an
  optional factor (`K`, `0.5 K`), never after a product or quotient (E0403).
  Meaning: a reading `x` in the new unit relates to a reading `e` in the
  defining expression E by `x = e / factor + c`. So `unit degC = K offset
  -273.15;` has `degC = K - 273.15`, i.e. `0<degC>` is 273.15 K. Stored as
  `scale = factor * scale(E)`, `offset = c`.
- `^` is lexed as the existing bitwise-xor symbol; the parser treats it as a
  power only inside a unit expression. No new lexer symbol.
- Resolution (`typeck/units.rs::declare_units`, before struct layout and
  function signatures): collect all unit items into a map, evaluate lazily with a
  visiting set (`unit N = kg*m/s^2;` may come before `unit kg;`). A cycle,
  a redeclaration, a zero or negative factor, `unit u = 5;` (no unit on the
  right) and a **dimensionless declaration** (Q13: conservative) are E0403.
- A unit name lives in **its own namespace**: `uint8 s = 3<m>;` has a variable
  `s` and a unit `s`. Unit names are looked up only inside `<...>` after a
  number type or literal, after `unit`, and after `::in`.
- Visibility: before modules every unit declared in the file or imported by
  `in si;` is global to the program. A user unit with the same name as an
  imported one is E0403 (declared twice).

AST: `ast::UnitDecl { name, name_span, def: Option<UnitDef>, offset:
Option<(String, Span)>, span }`, `UnitDef { factor: Option<(String, Span)>,
expr: UnitExpr }`, `enum UnitExpr { Name(String, Span), One(Span), Mul(Box,
Box), Div(Box, Box), Pow(Box, i32, Span) }`; a `units: Vec<UnitDecl>` list on
`ast::Program` (the program already keeps `imports` and `structs` as lists;
follow that shape). `ast::TypeName` gets `unit: Option<UnitSpec>`
(`UnitSpec { delta: bool, expr: UnitExpr }`); the number-literal node gets the
same field from the lexer (2.4). Keep `unit` private to the parser and
`units.rs` so that step 6's generic type arguments change it locally.

### 2.4 Literals and the `<` ambiguity

`9.6<s>` must not be confused with `9.6 < s`. The lexer decides with a purely
lexical rule (no knowledge of which names are units):

> **Unit-literal rule.** Directly after a number literal (digits, radix forms,
> exponent, and any integer/float suffix: `5u8<m>`, `2.5f32<m/s>`), if the very
> next character is `<` with **no whitespace between**, scan forward over the
> characters `A-Z a-z 0-9 _ / * ^ - ( )` and single spaces **only after the word
> `delta`** (`3.0<delta degC>`), until the first `>`. If a `>` is found and the
> scanned text parses as a `unit-expr` (grammar 2.3, `1` allowed, optional
> leading `delta`), the number token carries that unit and `<...>` is consumed.
> In every other case `<` is an ordinary less-than.

Consequences, all deliberate: `9.6 < s`, `9.6 <s`, `if (n < limit)`, `3 < 4`
are less-than; `3<x && y>2` has a space/`&` in the scan so it is less-than;
`9<s>>3` is the literal `9<s>` then `> 3`; `1<m>>=2` is `1<m>`, `>=`, `2`; a
scanned text that is not a valid unit-expr (`3<->`, `3<>`, `3<m*>`) falls back to
less-than and the parser reports its normal expression error; an unknown unit
name is E0402 in typeck, not a lexical problem. Only a **literal** takes a unit
tag; a runtime number gets a unit by `n * 1<m>` (the plain operand of `*` is a
scale) or, once Q12 is answered, otherwise.

`Tok::Num(NumLit)` carries `unit: Option<(String, Span)>` (the text and its
span); the parser builds the `UnitExpr` with the shared unit-expr parser (fed
from the stored text through a tiny sub-lexer, or the lexer produces a
`Tok::UnitSuffix`; pick whichever touches fewer lines). Spans point at the unit
text.

**Types.** In a type position `<` is unambiguous. `ty()` after a name that is a
number type, if the next token is `<`: parse `[delta] unit-expr`, expect `>`.
`>>` is one token, so `to<float64>()::in<km>()` is fine but a nested
`foo<float64<km>>` ends in `>>`: `expect_close_angle` splits it (used by `ty()`'s
unit part and the method-call `targs` loop at parser.rs ~880). Units on other
types (`string<m>`, `bool<m>`, a struct, `none<m>`) are E0405; generic arguments
on non-number types stay E0901. `float64<m/s>`, `float64<m / s>`, `float64<1/s>`,
`float64<kg*m/s^2>`, `uint8<m^2>`, `float64<delta degC>` all parse.

**Statement start.** `float64<m/s> v = 3.0<m/s>;` starts with `Ident <`, which
`at_binding` currently reads as an expression. Rule: `Ident` followed by `<` is a
declaration when a lookahead over tokens that can occur in a unit expression
(idents, `delta`, `1`, `* / ^ - ( )`, integers) reaches a `>` directly followed
by an `Ident`. Same lookahead after `mut` / `open`.

**`::in<unit>()`.** After `::`, the method named `in` takes a **unit expression**
in its angle brackets, not a type: `d::in<m>()`, `v::in<km/h>()`,
`c::in<delta degC>()`. The parser special-cases the method name `in` (the
reserved word after `::` is already accepted as a name) and stores the unit in a
new `Method.unit: Option<UnitSpec>` field (`targs` stays empty). `::to<T>()`
keeps types only.

**Context word.** `unit` leaves `KEYWORDS`. At the start of a top-level item,
`Ident("unit")` followed by `Ident` and then `;` or `=` is a unit declaration;
everything else is an ordinary identifier, so `static Circle unit() { ... }` and
`uint32 unit = 3;` work. A struct named `unit` is possible; `unit foo() {}` is a
function returning it (the third token is `(`, not `;`/`=`).

### 2.5 Typing values and arithmetic

`Val` gains a third case: `Val::UConst(Const, &'static Unit)`, an untyped
constant with a unit (`9.6<s>`, `100<m>`). It folds exactly like `Const`
(`consts.rs` is not changed; `units.rs` pairs the Const arithmetic with unit
arithmetic) and gets its number type only when it meets one (`float32<m> a =
3<m>;` fits exactly; a bare `3<m>` becomes `uint32<m>`, `2.5<m>` becomes
`float64<m>`). A `UConst` that does not fit follows W0102 and `[behaviour]
overflow` like a plain constant (`fit_const` on the numeric part after scaling).

Notation: `a: T<ua>`, `b: U<ub>`, `k` a plain number (constant or not).

| Expression | Rule | Result |
|---|---|---|
| `a + b`, `a - b`, `a % b` | same dimension, else E0401. Evaluated through a common base (below). | `T<ua>` (number type: left leads) |
| `a < b` and the other comparisons, `==`, `!=` | same dimension, else E0401. Compared through the common base. | `bool` |
| `a * b`, `a / b` | units multiply or divide; no conversion. If every dimension cancels: Q3, a plain number with the scale folded in. | `T<ua*ub>`, `T<ua/ub>`, or plain `T` |
| `a * k`, `k * a`, `a / k` | `k` is a plain number, any expression (it is a scale). | `T<ua>` |
| `k / a` | `1/ua`. | `T<1/ua>` |
| `a ** n` | `n` a **constant whole number**, else E0404; `n` keeps its own integer type. `dim` times `n`, `scale` to the `n`. If it cancels (`n = 0`) plain `1`. | `T<ua^n>` |
| `-a`, `+a` | unchanged | `T<ua>` |
| `a += b`, `a -= b` | as `+`/`-`, evaluated in `a`'s unit | unchanged |
| `a *= k`, `a /= k` | `k` plain only; a unit `b` would change `a`'s type: E0401 | unchanged |
| `a++`, `a--` on a unit value | **Q12 (open): conservative E0408** ("write `a += 1<m>`") | |
| `& \| ^ ~ << >>` with a unit on either side | E0406 (shift counts with a unit too) | |
| `&&`, `\|\|`, `!` | bool only, unchanged | |

**Plain constants beside unit values (Q10).** `*` and `/` accept any plain number.
For `+ - %` and the comparisons: **zero is unit-free** (`speed > 0`, `d + 0`,
`d == 0` are fine and mean zero of the other side's unit); any other plain
constant needs its unit (`d + 5` is E0408 with help "write `d + 5<m>`"). For a
**point** (2.8) zero is not unit-free (0 degC is not 0 K): `t > 0` is E0408 with
help `0<degC>`. A **non-constant** plain number beside a unit in `+ - %` or a
comparison (`n + d`, `d + n`, `n < d`): Q12 (open), conservative E0408 in both
orders.

**Common-base evaluation (the owner's prefix note).** For `+ - %` and
comparisons between two unit values of one dimension, let `g` = the gcd of the
two scales as rationals (`gcd(p1/q1, p2/q2) = gcd(p1,p2) / lcm(q1,q2)`), and
`ka = sa/g`, `kb = sb/g` (whole numbers). Then

```text
a + b   =  (a * ka + b * kb) / ka        // result in a's unit, a's number type
a < b   =  a * ka < b * kb
```

with the multiplications and the final division emitted as ordinary
`hir::BinOp` nodes on constant operands, omitted when the constant is 1. Same
unit on both sides gives `ka = kb = 1`: a plain addition with no scaling code (a
requirement: zero cost for the common case, and `2<mi> + 2<mi>` as typed values
is just `a + b`). `10<km> + 100<m>` as constants folds exactly to `10.1<km>`;
as typed floats it is `(10.0*1000 + 100.0)/1000`. Integers: the arithmetic of a
mixed-scale operation happens in a 64-bit integer of the left side's
signedness (so `uint8<km> + uint8<m>` does not overflow while computing) and the
final `/ ka` truncates toward zero (Q5); the result is stored in `a`'s type with
the usual `[behaviour] overflow` check and W0102 if `ka != 1` for integers. For a
64-bit left type the work is done in that type (overflow per behaviour).

**Left leads, still.** The result's number type is decided by the existing code:
`b`'s number type converts to `a`'s (W0102 if lossy). When `a` is plain and `b`
has a unit, `a` leads in number type and `b`'s unit becomes the result's for `*`
and `/` (`2.5 * dist` is `float64<m>`; `uint32 n; n * dist` is `uint32<m>`
with `dist` converted to `uint32`, W0102, as `hp * 1.5` today).

**Unit-aware path only when needed.** `binary_vals` calls `units::binary` first
if either operand's type has a unit or either is a `UConst`; otherwise the
existing code runs untouched, so all current tests are unaffected by
construction.

**Conversion to a typed target** (initialisers, assignment, arguments, `return`,
wherever `into(value, type, origin)` is called today): the target type's unit
leads. Same dimension: `x * p / q` with `r = from.scale / to.scale = p/q` in the
target number type (`* p` omitted when 1, `/ q` omitted when 1); floats use the
exact integer constant when it fits 2^53, else `r` rounded once (note in help);
integers: `/ q` truncates toward zero, W0102 when `q != 1`, and W0102 "can
overflow" when `max(T) * p` exceeds the target range (reuse W0102, new wording
only). Different dimension: E0401 with the message adapted to the context.
Plain into unit: Q12. Unit into plain: Q1 strip (below).

**Unit into a plain slot (Q1).** Same machinery with `to` = the dimensionless
unit of scale 1: the value is multiplied by `from.scale` (a point also subtracts
its offset first, 2.8). `uint8 s = 3<m>;` is 3; `uint32 t = 3<km>;` is 3000;
`uint8 big = 3<km>;` does not fit and follows the constant rule (W0102/overflow);
`float64 f = 5.0<g>` (g = 0.001 kg) is 0.005 in the base unit kg. The same strip
applies where a plain parameter, return or field receives a unit value.

### 2.6 `::to`, `::in`, `::text`, `::unitText`

- `x::to<T>()` changes the **number type** and keeps the unit: `d::to<uint32>()`
  on a `float64<m>` is a `uint32<m>`. `T` must be a bare number type; a unit in
  `T` is E0407 with help "to change the unit write `x::in<km>()`". W0102 is
  silenced as for any `::to`. (My reading of Q11, "`::to<T>()` changes the
  number type"; the owner can overrule.)
- `x::in<U>()` changes the **unit** and keeps the number type; `U` is a unit
  expression. Same dimension required, else E0407. Scaling as in the conversion
  rule above, but W0102 is silenced (you said so). On a point, `in<K>` applies
  the offset; `in<delta degC>` on a point is E0407 (a point is not a delta).
- `x::text()` on a unit value prints the number only (Q2); `io::outf` `%` and
  interpolation follow it; `%.N` rounds the number.
- `x::unitText()` is a `string` constant of the display name (2.2): `"km"`,
  `"m/s"`, `"kg*m/s^2"`, `"degC"`, `"delta degC"`. The program decides the layout:
  `io::outln(d::text() + " " + d::unitText());`. Q14 asks about deltas.

### 2.7 Types in the compiler

Units are a third dimension of a number type. Four places:

1. **`src/units.rs`** (new, pure data, no compiler dependencies): `Unit`,
   `UnitTable`, canonical form, arithmetic, formatting, the `is_valid_unit_text`
   check the lexer rule uses, unit tests.
2. **`types::Type`** gets `Type::Unit { of: &'static Type, unit: &'static Unit }`,
   interned like `Ref`/`Ptr`. `of` is always a bare `Int` or `Float`. Helpers:
   `Type::with_unit(self, Unit)` (returns `self` unchanged when the unit is
   dimensionless, Q3), `Type::bare(self)`, `Type::unit(self) -> Option<...>`.
   `is_int`, `is_float`, `is_numeric`, `int_range`, `float_exact_bits`,
   `converts_losslessly_to` delegate to `bare()` (a unit never changes whether a
   conversion of the number is lossless; scaling is judged in
   `typeck/units.rs`). `Display` prints `float64<m/s>`, `float64<N>` (declared
   name wins), `float64<delta degC>`.
3. **`src/typeck/units.rs`** (new, like `control.rs`/`strings.rs`): resolution,
   unit-aware arithmetic, conversion, `to`/`in`/`unitText`. `typeck.rs` gets
   only hook calls (2.10).
4. **Erasure.** Typeck keeps `Type::Unit` on `hir::Expr.ty`, locals, signatures
   and fields while it checks. Just before `typeck::check` returns the
   `hir::Program`, `units::erase(&mut hir::Program)` rewrites every `Type` in it
   with `bare()`. After that no pass past typeck (ownership, MIR, backends) can
   see a unit; ownership messages print bare number types; a unit number is
   `Copy` exactly like its bare number. `lib.rs` is untouched.
   **Do not match on `Type::Unit` in `hir.rs`, `mir/`, `codegen*.rs`, `cgen*.rs`
   (WP6 is rewriting them).** Instead `units::assert_erased(&hir::Program)` (a
   debug-only walk called at the end of `typeck::check` under
   `debug_assertions`) panics if a unit survives. `erase` and `assert_erased`
   walk expressions with one helper `children_mut(&mut Expr)` in
   `typeck/units.rs` that names every `ExprKind` variant explicitly (no
   wildcard, so a new variant is a compile error); when WP6 deletes
   `Copy`/`Move`/`Temp`/`Scoped`/`Stmt::Drop`, whoever merges second deletes those
   arms.

### 2.8 Points and deltas (Q8)

A unit with a nonzero `offset` is a **point** unit; `float64<degC>` is a point.
`delta degC` is the same dimension and scale with offset 0 (2.2).

| Expression | Result |
|---|---|
| point `-` point (same dimension) | delta in the left's scale: `float64<delta degC>` |
| point `+` delta, delta `+` point, point `-` delta | point in the point's unit (the delta converts by scale) |
| delta `-` point | E0409 |
| point `+` point | E0409 ("two points cannot be added") |
| point `*` anything, `/`, `**`, unary `-` | E0409 |
| delta `*` / `/` plain, delta `+ -` delta | as any ordinary unit |
| comparisons point vs point | allowed (right converts to the left through the base, offsets included) |
| comparison point vs delta, or point vs plain `0` | E0401 / E0408 |
| `::in<K>()` on a point | the base reading with offset applied; `20.0<degC>::in<K>()` is `293.15<K>` |
| strip to plain (Q1) | the base reading: `20.0<degC>` into a plain `float64` is 293.15 |

Conversion between points of different offsets/scales (`degC` to `degF` when a
user declares one): `x_new = (x - off_from) * scale_from / scale_new + off_new`.
Integer points use the same truncating rule as other scaled conversions.

### 2.9 The SI set (Q9)

`src/prelude/si.drh`, embedded with `include_str!`, written in Drachm itself and
parsed by the same unit-declaration parser. `in si;` (an `in` import like
`in io;`, handled where typeck.rs handles `import.module == "io"`) declares it
into the program; an unknown module name stays E0201 (update its help text: `io`
and `si`). Without `in si;` nothing is predeclared and `3<m>` is E0402 with the
help "add `in si;` or declare it with `unit m;`". Content (ordinary declarations,
no generated prefixes):

```cpp
unit m;  unit s;  unit kg;  unit A;  unit K;  unit mol;  unit cd;   // base units
unit g   = 0.001 kg;
unit N   = kg * m / s^2;      unit Pa  = N / m^2;        unit J = N * m;
unit W   = J / s;             unit C   = A * s;          unit V = W / A;
unit F   = C / V;             unit ohm = V / A;          unit S = 1 / ohm;
unit Wb  = V * s;             unit T   = Wb / m^2;       unit H = Wb / A;
unit Hz  = 1 / s;             unit rad;
unit mm  = 0.001 m;   unit cm = 0.01 m;      unit km = 1000 m;
unit mg  = 0.000001 kg; unit ms = 0.001 s;   unit us = 0.000001 s;  unit ns = 0.000000001 s;
unit min = 60 s;      unit h  = 3600 s;
unit kHz = 1000 Hz;   unit MHz = 1000000 Hz; unit GHz = 1000000000 Hz;
unit kJ  = 1000 J;    unit kW  = 1000 W;     unit kN  = 1000 N;
unit degC = K offset -273.15;
```

(`us` is microseconds since `µ` is not ASCII; `rad` stays a base unit so angles
do not silently mix with plain numbers; the file is the list to extend.
Whether `kg` or `g` is the base: Q15. `degC` is in the set because the owner's
example uses it; confirm in the same question.)

### 2.10 Hooks into `typeck.rs` (keep this list short and exact)

| Where | Change |
|---|---|
| import loop at the top of `typeck::check` (`import.module == "io"`) | accept `si`: declare the prelude units |
| `Globals::resolve(&TypeName)` | if `t.unit` is `Some`, resolve the base type, then `units::resolve_unit`, return `Type::Unit`. E0402/E0405 here |
| declaration pass | collect `ast::Program.units` into `Globals.units: UnitTable` before structs and functions resolve |
| `lay_out` | use `ty.bare()` |
| `Checker::literal` (number literals) | a literal with a unit yields `Val::UConst`, or a typed `Expr` of `Type::Unit` when it has a suffix |
| `binary_vals` | first line: `if units::involved(&lv, &rv) { return self.unit_binary(op, lv, lo, rv, ro, span); }` |
| `into` / `fit_const` | top of `into`: if the source or target type has a unit, `return self.unit_into(v, to, from)` |
| methods `"to"`, `"text"`, new `"in"`, `"unitText"` | per 2.6 |
| compound assignment, `++`/`--` | through `unit_binary` with the target as left operand |
| end of `typeck::check` | `units::erase`, then (debug) `units::assert_erased` |

Everything else (`control.rs`, `strings.rs`, ownership) is unchanged. `E0101`'s
"`to` converts between number types" check uses `is_numeric()`, which includes
unit types, so it needs no edit.

### 2.11 Diagnostics (wording is a draft; the owner reviews it)

`04xx` entries go in `src/diag/codes.rs` **after the `E0332` entry and before
`E0901`**, each with an `explain` text in the style of the neighbours.

```text
error[E0401]: units do not match
  --> main.drh:6:20
   |
 5 |     float64<m> dist = 100.0<m>;
 6 |     float64<s> bad = dist + time;
   |                      ---- ^ ---- this is `float64<s>`
   |                      |
   |                      this is `float64<m>`
   |
 note: `m` is a length and `s` is a time; they can only be added when they are the same kind of unit
 help: to get a speed, divide instead: `dist / time`
```

```text
error[E0402]: unknown unit `parsec`
  --> main.drh:3:19
   |
 3 |     float64<parsec> d = 1.0<parsec>;
   |                 ^^^^^^ no unit with this name is declared
   |
 help: declare it with `unit parsec = 3.0857e16 m;`, or add `in si;` for the SI units
```

E0403 bad unit declaration (one headline, a message per case): `` `a` is defined
in terms of itself (a -> b -> a) ``, `` `m` is declared twice ``, `a unit's
factor must be a number greater than zero`, `` `u` has no unit on the right;
write `unit u;` for a new base unit ``, `` `offset` goes only after a single unit
(`unit degC = K offset -273.15;`) ``, `unit names are plain ASCII letters, digits
and _`, and the Q13 case `` `pct` measures nothing: for a ratio use a plain
number ``.

E0404 a unit can only be raised to a constant whole power (`side ** n`; note:
the unit of the result depends on the exponent; help: `side * side` or a
constant). E0405 units cannot go here (`string<m>`; the power of `m` is too
large). E0406 `<<` (and `& | ^ ~ >>`) cannot be used on a value with a unit (note:
shifting and the bit operators change a pattern of bits, not a measure; help:
`len * 2`). E0407 cannot convert `float64<m>` to `float64<s>` (also: `to` with a
unit in `T`, help `::in<km>()`; `in<delta degC>` on a point). E0408 a plain
number cannot stand for a measure (`d + 5`, Q12 cases; help `d + 5<m>`). E0409
points cannot be added, scaled or subtracted from a delta (`degC + degC`,
`2.0 * t`, `delta - point`).

```text
warning[W0102]: converting `uint32<m>` to `uint32<km>` can lose information
  --> main.drh:6:25
   |
 6 |     uint32<km> k = metres;
   |                    ^^^^^^ this is `uint32<m>`
   |
 note: 1 km is 1000 m, so a whole-number count of metres is divided by 1000 and the remainder is dropped
 help: write `metres::in<km>()` to say the conversion is intended
```

(W0102 is reused for lossy and overflowing scaling, including unit-to-plain on
integers; only the text is new.)

---

## 3. Open items (small; each has a conservative behaviour)

These came out of the owner's answers; ask them with the next interview round.
Nothing here blocks U1-U4.

**Q12. Plain numbers and units without a constant.** Q1 allows unit to plain.
Not decided: (a) plain into unit, `float64<m> d = n;` and `= 5.0;`; (b) a
non-constant plain beside a unit in `+ - %` or a comparison, `n + d`; (c)
`d++` / `d--` on a unit value. Conservative: all three are E0408, with the help
"write `n * 1<m>`" (a plain operand of `*` is a scale, so that already attaches
a unit), "write `d + 5<m>`", "write `d += 1<m>`". Zero constants are accepted
(`float64<m> d = 0;`) by the zero rule. Options for (a)/(b): A. keep the wall,
attach only with `* 1<m>`; B. the plain side takes the unit when it leads the
conversion's other side (as constants do today); C. a worded attach, e.g.
`n in m` / `n::as<m>()`. Note Q1's strip already makes `n + d` legal if "the
right converts to the left's type" is followed literally (the unit side strips)
while `d + n` stays an error; A makes both errors.

**Q13. Declared dimensionless units.** Q3 collapses dimensionless results to
plain. Is `unit pct = 0.01 1;` (a named ratio) allowed? Conservative: E0403
("measures nothing"). Options: A. never (use plain numbers); B. allowed, a
`float64<pct>` is a tagged value that strips to its scale (0.85 for `85.0<pct>`)
and whose results still collapse.

**Q14. Deltas.** (a) Is `delta degC` the same type as `K` (my reading: same
dimension and scale, so interchangeable) or a separate type that needs
`::in<K>()`? (b) What does `::unitText()` print for a delta: `delta degC`
(conservative) or `degC`? (c) Is zero unit-free beside a point (`t > 0` for a
`float64<degC>`)? Conservative: E0408, write `0<degC>`.

**Q15. The shipped set.** Is `kg` the base mass unit with `g = 0.001 kg` (SI,
as written in 2.9), or `g` the base with `kg = 1000 g` (as in your example)?
Does `in si;` include `degC`? Which other units (`l`, `ml`, `mi`, `ft`, `lb`)
belong in `si` versus a later `imperial` module?

---

## 4. Work packages

### 4.1 Sequencing, and how this avoids conflicts with WP5 and WP6

Everything below can start now. WP5 (mutability) and WP6 (drops, one backend
path) run in parallel with it. The shared files and the rule for each:

| File | Who | Rule |
|---|---|---|
| `src/lexer.rs`, `src/parser.rs`, `src/ast.rs`, `src/types.rs`, `src/consts.rs` | units only | WP5/WP6 do not touch them |
| `src/typeck.rs` | all three | Units: only the hook lines of 2.10 and the new `Val` case. WP5 deletes `report_fixed_receivers`, `check_field_change`, `check_assignable`, `SelfUse`. WP6 deletes the drop plumbing (`owned`, `temps`, `scoped`, `Copy/Move/Temp/Scoped/Drop` construction). Do not reformat or reorder anything in the file. Whoever merges second rebases; hunks are in different functions. |
| `src/typeck/units.rs` | units only | new file, the home of all unit logic |
| `src/typeck/control.rs`, `strings.rs` | WP6 | units do not touch them |
| `src/hir.rs` | WP6 (deletes legacy variants) | units add nothing; no `Type::Unit` arms (2.7 item 4) |
| `src/mir/*`, `src/ownership/*`, `src/codegen*.rs`, `src/cgen*.rs`, `runtime/` | WP5/WP6 | units never touch them; erasure guarantees they see bare types |
| `src/diag/codes.rs` | all three | WP5: `E0311`, `E0312` right after `E0310`. Units: the `E04xx` block after `E0332`. WP6: nothing. |
| `src/lib.rs` | units: one `pub mod units;` line (in the module list, alphabetical); WP6: bodies of `build`/`build_unchecked` | disjoint lines |
| `tests/run.rs` | WP6 | units add only new files under `tests/run`, `tests/fail`, `tests/units.rs` |
| `site/content/*.md`, `site/content/order.json` | all three | units: the new `reference-units.md`, `reference-types.md` (one paragraph), `coming-later.md` (units line), `order.json` (one line). WP5/WP6: `reference-ownership.md` and their own examples. |
| `site/gen-errors.sh` output | all | rerun after your `codes.rs` change; on a conflict in `errors.md`, regenerate rather than merging by hand |

If WP5/WP6 are still open when a U-branch is ready, branch from the latest
master and rebase onto their merges rather than the reverse. Units never block
step 3 and step 3 never waits for units. `units::erase` is the one place that
names every HIR expression variant; see 2.7 item 4 for how it survives WP6.

Order: U1, then U2, then U3, then U4, then U5, then U6; U1 and the docs outline
can overlap with anything. Implementer strength: U1 standard, U2 standard
(careful with the lexer), U3 standard, U4 **strong**, U5 standard, U6 standard.

### U1: unit data (`src/units.rs`)

Implementer: standard model. Files: `src/units.rs`; `src/lib.rs` (the single line
`pub mod units;`). Nothing else.

Build: `Unit` (2.2, with `offset`), `UnitTable` (interning, display names,
name -> unit map, declaration order), `mul`, `div`, `pow`, `ratio`, `delta_of`,
`is_dimensionless`, `Display`; a neutral tree `UnitExprText` and
`parse_unit_text(&str) -> Result<UnitExprText, usize>` (recursive descent over
`&str`, ASCII names only, optional leading `delta`, `1` allowed, spaces only after
`delta`) with `is_valid_unit_text`; `gcd_rational`, and `scale_pair(a, b) ->
(ka, kb)` for the common-base rule of 2.5 (whole numbers). Exact arithmetic with
`BigRational` throughout; never an `f64` for a scale.

Tests (Rust unit tests in the file): canonical equality (`N` vs `kg*m/s^2`);
`km`/`m` ratio 1000; `g = 0.001 kg` scale 1/1000; `m/m` is dimensionless;
`pow` with negatives; `1/s`; display of declared, written and product units;
interning pointer equality; exponent overflow; point/delta: `delta_of(degC)`
equals `K` by content and displays `delta degC`; `scale_pair` for (1000, 1),
(1, 1), (5280 ft vs 1 ft), (1/1000, 1), (0.3, 0.2); `parse_unit_text` on every
form of the lexer table in U2 and on `3 m`, `m*`, `µ` (rejected).

Done when: `cargo test` green; no other file changed. No site change (nothing
user-visible).

### U2: lexer, parser, AST

Implementer: standard model. Files: `src/lexer.rs`, `src/parser.rs`,
`src/ast.rs`. Starts after U1.

Build (2.3, 2.4):
- remove `"unit"` from `KEYWORDS`; item-start recognition of the context word;
- unit-literal rule in `lexer::number` after the suffix is read (use
  `units::is_valid_unit_text`); `Tok::Num` carries the unit text and span;
- `ast::UnitDecl`, `UnitDef`, `UnitExpr`, `UnitSpec`, `Program.units`,
  `TypeName.unit`, literal `unit`, `Method.unit`;
- parse `unit` items incl. `offset`, the `ty()` unit part with `expect_close_angle`,
  the `at_binding` lookahead, and `::in<unit-expr>()`.

Tests: parser unit tests for `unit m;`, `unit N = kg*m/s^2;`, `unit Hz = 1 / s;`,
`unit km = 1000 m;`, `unit degC = K offset -273.15;`, `float64<m/s> v`, `mut
float64<m> d`, `open uint8<kg> w`, `float64<delta degC>`, `d::in<km/h>()`,
`d::in<delta degC>()`, `d::to<float64>()`, `9.6<s>`, `5u8<m>`, `2.5f32<m/s>`,
`1<1/s>`, `3.0<delta degC>`. The lexer ambiguity table: `3<4`, `3 < m`, `3<m`,
`3<m>`, `3<m>>2`, `3<x && y>2`, `3<>`, `3<m*>`, `a<b>c` (not after a literal:
unchanged), `if (n<limit)`, `9<s>>3`, `1<m>>=2`. Context word: `static Circle
unit() { ... }`, `uint32 unit = 3;`, `unit foo() {}` (a function returning a
struct `unit`) all parse as before; `unit m;` declares. New golden fail test
`tests/fail/units_syntax.drh`: `string<m> s;` (E0405 is raised in typeck, so
keep this case for U3), `3<m*>` expression error, a non-ASCII unit name
`unit µm = 0.000001 m;` (E0403 lands in typeck; here only that the parser
reports the character, decide by whichever stage sees it first and say which).

Site: none (nothing compiles yet beyond parsing). Done when: all existing tests
pass with unchanged output; the new parser tests pass; `drachm check` on a file
with only unit declarations and unit-typed signatures reports no syntax error.

### U3: types and resolution

Implementer: standard model. Files: `src/types.rs`, `src/typeck/units.rs` (new),
`src/typeck.rs` (hooks: import, `resolve`, declaration pass, `lay_out`),
`src/diag/codes.rs` (`E0402`, `E0403`, `E0405`, after `E0332`),
`src/prelude/si.drh` is **not** here (U5).

Build: `Type::Unit`, `bare`, `unit`, `with_unit`, `Display`; `Globals.units`,
`declare_units` (lazy evaluation, cycle and redeclaration, dimensionless check
per Q13, offset rules, ASCII), `resolve_unit`; E0402/E0403/E0405; `units::erase`
and `assert_erased` with the explicit `children_mut` walker (2.7 item 4) so
that unit types in signatures and fields never leave typeck; struct fields and
parameters of unit type lay out as the bare type.

Tests: `tests/fail/units_decl.drh` (cycle, redeclaration, zero and negative factor,
`unit u = 5;`, `unit pct = 0.01 1;`, `offset` after a product, unknown unit,
`string<m>`, `bool<m>`); `tests/run/units_decl.drh` (declarations in odd order,
`float64<N>` and `float64<kg*m/s^2>` interchangeable, units in struct fields and
signatures, a struct containing unit fields prints values to prove layout; a
function taking and returning `float64<m>` passing a value through, with no
arithmetic yet).

Site: none yet (a program can only declare and pass units). Done when: those
pass, all earlier tests unchanged, and `assert_erased` runs on every
`tests/run` program (it is a debug assertion at the end of `typeck::check`).

### U4: literals, arithmetic, conversion

Implementer: **strong model** (typeck arithmetic with exact rationals, the
common-base rule, the conversion table, points and deltas; the semantics are
spread over many cases and must not change any existing test). Files:
`src/typeck.rs` (hooks 2.10), `src/typeck/units.rs`, `src/diag/codes.rs`
(`E0401`, `E0404`, `E0406`, `E0407`, `E0408`, `E0409`; W0102 texts),
`site/` (below).

Build 2.5, 2.6 (`to`, `in`, `text`; `unitText` lands with U5's text decisions
but is trivial, do it here), 2.8, with the conservative behaviour for Q12, Q13,
Q14 marked `// OPEN(unitsQ12)` etc. (grep targets):
`Val::UConst` and its folding; `unit_binary` (all rows of the 2.5 table, the
zero rule, the common-base evaluation with `scale_pair`, the 64-bit work type
for integers); `unit_into` (conversion to a typed target, the Q1 strip to a plain
slot, W0102/overflow texts); `**` with a constant whole exponent; compound
assignment; the collapse of dimensionless results with scale folding (Q3: for
integers multiply by the numerator scale **before** the division and apply the
denominator scale to the divisor, so `4 km / 3 m` is `(4 * 1000) / 3`, not
`(4 / 3) * 1000`); points and deltas; `::in<U>()`; `::unitText()`.

Tests (all `tests/run` cases run on both backends):
- `units_arith.drh`: the SPEC example (`dist / time` is `float64<m/s>`);
  `N = kg*m/s^2` from `F = m * a`; `**` with a constant; `1/s` and `Hz`;
  compound assignment; comparisons across scales; left-leads on number types
  (`2.5 * dist`, `uint32<m>` times `float64`); zero beside a unit
  (`speed > 0`, `d + 0`); a plain non-constant scale (`n * dist`, `dist / n`).
- `units_scale.drh`: `10<km> + 100<m>` is `10.1` km; `10<m> + 1<km>` is `1010` m;
  `2<mi> + 2<mi>` (user-declared `ft`/`mi`) is 4 mi with no scaling code in the
  generated C (see the Rust test); typed float and integer scaling;
  `uint8<km> s = n` with `n` = 300 m is 0 (and a `.warn` line for W0102);
  `::in<km>()` and `::in<m>()`; scaling through a function argument and
  `return`; unit to plain strip (`uint8 s = 3<m>` is 3, `uint32 t = 3<km>` is
  3000, `float64 f = 5.0<g>` is 0.005); dimensionless collapse
  (`4.0<km> / 2.0<m>` is 2000.0, `4<km> / 3<m>` as uint32 is 1333);
  `::to<float32>()` keeps the unit; `::text()` prints the number; `::unitText()`
  prints `km`, `m/s`, `kg*m/s^2`.
- `units_points.drh`: `unit K; unit degC = K offset -273.15;`; `b - a` is a
  delta; point + delta; `20.0<degC>::in<K>()` is 293.15; point comparison;
  strip of a point.
- `units_erase.drh` and a Rust test `tests/units.rs`: a loop summing
  `float64<m>` prints the same as the same program without units, and
  `drachm::build` output (`-O` C text via the library) of a unit-free-arithmetic
  program with units on its types is **byte for byte equal** to the same program
  with bare types (no scaling code).
- `tests/fail/units_mismatch.drh`: E0401 for `dist + time`, assignment
  `float64<m> d = time;`, argument and return mismatches, `<` across kinds.
- `tests/fail/units_ops.drh`: E0404 (`side ** n`), E0406 (`len << 2`, `len & 1`),
  E0407 (`d::in<s>()`, `d::to<float64<km>>()`), E0408 (`d + 5`), E0409
  (`degC + degC`, `2.0 * t`).
- `tests/fail/units_open.drh`: the conservative E0408s for Q12 (`float64<m> d =
  n;`, `n + d`, `d++`), the Q13 E0403, the Q14 `t > 0` E0408, so answering each
  is a visible test change.

Site (same commit): new `site/content/reference-units.md` (declaring units,
`T<unit>` types, `9.6<s>` literals and the `<` rule, the arithmetic table, scaled
conversion and the common-base rule, `::to` vs `::in`, `::text` vs `::unitText`,
points and deltas, strip to plain, diagnostics E0401-E0409, each with a short
example), added to `site/content/order.json` under Reference after
`reference-types`; `site/examples/units.drh` + `.out` (real program, no compiler
output, so no warnings; units declared by hand because `in si;` is U5), and a
second `units-points.drh` + `.out`; embed them with the include mechanism the
other pages use; one paragraph and a link in `reference-types.md`; rerun
`site/gen-errors.sh`. Document only what compiles. Leave `coming-later.md` for U5
(units are not complete without `in si;`), but if the "Maths" line mentions units
now, change it to say what still lacks.

Done when: the above pass; `cargo test` green including `optimised_output_matches`
for every new run case; `drachm check` on `examples/stupid.drh` reports E0402
for `m` (no `in si;` yet); every `OPEN(units` marker is one of Q12, Q13, Q14.

### U5: `in si;`, the scratch program

Implementer: standard model. Files: `src/prelude/si.drh` (2.9, embedded with
`include_str!`), the import hook in `typeck::check`, E0201's help text,
`tests/run/units_si.drh`, `tests/run/units_scratch.drh` (the owner's program
with `in si;` and `::text()`; prints `3`; do **not** change `examples/stupid.drh`),
`site/`.

Build: `in si;` parses with the existing import code, loads the prelude through
the same unit-declaration path as user units, and a user unit that redeclares an
imported name is E0403. Q15 is open: ship the 2.9 content (`kg` base) and
`// OPEN(unitsQ15)` the one line at the top of `si.drh`.

Tests: `units_si.drh` uses several SI units (`N` from `kg * m / s^2`, `J`, `Hz`,
`km/h` via `min`/`h` conversions, `ms` to `s`, `degC` to `K`); a fail test
`tests/fail/units_no_si.drh` for `3<m>` without `in si;` (E0402 with the help
text); another for a user `unit m;` after `in si;` (E0403).

Site (same commit): `reference-units.md` gets "The SI set" (what `in si;` brings,
that prefixes are ordinary declarations you can write yourself, the list) and
the `in si;` line in `site/content/install.md`/`index.md` examples only if they
list imports; `site/examples/units-si.drh` + `.out`; remove "units of measure"
from the "Maths" line of `site/content/coming-later.md` (keep vectors and
matrices); rerun `site/gen-errors.sh` if codes changed.

Done when: `cargo test` green; the only remaining `OPEN(units` markers are Q12,
Q13, Q14, Q15; `examples/stupid.drh` still reports what it reports (E0402 `m`
without `in si;`; with `in si;` its `::to<string>` stays E0101, a strings-area
matter).

### U6: answers, SPEC and docs sync

Implementer: standard model. Runs after the owner answers Q12-Q15 (and may be
split per question). For each answered question: replace its conservative
behaviour and its `OPEN(unitsQn)` markers (list the resolved markers in the
commit message), update or add the fail/run tests named above, and update
`reference-units.md`. Then, once and in the same series:
`docs/SPEC.md` 4b, section 1 (`unit` is a context word), section 10 (add `04xx`),
`docs/PLAN.md` step 4.5 status (done) and its "Open" paragraph (remove), this
file (retire it or reduce it to a pointer; keep the decisions table), and the
living design doc (Claude Docs, link in `CLAUDE.md`).

Done when: no `OPEN(units` marker remains; `cargo test` green (it runs the site
examples); `site/gen-errors.sh` produces no diff beyond the new codes; the site
lists only what the compiler accepts.

### Pitfalls (all packages)

- Do not give `Type::Unit` a unit-aware meaning in `Type::converts_losslessly_to`;
  it is about the number only. All scaling judgement lives in `typeck/units.rs`.
- Erase **after** all diagnostics are produced and before the ownership pass; do
  not erase per expression while checking.
- The lexer rule needs no symbol table. Do not look at declared units from the
  lexer or parser.
- `^` in a unit expression is the existing XOR symbol; never change how `^`,
  `*` or `/` parse elsewhere. `>>` splitting must not change `x >> 2` in
  expressions.
- Exact arithmetic: `BigRational` for every scale and offset; emit an `f64`
  constant only at the very end, once.
- `UConst` folding must not change the result type of existing plain-constant
  expressions: only expressions containing a unit literal take the new path.
- C text and Cranelift output of a program must not change because a type has a
  unit: the only emitted differences are the scaling multiplications and divisions
  the rules above require.
- `in` is a reserved word; after `::` the parser already accepts it as a method
  name. Keep that and parse the unit argument there, nowhere else.
- Do not touch `examples/stupid.drh`.
