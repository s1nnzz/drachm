# Step 4.5: units of measure (implementation spec)

Status: **design drafted, blocked on owner answers** (section 3). Work package
U1 (pure data) can start now; U2 onward starts after step 3's WP2 merges (section
5). Read first: `docs/SPEC.md` section 4 (Numbers) and 4b (Units of measure),
`docs/PLAN.md` "Step 4.5", `CLAUDE.md` ("Working with the owner", "Keeping the
docs site current"), `src/types.rs`, `src/consts.rs`, `src/typeck.rs`
(`Val`, `binary_vals`, `into`, `fit_const`, the `"text"`/`"to"` methods),
`src/lexer.rs` (`number`, `KEYWORDS`), `docs/impl/wp0a.md` (brief format).

Rules for the implementer:
- **The owner makes the language decisions.** Section 3 lists what is open.
  Until a question is answered, implement the *conservative behaviour* named
  there (an error or `E0901 not supported yet`, never a guess), and mark the
  place with `// OPEN(unitsQn)`. Do not choose among the options yourself.
- Units are compile-time only: zero runtime cost, nothing unit-shaped reaches
  HIR consumers (MIR, codegen, cgen). Both backends must behave identically;
  `cargo test` runs every `tests/run` case both ways.
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
> - The stdlib ships SI units (m, s, kg, N, J, W, …) ready to use.

SPEC 1 (Lexical): `unit` is in the keyword list. SPEC 4 rules that units must
cooperate with:

> - Mixed types: the left side leads. In `a + b`, `a < b` and every other
>   two-sided operator, the right side converts to the left side's type (W0102
>   if lossy) and the operation is done in that type. [...] Except counts: the
>   right side of `<<`, `>>` and integer `**` keeps its own integer type.
> - An untyped constant on the right of a typed value takes that value's type
>   (`total + 3`). On the left it takes the right side's type if it fits there
>   exactly (`3 + total`); if not, it gets its own default type and leads.
> - All numeric types convert automatically. Lossy conversions print a big
>   warning (W0102); lossless ones are silent. Explicit conversion silences it:
>   `(damage * 1.5)::to<uint8>()`.
> - `::text()` on a number or bool gives its text. [...]

SPEC 10 (Diagnostics): `E` and `W` share one number space; `00xx` syntax, `01xx`
types and conversions, `02xx` names and bindings, `03xx` ownership, `09xx` not
supported yet. **This step adds `04xx` for units**; update that sentence in
SPEC section 10 and the design doc when U6 lands.

Nothing else is decided. Everything in section 2 beyond the quotes above is a
proposal built on them; section 3 holds the choices that are the owner's.

### The scratch file

`examples/stupid.drh` (untracked, the owner's) is:

```cpp
in io;
none main() {
    uint8 s = 3<m>;
    io::outln(s::to<string>());
}
```

It asks four things at once: may a unit literal go into a plain `uint8`
(Q1), may a `uint8` carry a unit at all (Q4), what does `to<string>` mean on a
number (today `to` is numbers only: E0101 "`to` converts between number
types, not to `string`"; this is a separate question for the strings area, not
units, but Q2 depends on it), and what text comes out (Q2). `m` is also not
declared there, so with Q9 unanswered it is E0402 today.

---

## 2. The design

### 2.1 Where units live in the compiler

Units are a third dimension of a number type. Four places:

1. **`src/units.rs` (new, pure data, no compiler dependencies).** `Dim`, `Unit`,
   `UnitTable`, canonical form, arithmetic on units, formatting, unit tests.
2. **`types::Type`** gets one variant, `Type::Unit { of: &'static Type, unit:
   &'static Unit }`, interned with the same `intern_type` trick as `Ref`/`Ptr`.
   `of` is always a bare `Int` or `Float`. Helpers: `Type::with_unit(self, Unit)`
   (returns `self` unchanged if the unit is dimensionless-and-scale-1, see Q3),
   `Type::bare(self) -> Type` (strips the unit; identity on other types),
   `Type::unit(self) -> Option<&'static Unit>`. `is_int`, `is_float`,
   `is_numeric`, `int_range`, `float_exact_bits` and `converts_losslessly_to`
   delegate to `bare()` (a unit never changes whether a conversion of the
   underlying number is lossless; scaling is judged separately, 2.7).
   `Display` prints `float64<m/s>` (2.9). Exhaustive matches on `Type`
   (`hir::scalar_size`, `codegen::cl_type`, `cgen::c_type`) get an arm that
   delegates to `bare()`, as a safety net only (see erasure below).
3. **`src/typeck/units.rs` (new, like `control.rs` and `strings.rs`)** holds all
   checking logic: resolving unit expressions, the unit-aware arithmetic,
   coercion, `to`. `typeck.rs` gets only hook calls (2.8), so that the file
   step 3 is rewriting is touched in a handful of small places.
4. **Erasure.** Typeck keeps `Type::Unit` on `hir::Expr.ty`, `Local.ty`,
   function signatures and struct fields while it checks, because the checks
   need it. Just before `typeck::check` returns the `hir::Program`, a single
   pass `units::erase(&mut hir::Program)` rewrites every `Type` in it with
   `bare()` (locals, expression types, parameter and return types, field types,
   struct layouts are computed from bare types in the first place). After
   that, no pass past typeck (MIR, ownership, codegen, cgen) can see a unit.
   `lib.rs` is untouched. A debug assertion in `codegen::cl_type` /
   `cgen::c_type` (`Type::Unit` is `unreachable!`) keeps this honest in tests.
   Ownership error messages (step 3) therefore print bare number types; units
   never affect moves, copies or borrows (a unit number is `Copy` exactly like
   its bare number).

### 2.2 Unit representation

A unit is its **canonical form**: a base-unit exponent vector plus an exact
rational scale factor.

```rust
/// Interned, leaked: `&'static Unit` is Copy, Eq and Hash by content.
pub struct Unit {
    /// Sorted by base name, no zero exponents. `m/s^2` is [("m",1),("s",-2)].
    pub dim: Vec<(&'static str, i32)>,
    /// How many of the base units (in `dim`) one of this unit is.
    /// `km` is 1000, `m` is 1, `g` (if `unit g = 0.001 kg;`) is 1/1000.
    /// `BigRational` from `num_rational` (already a dependency, used by consts.rs).
    pub scale: BigRational,
}
```

- Two units are the same *type* exactly when `dim` and `scale` are equal, so
  `unit N = kg * m / s^2;` and the written expression `kg*m/s^2` are the same
  type, and `float64<N>` equals `float64<kg*m/s^2>`.
- Base units are the ones declared `unit m;`. They are identified by name
  (`&'static str` via `types::intern`); `scale` of a base unit is 1.
- **Display name.** A side table in `UnitTable` maps a canonical `Unit` to the
  name used in messages: the first `unit NAME ...;` that declared it, else (for
  an expression) the expression as written, normalised (`m/s`, `kg*m/s^2`), the
  first time that canonical unit was formed. Not part of identity. A canonical
  unit with no declared name and no written form (a product made by `*`) is
  formatted from its operands' display names (`km*m`) at creation and stored.
- Arithmetic on units (all exact, `BigRational`): `mul` adds exponents and
  multiplies scales; `div` subtracts and divides; `pow(n: i32)` multiplies
  exponents by `n` and raises the scale to `n`; `ratio(from, to)` is `from.scale
  / to.scale`, defined only when `dim` is equal. `is_dimensionless()` is `dim
  .is_empty()`. Exponents are `i32`; overflow past `i16` range is E0405 (absurd).
- A scale of zero or less is impossible by construction (E0403).

### 2.3 Declaring units

Grammar (top-level item, anywhere in the file; order of declaration does not
matter, cycles are an error):

```text
unit-item  := "unit" NAME ";"                      base unit
            | "unit" NAME "=" unit-def ";"         derived / scaled
unit-def   := [NUMBER] unit-expr | NUMBER "/" unit-expr
unit-expr  := unit-term { ("*" | "/") unit-term }
unit-term  := NAME [ "^" ["-"] INTEGER ] | "(" unit-expr ")" | "1"
```

- `NUMBER` is a decimal or whole literal, read exactly (`0.001`, `1609.344`,
  `3600`); its value times the scales of the names in `unit-expr` is the new
  unit's scale. It must be positive and non-zero (E0403). `1000 m` is "number
  juxtaposed with a unit expression" and means multiply; the number is allowed
  only at the very front of `unit-def`. `1 / s` is therefore `NUMBER "/"
  unit-expr` (this is how Hz is written); inside `unit-expr` the only number is
  the dimension-less `1`.
- `^` takes an integer exponent (optionally negative) and binds tighter than `*`
  and `/`, which are left-associative. `^` is lexed as the existing bitwise-xor
  symbol; the parser only treats it as power inside a unit expression (SPEC 4:
  "`^` is XOR, never power, outside unit expressions"). No new lexer symbol.
- Resolution (`typeck/units.rs::declare_units`, run before struct layout and
  function signatures, because those mention units): collect all `unit` items
  into a map `name -> ast decl`; evaluate lazily with a visiting set so that
  `unit N = kg*m/s^2;` may appear before `unit kg;`. A cycle (`unit a = 2 b;
  unit b = 2 a;`) is E0403. A derived unit's `dim` is the sum of the dims of the
  names it uses; a base unit adds itself.
- A unit name lives in **its own namespace**: `uint8 s = 3<m>;` has a variable
  `s` and a unit `s` with no conflict, and a unit named like a type or function
  does not conflict either, because unit names are only looked up inside
  `<...>` after a number type or number literal and after `unit`. Redeclaring a
  unit is E0403.
- **Visibility.** Before modules (step 7) every unit declared in the file is
  global to the program, like structs and functions are today. Where the SI set
  comes from is Q9.

AST: `ast::Item::Unit(UnitDecl { name, name_span, def: Option<UnitDef>, span })`,
`UnitDef { factor: Option<(String /*literal text*/, Span)>, expr: UnitExpr }`,
`enum UnitExpr { Name(String, Span), One(Span), Mul(Box, Box), Div(Box, Box),
Pow(Box, i32, Span) }` (spans on the compound forms too, for messages).
`ast::TypeName` gets `unit: Option<UnitExpr>`; `ast::ExprKind::Num` (or the
literal node, check `ast.rs`) gets the same `unit` field from the lexer (2.4).
When step 6 generalises `TypeName` to generic arguments, `unit` becomes one of
the argument kinds; keep the field private to the parser/`units.rs` API so that
change is local.

### 2.4 Literals and the `<` ambiguity

`9.6<s>` must not be confused with `9.6 < s`. The lexer decides, with a fixed,
purely lexical rule (no knowledge of which names are units):

> **Unit-literal rule.** Directly after a number literal (digits, radix forms,
> exponent, and any integer/float suffix, so `5u8<m>`, `2.5f32<m/s>`), if the
> very next character is `<` with **no whitespace between**, then scan forward
> over the characters `A-Z a-z 0-9 _ / * ^ - ( )` (no whitespace) until the first
> `>`. If a `>` is found and the scanned text is non-empty and parses as a
> `unit-expr` with `1` allowed (grammar 2.3, ignoring whether the names are
> declared), the number token carries that unit and `<...>` is consumed. In every
> other case, the `<` is an ordinary less-than.

Consequences, all deliberate:

- `9.6 < s`, `9.6 <s`, `if (n < limit)` and `3 < 4` have whitespace or a
  non-literal on the left: less-than, as today.
- `3<m>` is a unit literal even if the programmer meant `3 < m > ...`; the second
  `>` would be a type error anyway (`bool > number`).
- `3<x && y>2` has a space/`&` in the scan: the scan stops at the first non-unit
  character, finds no `>`, so it is less-than.
- `x<m>`, `f()<m>`, `(2+3)<m>`: only a *literal* takes a unit tag. A tag on a
  non-literal is not part of this rule; how to attach one is Q1.
- The scan stops at the first `>`: `9<s>>3` is the literal `9<s>` followed by
  `> 3`. `1<m>>=2` is the literal `1<m>`, then `>=`, then `2`. (The parser
  unit tests in U2 pin these cases.)
- A scanned text that is not a valid unit-expr (`3<->`, `3<>`, `3<m*>`) falls
  back to less-than, and the parser then reports its normal expression error.
  An unknown unit name is *not* a lexical problem: it is E0402 in typeck.
- `Tok::Num(NumLit)` gets `unit: Option<(UnitExpr-source-string, Span)>`; the
  lexer stores the text and span, the parser builds the `UnitExpr` with the same
  unit-expr parser used for types (shared function in `parser.rs`, fed from the
  stored text via a tiny sub-lexer, or the lexer produces
  `Tok::Num` followed by a `Tok::UnitSuffix(Vec<Tok>)`; the implementer picks
  whichever touches fewer lines. Spans must point at the unit text.)

**Types.** In a type position the parser knows it is reading a type, so `<` is
unambiguous. `ty()` (parser.rs) after the type name: if the name is a number type
(`types::Type::from_name(name)` is `Ok` and numeric) and the next token is `<`,
parse `unit-expr` then expect `>`. Details:

- `>>` is one token. `to<float64<km>>()` ends in `>>`, so `expect(">")` inside
  type/type-argument parsing must split a `>>` into two `>` (and `>>=`
  cannot occur there). Implement as `Parser::expect_close_angle` used by
  `ty()`'s unit part and the method-call `targs` loop (parser.rs ~881).
- Units on other types (`string<m>`, `bool<m>`, a struct, `none<m>`) are E0405.
  Generic arguments on non-number types stay E0901 as now.
- `float64<m/s>` `float64<m / s>` (spaces allowed inside a type's brackets)
  `float64<1/s>` `float64<kg*m/s^2>` `uint8<m^2>` all parse.

**Statement start.** `float64<m/s> v = 3.0<m/s>;` starts with `Ident <`, which
`at_binding` (parser.rs:394) currently treats as an expression. Rule: at a
statement start, `Ident` followed by `<` is a declaration when a lookahead over
tokens that can appear in a unit expression (idents, `1`, `* / ^ - ( )`
integers) reaches a `>` that is directly followed by an `Ident` (the variable
name). Everything else stays an expression statement. (`a < b > c;` is not a
valid expression statement anyway.) Same lookahead after `mut` / `open`.

### 2.5 Typing values

`Val` gains a third case: `Val::UConst(Const, &'static Unit)`, an *untyped
constant with a unit* (`9.6<s>`, `100<m>`). It folds exactly like `Const`
(`consts.rs` is not changed; `units.rs` pairs the Const arithmetic with unit
arithmetic) and, like a plain constant, gets its number type only when it meets
one: `float32<m> a = 3<m>;` fits exactly, a bare `3<m>` with nothing around it
becomes `uint32<m>`, `2.5<m>` becomes `float64<m>`. That keeps the SPEC rule for
constants (`total + 3`, `3 + total`) working for unit constants as well:

- `10<km> + 100<m>`: both are `UConst`, so the whole thing folds exactly: `100 m`
  converts to `1/10 km`, the sum is the exact decimal 10.1, typed `float64<km>`
  (this is the SPEC example). `10<m> + 1<km>` is exactly 1010, a whole number,
  so `uint32<m>`. Rounding only arises for **typed** operands (Q5).
- `dist + 1<m>` with `dist: float64<m>`: the constant adopts `float64<m>`.
- A `UConst` that does not fit the number type it goes into follows W0102 and
  `[behaviour] overflow` exactly like a plain constant (`fit_const` is reused on
  the numeric part after scaling to the target unit).

For a typed value, `hir::Expr.ty` is `Type::Unit`; the HIR node shapes are
unchanged (a unit number is represented by the same `Int`/`Float` nodes).

### 2.6 Arithmetic rules

Notation: `a: T<u>` is a number of type `T` and unit `u`; `k` a plain (unitless)
number; "dim(u)" is `u.dim`. "Convert to the left" means 2.7.

| Expression | Rule | Result |
|---|---|---|
| `a + b`, `a - b`, `a % b` | `dim(ua) == dim(ub)` else E0401. `b` converts to `ua` (2.7). | `T<ua>`; numeric type by the existing left-leads rule |
| `a < b`, `<=`, `>`, `>=`, `==`, `!=` | same dimension else E0401; `b` converts to `ua` | `bool` |
| `a * b` | units multiply: `dim` adds, `scale` multiplies. No conversion of either side. | `T<ua*ub>` (numeric: left leads) |
| `a / b` | units divide. | `T<ua/ub>` |
| `a * k`, `k * a`, `a / k` | `k` is dimensionless: unit unchanged. | `T<ua>` |
| `k / a` | `1/ua`. | `T<1/ua>` (`float64<1/s>`) |
| `a ** n` | `n` must be a **constant whole number** (E0404 otherwise); `n` keeps its own integer type (SPEC "counts"). `dim` multiplied by `n`, `scale` to the `n`. Negative `n` only where bare `**` accepts a negative whole exponent (float base). | `T<ua^n>` |
| `-a`, `+a` | unchanged. | `T<ua>` |
| `a += b`, `a -= b`, `a++`, `a--` | as `+`/`-`: `b` converts to the **target variable's** unit. `++` adds a plain 1 only if Q10 allows; otherwise E0401. | unchanged type |
| `a *= k`, `a /= k` | `k` plain only; `a *= b` with a unit `b` would change `a`'s type: E0401 ("the result would be `T<..>`, not `T<ua>`"). | unchanged type |
| `&`, `\|`, `^`, `~`, `<<`, `>>` with a unit on either side | not meaningful on measures: E0406. (A plain `k << 2` is unaffected. Shift *counts* with units are E0406 too.) | |
| `&&`, `\|\|`, `!` | bool only, unchanged. | |

- **Left side leads, still.** In `a op b` the numeric type of the result is
  decided by the existing code: `b`'s number type converts to `a`'s (W0102 if
  lossy) and the operation happens in `a`'s number type. When `a` is a plain
  number and `b` has a unit, `a` leads in number type and `b`'s unit becomes the
  result's (`2.5 * dist` is `float64<m>`; `uint32 n; n * dist` is `uint32<m>`
  with `dist` converted to `uint32`, W0102, same as `hp * 1.5` today).
- **Unit-aware path only when needed.** `binary_vals` calls
  `units::binary(...)` first if either operand's type has a unit (or either is a
  `UConst`); otherwise the existing code runs untouched, so all current tests
  are unaffected by construction.
- Constants beside unit values (`speed > 0`, `d + 5`, `d * 2`): `*`, `/` and `**`
  accept plain constants as dimensionless scalars; for `+ - % <` etc. it is Q10.
- Math functions (`math::abs`, `sqrt`, `min`, `max`, ...) do not exist yet
  (stdlib is step 7); when they do, they need a way to be generic over units
  (Q6). Nothing in this step provides them.
- **Dimensionless results** (`m/m`, `km/m`): Q3. Conservative behaviour for now:
  the result keeps its tag as `T<1>` for scale 1 and is **accepted wherever a
  plain number is** (it is a plain number); a dimensionless result whose scale
  is not 1 (`km/m`) is E0901 "dimensionless results with a scale are not
  decided yet".

### 2.7 Converting between scaled units

"Scaled units of the same kind convert automatically, to the unit of the first
operand" (SPEC). Implementation, for a value `b: Tb<ub>` converting to `Ta<ua>`
with `dim(ua) == dim(ub)` (otherwise E0401, or E0407 inside `::to`/`::in`):

1. `r = ub.scale / ua.scale`, an exact rational `p/q` in lowest terms.
2. The number type converts first, by the existing rules (`Convert`, W0102 when
   lossy) — **left leads**: the scaling happens *in `a`'s number type*.
3. Scale: if `r == 1`, nothing is emitted (same unit, or two names for one
   unit). Else emit, in `a`'s number type, `(x * p) / q` using the existing
   `hir::BinOp::Mul` / `Div` nodes with constant operands (omit `* p` when
   `p == 1`, omit `/ q` when `q == 1`). For floats the constant `p`/`q` is the
   exactly representable integer if it fits 2^53, otherwise `r` rounded once to
   the type (note in the help text). Division by `q` rather than multiplying by
   the rounded reciprocal keeps `100<m>` -> `0.1<km>` exact to the float.
4. Integers: `* p` overflows per `[behaviour] overflow` like any `*`.
   `/ q` truncates toward zero: the loss is Q5. A typed conversion with `q != 1`
   on integers is the "lossy" case; a conversion with `q == 1` (km -> m) is
   silent unless it can overflow the type's range (judge with `int_range`: if
   `max(Ta) * p` exceeds the range, W0102 "converting `uint8<km>` to
   `uint8<m>` can overflow"; reuse W0102, new wording only).
5. Conversion happens at: both operands of `+ - % comparisons`, `=`/`+=` into a
   typed target, initialisers, function arguments, `return` values — i.e.
   wherever `into(value, type, origin)` is called today, with the *target
   type's* unit as `ua` (the declared type leads). A value of a different
   dimension is E0401 in all those places with the message adapted to the
   context ("`speed` is `float64<m/s>` but `float64<m>` is needed here").

`::to<T>()`: when `T` carries a unit, `x::to<float64<km>>()` converts the number
type and the unit in one step (same machinery as above; W0102 silenced as for
any `::to`). When `T` is a bare number and `x` has a unit, or the reverse:
Q1/Q11. Different dimensions are E0407.

### 2.8 Hooks into `typeck.rs` (keep this list short and exact)

| Where | Change |
|---|---|
| `Globals::resolve(&TypeName)` | if `t.unit` is `Some`, resolve the base type, then `units::resolve_unit` and return `Type::Unit`. E0402/E0405 reported there. |
| item collection (`Globals::new` / the declaration pass) | collect `Item::Unit` into `Globals.units: UnitTable` before structs/functions are resolved. |
| `lay_out`, `hir::scalar_size` | use `ty.bare()`. |
| `Checker::literal` (the number-literal case) | a literal with `unit` yields `Val::UConst` or, with a suffix, a typed `Expr` of `Type::Unit`. |
| `binary_vals` (typeck.rs:3088) | first line: `if units::involved(&lv,&rv) { return self.unit_binary(op, lv, lo, rv, ro, span); }`. |
| `into` / `fit_const` (typeck.rs:2184, 2276) | at the top of `into`: if source or target type has a unit, `return self.unit_into(v, to, from)`. |
| methods `"to"` and `"text"` (typeck.rs ~2872-2900) | `to`: accept unit-carrying `T` (2.7); `text`: Q2. |
| compound assignment / `++` / `--` | route through `unit_binary` with the target as the left operand. |
| end of `typeck::check` | `units::erase(&mut program)`. |

Everything else (`control.rs`, `strings.rs`, ownership) is unchanged. `E0101`'s
"`to` converts between number types" check uses `to.is_numeric()`, which now
includes unit types, so it needs no edit.

### 2.9 Text, names in messages

- Type display: `float64<m/s>`, `uint8<kg*m/s^2>`, `float64<1/s>` (the
  declared name wins when one exists: `float64<N>`). Exponents print as `m^2`,
  `s^-1` only when no quotient reads better (`1/s`).
- **`::text()`**: Q2. Conservative: `x::text()` on a unit value is E0901 "text of
  a value with a unit is not decided yet" (use `x::to<float64>()::text()` once Q1
  allows it, or the whole thing is blocked until answered).
- `io::outf("%.2")` and interpolation (`{d}`) follow whatever `::text()` does;
  `%.N` rounds the number part. Nothing separate to decide.

### 2.10 The SI set

A file `src/prelude/si.drh` (embedded with `include_str!`) written in Drachm
itself, parsed by the same unit-declaration parser. Content:

```cpp
unit m;  unit s;  unit kg;  unit A;  unit K;  unit mol;  unit cd;   // the seven base units
unit g   = 0.001 kg;
unit N   = kg * m / s^2;      unit Pa  = N / m^2;        unit J = N * m;
unit W   = J / s;             unit C   = A * s;          unit V = W / A;
unit F   = C / V;             unit ohm = V / A;          unit S = 1 / ohm;
unit Wb  = V * s;             unit T   = Wb / m^2;       unit H = Wb / A;
unit Hz  = 1 / s;             unit rad;                  // rad stays a base unit
unit mm  = 0.001 m;   unit cm = 0.01 m;      unit km = 1000 m;
unit mg  = 0.000001 kg; unit ms = 0.001 s;   unit us = 0.000001 s;  unit ns = 0.000000001 s;
unit min = 60 s;      unit h  = 3600 s;
unit kHz = 1000 Hz;   unit MHz = 1000000 Hz; unit GHz = 1000000000 Hz;
unit kJ  = 1000 J;    unit kW  = 1000 W;     unit kN  = 1000 N;
```

(Metric prefixes beyond these are not generated; adding more is a one-line
change. `rad` as a base unit means angles do not silently mix with plain numbers;
that is Q3-adjacent and can be revisited with it. `min` as a unit name is fine:
unit names are their own namespace. `us` stands for microseconds, since `µ` is
not an identifier character.) Where this file is visible from is Q9; the
conservative behaviour for now is that **nothing is predeclared**: the owner's
scratch file gets E0402 until Q9 is answered, and all tests declare their own
units. The prelude file is added in U5 only after Q9.

### 2.11 New diagnostics (wording is a draft; the owner reviews it)

Numbers in `04xx`, entries in `src/diag/codes.rs` after the `E03xx` block and
before `E0901`; each with an `explain` text in the style of the existing ones.

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
 help: declare it with `unit parsec = 3.0857e16 m;` (the standard library's SI units are not in scope here)
```

```text
error[E0403]: bad unit declaration
 | unit a = 2 b;  unit b = 2 a;     -> `a` is defined in terms of itself (a -> b -> a)
 | unit m;  unit m;                  -> `m` is declared twice
 | unit z = 0 m;  unit n = -2 m;     -> a unit's scale must be a number greater than zero
 | unit u = 5;                       -> `u` has no unit on the right; write `unit u;` for a new base unit
```

```text
error[E0404]: a unit can only be raised to a constant whole power
  --> main.drh:8:16
   |
 8 |     float64<m^2> a = side ** n;
   |                              ^ `n` is not a constant
   |
 note: the unit of the result depends on the exponent, so it must be known while compiling
 help: write the power out (`side * side`) or use a constant: `side ** 2`
```

```text
error[E0405]: units cannot go here
 | string<m> s;            -> only number types can have a unit; `string` is text
 | float64<m^70000> x;     -> the power of `m` is too large
```

```text
error[E0406]: `<<` cannot be used on a value with a unit
  --> main.drh:4:15
   |
 4 |     uint32<m> a = len << 2;
   |                   ^^^ this is `uint32<m>`
   |
 note: shifting and the bit operators change a pattern of bits, not a measure
 help: for a doubled length write `len * 2`
```
(Also covers `& | ^ ~` with a unit operand.)

```text
error[E0407]: cannot convert `float64<m>` to `float64<s>`
  --> main.drh:5:23
   |
 5 |     float64<s> t = d::to<float64<s>>();
   |                    ^ this is `float64<m>`, a length; `s` is a time
   |
 note: `to` can change the unit only between units of the same kind (m, km, mm, ...)
```

```text
warning[W0102]: converting `uint32<m>` to `uint32<km>` can lose information
  --> main.drh:6:25
   |
 6 |     uint32<km> k = metres;
   |                    ^^^^^^ this is `uint32<m>`
   |
 note: 1 km is 1000 m, so a whole-number count of metres is divided by 1000 and the remainder is dropped
 help: write `metres::to<uint32<km>>()` to say the conversion is intended
```
(W0102 is reused for lossy and overflowing scaling so the existing
`::to` silencing works; only the text is new.)

Conservative-behaviour diagnostics for undecided questions use `E0901` with the
quoted messages in section 3 ("not decided yet"); they are removed question by
question as the owner answers. Unit/plain mixing is E0408 (draft):

```text
error[E0408]: a plain number cannot stand for a measure
  --> main.drh:4:19
   |
 4 |     uint8 s = 3<m>;
   |               ^^^^ this has the unit `m`, `uint8` has none
   |
 help: write `uint8<m> s = 3<m>;` to keep the unit, or (when Q1 allows it) `3<m>::to<uint8>()`
```
(and the reverse: "`float64<m>` is needed here, found a plain `float64`").
Shapes that depend on Q1 are marked in code with `// OPEN(unitsQ1)`.

---

## 3. Open language questions for the owner

Each question: why it matters, 2-4 options, a Drachm sketch per option. Nothing
here is recommended; some options are deliberately unlike any other language.
"Until answered" is what the compiler does meanwhile.

### Q1. Unit values and plain numbers: `uint8 s = 3<m>;`

Why: the owner's scratch file assigns a unit literal to a plain `uint8` and then
prints it. Whether the unit wall is strict decides every interaction with code
that does not know about units (`io::outln`, `math`, loop counters, array
indexes) and how a unit is attached to a runtime number (`x<m>` is not
literal syntax, 2.4). Also covers the reverse (plain into unit).
Until answered: both directions are E0408, except a bare constant is not
allowed to pick up a unit either.

- **A. Strict wall, explicit doors.** Never implicit. Strip with a method that
  names what happens, attach by multiplying a unit literal.
  ```cpp
  uint8 s = 3<m>;                   // E0408
  uint8 s = 3<m>::bare();           // 3, "the number as written in its own unit"
  float64<m> d = n * 1<m>;          // attach: plain n times one metre
  ```
- **B. Lossy-style: a warning, `::to` silences it.** Same shape as W0102.
  Units dropped or added automatically with a new warning W0410 "dropping the
  unit `m`" / "treating a plain number as `m`"; `::to<uint8>()` says "I mean
  it".
  ```cpp
  uint8 s = 3<m>;                   // warning[W0410], s is 3
  uint8 t = 3<m>::to<uint8>();      // silent
  float64<m> d = n;                 // warning[W0410], n is taken to be metres
  ```
- **C. Strip to the base unit.** A unit value in a plain slot is converted to
  the canonical base units first, so the plain number is always the SI-style
  amount; attach only by constructor-like call `m(3)` style (the unit name as
  a function).
  ```cpp
  uint8 s = 3<km>;                  // allowed, s is 3000 (in m); 3<m> gives 3
  float64<km> d = <km>(2.5);        // hypothetical attach form
  ```
- **D. Strip only when the unit is the plain number's own.** A plain slot may
  receive a unit value, but that value's scale must be 1 (a base or an
  unscaled derived unit), so there is no hidden factor.
  ```cpp
  uint8 s = 3<m>;                   // fine, s is 3
  uint8 t = 3<km>;                  // E0408: km is scaled, write 3<km>::to<float64<m>>()...
  ```

### Q2. What does `::text()` print for a unit value?

Why: it is the first thing every program does with a number, and the choice is
hard to change later (golden outputs everywhere). It also decides `outf` and
string interpolation. Also relevant: `s::to<string>()` in the scratch file; today
`to` rejects `string`, so this question assumes `::text()` is the way, and a
separate question for the owner is whether `to<string>` should exist at all.
Until answered: `::text()` on a unit value is E0901 (see 2.9).

- **A. With the unit, a space between.**
  ```cpp
  io::outln((100.0<m> / 9.6<s>)::text());   // 10.416666666666666 m/s
  io::outln(3<km>::text());                 // 3 km
  ```
- **B. Number only.** The unit is a compile-time tag and has no runtime
  presence, so there is nothing to print.
  ```cpp
  io::outln(3<km>::text());                 // 3
  ```
- **C. Two spellings.** `::text()` is the number only; a second method prints
  the unit too (name to be chosen: `::label()`, `::withUnit()`).
  ```cpp
  io::outln(3<km>::text());                 // 3
  io::outln(3<km>::withUnit());             // 3 km
  ```
- **D. The unit as a separate value of the text.** `::unitText()` returns just
  `"km"`, `::text()` just the number, so the program decides the layout
  (`"{d::text()} {d::unitText()}"`); nothing joins them for you.

### Q3. Dimensionless results: `m / m`, `km / m`, `rad`

Why: ratios are everywhere (`distance / distance`, efficiency, `time / time`).
The scale subtlety: `4<km> / 2<m>` has no dimension but a factor of 1000.
Until answered: dimension-free with scale 1 is a plain number; with scale not 1
is E0901 (2.6).

- **A. Always collapse to a plain number, folding the scale in.** `4<km> /
  2<m>` is a plain `2000`.
  ```cpp
  float64 ratio = 4.0<km> / 2.0<m>;   // 2000.0
  ```
- **B. Stay tagged.** The result has the unit `km/m` (a "dimensionless unit with
  a factor"), converts to a plain number only explicitly, so `4<km>/2<m>` is
  `float64<km/m>`.
  ```cpp
  float64<km/m> r = 4.0<km> / 2.0<m>;
  float64 ratio = r::to<float64>();   // 2000.0
  ```
- **C. Collapse only when the factor is 1.** `m/m` is plain; `km/m` stays a
  tagged value that must be converted (as B) so nothing is silently multiplied.
- **D. Dimensionless units are declarable units.** `m/m` is plain; scaled
  ratios are written with their own name, like any unit.
  ```cpp
  unit percent = 0.01 1;
  float64<percent> eff = 85.0<percent>;
  float64 plain = eff::to<float64>();  // 0.85
  ```

### Q4. Units on integers

Why: `uint8<m>` is easy to allow but the scaled conversion of integers is lossy
(Q5), and the scratch file uses `uint8`. Also affects `::to`.
Until answered: allowed on every number type; Q5 governs scaling.

- **A. All number types, no special rules.** Same as floats.
  ```cpp
  uint8<m> n = 3<m>;
  ```
- **B. Floats only.** Integers cannot carry a unit; counting things is `uint32`,
  measuring is `float`.
  ```cpp
  uint8<m> n = 3<m>;                  // E0405: a unit needs a float type
  ```
- **C. Integers allowed, but only whole-number scale relations.** A scaled
  conversion between integer units must be a whole multiple (`km` to `m` yes;
  `m` to `km` is an error), so no value is ever rounded.
  ```cpp
  uint32<km> k = 2<km>;
  uint32<m>  m = k;                   // 2000 m
  uint32<km> back = m;                // E0409: not a whole number of km
  ```
- **D. Integers allowed; the integer unit is its own tick.** A unit applied to
  an integer type fixes the *smallest* step: `uint32<m>` counts whole metres
  and a conversion to a bigger unit must say how to round (`::roundTo<>`
  etc.).

### Q5. Scaled conversion rounding in integer types: `10<km> + 100<m>`

Why: with untyped constants SPEC's `10<km> + 100<m>` is exactly 10.1 (2.5), but
with typed integers `uint32<km> a; uint32<m> b; a + b` has to put `b` in km.
Until answered: truncate toward zero with W0102 (2.7, step 4), matching how a
float is cut to an integer today.

- **A. Truncate toward zero, W0102.** Same as float to integer today.
  ```cpp
  uint32<km> a = 10; uint32<m> b = 100;
  uint32<km> c = a + b;               // 10 (warning W0102: 100 m is 0 km)
  ```
- **B. Round to nearest, W0102.**
  ```cpp
  uint32<km> c = a + b;               // 10 (0.1 rounds to 0), but 600<m> gives 1
  ```
- **C. Error; the finer unit wins.** An integer operation between two units
  where the left cannot hold the right exactly is rewritten to the finer unit
  (the left no longer leads for integers), result `uint32<m>` 10100. No loss.
  ```cpp
  uint32<m> c = a + b;                // 10100 m; assigning that to uint32<km> is the W0102 case
  ```
- **D. Error always.** Mixed scales of integers need an explicit
  `::to<...>()` first.
  ```cpp
  uint32<km> c = a + b;                // E0409
  uint32<km> c = a + b::to<uint32<km>>();  // fine (and truncates, by your word)
  ```

### Q6. Functions generic over units, before step 6 generics

Why: `float64<m> double(float64<m> x)` is useless for `s` and `kg`; the
math library (`abs`, `min`, `clamp`) needs this. Step 6 will have generics,
which are out of scope here.
Until answered: a unit in a signature is a fixed unit (the only way); math
helpers do not exist yet.

- **A. Nothing until step 6.** Write one function per unit; plain numbers pass
  unit values only via Q1's explicit strip/attach.
- **B. A named wildcard in unit position, built in.**
  ```cpp
  float64<any u> twice(float64<any u> x) { return x * 2; }
  float64<any u> lerp(float64<any u> a, float64<any u> b, float64 t) { ... }
  ```
  `any u` binds on first use; the same name must be the same unit; `float64<any
  u * any v>` is not allowed (only whole-unit wildcards).
- **C. Unit parameters when generics arrive.** Declare it as a kind of generic
  parameter; step 4.5 ships nothing and reserves the syntax.
  ```cpp
  float64<U> twice<unit U>(float64<U> x) { return x * 2; }
  ```
- **D. Positional wildcard.** `<any>` means "the unit of the first argument that
  has one", the result takes the same.
  ```cpp
  float64<any> twice(float64<any> x) { return x * 2; }
  ```

### Q7. Is `unit` still a reserved keyword? (clash with SPEC section 5)

Why: `unit` is in `lexer::KEYWORDS`, and SPEC section 5's own example is
`static Circle unit() { return new Circle of (1.0); }`. As written, that
method name does not lex. Method and variable called `unit` are natural names.
Until answered: `unit` stays reserved (nothing changes today); the SPEC example
fails to parse.

- **A. Keep `unit` reserved; rename the SPEC example** (`static Circle
  unitCircle()`).
- **B. Context-only, like `leaving`/`stays`.** `unit` begins a declaration only
  as the first token of a top-level item followed by a name; everywhere else
  it is an ordinary identifier (the lexer drops it from `KEYWORDS`; the parser
  recognises `Ident("unit")` at item start).
  ```cpp
  unit m;                              // declaration
  static Circle unit() { ... }         // method named unit
  ```
- **C. Another word for the declaration**, in Drachm's wordy style, freeing
  `unit` as a name.
  ```cpp
  measure m;   measure N = kg * m / s^2;   measure km = 1000 m;
  ```
- **D. Declaring by a longer phrase** that cannot clash: `unit of measure m;`
  / `base unit m;` / `derived unit N = ...;` with `unit` itself staying a
  name.

### Q8. Offset (temperature-like) units: degrees C and K

Why: `degC = K - 273.15` is not a pure scale, and `20°C + 20°C` is not 40°C.
The canonical form (exponent vector + scale) cannot express it; supporting it
adds a third component and some rules.
Until answered: a unit with an offset is E0901 (the declaration grammar has no
syntax for it yet).

- **A. Not supported.** Celsius is a user function pair or a plain number;
  `K` is the temperature unit, differences in K.
  ```cpp
  float64<K> t = 293.15<K>;
  float64 c = t::bare() - 273.15;       // Q1 form
  ```
- **B. Offset units convert only.** A unit may have an offset; values in it can
  be compared and `::to`-converted, but arithmetic on them (`+ - * /`) is
  rejected until converted to the plain unit.
  ```cpp
  unit degC = K offset -273.15;
  float64<degC> c = 20.0<degC>;
  float64<K> k = c::to<float64<K>>();   // 293.15
  c + c;                                // E0410: degrees C cannot be added; use K
  ```
- **C. Point and delta.** Offset units are *points*; the difference of two
  points is a *delta* in the unit's scale: `degC - degC` is `float64<delta
  degC>`, `degC + delta` is `degC`, `degC + degC` is an error. Original to
  Drachm; heavier rules.
  ```cpp
  unit degC = K offset -273.15;
  float64<degC> a = 20.0<degC>, b = 25.0<degC>;
  float64<delta degC> d = b - a;        // 5.0
  float64<degC> c = a + d;
  ```
- **D. Everything is a unit, nothing is special:** an offset is an optional
  *affine map* on any unit and is applied only by `::to`; arithmetic on the
  tagged value uses the raw number (so `20.0<degC> + 5.0<degC>` is 25 degC and
  the programmer is trusted).

### Q9. Where do the SI units live before modules exist?

Why: modules are step 7, but the SPEC promises SI units "ready to use", and
the scratch file uses `m` with no declaration. Names like `m`, `s`, `g`, `min`,
`h`, `T`, `C` are common; since units have their own namespace (2.3) they do not
clash with variables, but they would clash with user units.
Until answered: nothing predeclared (2.10).

- **A. Always in scope** (a built-in prelude). A user `unit m;` is E0403
  (already declared); the user can redeclare only by naming it differently.
- **B. Imported with the existing `in` line**, embedded in the compiler:
  ```cpp
  in io;
  in si;                                // the SI set; `in io;` already exists
  ```
  Without `in si;` units must be declared by hand.
- **C. A file flag** (`//--!$units: si`), like the other file flags; default
  none. Also makes `drachm.toml` able to set it project-wide.
- **D. Nothing shipped until step 7.** Users declare their own; SI arrives as
  a stdlib module with real `in si;` resolution.

### Q10. Plain constants beside unit values: `speed > 0`, `d + 5`, `d * 2`

Why: `*` and `/` by a plain constant are clearly fine (`d * 2`); `+`, `-` and
comparisons are where unit safety lives or leaks.
Until answered: `* / **` take plain constants; `+ - % compare` with a plain
constant is E0401, with no exception for zero.

- **A. Only `* /` take plain numbers.** To add five metres write `5<m>`; to
  compare with zero write `0<m>`.
  ```cpp
  if (speed > 0<m/s>) { ... }
  d = d + 5;                            // E0401
  ```
- **B. A constant takes the other side's unit** (as it takes its type today).
  ```cpp
  if (speed > 0) { ... }                // compares with 0 m/s
  d = d + 5;                            // adds 5 m
  ```
- **C. Only zero is unit-free.** `0` is the same in every unit; any other
  constant needs its unit.
  ```cpp
  if (speed > 0) { ... }                // fine
  d = d + 5;                            // E0401
  ```
- **D. Constants take the unit in comparisons only**, never in `+ -`.

### Q11. How to say "same number type, other unit"

Why: `::to<T>()` takes a *type*; converting `km` to `m` while keeping the
number type means repeating it, and units are not types. Related to Q1.
Until answered: only `x::to<float64<km>>()` (a full type) works (2.7).

- **A. Full type only.**
  ```cpp
  float64<m> m = d::to<float64<m>>();
  ```
- **B. `to` also accepts a unit name** in the type-argument slot; it keeps the
  number type.
  ```cpp
  float64<m> m = d::to<m>();            // d is float64<km>
  float64 f   = d::to<float64>();       // number type only (Q1 decides if allowed)
  ```
- **C. A separate method for units**, so `to` changes only the number type.
  ```cpp
  float64<m> m = d::in<m>();            // `in` after `::` is already allowed
  uint8<m>   n = d::in<m>()::to<uint8<m>>();
  ```

**Eleven open questions.**

---

## 4. Plan of work

Packages for `docs/impl/`-style briefs; split into files when scheduled.

### Sequencing against step 3 (ownership)

Step 3 owns the parser, typeck and HIR right now. WP0a and WP1 phase B are
merged; WP2, WP3, WP4 run in parallel next, then WP5-WP9 (see
`step3-ownership.md` section 6). Units touch `lexer.rs`, `parser.rs`, `ast.rs`,
`types.rs`, `typeck.rs` and `codes.rs`, the same files.

- **U1 (`src/units.rs` only) can start immediately.** It is a new file with no
  dependents, plus `pub mod units;` in `lib.rs` (a one-line addition, no
  conflict risk beyond the `mod` list).
- **U2 onward starts after step 3's WP2 merges** (WP2 removes `Flow` from
  typeck and rewrites a large part of it; WP5 later touches
  mutability checks in the same file). WP3 (self mode, `ownership/`) and WP4
  (backends on MIR: `codegen.rs`, `cgen.rs`) do not block, but U6's erase
  assertion in `cl_type`/`c_type` must rebase after WP4. Do U2 and U3 while WP4
  is still running; they do not touch backends.
- Anything that must touch `typeck.rs` goes through the hooks in 2.8 and the new
  `typeck/units.rs`; do not reformat or reorganise `typeck.rs`.
- `codes.rs`: add the `04xx` block between the `E03xx` block and `E0901`.
  If WP5-WP9 append codes at the same place, rebase and keep both.
- Merge order rule: units never block step 3 and step 3 never waits for units.
  If WP5 or later is still open when U2 is ready, branch from the latest merged
  master and rebase U-branches onto step 3's merges rather than the reverse.
- Open questions do **not** block U1-U4: each has a conservative behaviour in
  section 3, marked `// OPEN(unitsQn)`. They block U5 (Q9) and the final docs.

### U1: unit data (new `src/units.rs`)

Files: `src/units.rs`, `src/lib.rs` (`pub mod units;` only).
Implement `Unit`, `UnitTable` (interning, display names), `mul`, `div`, `pow`,
`ratio`, `Display`, and the parse of a unit expression string into a `UnitExpr`
**without** the AST dependency (a small recursive descent over `&str` returning
a neutral tree, so the lexer rule 2.4 can reuse `UnitExpr::is_valid_text`).
Tests (Rust unit tests in the file): canonical equality (`N` vs `kg*m/s^2`),
`km`/`m` ratio 1000, `m/m` is dimensionless, `pow` with negatives, `1/s`,
display of derived units, interning pointer equality, overflow of exponents.
Done when: `cargo test` green; no other file changed.

### U2: lexer, parser, AST

Files: `src/lexer.rs`, `src/parser.rs`, `src/ast.rs` (+ `Q7` outcome).
- Unit-literal rule 2.4 in `lexer::number` after the suffix is read; `Tok::Num`
  carries the unit text and span.
- `ast::UnitExpr`, `Item::Unit`, `TypeName.unit`, literal `unit` field; parse of
  `unit` items (2.3); `ty()` unit part with `>>` splitting; `at_binding` rule.
- `Q7` default: `unit` stays in `KEYWORDS`.
Tests: parser unit tests for each form (`unit m;`, `unit N = kg*m/s^2;`, `unit
Hz = 1 / s;`, `unit km = 1000 m;`, `float64<m/s> v`, `mut float64<m> d`,
`to<float64<km>>()`, `9.6<s>`, `5u8<m>`, `2.5f32<m/s>`, `1<1/s>`), and the
lexer ambiguity table: `3<4`, `3 < m`, `3<m`, `3<m>`, `3<m>>2`, `3<x && y>2`,
`3<>`, `3<m*>`, `a<b>c` (not after a literal: unchanged), `if (n<limit)`.
New golden fail test `tests/fail/units_syntax.drh` (`E0003`-style expression
errors from the fallback path, `E0405` for `string<m>`).
Done when: all existing tests pass with unchanged output; the new parser tests
pass; `drachm check` on a file with only unit declarations and unit-typed
signatures reports nothing wrong at the syntax level.

### U3: types and resolution

Files: `src/types.rs`, `src/typeck.rs` (hooks), `src/typeck/units.rs` (new),
`src/hir.rs` (`scalar_size` arm).
`Type::Unit`, `bare`, `unit`, `with_unit`, `Display`; `Globals.units`,
`declare_units`, `resolve_unit`; `Globals::resolve` hook; E0402, E0403, E0405.
Struct fields and parameters with unit types lay out as the bare type.
Tests: `tests/fail/units_decl.drh` (cycle, redeclaration, zero scale, `unit u =
5;`, unknown unit, `string<m>`); `tests/run/units_decl.drh` (declarations in
odd order, aliases `float64<N>` and `float64<kg*m/s^2>` interchangeable, units
in struct fields and signatures, prints a value to prove layout).
Done when: those pass and all earlier tests are unchanged.

### U4: arithmetic, conversion, erasure

Files: `src/typeck.rs` (hooks 2.8), `src/typeck/units.rs`, `src/diag/codes.rs`
(`E0401`, `E0404`, `E0406`, `E0407`, `E0408`; W0102 texts), `src/hir.rs`/
`units::erase`, `src/codegen.rs` / `src/cgen.rs` (debug-assert arms only).
Implements 2.5-2.8: `Val::UConst`, `unit_binary`, `unit_into`, `to` with unit
types, compound assignment, comparisons, `**`, scaling emission, the
conservative behaviour for Q1, Q2, Q3, Q5, Q8, Q10, Q11, and `units::erase`.
Golden tests (all `tests/run` cases run both backends):
- `units_arith.drh`: the SPEC example (`dist / time` is `float64<m/s>`),
  `N = kg*m/s^2` from `F = m * a`, `**` with a constant, `1/s`, `Hz`, compound
  assignment, comparisons across scales, left-leads on number types
  (`2.5 * dist`, `uint32<m>` times `float64`).
- `units_scale.drh`: `10<km> + 100<m>` is 10.1 km, `10<m> + 1<km>` is 1010 m,
  typed float and integer scaling, `::to<float64<km>>()`, scaling through a
  function argument and a `return`, W0102 expected in `units_scale.warn` for the
  integer km case (output lines checked via `.warn`).
- `units_erase.drh`: proves zero cost: a loop summing `float64<m>` equals the
  same program without units byte for byte in output; and a Rust test in
  `tests/units.rs` that builds both versions with `drachm::build` and compares
  the C source (`-O`) for the unit-free parts being identical, i.e. no scaling
  code in unit-free programs.
- `tests/fail/units_mismatch.drh`: E0401 for `dist + time`, assignment
  `float64<m> d = time;`, argument and return mismatches, `<` across kinds.
- `tests/fail/units_ops.drh`: E0404 (`side ** n`), E0406 (`len << 2`, `len & 1`),
  E0407 (`d::to<float64<s>>()`), E0408 (`uint8 s = 3<m>;` and the reverse).
- `tests/fail/units_ambiguous.drh`: the E0901 not-decided-yet messages for
  `::text()`, `km/m`, offset units, so removal of each is a visible test change.
Done when: the above pass; `cargo test` green; `optimised_output_matches`
passes for every new run case; `drachm check` on `examples/stupid.drh` gives
E0402 for `m` and E0408 for the assignment (it only becomes a working program
once Q1, Q2, Q9 are answered).

### U5: SI set (after Q9) and the owner's open answers

Files per the chosen option for Q9: `src/prelude/si.drh` (2.10), a hook in
`Globals::new` / the import handling near typeck.rs:248 (the `io` import),
plus the code removals for every question answered (each `// OPEN(unitsQn)`
marker is a grep target; list the markers resolved in the commit message).
Tests: `tests/run/units_si.drh` using several SI units; the scratch program as
a golden test in the form the owner's answers make valid; each answered
question gets a run or fail test that pins the owner's chosen behaviour.
Done when: no `OPEN(units` markers remain; `cargo test` green.

### U6: docs and site (in the same commit as the user-visible change)

Per `CLAUDE.md`:
- `site/content/reference-units.md` (new): declaring units, `T<unit>` types,
  `9.6<s>` literals and the `<` rule, the arithmetic table (2.6), scaled
  conversion, `::to`, text output, the SI set, diagnostics E0401-E0408 with
  short examples. Add `reference-units` to `site/content/order.json` under
  Reference after `reference-types`.
- `site/content/tour-numbers.md`: a short paragraph and link.
- `site/examples/units.drh` and `units.out` (real program; checked both ways by
  `cargo test`; no compiler output allowed, so no warnings in it). Also embed
  it in the new page with the same include mechanism the other pages use.
- `site/content/coming-later.md`: remove "units of measure" from the "Maths"
  line; keep vectors and matrices.
- Rerun `site/gen-errors.sh` after the `codes.rs` changes.
- `docs/SPEC.md` 4b/section 10/section 5, `docs/PLAN.md` step 4.5 status, and
  the living design doc (Claude Docs) get the owner's answers to section 3 and
  the new `04xx` range; this spec file is then updated or retired.
Done when: `cargo test` green (it runs the site examples), `site/gen-errors.sh`
produces no diff beyond the new codes, and the page lists only what the
compiler accepts today.

### Pitfalls

- Do not give `Type::Unit` to `Type::converts_losslessly_to` unit-aware
  meaning; it is about the number only. All scaling judgement lives in
  `typeck/units.rs`.
- `hir::Expr` types must be erased *after* all diagnostics are produced and
  before the MIR pass; do not erase per expression while checking.
- The lexer rule needs no symbol table. Do not look at declared units from the
  lexer or parser.
- `^` in a unit expression is the existing XOR symbol; never change how `^`
  parses anywhere else. Same for `*` and `/`.
- `>>` splitting must not change `x >> 2` in expressions (only in type and type
  argument parsing).
- Exact arithmetic: use `BigRational` for every scale; never an `f64` for the
  factor until the final constant is emitted.
- `UConst` folding must not change the result type of existing plain-constant
  expressions: only expressions containing a unit literal take the new path.
