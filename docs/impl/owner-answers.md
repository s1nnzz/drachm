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
Units Q12b (2026-11-11): assigning or declaring a unit variable with a plain value attaches the unit: `float64<m> d = 5;` is 5 m, `d = 0;` is 0 m. Only operators are strict (`d + 5` is an error).

# WP8 / WP9 questions, 2026-11-11
- WP8 Q1 `lives p` on a holder parameter: owner picked "whole parameter" and said "just do what u think is best and is in line with the rest of the lang". Kept: `lives p` keeps the whole parameter `p` borrowed while the result lives (matches Q10ii: function `lives` names whole parameters only, no paths).
- WP8 Q2 naming `self` in `lives`: a `from` clause, `&string pick(&string key) lives from self or key { ... }`. My reading: sources are joined with `or`; `lives from ...` is the form that can name `self`, and plain `lives a, b` stays for parameters (both spellings may name parameters).
- WP9 Q1 (#8): everything that touches a pointer value needs `unsafe`, moving/passing/returning/storing included; the body of an `unsafe` function counts as one unsafe block (no inner block needed).
- WP9 Q2: converting between `*T` and `*fixed T` is explicit both ways, inside `unsafe`: `p::freeze()` (`*T` -> `*fixed T`) and `p::thaw()` (`*fixed T` -> `*T`).
- Input Q1 (2026-11-11): text to number is forgiving: surrounding spaces ignored, a leading `+` allowed, `.5` and `5.` fine for floats; out-of-range is still bad text. (Line endings, invalid UTF-8 and `to<bool>`/units keep the interim behaviour until asked.)

# Step 4 rest / step 5 questions (step5-overview.md section 8), 2026-11-12
Q1 text patterns: the opposite of `case literal`: a plain `case "a*b"` matches the text exactly (a `*` is just a star); globs are written `case globbed "*.png"`, where `*` is a wildcard.
Q2 for-each over a string: both, by the type written: `for (string c of s)` gives an owned copy per character (may be kept), `for (&string c of s)` a borrowed view; the compiler skips the copy when the body never keeps `c`.
Q3 owning payloads in a `case`: the pattern says `take` or `copy`: `case Text(take string body)` moves the payload out (the matched value is used up), `case Text(copy string body)` copies it; a bare `string body` for an owning payload is an error with both fix-its. Copy payloads (numbers, bools, Copy structs) bind plainly.
Q4 patterns: all of them: guards (`case Circle(float64 r) if r > 10.0`), several patterns per body (`case A(..), B(..)`), nesting (`case Some(Circle(float64 r))`), and values inside patterns (`case Rect(2.0, float64 h)`).
Q5 optionals: `T` wraps to `T?` automatically wherever a `T?` is expected (`uint32? x = 5;`, `return 5;`); the test is wordy: `if (x has value)`, `if (x is empty)` (`has value` / `is empty` after an optional expression). `x == none` is not the test (my reading: error with the `is empty` fix-it).
Q6 enums: `==` / `!=` only for enums without payloads (`if (d == North)`); no methods inside enums: behaviour is plain functions taking the enum.
Q7 variant names: the expected type decides first; with no expected type, the struct/function of that name wins; `Shape::Circle(1.0)` names the variant explicitly. A variant may share a name with a struct.
Q8 constructors: writing a constructor removes the automatic field-order one, unless the struct says `Person(auto);`, which keeps it alongside the written ones. (Named values and delegation: my reading, unasked: named values match the parameter names of the chosen overload; delegation stays E0901.)
Q9 reading set fields while filling: allowed only with the word `now`: `fill p { p.hp = 5; p.max = now p.hp * 2; }` (same in constructors: `now self.r`). A plain read of an incomplete value stays an error with the `now` fix-it; methods on the incomplete value are not allowed.
Q10 defs: named defs can be used (`fill p with Veteran { }`), def values are any expression that doesn't read fields, AND a def may extend another: `def Veteran of Player from PlayerStart { hp = 200; }`.
Q11 failure messages are ordinary strings: write `f"bad config key ${k}"` to interpolate (the SPEC example changes).
Q12 escaping failures print to stderr with an `error: ` prefix, then one `because:` line per cause: `error: couldn't load save` / `  because: no such file "save.dat"`; `${e}` prints the whole chain.
Q13 `::to<T>()` on text fails with a built-in `ConvertError` with one variant per cause: `Empty`, `NotANumber`, `OutOfRange` (replaces step 4.6's exit-101 interim once failures exist).
Q14 handlers: may name a single variant (`fails with (BadConfig e) => {}`), `any e` can be re-raised with `fail e;` (when the enclosing function lists it), AND `e` has members (owner's note "and members"): `e::message()`, `e::exit()`, `e::cause()`.
Q15 `maybe` fallback: a single value or a parenthesised expression; anything longer is an error asking for parentheses (`(parse(s) maybe 0) + 1`, `parse(s) maybe (n * 2)`).
Q16 ignoring a failure: both spellings: `fs::close(f) fails with none;` and `fs::close(f) ignore failure;`.
Q17 failing functions may be anything, operator methods and `drop()` included (`bool ==(Port o) fails X`, `(p == q) maybe false`). (My note: a failing `drop()` that runs at scope end has no caller to handle it; until asked, a failure escaping an automatic drop is reported like an escape from `main`.)
Q18 exit codes 0-255; the failure's code wins over a number `main` returns.

# Step 6 questions (step6-overview.md), 2026-11-12
Q1 unconstrained `T`: checked per use (at each instantiation, as if `T` were written out); `max<Circle>` errors inside `max`, pointing at the call. Constrained parameters (`<T with Shape>`) are checked at the definition.
Q2 unit arguments to your own generics are marked at the use: `Vec3<float64, unit m/s> v;` (declaration `struct Vec3<T, unit U>`).
Q3 borrows as type arguments are allowed and the container becomes a borrower with inferred lifetime: `Stack<&string> s; s::push(&name);` ties `s` to `name`.
Q4 interfaces are only ever borrowed (`&Shape`); owned interface values like `Shape[?]` are errors (use an enum or `&Shape`).
Q6 built-in types and interfaces: a separate declaration lets any type promise an interface (`uint32 with Zero { static uint32 zero() { return 0; } }`, anywhere in the project), AND the standard ones are already built in for numbers and text (owner: "id say that would already be inbuilt"; my reading: `Zero`, `Ordered`, `Equal`, `Text` come predeclared for numbers/`string`).
Q7 inferring `T`: the first argument that has a type decides; untyped constants adapt or fail (`max(a_u8, 300)` is an error like `uint8 x = 300;`); an all-constant call uses the default constant type.
Q8 for-each over owning items: like text: `string n` gives a copy per item (skipped when never kept), `&string n` a view, `&mut string n` a changeable view.
Q9 maps: `m[k] = v` adds or replaces; `m[k] += x` on a missing key stops the program; methods exist too (owner: "methods too"; my reading: `m::set(k, v)`, `m::remove(k)`, `m::get(k)` returning `T?`).
Q10 lists: negative indexes count from the end like text (`xs[-1]` is the last item); methods `push`, `pop` (`T?`), `get(i)` (`T?`), `insert(i, v)`, `remove(i)`, `clear()`, `contains(v)`, `join(sep)`, `first()`/`last()` (`T?`). Out-of-range `[]` stops the program.
Q11 moving an item out by index: `take xs[i] leaving v` (the list keeps its length), and also `take xs[i] reorders` (owner's note: moves the item out, fills the slot with the last item and shrinks; order not kept).
Q12 array lengths as generic value parameters: `struct Buffer<T, uint32 N> { T[N] items; }`.
Q13 a map key type with both `key()` and its own `==`: `key()` wins for maps; `==` stays what `==` is.
Q14 float map keys are allowed, hashed and compared by bits: `NaN` equals itself, `-0.0` and `0.0` are different keys.
Q15 methods with their own type parameters only through interface methods (`interface Mappable<T> { U[?] map<U>((T) => U f); }`); a struct's own methods use only the struct's parameters.
Q16 walking a map: two bindings `for (string k, uint32 v of ages)`, or an iteration method (owner: "or an iteration method"; my reading: `ages::each((string k, uint32 v) => { ... })`).
Q17 function values only go down: parameters, locals, calls; storing one in a struct/list or returning it is an error (`move` lambdas too).
Q18 lambdas that change captures: inferred like `self`: the capture becomes a `&mut` borrow automatically (`names::each((string n) => { count++; });` with `mut uint32 count`).
Q19 `Iterable`: two levels: the type gives `iter()`, a fresh walker with `T? next() changes self`; `for` walks `c::iter()`, so the value itself isn't spent and can be walked again.
Q20 a list or map declared without a value starts empty and is usable at once (`mut string[?] xs; xs::push("a");`).
Q5 interfaces as types: any interface can be used as `&I` (its `static` methods and `Self` members just aren't callable through it); upcasts are automatic (`&Shape3D` passes as `&Shape`), EXCEPT where the parameter says `exclusive`: `none show(exclusive &Shape s)` accepts only a value whose interface is exactly `Shape`, not a sub-interface (owner's note; `exclusive` is a new word; details still to design: E0901 until then).

# S6B follow-ups, 2026-11-12
- Built-in interfaces: keep the set S6B chose: `Zero` (`static Self zero()`: numbers, `string` gives `""`), `Equal` (`==`: numbers, `bool`, `string`, every struct), `Ordered extends Equal` (`<` `<=` `>` `>=`: numbers only), `Text` (`string text()`: numbers, `bool`). A program's own interface of the same name replaces the built-in one.
- `extends` takes several parents joined with `and`: `interface Solid extends Shape and Named { ... }`.
- A separate `TYPE with I { ... }` block sees only the type's public members. `TYPE with trusted I { ... }` sees private members too (owner proposed "partner", then picked `trusted` from the offered words).

# S6C.2 follow-ups, 2026-11-12
- `get(i)`, `first()`, `last()` on owning items give a view `&T?` (borrows the list); `copy names::first()` gives an owned `T?`. Mirrors `&xs[i]` / `copy xs[i]`.
- `contains` on owning items takes the value written with `&`: `names::contains(&name)`; without `&` it would move.
- Split edges: `""::split(",")` is `[""]`; `lines()` drops one trailing line end and `""::lines()` is `[]`; `split("")` gives the characters like `chars()` (does not stop the program).

# Iterable, 2026-11-13 (replaces the walker design of Q19)
- No separate walker. `interface Iterable<T> { T? at(uint64 i); }`: `for (T x of thing)` calls `thing::at(0)`, `at(1)`, ... until `none`. The value itself is not spent and can be walked again.
- Borrowing walkers: owner said "u pick"; moot without walkers (my note: `at` is an ordinary method taking `self` shared, so the value can't change during the loop, like lists).
- A type may be `Iterable` over several item types (`Graph with Iterable<Node> and Iterable<Edge>`); the loop's binding type picks which: `for (Node n of g)` vs `for (Edge e of g)`.

# S6G/S6D follow-ups, 2026-11-13
- Any struct may have two methods with the same name and parameters that differ only in result (as built for two `Iterable<T>`s); calling one by name is E0632.
- A map key type with its own `==` and no `key()` stays an error (E0627): add `key()`.
- OrderedMap: replacing a value keeps the entry's place; remove then re-add puts it last. Walking a map gives copies of keys and values (views later, with optionals that hold borrows).

# S6A.3 follow-ups, 2026-11-13
- A call may write a unit argument: `twice<unit m>(3.0<m>)`.
- When no argument carries a unit, `U` cannot be told (E0601); write `twice<unit m>(3.0)`.
- `U` inside a bigger unit in a parameter (`float64<U/s> v`) should be solved from the argument (`dist(3.0<m/s>, 2.0<s>)` gives `U = m`). Not built yet (E0901 today): to do.

# Q3 follow-ups, 2026-11-13
- An optional that holds a borrow is written `maybe &string` (e.g. `maybe &string f = names::first();`); `&string?` stays a borrow of an optional. Built today as `Optional<&string>`: to change.
- `List<&string>` for a list of borrows; `List<X>` means `X[?]` for any `X`.
- `==` through borrows compares what they point at; owner adds `===` for a strict address compare, and asks whether value-vs-address compare needs its own design beyond strings (ideas to be offered).
- Address compare is written in words, not `===`: `a same as b` (and `not (a same as b)`). Rules (my reading, from the option picked): allowed only when both sides are borrows or hold borrows (lists, optionals, maps of borrows compared item by item by address); on owned values it is an error.
- `xs::containsRef(&p)` is the address version of `contains`. Method and function names are camelCase (owner confirmed).
- `a not same as b` is the negative form (alongside `!(a same as b)`).
- `same as` is not allowed on maps of borrows or on structs/enums that hold borrows (an error, not an interim). Owner's note on the meaning: two borrows are the same when they point at the same address; the binding names don't matter.
- `in si` clash: keep the error when a type parameter is named like a unit (`<T>` with tesla in scope). Owner adds: write `T as unit` to mean the unit explicitly (tesla), same for `S as unit` (my reading: inside `<...>` of a number type, e.g. `float64<T as unit>`).

# Step 6 leftover follow-ups, 2026-11-13
- SI tesla is renamed: the unit is `tesla`, not `T` (owner: "just rename the unit T to tesla and this all goes away").
- Other single-letter units (`K`, `V`, `A`, `S`, `N`, `C`, `F`, `H`, `W`, ...): a type parameter with the same name wins inside its scope, with a warning (replaces E0605 the error); `K as unit` reaches the unit there.
- `X as unit` stays as a harmless explicit spelling (`float64<tesla as unit>` = `float64<tesla>`).
- A `by` step's sign sets the direction: `10..0 by -2` is 10, 8, 6, 4, 2; a sign that points away from the end gives an empty loop; a zero step stops the program at run time (my reading: a literal zero is a compile error).
- Float ranges with `by`: value k is `start + k*step` (no drift); the loop stops at the last value not past the end.
- `take xs[i] leaving v`: negative indexes count from the end, like `xs[i]` (confirmed).

# Q12 details (array length parameters), 2026-11-13
- A length argument is any compile-time constant expression: `Buffer<sint32, 16>`, `Buffer<sint32, SIZE * 2>`.
- Functions may take length parameters and they are worked out from the argument: `sint32 sum<uint32 N>(&sint32[N] a)`, `sum(arr)` gives `N = 4`.
- A length/value parameter may be any integer type (owner: "any integer btw int64 doesnt exist its either uint or sint"), i.e. `uintN`/`sintN`; the body may use it as a constant of that type (my reading, from the option picked: `uint32 capacity() { return N; }`; non-length uses like `struct Mod<uint64 P>` allowed).

# Step 7 batch A (docs/impl/step7-modules.md Q1-Q4), 2026-11-13
- Q1 module paths: dots or words, two spellings of the same thing: `in game.physics;` = `in physics inside game;`.
- Q2 paths are relative to the importing file's folder; a path from the source root starts with the project's name from the config: `in mygame.game.physics;`. Climbing one folder up: not answered (my reading: write it from the root; no `up` form for now).
- Q3 exports: an `out` list and `out` on a declaration, mixed freely in one file; the list may rename (`out Vector3 as Vec3;`). Re-exporting an imported name: not asked separately (open).
- Q4 a local name that clashes with an imported module shadows it in its scope, with a warning.

# Step 7 batch B (Q5-Q8), 2026-11-13
- Q5 several items: a comma list, each may be renamed: `in Vector3 as V3, gravity from physics;`.
- Q6 two modules with the same file name (`game/audio`, `engine/audio`): an error at the second import, with the fix-it `in engine.audio as engineAudio;`.
- Q6b `X with I { }` blocks and `def`s follow the type: visible wherever the struct or the interface is visible (`in Circle from shapes;` brings Circle's `with` blocks). For `def`s, my reading: visible where the type they define on is visible.
- Q7 `in *` clashes (with `import_clash = warn`): a local declaration beats imports; between imports the first wins; the warning is at the later import line.
- Q8 import cycles are allowed (declarations see each other across files), except cross-file recursion: a function in a.drh that calls into b.drh which calls back into a.drh is an error (owner: "allowed unless code calls itself"; clarified as cross-file mutual recursion, not top-level init loops or types). My reading: recursion within one file stays fine; the error shows the call chain.

# Step 7 batch C (Q9-Q12), 2026-11-13
- Q9 the toml says what a project builds: `[project] name = "geometry"`, `kind = "library"` (`"program"` by default), `entry = "src/main.drh"`. 9b (how a dependency's modules are named) not asked separately: open.
- Q10 scripts: top-level variables are script-wide state that functions can read and change (`uint32 total = 0; none addTo(uint32 n) { total += n; }`). 10b (`main()` in a script) not asked: open.
- Q11 a `//--!$` file flag may set only keys that change one function's code (`overflow`, `unsigned_negate`, `bounds`); the others are project-wide (a flag setting them is an error). A dependency obeys its own drachm.toml.
- Q12 dependencies in step 7: path and git (cloned into a cache folder, tag pinned, a lock file): `sdl = { git = "https://...", tag = "v0.4" }`. Registry entries stay E0901.

# Step 7 batch D (Q13-Q15) and Q12 follow-ups, 2026-11-13
- Q13 first stdlib cut in PLAN order, including env and fs: io (files) -> string -> convert -> math -> env -> fs -> time -> process. 13b/13c not asked: open.
- Q14 both spellings call the same function: `math::sqrt(2.0)` and `2.0::sqrt()`. 14b (`math::pi` vs bare `pi`): open.
- Q15 a failed `assert(add(2, 3) == 5)` prints "assertion failed: add(2, 3) == 5" and the line. 15b/15c: open.
- Named constants: a top-level non-`mut` value with a constant initializer is usable in types and array sizes (`uint32 SIZE = 8; uint8[SIZE] raw;`); `fixed` is optional and makes it a checked constant (an error if the value is not constant): `fixed uint32 MAX = SIZE * 2;` (owner: "fixed would just make it better"). Values from the run (`io::inln()`, function calls) are not usable in types.
- Value-parameter functions keep the borrow at the call: `sum(&arr)` (E0321 rule).
- Any constant arithmetic on value parameters inside types is allowed: `T[N + 1]`, `Buffer<T, N * 2>`, checked per instance.

# Follow-ups, 2026-11-13 (late)
- A range includes its end when the step lands on it, both ways: `10..0 by -2` is 10, 8, 6, 4, 2, 0; `0..10 by 2` is 0, 2, ..., 10 (checked: built that way).
- An unsigned loop with a literal negative step counts down anyway (the step's sign is read before the type), with a warning: `for (uint32 i of 10..0 by -2)` gives 10, 8, ..., 0 (owner: "warn tho"). Today it is W0102 and wraps: to change.
- Step 7 Q10b: a script may define `main()` or have top-level statements, not both (with `main()`, the top level holds declarations only).
- Step 7 Q14b: constants are qualified, `math::pi`, `math::e`. Q15b: `assert` works in normal code and runs in every build, `-O` too.

# Step 7 follow-ups after WP1, 2026-11-13
- The showcase becomes a project folder when imports work across files: `examples/showcase/` with `drachm.toml`, `src/main.drh` and modules; the site page reads all its files.
- `in * from game.audio as snd;` brings every exported name in bare and also under `snd::` (`play()` and `snd::play()` are the same). Owner's note: "in * from x as y means it puts all of it in y" (my reading: everything is reachable as `snd::name`; the bare names come with it as in the option picked).
- Correction (owner, same day): `in * from x as y` puts everything under `y::` only, no bare names, unless written `also bare`: `in * from game.audio as snd also bare;` gives both `play()` and `snd::play()`.
- Climbing folders: a string path relative to the importing file, with `..`: from `src/game/physics.drh`, `in * from "../runtime/scheduler.drh" as scheduler;`. My reading: a string path works wherever a module path does (`in "../runtime/scheduler.drh";`, `in tick from "../runtime/scheduler.drh";`), and without `as` the module is named after the file (`scheduler`).

# Package manager, 2026-11-13
- Owner wants Drachm's own package manager, in the spirit of pnpm and bun, working off git repos until the central registry (the Agora, SPEC 10) exists.
- `drachm add github.com/someone/sdl-drh@v0.4`, `drachm remove sdl-drh`, `drachm update`: the commands write `drachm.toml` and the lock file.
- Downloaded packages live per project, in the project's `deps/` folder (`project/deps/sdl/`), not a global store.
- Versions of git packages are git tags (`v0.4.1`); `@^0.4` picks the newest matching tag.
- Built after step 7, as its own step (7.6). Step 7 keeps plain git dependencies + lock file (WP3).
- Owner's note: "this can link into the C compat stuff" (my reading: a package can wrap a C library, i.e. carry C headers/sources for `in * from c."..." as x;` imports and the link flags, and the manager fetches/builds those too; to design with the C-header step).

# C header import, 2026-11-13
- Scheduled straight after step 7 (PLAN step 7.1), before the package manager (7.6) and the optimiser (7.5).
- Owner on `unsafe`: "idk about making them always unsafe since that might be a pain in the ass to use unless there is no other way around it". SPEC said all C calls are unsafe; this is now open: offer designs where calls are safe when possible.

# WP2 follow-ups, 2026-11-13
- A call through a module may be chained with either `::` or `->`: `physics::gravity(2.0)::text()` and `physics::gravity(2.0)->text()` both work, but only in this situation (the module call does not count as the first call).
- An explicit item import that clashes with a local declaration or another import is an error (E0232), confirmed. Only `in *` clashes warn.
- A local declaration beating an `in *` name stays a warning (W0240), and the message should ask "did you mean to use the module's `X`?" (help pointing at `m::X`).
- All modules are per file, `si` too: a file that uses SI units needs its own `in si;`.

# C bindings and library checks, 2026-11-13
- Safe wrappers around unsafe C calls.
- Native libraries are specified in `drachm.toml`.
- `drachm check` checks a library without requiring `main()`.

# C header imports batch A, 2026-11-13
- The owner delegated import scope. Expose declarations from the named header and its transitive includes under the import alias. This lets umbrella headers expose the API they collect. Repeated declarations of the same C symbol within one import resolve to one declaration.
- Use one `[native]` table in `drachm.toml` for include directories, definitions and libraries.
- Scripts can check C headers with the default search path. Linking requires a project manifest.
- A dependency lists its public module paths in its manifest.
- Consumers automatically inherit native link requirements from the dependencies they import.

# Script state follow-ups, 2026-11-13
- Writes to script-wide variables require `mut`.
- Initialize all top-level declarations before the first top-level statement.

# Filesystem and OS modules
- Report filesystem failures by cause using separate failure types, such as `FileMissing` and `AccessDenied`.
- Start `os` with platform, current directory and environment lookup. Process commands remain separate.
- `fs::write` replaces an existing file. `fs::create` fails if the file already exists.
- `fs::create` takes a path with optional initial text and returns a file object for reading and writing lines. The same object type should serve the filesystem reader and writer APIs.
- `os::getEnv` returns `string?` and gives `none` for a missing variable.
- After `fs::create(path, text)` writes initial text, the returned file's cursor starts at the beginning.
- `fs::open(path, Read or Write or ReadWrite)` opens an existing file and returns the same `File` type in all three modes.
- `fs::open(path, Write)` preserves existing contents and appends writes at the end.
