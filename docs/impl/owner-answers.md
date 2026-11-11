# Owner answers, 2026-11-10 (to be folded into docs/SPEC.md, the design doc and docs/impl/step3-ownership.md)

Q1 deref: borrows always mean their target. `r = 5;` writes through. `r = &mut other;` re-points r: allowed because the right side is a reference; the borrowed thing can never be assigned a pointer, so the RHS type disambiguates. Reads automatic (`q + 1`).
Q2 call site: `&` always written for shared borrows: `show(&p);` (and `&mut` as already decided).
Q3 passing on a held borrow: `add(lend s);` (new keyword `lend`).
Q4 two-phase: allowed (`s::append(s::len()::text())`); only borrows still alive when the receiver changes are errors (`xs::push(xs::first())`).
Q5 mut fields through `&`: never. `mut` fields are only special on values you own (immutable binding). Through `&` everything is read-only; changing needs `&mut`. Owner: "if you need it to be changeable you can't only look at it."
Q6 open: allows field writes, `open` parameters (`none fill(open string s)`), and declaring without a value and assigning once later.
Q7 partial moves: must say `take`: `string n = take p.name;`. Plain `string m = p.name;` (moving type, owned struct) is an error with fix-it "write `take p.name` or `copy p.name`". After `take`, `p.name` has no value (compile-time, like `uint32 x;`): reading it or using `p` whole is an error until reassigned. `take p.title leaving "none"` puts a value back in the same statement (field keeps being usable). Fields still holding values are dropped at scope end.
Q8 consuming self: not allowed; methods only borrow `self`.
Q8b auto-copy: no hidden copies anywhere. `return self.name;` from a borrowed self is an error: "self is borrowed; write `return copy self.name;`". (Owner considered "copy when leaving the context" but found it inconsistent.)
Q9 lives on functions: required only if ambiguous (body-derived): inferred when the returned borrow can only come from one parameter; E0308 when several.
Q10 struct lives: names the borrow fields themselves: `struct Pair lives a, b { &string a; &string b; }`; each may borrow from a different place. (Path form `lives p.src` for functions not asked yet.)
Q11 copy: `copy r` on a borrow copies the pointed-at value (own `string`). `copy n` on a Copy value warns (W0322 "`copy` does nothing for uint32"), a redundancy lint that should be disableable in config `[warnings]`.
Q12 32-byte rule: rule applies; size = memory size including padding (confirmed: field order can change Copy-ness), plus opt-out word: `struct Handle stays { ... }` never copies automatically.
Q13 temporaries: borrowing a temporary in a declaration keeps it alive to the end of the block.
Q16 interfaces: interface methods that may change self say `changes self` after the parameters; struct methods stay inferred and may not change more than the interface allows. (`changes self` only in interfaces.)
Q17 cleanup: a method with the reserved name `none drop() { }` runs when the value is destroyed. (Implies: structs with drop() can't be partially moved/taken? -> ask; drop order becomes visible.)
Q14 raw pointers: made with `address of n` (new wordy operator). Everything touching a pointer value (making, copying, comparing, dereferencing) must be inside `unsafe { }`. One pointer kind `*T` (may be written through; no `*mut T`). Null allowed: a pointer can be `none`, tested with `p == none`. `unsafe` functions exist: `unsafe` before the return type; calling one needs an `unsafe { }` block.
Assembly (owner request): whole functions written in assembly, no inline mixing: `unsafe uint64 cycles() asm { rdtsc; shl rdx, 32; or rax, rdx }` (instructions one per line, body is the asm). Called like any unsafe function. Schedule after C header import (Later). Details (register/ABI mapping, which syntax flavour Intel/AT&T, other CPUs) still to design.
Q18 clause order: either order for `lives`/`fails` on functions and `lives`/`with` on structs; `drachm fmt` picks one.
Q15 copying: `*T` pointers MOVE (`copy p` duplicates). `&mut T` moves too: `&mut string r2 = r;` uses up r. (`&T` copies, R8.)
drop()+take: on a struct with `drop()`, plain `take x.f` is an error; `take x.f leaving v` is allowed (struct stays complete).
Q10ii: `lives` on functions names whole parameters only (`lives p`), no paths.
STILL OPEN: Q19 wording of the 03xx message drafts (show with the other draft wordings).

# Step 2/4 follow-ups (same day)
"""-strings: the newline right after the opening `"""` is dropped; the column of the closing `"""` is the left edge and that much leading whitespace is removed from every line. A line indented less than that is NOT an error: it just loses all its leading whitespace ("clamps"). Applies to f""" too.
outf: every unescaped `%` is a placeholder (keep). `\%` = literal percent. NEW: `%\` ends a placeholder's options: `io::outf("%\5 apples!", "£")` prints "£5 apples!" (the `5` is text), whereas `"%5 apples!"` is width 5. (My reading of the owner's note "%\ in general should negate anything after the %".)
Operator methods: overloadable by right-hand type, resolved like constructors (exact, lossless, lossy+warning). Owner note: `v * w` as dot product was a bad example; dot would be a named method.
Range loops: `by` step word: `for (float64 x of 0.0..1.0 by 0.25)`, `for (uint32 i of 0..10 by 3)` = 0,3,6,9 (stops at the last value not past the end; never clamps to the end). Float ranges require `by`? (option text said float ranges MUST say their step with `by`; integer ranges may.)

# Step 3 edge cases, 2026-11-11 (spec "Still open" #1, #3, #6, #9)
#1 borrow of a borrow: `&mut s` where `s` is already a `&mut` binding is an error with fix-it "write `lend s`". No `&&T` types.
#3 `&mut` where `&` is expected: must be written `show(&m)` (an explicit shared borrow through the `&mut` you hold); bare `show(m)` is an error with fix-it "write `&m`". (So `&m` on a `&mut` binding is the one allowed borrow-of-a-borrow: it lends a shared view for the call.)
#6 calling drop yourself: `x::drop()` is an error. New statement `destroy x;` runs `drop()` now and ends `x` (like a move; nothing runs at scope end). `destroy` becomes a keyword.
#9 `address of` an immutable binding: allowed, gives a read-only pointer of a second pointer kind `*fixed T` (cannot be written through). `address of` a `mut`/`open` binding gives `*T`.
Still open: #2 `lend` on a `&T`, #5 more `take` forms, #7 multi-slot borrow fields, #8 pointers outside `unsafe` / inside unsafe fn bodies, #10 `none` as a non-pointer value (likely step 5 `maybe`).

# Field-by-field filling and defaults, 2026-11-11
- A struct declared without a value can only be filled through a `fill` block: `fill p { p.name = "a"; p.hp = 5; }` must give every field a value by its `}` (error there if one is missed). `fill partial p { ... }` sets some fields and leaves `p` incomplete. Until `p` is complete, `fill` / `fill partial` are the only ways to touch `p` (no reads, borrows or plain field assignments).
- Defaults: a `def` is a named template of field values, decoupled from the struct, and a struct may have several: `def PlayerStart of Player { hp = 100; title = "none"; }`. A struct names its default def in its head: `struct Player defaults PlayerStart { ... }`. Shorthand: `def Player { ... }` (a def with the struct's own name, no `of`) and then `struct default Player { ... }`. `fill p with defaults { }` starts from the struct's default def and the block sets the rest.
- My reading, not yet confirmed: `fill p with Veteran { }` picks a named def other than the default.
- Writing through a raw pointer field (`unsafe { *self.counter += 1; }`) does not change `self`: the method stays `&self` ("counter is a reference and not the value itself").
- Scheduled: `fill`/`def`/`defaults` with step 5 (constructors). In step 3, assigning a field of a struct that has no value is E0332 with help "fill it with `fill p { ... }` (coming in step 5)", and `fill` itself is E0901.

# Units of measure (step 4.5), 2026-11-11 (spec docs/impl/step4.5-units.md Q1-Q11)
Q1 unit -> plain: allowed, strips to the base unit amount: `uint8 s = 3<m>;` is 3, `uint32 t = 3<km>;` is 3000. (Owner: "i kinda forgot these units need syntax and they're not just magically working".)
Q2 `::text()` prints the number only; `::unitText()` gives the unit text ("km"); the program lays them out.
Q3 dimensionless results always become a plain number with the scale folded in: `4.0<km> / 2.0<m>` = 2000.0.
Q4 units on every number type. Integers can't hold fractions: `uint8<km> s = n;` with n = 300 m gives 0.
Q5 (from Q4's note) integer scaled conversion truncates toward zero (my reading: with W0102 like any lossy conversion).
Q6 no unit-generic functions until step 6 generics (`float64<U> twice<unit U>(float64<U> x)`).
Q7 `unit` is a context word: it declares only at the start of a top-level item; elsewhere an ordinary name (`static Circle unit()` is fine).
Q8 offset units are points and deltas: `float64<degC>` points, `b - a` is `float64<delta degC>`, point + delta is a point, point + point is an error. Declared `unit degC = K offset -273.15;`.
Q9 SI units come with `in si;` (built into the compiler until modules exist).
Q10 `*` and `/` take plain numbers (they're scales); zero is unit-free; any other plain constant in `+ - %` or a comparison needs its unit (`d + 5<m>`).
Q11 `::in<m>()` changes the unit and keeps the number type; `::to<T>()` changes the number type.
Prefixes: NOT automatic. Scaled units are ordinary declarations (`unit g; unit kg = 1000 g; unit l; unit ml = 0.001 l; unit ft; unit mi = 5280 ft;`); `in si;` ships the common ones declared that way. Arithmetic on scaled units goes through the base and back: `2<mi> + 2<mi>` = (5280*2 + 5280*2) ft / 5280. Unit names are ASCII only (no `µ`).

# 2026-11-11, later
Q-MF (wp5.md): a method that writes only `mut` fields is inferred as a third kind ("changes only mut fields"): callable on an owned immutable value (`Counter k; k::hit();` is fine), not through a `&` borrow (E0311), fine through `&mut`.
Units Q12: a plain variable beside a unit value follows the same rule as plain constants: `*` and `/` are fine, `+ - %` and comparisons need a unit; `d++` adds one of `d`'s own unit.
Units Q15: `kg` is the base mass unit (SI): `unit kg; unit g = 0.001 kg;` so `uint32 x = 2<kg>` is 2 and `uint32 y = 500<g>` is 0.
Still open: units Q13 (declared dimensionless units like `unit pct = 0.01 1;`), Q14 (is `delta degC` the same type as `K`; `::unitText()` of a delta; zero beside a point). Conservative defaults in the spec stand meanwhile.
Units Q14 (owner, then my choices as the owner asked "for the other ones u choose"): owner: a `delta degC` is the scientific change of temperature, so 1 delta degC is the same size as 1 K: `delta degC` and `K` have the same dimension and scale 1, and convert freely (`float64<K> d = (b - a)::in<K>();`, and adding a `delta degC` to a `K` value is fine). Mine: `::unitText()` of a delta prints `delta degC`; a bare `0` is NOT unit-free beside a point (0 degC is a real temperature): write `0<degC>`.
Units Q13 (my choice): declared dimensionless units are allowed: `unit pct = 0.01;` (no base unit after the factor). They behave like any scaled unit: `85<pct>` stripped to a plain number is 0.85, and `pct * pct` is a plain number (Q3).

# Input, 2026-11-11
- `io::inln()` returns the next line of standard input as a `string`, without the newline.
- At end of input it returns `""`; `io::ended()` (a `bool`) says whether input is finished: `while (!io::ended()) { string line = io::inln(); ... }`.
- Text to number reuses `::to<T>()`: `line::to<uint32>()`. It can fail on bad text, so it becomes a failing call with step 5 (`line::to<uint32>() maybe 0`). Until step 5 (my interim choice, not asked): bad text stops the program with a runtime error (exit 101) naming the text and the type.
- Scheduled after WP6 (it touches typeck builtins, the runtime and the backends WP6 is rewriting).
