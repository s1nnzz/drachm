# Drachm Implementation Plan

How to build the Drachm compiler from v0.1 to the language in `docs/SPEC.md`.
Written so a new session with no memory of past conversations can continue.

**Before starting any step:** read `CLAUDE.md`, `docs/SPEC.md`, and this step's
**Open** list. Open items are language decisions for the owner to make. Ask
before building them (rules in `CLAUDE.md`). Engineering choices (data
structures, algorithms, crates) are yours, but note anything that changes what
users see.

**After finishing a step:** tick it in the Progress table, move any newly decided
answers into `docs/SPEC.md` and the design doc, and note leftovers under that step.

## Progress

| Step | Name | Status |
|---|---|---|
| 0 | Foundations: lib split, test harness | done |
| 1 | Migrate v0.1 to Drachm syntax | done (256-bit, float8/16 deferred) |
| 2 | Strings and structs | done (`\u{NAME}` escapes, user `copy`, `split`/`lines`/`chars` later) |
| 3 | Ownership and borrow checking | done: MIR, moves, drops, mutability, borrow checker, `lives` inference, pointers/`unsafe`/`*fixed T` (benchmarks to rerun on a quiet machine; pointer `==` between two pointers left) |
| 4 | `for`, `skip`, `match` | done: `match` on text (exact patterns, `globbed` globs), `for` over characters (`string` copies, `&string` views), lists in step 6 |
| 4.5 | Units of measure | done (`in si;`, points and deltas, scaled integers wrap; decisions in `docs/impl/step4.5-units.md`) |
| 4.6 | Input: `io::inln()`, `io::ended()`, text `::to<T>()` | done (interim: bad text stops the program with exit 101; forgiving number text (spaces, `+`, `.5`); `\r\n` lines; invalid UTF-8 becomes U+FFFD) |
| 5 | Constructors, enums, `fails` / `maybe` | done (the `crash` setting of `unhandled_fail` waits for step 7): S5A.1 done (enums with data, `match` with bindings/`take`/`copy`/guards/nesting/alternatives/values, exhaustiveness, `Switch`; `==` for payload-free enums) and S5B done (constructors, overloads, `new ... of`, named values, `Person(auto);`, `fill`, `fill partial`, `def`, `defaults`, `now`); optionals done (S5A.2, below); failures done (S5C and S5D, below: `failure`, `fails`, `fail`, `maybe`/`fails` on calls, handlers, `because`, `ConvertError`, exit codes, failing constructors and `drop()`) |
| 6 | Generics, interfaces, collections | done (S6Z sweep, 2026-11-13): generic functions, structs and enums (inferred and written type arguments, borrows as type arguments, units); interfaces (`with`, `and`, `extends`, `Self`, `changes self`, built-in `Zero`/`Equal`/`Ordered`/`Text`, `with trusted`, generic interfaces, `&Shape` with vtables, `Iterable<T>` with `at(i)`); arrays, lists and maps (indexing, the methods, `take xs[i] reorders`, views of owning items, scalar and composite keys with `key()`, two-variable walks, `each`); function types, lambdas with inferred captures. The interims still open are listed in the step 6 section, each with its question number |
| 7 | Modules, config, scripts, stdlib | in progress: WP1 done (source map with global offsets, import/export grammar, `src/modules.rs` graph loader, project folders, E0230-E0235); WP2 done (names across files, `src/resolve.rs`); WP9 done (`test` blocks, `assert`, `drachm test`; see the WP9 paragraph); follow-ups 2026-11-13: a call through a module chains with `::` or `->` (done); W0240's help asks whether the module's name was meant (done); `in si;` is per file (done, E0402 in a file without it); spec `docs/impl/step7-modules.md` |
| 7.1 | C header import (`in * from c."SDL2/SDL.h" as sdl;`), right after step 7 | not started; owner, 2026-11-13: scheduled next after step 7; not all calls `unsafe` if avoidable |
| 7.6 | Package manager (`drachm add/remove/update`, git tags as versions, per-project `deps/`, later the Agora; ties into C header packages) | not started; owner answers "Package manager, 2026-11-13" |
| 7.5 | MIR optimiser: SSA, inlining, scalar replacement, constant folding, loop opts, check elimination, alias info from borrows | not started |
| 8 | Language server | partly done (pulled forward): `drachm lsp` (`src/lsp/`, `lsp-server`), diagnostics on change/save for files and projects, parser recovery for unclosed blocks, hover, go to definition and completion from a typeck side table (`src/typeck/index.rs`), VS Code client; left: inlay hint for the inferred `self` mode, imports completion, rename, references, formatting |
| D | Documentation site (cross-cutting) | live locally: lessons 1-9, reference, design pages with status badges |
| D2 | Site redesign | not started: a full visual re-look of the docs site to make everything look super duper sexy (owner, 2026-11-13); see "Site redesign" below |

Later, unscheduled: concurrency (threads and async), `drachm fmt`, the Agora registry.

---

## Where things stand

**Current summary and next steps: `docs/HANDOFF.md`.** The paragraph below is
an older snapshot.

Steps 0, 1 and 2 are done, and the control-flow part of step 4: the compiler
takes Drachm syntax (`.drh`), all number types up to 128 bits, `bool`, `none`,
owned `string`s with interpolation and `io::outf`, structs (fields that may be
strings, methods, operators), `for`/ranges/`break N`/`continue N`/`skip`/
`match`, and has two backends. A string, or a struct holding one, moves when it
is assigned, passed or returned; `copy x` is the only deep copy; drops are
decided on the MIR (`src/mir/drops.rs`) and a `drop()` method runs when its
struct dies (step 3, WP6).

| File | Role |
|---|---|
| `src/lib.rs` | the compiler as a library: `check(src)` → `hir::Program` + diagnostics, `build` → executable |
| `src/lexer.rs` | hand-written lexer → `Vec<Token>`; number literals become exact constants here |
| `src/parser.rs` | recursive descent + precedence climbing → `src/ast.rs`; recovers at `;`, `}` and the next function; enforces the bitwise-parentheses and `::`/`->` rules |
| `src/consts.rs` | untyped constants: exact big-integer / fraction arithmetic |
| `src/types.rs` | built-in types, struct types by id (names interned), lossless-conversion table |
| `src/typeck.rs` | names, types, left-side-leads conversions, W0102, definite assignment, all-paths-return, ownership (copies, temporaries, drops) → `src/hir.rs`; structs, fields, methods, operators |
| `src/typeck/control.rs` | loops, `break`/`continue`, `if` chains, `skip`, `match` (patterns, W0110/W0111), and the flow state along every jump; jumps drop the owning values they leave |
| `src/typeck/strings.rs` | string literals and interpolation, string methods and operators, `io::outf` format parsing |
| `src/hir.rs` | the typed tree both backends consume: every conversion explicit, names resolved, ownership explicit (see its module comment); struct layouts |
| `src/codegen.rs` | plain builds: HIR → Cranelift IR → object file (`cranelift-object` 0.137) |
| `src/cgen.rs` | `-O` builds: HIR → C (one `const` temporary per sub-expression) |
| `src/link.rs` | runs `cc` (`clang` on Windows); caches the compiled runtime in `~/.cache/drachm` (`%LOCALAPPDATA%\drachm` on Windows) |
| `src/diag/` | `Diagnostic`, `Span`, `Source`; `render.rs` (SPEC section 10 layout, colour); `codes.rs` (code table) |
| `src/main.rs` | thin CLI: `build`/`run`/`check`/`explain`, `-O` |
| `runtime/runtime.c` | printing, panics, values as text (numbers, floats, `%.N`, zero padding), float→int wrapping, 128-bit division, integer power. Linked for plain builds, pasted inline for `-O` (with `runtime/strings.h`) |
| `src/typeck/generics.rs`, `interfaces.rs`, `dynamic.rs`, `lambdas.rs`, `lists.rs`, `maps.rs`, `iterable.rs`, `units.rs` | step 6: generic instances and their checks, interface conformance and constraints, `&Shape` borrows and their dispatch, function values and captures, lists and arrays, maps, `Iterable<T>`, units of measure |
| `runtime/lists.h` | the list and map C interface (both backends); the Rust side is in `runtime/strings/` (`src/lists.rs`, `src/maps.rs`, `src/hash.rs`) |
| `runtime/strings.h`, `runtime/strings/` | the string runtime: a Rust staticlib (grapheme-aware methods, live-buffer counter) behind a C interface; built by `build.rs`, linked into every program |
| `tests/run.rs` | golden test harness (how to add a case is at the top of the file); programs run with `DRACHM_CHECK_LEAKS=1` and a leak or a double free fails the case |
| `bench/` | benchmarks with C and Rust twins; `bench/run.sh` prints a comparison |

Backend notes:
- Symbols: functions `drachm_f_<name>`, methods `drachm_m_<len><Struct>_<name>`
  (`op_add` and so on for operators), automatic `==` `drachm_eq_<Struct>`; a
  generated C `main` calls `drachm_f_main`. In C, structs are
  `drachm_s_<Struct>` with fields `f_<name>`, and structs that own strings get
  `drachm_copy_<Struct>` / `drachm_drop_<Struct>`.
- The two backends must agree bit for bit. Anything with room to disagree
  goes in `runtime/runtime.c` and is shared. `optimised_output_matches` in
  `tests/run.rs` builds every run case both ways and compares.
- Cranelift 0.137: `FunctionBuilder::finalize(cfg)` takes `module.isa().frontend_config()`;
  `declare_var(ty)` returns the `Variable`; use `bxor_imm_u`; narrow `iconst`
  immediates are sign-extended; 128-bit values in signatures need the
  `enable_llvm_abi_extensions` flag; 128-bit values cross into the C runtime
  as two 64-bit halves (`isplit`/`iconcat`).
- After `return`, emission continues in a fresh unreachable block (`ensure_open`).
- C backend: all integer arithmetic is done in an unsigned type of at least
  32 bits and cast back, so overflow wraps and C's `int` promotion never
  bites. 64-bit division first tries a 32-bit division when both sides fit.
  Compiled with `-O3 -ffp-contract=off` and no fast-math.

---

## Step 0: Foundations

**Goal:** make the codebase ready to grow and to be checked automatically.

Work:
1. Split into a library crate (`src/lib.rs`: lexer, parser, checker, codegen,
   diagnostics) plus a thin CLI binary. The language server (step 8) and tests
   need the library.
2. A golden test harness (`tests/run.rs`, a cargo integration test):
   - `tests/run/*.drh` are compiled and run; stdout must equal the matching
     `.out` file, and the exit code an optional `.code` file.
   - `tests/fail/*.drh` must fail to compile; stderr must contain each line of
     the matching `.err` file (match on error codes and messages, not exact layout).
   - Make it easy to add a case: one `.drh` plus one expectation file.
3. Diagnostics module: `Diagnostic { severity, code, message, labels: Vec<(Span, String)>, help }`,
   rendered in the SPEC section 10 style (coloured when stdout is a TTY). Every
   phase collects several diagnostics instead of stopping at the first.
   Start the error code table in `src/diag/codes.rs` (code, title, long text for
   `drachm explain`).
4. `git` hygiene: the repo has no commits yet. Ask the owner before the first commit.

**Done when:** `cargo test` runs the existing v0.1 examples as golden tests, and
errors render with codes and labels.

**Step 5 progress, S5A.1 (enums):** `enum` declarations, bare and `Enum::Variant` values (expected type first), `match` with typed bindings,
`take`/`copy` for owning payloads, guards, `case A, B` with agreeing bindings, constants and nested patterns, W0110/W0111 over the
pattern matrix (guarded cases never cover), `Switch` for three or more variants in both backends, drop and copy glue per owning enum.
Interims taken: a guard sees copied bindings but not `take`n ones (they exist after the guard); text constants inside variant patterns
are E0901; a variant cannot hold a borrow or pointer (E0901); no niche optimisation (a `string?` will be bigger than a `string`,
later item under Performance); E0210 reads "`Tree` contains itself" when an enum is in the cycle.

**Step 5 progress, S5A.2 (optionals):** `T?` is an enum per distinct `T` (`src/typeck/optionals.rs`, variants `none` = tag 0 and
`Some` = tag 1), with `none` as a value, `uint32? x;` starting as `none`, a `T` wrapping to `T?` wherever a `T?` is expected
(owner Q5), `x has value` / `x is empty`, `case uint32 a` / `case none` (W0110 says "does not handle a value"), and
`x maybe fallback` as an HIR node `ExprKind::Maybe` lowered in `mir/build.rs` (E0530-E0534). Interims taken: `maybe` on an
owning optional in a variable is E0901 (only rvalues and Copy places); `none` as a constructor argument is E0530 (the
constructor's parameter types are not used for it yet; a `T` argument does wrap); `T??` and spelled-out `Optional<T>` are E0901;
`x == none` is E0533 (the owner's `is empty` fix-it). Also fixed: a number with a unit is Copy in a `case` binding.

**Open (ask owner):** none.

Done (2026-11-10). Notes for later steps:
- Every phase takes `&mut Vec<Diagnostic>` and never stops at the first error.
  The type checker runs only if lexing and parsing were clean. Expressions with
  an error inside have type `None`, so one mistake is reported once.
- Error codes share one number space between `E` and `W`: `00xx` syntax, `01xx`
  types and conversions, `02xx` names/declarations/bindings, `03xx` ownership.
  In use: E0001-E0005, E0101, E0103-E0105, E0201-E0205 (W0102, E0301, E0302 are
  reserved by the SPEC). Every new code needs an entry in `src/diag/codes.rs`.
- Messages quote code with backticks. The message texts and `explain` texts
  still talk about v0.1 syntax (`var`, `fn`); step 1 rewrites them.
- Tests accept `.dr` and `.drh` until step 1 renames everything; then drop
  `"dr"` from `EXTENSIONS` in `tests/run.rs`.
- `DRACHM_BLESS=1 cargo test` regenerates `.out`/`.code` files.
- Speed baseline (`bench/run.sh`, Drachm `-O` vs `cc -O2` vs `rustc -O`):
  `fib` 1238 ms vs 298 / 569 ms; `primes` 4984 ms vs 4591 / 2919 ms. Cranelift
  does no inlining, loop optimisation or recursion elimination, so call-heavy
  code is 2-4x behind LLVM/GCC. See "Performance" below for the decision.

---

## Step 1: Migrate v0.1 to Drachm syntax

**Goal:** this program compiles and runs natively:

```cpp
in io;

uint32 add(uint32 a, uint32 b) {
    return a + b;
}

none main() {
    mut uint32 total = 0;
    while (total < 10) {
        total = add(total, 3);
    }
    io::outln(total::text());
}
```

Work:
1. Lexer: new keywords (SPEC section 1), `::`, `->`, `++`, number suffixes
   (`5u8`, `5s`, `2.5f32`, `5.0f`), decimal literals. Rename the extension to `.drh`.
2. Parser:
   - Declarations start with a type: `Type name(...) { }` is a function and
     `Type name = expr;` a binding (both may start with `mut`). Parse a type,
     then an identifier, then look at `(` vs `=`/`;`.
   - Conditions require parentheses.
   - `in io;` is parsed but only `io` is accepted for now (full modules come in step 7).
   - Uninitialised `mut uint32 x;`.
3. Types: all of `uint8..uint128`, `sint8..sint128`, `float32`, `float64`, `bool`, `none`.
   - Untyped constants: evaluate constant expressions exactly (use `i128`/`f64`,
     or a big-int for 256-bit later), then type by result (SPEC section 4).
   - Automatic numeric conversion at assignments, arguments and returns, with
     W0102 on lossy ones, plus `::to<T>()` for explicit conversion.
   - Overflow wraps (Cranelift's default `iadd` etc.). Config comes in step 7.
   - Definite assignment: reading a maybe-unassigned binding is an error.
4. Defer `uint256`/`sint256`, `float8` and `float16` to the end of this step or
   later. 128-bit uses Cranelift's `I128`. 256-bit and `float8` need software
   arithmetic in the runtime.
5. Builtins: `io::out`, `io::outln` on strings. Until strings exist (step 2),
   allow string *literals* only, and give numbers `::text()` as a special case
   lowered to a runtime print. Remove `print`.
6. `none main()` exits 0; `sint32 main()` returns the exit code.
7. Operators: `**`, bitwise `& | ^ ~ << >>`, `++ --`, all compound assignments;
   `/` on integers truncates; integer division by zero panics, float follows IEEE.
   Unary `-` on unsigned wraps (config `unsigned_negate` arrives in step 7).
8. `/* */` block comments (not nested).
9. Port all v0.1 examples and fail tests to the new syntax.

**Done when:** the program above runs; fail tests cover a type mismatch,
assigning to an immutable binding, a missing return, reading an unassigned
binding, and a lossy-conversion warning.

**Open (ask owner):** none (all answered 2026-11-10).

Done (2026-11-10), except `uint256`/`sint256`, `float8` and `float16`, which
report E0901 "not supported yet". Decisions made during this step are in SPEC
sections 2 and 4 (operator grouping, left side leads, constants, conversions,
shifts, literals, `++`/`--`, float text). Notes for later steps:
- Overflow behaviour is hard-wired to **wrap**: constants that don't fit,
  runtime conversions, shifts. Step 7 adds the `saturate` and `panic` modes;
  the places to change are `fit_const`/`check_shift_amount` in `typeck.rs`,
  `Convert`/`Shift` in both backends, and the `drachm_f64_to_*` helpers.
- Strings exist only as the argument of `io::out`/`io::outln` (`Type::Str`,
  `ExprKind::Str`/`Text`). Step 2 replaces this with real strings.
- `while (true)` counts as never finishing (for missing-return and
  assignment checks) unless a `break` targets it (done in step 4).
- Shown to the owner as drafts, not yet confirmed: the wording of W0102,
  W0106 and the new errors; `inf`/`-inf`/`nan` as float text; constant
  division by zero being a compile error for decimals; `<<` next to `>>`
  needing parentheses.
- Speed (`bench/run.sh`, ms, Drachm `-O` / C `-O3` / Rust `-O3` / plain):
  collatz 328 / 378 / 387 / 855; fib 97 / 145 / 232 / 435; mandel 169 / 225 /
  205 / 221; primes 922 / 1573 / 895 / 1713.

---

## Step 2: Strings and structs

**Goal:** the first values that own heap memory.

Work:
1. Runtime: move the runtime from C to a Rust `staticlib` crate (`runtime/`)
   so it can use `unicode-segmentation` for graphemes. Keep the C ABI
   (`extern "C"`, `#[no_mangle]`). Link the `.a` instead of compiling C.
2. String representation: `{ ptr, len_bytes, cap }`, UTF-8, owned. Literals live
   in read-only data and are copied on first ownership. Grapheme-aware `len`,
   `sub`, `reverse` and `chars` go through the runtime.
3. All string methods in SPEC section 4, each returning a new string. Method
   call syntax: `::` for the first call and `->` for the rest. Enforce that order
   in the parser, with a helpful error and a fix-it.
4. String joining with `+`, `..` and `::join`; `::text()` on numbers and bools;
   `f"..."`, backtick and `"""` strings with `${expr}` interpolation; escapes
   including `\u{1F600}` and `\u{UNICODE NAME}` (embed a name table, e.g. the
   `unicode_names2` crate); `io::outf` where `%` prints any value and `%.N`
   sets decimals (check argument count at compile time when the format is a literal).
5. Structs: fields, `public:`/`private:` sections (unlabelled: fields private,
   methods public), methods with implicit `self`, operator methods named by
   symbol (`Vector3 +(Vector3 o)`),
   `static` methods, field access `self.x` / `p.x`. Lay out structs C-style.
   Large structs are passed by pointer in the ABI.
6. Values are destroyed at the end of their scope, in reverse declaration order.
   Moves are not tracked yet, so to stay safe in this step every assignment or
   pass of a string deep-copies. Step 3 replaces this with real moves.

**Done when:** golden tests cover every string method (including emoji and
accents), interpolation, `outf`, and structs with methods.

Also in this step: the field-order and named-field construction shorthand
(`Vector3 p = 1, 2, 3;` / `Vector3 p = z: 3, x: 1, y: 2;`, never mixed), so
structs are usable before full constructors arrive in step 5.

Decided (2026-11-10): string method table confirmed, plus an optional fill
text on `padLeft`/`padRight`; `io::outf` literal percent is `\%`; outf options
are width/padding (`%8`, `%-8`, `%08`) and bases (`%x %X %b %o`) besides `%.N`.

**Open (ask owner):** none before the step. Found while building it (each
has a conservative behaviour for now, marked `OPEN` in the code where it is
one place). Strings:
- `"""` text is kept exactly as written between the delimiters, leading
  newline and indentation included. Strip the first newline? Dedent?
- In `io::outf`, a letter right after a placeholder (`%d`, `%s`, `100%x`):
  `x X b o` always pick a base, any other letter is an error (E0009). A digit
  right after `%` is always a width (`"%5 apples"` pads), and `%` before a
  space is a placeholder (`"50% off"` prints a value). Is that the rule?
- `%%` is rejected (E0009, "write `\%`") rather than read as two placeholders.
- `%0` (a width of 0) and `%-08` (zeros on the right) are errors.
- `%08` on non-numbers is an error; on `inf`/`nan` it pads with spaces (as C does).
- A `\` at the end of a line inside a string is an error (no line continuation).
- `${...}` may contain string literals with the same quote (`f"${a + "!"}"`):
  the lexer reads the inner expression as ordinary tokens.

Structs (decided while building, not by the owner; conservative errors where unsure):
- after a field access the chain starts over: `p.pos::len()`, `p::get().x::text()`;
- a field and a method of one struct may not share a name (E0202);
- one method per operator symbol (no overloading by parameter type);
- `==` / `!=` methods must take the struct's own type, and comparison
  operator methods must return `bool` (E0113);
- a written `!=` method is used for `!=`; otherwise `!=` is `!(a == b)`;
- the automatic `==` compares struct fields with their own `==` (custom or
  automatic), and floats by IEEE (`NaN != NaN`);
- `self = ...` is an error (E0005), as is calling a method that changes
  `self` on a temporary value (E0205);
- fields have no default values; a field cannot be `static`, a method cannot be `mut`;
- a struct may have no fields; a one-field struct can be built from one
  value (`Wrap w = 42;`);
- operator methods never get W0208 (they cannot be `static`).

Strings and structs together (found when the branches were merged):
- A struct with a `string` field is copied like a string: every assignment,
  pass and return deep-copies it (the step 2 rule; SPEC section 6 says such
  structs move, which step 3 brings).
- `${p}` and `io::outf`'s `%` on a struct are errors (E0101), even when the
  struct has a `text()` method (the help suggests calling it). Should they
  call it?
- The automatic `==` compares string fields by their text (bytes), like `==`
  on strings.
- `match` on a struct value is an error (E0101: numbers and bools only).

Done (2026-11-10). Strings and structs were built on separate branches and
merged; the notes below are per part, then what joining them took.

Strings. Built: `string`/`str`,
all literal forms (`"..."`, `f"..."`, backticks, `"""`, `f"""`) with
`${expr}` and the `\%` escape, `+`/`+=`/comparisons, `::text()` on
everything, every SPEC method except `split`/`lines`/`chars` (E0901: they
need lists), `io::outf` with every option. Not built: `\u{UNICODE NAME}`
escapes (still E0008), user `copy x`. Notes for later steps:
- **Ownership lowering (for step 3).** Typeck makes it explicit in the HIR and
  the backends add nothing (module comment in `src/hir.rs`). String
  expressions are *places* (`Local`, `Str`, `Temp`; borrowed) or *owned
  values* (everything else). Inserted nodes: `Copy(place)` (the deep copy on
  every assignment, argument and return; user `copy x` does not exist yet),
  `Move(local)` (only for the hidden local in `s = v`, which becomes
  `$t = v; Drop(s); s = Move($t)`), `Temp(local, value)` (an owned value
  stored in a hidden local so it can be borrowed), `Scoped(expr, locals)`
  (evaluate, then drop: statement temporaries die at the end of their
  statement, and `return v` drops everything in scope after making `v`),
  and `Stmt::Drop(local)` at the end of each block in reverse declaration
  order (parameters at the end of the function). Every string local starts
  empty and `Drop` empties it, so dropping one that was never set (an
  unassigned binding, a temporary inside the unevaluated side of `&&`) is
  harmless: that stands in for drop flags. Step 3 removes `Copy`, `Move`,
  `Scoped` and `Drop` from typeck and lets MIR build them; `Temp` maps to a
  MIR temporary. `s += t` is `StrCall(Append, [Local s, t])`, an in-place
  append (`drachm_str_append`, amortised growth).
- **ABI.** Cranelift: a string is a 24-byte stack slot (`drachm_str`). A
  string parameter is passed as a pointer to a value the callee takes over
  (it copies the 24 bytes into its own slot and drops it); a string result
  goes through a hidden first pointer parameter. C backend: `drachm_str` by
  value both ways. Runtime functions always take pointers.
- **Runtime split.** Unicode work is in the Rust library (`runtime/strings/`,
  declared in `strings.h`); number/float/bool text, `%.N` (`drachm_text_fixed`,
  libc `%.*f`, so ties round to even on the binary value) and `%0N` padding
  are in `runtime.c`, shared by both backends. `drachm_panic` is never
  `static` (the Rust library calls it) and exits with `_Exit(101)`, so a
  panic is not reported as a leak. `-O` output pastes `strings.h` in place of
  runtime.c's `#include "strings.h"`.
- **Leak checking.** The runtime counts live buffers; `tests/run.rs` runs every
  program with `DRACHM_CHECK_LEAKS=1` and fails a case that prints
  `drachm: leaked N strings`.
- **`io::outf`** needs a plain literal format (anything else is E0901). The
  lexer keeps string pieces apart (`lexer::StrPiece`: raw text, escapes,
  interpolations), so only a `%` written directly is a placeholder and errors
  point into the format. The call becomes one `Out` of a `Join` of pieces.
- New codes: E0009 (bad placeholder), E0108 (option does not suit the value).
- Index arguments are `sint64`, counts `uint64`, so passing a `uint64` (such
  as `s::len() - 1`) to `sub` warns (W0102); negative indexes avoid it.
- Speed (`bench/run.sh`, ms, Drachm `-O` / C / Rust / plain): strings 254 /
  95 / 145 / 244 (the others unchanged: collatz 328, fib 119, mandel 167,
  primes 860). The string runtime skips grapheme segmentation for plain ASCII.
  What remains is allocation: each turn of the loop makes five strings (the
  number's text, the interpolation, `repeat`, the `+`, and `sub`) where Rust
  makes two. Writing an `f"..."`'s pieces straight into one buffer, and
  appending `+` into a temporary that is about to die, would remove most of
  them; elsewhere step 3's moves remove the deep copies.

Structs: item 5 and the construction shorthand. Golden tests `structs`,
`structcalls`, `operators`, `layout`, `structwarn`; fail tests
`structsyntax`, `structdecl`, `structuse`, `structmut`. Notes for later steps:
- HIR: `Type::Struct { id, name }` (the name is interned so `Type` stays
  `Copy`); `hir::Program::structs` holds each layout (C-style, computed once
  in `hir::layout`). New expressions `Field`, `Struct` (field values in
  evaluation order); `Stmt::Assign` and `IncDec` take a `Place` (local plus
  field path). Methods are ordinary `hir::Func`s; `has_self` makes parameter
  0 the receiver, which refers to the caller's storage when the argument
  `is_place()`. Operators on structs become `Call`s of the operator method;
  automatic `==` is a generated function per struct (`drachm_eq_<Name>`).
- ABI: Cranelift keeps structs in stack slots and passes them by address; a
  struct result is written through a hidden first parameter. Every struct
  expression yields a fresh copy, so a callee owns its struct arguments'
  memory. The C backend emits real C structs (in containment order), passes
  them by value and `self` as a pointer.
- Copies are whole-value for now; step 3 adds moves and the ≤ 32-byte copy
  rule. Step 3 should also reuse the "changes `self`" analysis in `typeck.rs`
  (`SelfUse`, `report_fixed_receivers`) for the `&self` / `&mut self`
  inference, and replace the "temporary receiver" error with real borrows.
- Step 5: the construction shorthand currently always means "all fields in
  declaration order", ignoring visibility. Constructors, `new T of (...)` and
  the automatic constructor disappearing must replace `construct` in typeck.
- New codes (renumbered when merged with strings and control flow): E0112
  (no such operator), E0113 (invalid operator method), E0114 (struct
  construction), W0208, E0209-E0213. E0205's explanation now covers fields and
  changing methods. The decisions taken here are in the Open list above.

Strings and structs together. Golden tests `struct_strings`, `struct_flow`;
fail test `structstrings`.
- **Owning types.** `string` and every struct with a `string` field
  (directly or in a struct field; `StructDef::owns`, `Program::owns`) are
  *owning*: everything the strings notes above say about string places, owned
  values, `Copy`, `Move`, `Temp`, `Scoped` and `Drop` holds for them. Places
  are `Local`, `Temp` and `Field` of a place (plus `Str` for strings). Plain
  structs keep the struct branch's rule: any expression, copied by bits.
- `Stmt::Drop` takes a `Place`, so `p.name = v` becomes `$t = v;
  Drop(p.name); p.name = Move($t)`, and `p.name += t` an in-place `Append` on
  the field. A field of an owning struct that is not a place (`make().name`) is read
  from a `Temp`; so is a method's receiver.
- Method receivers of owning structs are always places (by address); the
  right side of an operator method is an owned copy. The automatic `==`
  works like a method: `drachm_eq_<Name>` has `has_self`, borrows its left
  side and drops its (copied) right side.
- Cranelift: a struct parameter's memory is the caller's fresh copy, which the
  callee takes over and drops; `copy_bits` / `deep_copy` / `drop_value` /
  `clear` walk a type's leaves (numbers, bools, strings). Assigning a plain
  struct still makes the value apart and copies it in, since the value may
  read the place; an owning place is always empty when assigned, so the value
  is written straight into it.
- C: owning locals start `= {0}`; owning arguments are made into their own
  variable like strings.
- Checked with `-fsanitize=address,undefined` on every run case (no reports)
  besides the leak counter.
- Speed: unchanged by the merges. Measured side by side with the pre-merge
  compiler (Drachm `-O`, best of 7, on a machine running about 2.7x slower
  than for the numbers above): collatz 935 / 871 ms, fib 284 / 279, mandel
  482 / 475, primes 2488 / 2505, strings 770 / 769 (before / after).

---

## Step 3: Ownership and borrow checking

**Goal:** moves, copies, borrows and lifetimes are checked at compile time
(SPEC section 6).

Work:
1. Add a mid-level IR (MIR) between the AST and codegen: a control-flow graph of
   basic blocks with explicit places (locals, fields), moves, copies, borrows,
   drops and calls. Lower the AST to MIR, and MIR to Cranelift. Every later
   analysis (moves, borrows, definite assignment, `skip` lowering) is easier on
   a CFG.
2. Copy vs move: a type is Copy if it is a number or bool, or a struct whose
   fields are all Copy with a total size ≤ 32 bytes. `copy x` deep-copies.
3. Move checking (dataflow "maybe moved" per place): E0301 use after move,
   with labels at the move and the use, and a `help:` suggesting `copy`.
4. Drop elaboration: drop only places that are initialised and not moved. Use
   drop flags where it depends on the path taken.
5. Borrows `&` / `&mut`: a non-lexical lifetime borrow checker (liveness-based,
   in the style of Rust's NLL). Shared XOR mutable. A borrow must not outlive
   its owner.
6. Lifetime inference for function signatures. Returning a borrow derived from
   exactly one borrowed parameter is inferred. Ambiguous cases ask for
   `lives a, b`. Check `lives` clauses at the definition and use them at call
   sites.
7. Infer `self`'s borrow mode from method bodies: a method that assigns
   `self.x` or calls a mutating method is `&mut self`, otherwise `&self`.
   Error messages should say which was inferred and why.
8. `unsafe { }` and `*T`, gated by the `//--!$unsafe: true` file flag. This
   needs the file-flag parser, which the full config later reuses.
9. Remove step 2's deep-copy-on-assign workaround. (Done in WP6.)

**Done when:** fail tests cover use after move, mutation through `&`, two live
`&mut`, `&` and `&mut` overlapping, returning a borrow of a local, a missing
`lives`, and unsafe without the flag. Golden tests show no double frees (run
with a runtime allocation counter in debug builds).

Decided for this step (2026-11-10): `mut` vs `open` bindings, `mut` fields
(always changeable), explicit `&mut` at call sites, borrowing struct fields
with `struct X lives a { }`, no auto-copy of pointer-holding structs, the
E0302 message style (SPEC section 6). Write the other borrow errors in that same style.

**Open (ask owner):** wording of any new error kinds beyond E0301/E0302, shown as drafts.

---

## Step 4: `for`, `skip`, `match`

Work:
1. C-style `for (init; cond; step)` and for-each `for (T x of xs)`. For-each needs
   an iteration protocol; strings and ranges first, lists in step 6.
2. `skip`, `skip N`, `skip to next`, with the exact rules in SPEC section 3.
   Lower in MIR by keeping a stack of "if levels" (each records its chain-exit
   block and its next-branch block). Loops push a wall. `match` cases push a
   level whose next branch is the next case's test. Compile errors for N too
   large, `skip` outside any `if`, and `skip to next` with no next branch.
3. `match` on integers, bools and strings: `case a, b`, `case _`, no
   fallthrough. The W-code warning for non-exhaustive matches.
4. `break N` / `continue N` (N loops, default 1), inclusive ranges `a..b`,
   match patterns: values, lists, ranges, strings, `*` globs (`"/api/*"`).

**Done when:** golden tests cover every `skip` example in the SPEC, including
walls, `match` levels and `skip to next` inside a case.

Decided: `skip to next` from the last branch is a compile error; lambdas are
walls for `skip`.

**Answered 2026-11-12:** a plain string pattern matches exactly (`*` is a character); `globbed "..."` makes `*` a wildcard. (Was: how to match a literal `*`, assumed
`\*`). Found while building control flow (conservative for now):
- range loop variables must be integers (floats and `bool` are an error);
- `_` must be alone in its `case`;
- an empty range pattern (`case 9..2`) is an error rather than a warning;
- the count after `break`/`continue`/`skip` must be a plain literal without a suffix;
- a `case` value must be an untyped constant expression or a typed literal
  with `-`, `~`, `!` (typed arithmetic such as `5u8 + 1u8` is rejected as not
  a constant);
- W0110 lists missing values only for `bool` and 8/16-bit integers;
- constant `true`-valued conditions other than the literal `true` (e.g.
  `while (1 == 1)`) still count as possibly ending the loop.

Control flow done (2026-11-10): C-style `for`, for-each over inclusive integer
ranges, `break N` / `continue N`, `skip` / `skip N` / `skip to next`, and
`match` on integers, floats and bools (values, lists, ranges, `_`). Golden
tests: `tests/run/{loops,ranges,skip,match,matchwarn}.drh`, `tests/fail/{jumps,
loops,patterns,flow,forsyntax}.drh`. Notes for later steps:
- Step 4 done (2026-11-12): `match` on `string` (plain patterns are exact;
  `case globbed "/api/*"` is a glob, `*` the only wildcard; W0110/W0111 by the
  partial coverage rule) and `for (string c of text)` (an owned copy per
  character, grapheme clusters, via `drachm_str_byte_len` / `drachm_str_next_char`)
  and `for (&string c of text)` (a borrowed view, `drachm_str_char_view`).
  Not done, optional and behaviour-neutral: the owner's "skip the copy when the
  body never keeps `c`" optimisation. Lists come in step 6 (via `Iterable<T>`).
- HIR: an `if` chain and a `match` are both `Stmt::If { label, branches }`
  (a branch with no test always runs); every loop is `Stmt::Loop { label,
  cond, body, step }`; jumps are `Stmt::Jump(Break | Continue | Leave | Next)`
  naming a label. `match` evaluates its value into a hidden local and turns
  each case into a test. Cranelift gives each branch test and each loop
  step/exit its own block; the C backend uses `goto` to labels
  `ifL_bN` / `ifL_end` / `loopL_next` / `loopL_end` (forward jumps only).
- A range loop is lowered to `if (start <= end) { counter = start; loop {
  x = counter; body; step: if (counter == end) break; counter += 1; } }`,
  so it never overflows at a type's largest value.
- The exact `skip` counting rule is written at the top of `src/typeck/control.rs`.
  A chain (`Stmt::If` in the AST holds all its `else if`s) is one level.
- Strings (added when merged with step 2): every jump (`break`, `continue`,
  `skip`) first drops the owning locals (strings, structs holding strings) of
  each scope between it and its target, innermost and latest first
  (`Target::owned` records the target's scope depth). Conditions, `match`
  values and range bounds drop their temporaries right after they are
  evaluated. A C-style `for`'s init binding is dropped after the loop.
  Golden tests `string_jumps`, `struct_flow`.
- Flow: each loop and level collects the flow state at every jump to it;
  `while (true)` / `for (;;)` fall out of the bottom only through a `break`.
  `skip to next` joins into the next branch's entry state (so an immutable
  binding assigned before it and again in the next branch is an error).
- Exhaustiveness works on intervals of an order-preserving key per type
  (signed integers with the sign bit flipped, floats by bit pattern with
  `-0.0` = `0.0`). Floats are only ever covered by `case _` (NaN).
- New codes (renumbered when merged with strings): E0010 (range out of
  place), E0011 (`break`/`continue` target), E0012 (`skip` target), E0013
  (`for` step), E0109 (invalid pattern), W0110 (missing case, the SPEC's
  `missing-case`), W0111 (unreachable case).
- The conservative choices made here are in the Open list above.

---

## Step 4.5: Units of measure

Owner (2026-11-11): bring units forward from "Later" as their own small step
after step 3, numbers only (no generics needed): units are compile-time tags on
number types, checked by typeck, erased before codegen (zero runtime cost).
SPEC section 4 has the decided basics (`unit m;`, derived `unit N = kg * m /
s^2;`, scaled `unit km = 1000 m;`, `float64<m/s>`, literals `9.6<s>`, scaled
units convert to the first operand's unit, SI units in std). Implementation
spec and decisions: `docs/impl/step4.5-units.md`.

Done (2026-11-11): all owner answers Q1-Q15 are implemented (`src/units.rs`,
`src/typeck/units.rs`, `src/prelude/si.drh`). Not built: unit-generic functions
(step 6, Q6); the mutability message for a unit variable shows the bare type
(`mut float64 d`), because units are erased before the MIR.

**Open (ask owner):** none.

---

## Step 5: Constructors, enums, `fails` / `maybe`

Work:
1. Constructors (SPEC section 5): methods named after the struct, overload
   resolution by argument types (exact match first, then lossless conversion,
   then lossy with a warning). Error on ambiguity. The automatic field-order
   constructor exists when none is written.
2. The declaration shorthand `Type x = a, b, c;` resolves to a constructor;
   `new Type of (...)` works anywhere.
3. Enums with data: tagged-union layout (tag plus a payload sized to the largest
   variant). Bare variant names resolve from the expected type. `match` with
   typed bindings. Exhaustiveness warning listing the missing variants.
4. `Optional<T>` as a stdlib enum; `T?` sugar; `none` as a value;
   `uint32? x;` starts as `none`.
5. `failure` declarations (variants with message strings, interpolated from
   their fields, plus optional `exit N`), `because` cause chains, and exit codes
   when a failure escapes `main`. Failures: `fails E1, E2` in signatures, `fail X;`, and at call sites `maybe v`,
   `fails`, `fails with (E e) => { ... }` (chained, one per type), and `maybe v;`
   inside handlers. Lower a failing function to return a
   `{ tag, union(value, error) }` pair (no unwinding). `return` inside a
   handler leaves the enclosing function. `maybe` on optionals.
6. The `unhandled_fail` check: compile error by default. The `crash` mode is
   wired up in step 7.

**Done when:** golden tests cover constructor overloads, the field-order
constructor, enum matching, optionals with `maybe`, and every failure form,
including several error types and propagation through two functions.

Decided (2026-11-10): `failure` declarations, ambiguity rule for bare
`fail X`, lazy `maybe` with any expression, failing constructors, optional
patterns `case uint32 a` / `case none`, `because` chains (SPEC section 7).

Also: `fails with (any e)` and the `otherwise (any e)` arm; `${e}` prints a
failure's message.

Decided (2026-11-11): `fill p { }` / `fill partial p { }` blocks for structs
declared without a value, and `def` templates of default values (`def X of
Player { }`, `struct Player defaults X { }`, shorthand `def Player { }` +
`struct default Player { }`, `fill p with defaults { }`). Details in
`docs/impl/owner-answers.md`.

Done (S5B, 2026-11-12, `docs/impl/step5-B-constructors.md`): constructors
(`src/typeck/construct.rs`), the shared overload ranking (`src/typeck/overload.rs`,
exact, lossless, lossy; ties E0502), `new Type of (...)`, the automatic
constructor (kept only when no constructor is written, or `Person(auto);`),
named values matching parameter names, `fill` / `fill partial` / `now`, `def`
with `from`, `struct ... defaults` (`src/typeck/fill.rs`). Initialisation checks
in the MIR (`Mark::InitWrite`, `hir::CheckKind`) reuse the per-field analysis in
`mir/moves.rs`; no new analysis pass. Interims: delegation between constructors
is E0901; `fails` on a constructor is E0901; `fill` only of local bindings (E0901
for parameters, fields, `self` and derefs); `def` values must not read fields.

**Open (ask owner):** none.

---

## Step 6: Generics, interfaces, collections

**Status: done (2026-11-13, the S6Z sweep).** Built by the packages S6A (generics),
S6B (interfaces), S6E (`&Shape`, vtables), S6F (lambdas), S6R and S6C (lists),
S6D (maps) and S6G (`Iterable`), with owner answers in `docs/impl/owner-answers.md`
("Step 6 questions" and the follow-ups). The overview is `docs/impl/step6-overview.md`.
Reference pages: [Generics](../site/content/reference-generics.md) and
[Arrays and lists](../site/content/reference-collections.md) (in the site
sources); lessons 15 and 16 run as tests.

Compiler files: `src/typeck/{generics,interfaces,dynamic,lambdas,lists,maps,iterable,units}.rs`,
vtables in `src/mir/mod.rs` and `src/typeck/dynamic.rs`, the list and map runtime in
`runtime/lists.h` with the Rust side in `runtime/strings/` (`src/lists.rs`, `src/maps.rs`, `src/hash.rs`).

Interims still open (each is E0901 unless noted; the question says what the owner decides):
- `exclusive &Shape` (Q5): the owner's word; details still to design.
- Borrowing a generic interface, `&Read<uint32>` (Q3 family; E0901).
- Interface methods with type parameters of their own, and struct methods with
  their own type parameters (Q15).
- A constraint on a unit parameter, `unit U with Shape` (Q2 family; E0901).
- Value parameters (Q12) are built, with named constants and constant arithmetic
  in types; a constraint on one and one on an interface are E0901. Top-level
  values of the run (non-constant, `mut`) wait for scripts (step 7).
- `take xs[i] leaving v` (Q11; keeps length and order; E0901).
- Views of map entries, `for (&K k, V v of m)` (Q16 follow-up; E0901).
- A `&mut T x` loop variable over `Iterable<T>` (E0901: `at` hands out values).
- Writing a required field through `&mut Shape`, methods returning borrows through
  `&Shape`, holders and lists of borrows as `&Shape` items (`==` and `contains` on
  them): E0901 (S6E).
- Borrowed captures in lambdas and a borrow returned from a lambda; mutable item
  callbacks for `each`; changing a fixed `move` capture (E0901; tentative in the site).
- A generic function called with a borrow type argument, `same<&string>(&s)`, and a
  generic struct used as a field of a non-generic struct (E0901).
- Enums and generic instances promising an interface from outside (E0901).
- Config settings `bounds` and `missing_key` (step 7).

Leftover follow-ups (owner, 2026-11-13, "Step 6 leftover follow-ups"):
- SI unit `T` is `tesla` (done, commit "Rename the SI unit T to tesla").
- A type parameter named like a unit is warning W0618 (was E0605); inside its
  scope the plain name is the type (E0402 where a unit is meant); `X as unit`
  names the unit anywhere (done).
- Range `by` sign: a negative step counts down, a step pointing away gives an empty
  loop, a run-time zero step stops with `range step is zero` (exit 101), a literal
  zero is E0115 (done). Open: `10..0 by -2` (see SPEC: 0 is included by the rule).
- Float ranges with `by`: value k is `start + k * step`, stop at the last value not
  past the end; without `by` E0101 (done).

Decided, in progress in another package (not built, and not described as built
anywhere in this sweep): `maybe &T` (an optional that holds a borrow), `a same as b`
and `xs::containsRef(&p)`. They are in `docs/impl/owner-answers.md` (Q3 follow-ups).

Decided and built (see `docs/SPEC.md` sections 4, 5 and 8): `&Shape` dynamic
dispatch (vtables); `with A and B`; interface static methods, required fields,
`extends` with `and`, `Self`, no default bodies; arrow function types; `Map` order
unspecified with `OrderedMap` in insertion order; `Iterable<T>` with `T? at(uint64 i)`
(no walker); `==`/hashing automatic, with `key()` deciding map keys even when
`==` is custom (Q13).

The answered questions are in `docs/impl/owner-answers.md`. Q1 to Q20 are all answered;
the interim behaviour that differs from an answer is listed above.

---

## Step 7: Modules, config, scripts, stdlib

WP5 first slice: bare and configured scripts can use a shebang and ordered
top-level statements with local imports, or keep explicit `main`. Both
backends use the regular native executable path. Shared runtime declarations
and unresolved generated-entry exits remain E0901.

Work:
1. Modules (SPEC section 9): resolve `in` paths against `src/` and dependencies.
   `out` export lists. Aliases. `in *` with clash detection (`import_clash`).
   Every module is one file. Build a module graph; detect cycles.
2. `drachm.toml` and `srachm.toml` parsing (`toml` crate). `[behaviour]`
   settings change codegen: overflow wrap/saturate/panic, bounds checks,
   `missing_key`, `unhandled_fail`. `//--!$key: val` file flags override per file.
3. Projects vs scripts: find the nearest `drachm.toml` / `srachm.toml`. Script
   mode allows top-level statements (lowered into a generated main),
   auto-imports core, crashes on unhandled failures and hides warnings.
4. CLI: `drachm new`, `build`, `run`, `check`, `test` (runs `test "name" { }`
   blocks; `assert`), `explain`. Dependencies by path first, then git; the
   Agora registry later.
5. Stdlib modules in priority order: `io` (files too), `string`, `convert`, `math`,
   `env`, `fs`, `time`, `process`. Then `thread` (Channel, Mutex) and `net`/`http`,
   with async as a sub-project (see Later).

**Done when:** a multi-file project with aliases builds; a script with no
toml runs top-level code; behaviour settings visibly change overflow and bounds
behaviour in golden tests.

Decided (2026-11-10): scripts JIT in memory (`cranelift-jit`), shebang
lines, local imports, deps in `srachm.toml` and via `//--!$dep name: ver`,
`os-cpu` target names, `drachm fmt` with few knobs and no `=` alignment.

**Step 7 progress (WP1, built):** `diag::SourceMap` (one byte-offset space, first
file at 0, so single-file spans and output are unchanged; `render_all_map`),
`Import` with path / items / glob / alias and `Program.exports`, `modules::load`
(owner paths: relative to the importing file's folder, project name for the
root, dots = words, cycles load each file once), `drachm build|run|check
FOLDER`. Interims (narrowest readings): the project name is read from
`[project] name` of `drachm.toml` by a small scanner until WP3 (`toml` crate);
no way to climb a folder up; re-exporting an imported name and `out` on a
`with` block or `def` are E0901 / rejected; the same-name import check (Q6)
covers module imports, item clashes wait for WP2; cross-file mutual recursion
(Q8) is a WP2 check. Next: WP2 (scoped names), then the rest.

**Step 7 progress (WP3, built):** `src/config.rs` reads `drachm.toml` and
`srachm.toml` (`toml_edit`, not the `toml` crate: it keeps spans on keys and
values for the diagnostics): `[project]` name / kind (`program` default,
`library`) / entry (`src/main.drh`), `[dependencies]` by `path` or `git` with an
exact `tag`, `[build]` target / opt, `[warnings]` as_errors (stored only),
`[behaviour]` six settings (stored only, WP4 applies them). Codes E0800-E0814
(08xx). `config::open` replaces the scanner: a folder is a project, a file takes
the nearest config above it, and both `drachm.toml` and `srachm.toml` in one
folder is E0807. `kind = "library"`: `run` and `build` are E0811, `check` runs
the library checks without requiring `main`. `src/deps.rs`: path deps are folders; git deps clone into
`deps/NAME/` of the project (owner, 2026-11-13: per project, not a user cache),
`drachm.lock` pins the commit (`[[git]]` url, tag, commit), a moved tag is E0809
only when the commit must be fetched, two sources under one name E0804, cycles
E0810. `Opened.deps` gives the dependency roots (name, dir, src, own config);
wiring their modules into `in` paths is WP2. `drachm new NAME` (E0805, E0813),
the `Command` enum in `main.rs` (flags only where they apply; input defaults to
`.`). File flags: `overflow`, `bounds`, `unsigned_negate` are checked and kept
per file (`FileFlags.behaviour`; WP4 plumbs them), `missing_key`,
`unhandled_fail`, `import_clash` in a file are E0806. Interims: no registry
(E0901); cross target E0901; `[build] opt = true` acts as `-O`.
Tests: `config` and `flags` unit tests, `tests/deps.rs` (local `file://` repos),
`tests/cli.rs` (`new` then `run`, library, entry, errors).

**Step 7 progress (WP2, built):** `src/resolve.rs` resolves every name of the
module graph (scope per file: own declarations, module bindings, imported
items, `*`) and joins the files into one `ast::Program` that `typeck::check`
takes unchanged; the entry file keeps bare names, other files' declarations are
`MODULE::name`. Parser: `module::Name` in types, `new`, `fails`, handlers,
patterns, array lengths and paths; `out` before a named constant; string file
paths (`in "../x/y.drh"`, may climb) and `in * from m as y also bare`.
Owner's rulings applied: `in * ... as y` is under `y::` only, `also bare`
for both. New codes E0236 (private name), E0237 (no such name), E0238 (files
call each other in a circle), W0240 (imported name hidden), W0241 (name hides a
module); E0232 also covers clashing item imports. The showcase is a project
folder `examples/showcase/` (expectations in its `expect.out`); site examples
may be folders (`site/examples/NAME/`, `expect.out`). Interims: calls through
methods are not followed by the recursion check; `//--!$unsafe` of the entry
file decides for all files. Each file using SI units needs its own `in si;`.
A local declaration or item clashing with an explicit item import is E0232
(only `*` clashes warn). Applying `import_clash = error` waits for WP4.

**Step 7 progress (WP4):** `bounds` is applied in shared MIR lowering. File
flags override the project default, negative indexes still count from the
end, and constant array indexes are checked by the type checker.
`unsigned_negate` is applied during type checking. Signed negation of
`uint128` is E0901 until `sint256` is supported. `missing_key = "panic"`
uses the same checked map lookup as a compound assignment.

**Step 7 progress (WP9, built):** `assert(cond);` is a statement (`Stmt::Assert`,
lowered to an `if` that stops with `StopKind::Assert`). It runs in every build,
`-O` included. A failure prints `drachm: panic: assertion failed: TEXT` and
`  --> FILE:LINE` on stderr and exits 101; the text is the condition as written,
the place is filled in when the files are loaded (`hir::Assertion`, `lib.rs`
`locate_asserts`). `assert(cond, "text");` (owner, 2026-11-13) prints
`assertion failed: COND: text`; the text is a plain string literal (E0003). `test "name" { }` is a function that takes nothing and returns
nothing (`ast::Func.test`), named `test "name"` in the program, listed in
`hir::Program.tests`. Test names are unique per file (E0239). A test in a file
with top-level statements is E0901. `drachm test FILE|FOLDER [-O]` builds one
program per test (its function as `main`, `lib::build_test`) and runs each with
its output captured: a failing test does not stop the others, and only a failing
test's output is shown. Exit 1 if any test failed. Libraries have tests too.
Tests: `assert` golden cases in `tests/run` and `tests/fail`, `test` blocks in
`tests/run/test_blocks_build.drh`, the showcase's `rules` tests, `tests/cli.rs`.
Interims (narrowest readings, owner to decide; Q15): no name filter (the path
is the only argument); a failing test does not stop the others; the
test bodies are compiled into normal builds (type-checked and compiled, never
run); `drachm test` builds once per test, so a large suite builds slowly; the
`[warnings] as_errors` names (Q15c) are unchanged.

**Step 7 progress (WP9 follow-ups, 2026-11-13, built):** `assert(cond, "text")`
(see above). `expect failure F from call;` is a statement of a `test` block
(`Stmt::ExpectFailure`, E0014 outside a test). Typeck lowers it to a labelled
`if` with no test: a `Handled` call whose arm for `F` does `Leave` (the test
goes on) and whose other arm stops with `Assertion`s for "did not fail" and
"a different failure"; the `Stop` after the call is reached only when the call
did not fail. E0545 when the call can never raise `F`, E0003 when it cannot
fail, E0540 when `F` is not a failure. Shared MIR fix found on the way: the
result temp of a `Handled` call is owned only from the join on, so a `return`
or `Leave` in a handler no longer drops an unassigned value (it was a MIR
validation error before, for any `return` in a handler of a value call).
Tests: `tests/fail/expect_*.drh`, `tests/run/handler_early_leave.drh`,
`tests/run/expect_failure_build.drh`, `tests/cli.rs`
(`test_expect_failure_passes_or_stops_the_test_with_a_message`), the showcase's
`rules` tests, `site/examples/test-blocks.drh`.

**Open (ask owner):** the full `[warnings]` name list (grows as warnings are added).
Also: a `deps/`
folder in `.gitignore` or committed? Whether `drachm new` should write a
`.gitignore`. Dependency imports report E0901 while Q9b (module names and exports) is open.

---

## Step 7.1: C header import

Owner, 2026-11-13: scheduled straight after step 7, before 7.6 and 7.5 ("that's
when proper stuff can be done with the lang"). SPEC design: `in * from
c."SDL2/SDL.h" as sdl;` read via libclang (`clang-sys`), C types mapped to
Drachm types. Imported C calls are unsafe. Drachm code exposes safe wrappers,
and projects specify native libraries in `drachm.toml`. The implementation spec
still needs decisions on pointers, structs, enums, unions, macros, callbacks,
strings, and ownership of returned memory.

---

## Step 7.6: Package manager

Owner, 2026-11-13 (owner-answers "Package manager"): Drachm's own package
manager in the spirit of pnpm/bun, built after step 7. `drachm add URL@^0.4`,
`drachm remove`, `drachm update` write `drachm.toml` and the lock file; packages
live per project in `deps/`; versions are git tags (`^` ranges pick the newest
matching tag); git repos until the Agora registry exists. Packages may wrap C
libraries (headers, sources, link flags) for `in * from c."..." as x;`.
**Open:** lock file format, resolution of shared transitive deps (one version
or several), `deps/` in git or ignored, build scripts for C packages, Agora
protocol.

---

## Step 7.5: MIR optimiser

Owner (2026-11-11): make Drachm fast without leaning on C or Rust. Plain
builds keep Cranelift for instruction selection and register allocation; the
optimising happens on the MIR, so plain builds approach `-O` and `-O` can
eventually stop going through C.

After step 7 because inlining across modules needs modules, generics
(step 6) decide what gets specialised, and the MIR must be stable first.

Work, roughly in payoff order, each pass measured with `bench/run.sh`:
1. SSA form for the MIR (and back out for the backends).
2. Inlining (small and single-use functions; most of the `fib` gap).
3. Scalar replacement of structs (fields in registers, no memory copies).
4. Constant propagation and folding, dead code and unreachable block removal.
5. Loop-invariant code motion, common subexpressions, induction variables.
6. Removing proven-unneeded checks (division by zero, later list bounds) and
   no-op drops/moves.
7. Alias information from the borrow checker: a `&mut` never aliases, which
   C cannot assume without `restrict`.

`-O` via C stays as the reference until plain builds match it on every bench.

**Open (ask owner):** whether `-O` keeps going through C after this step, or
plain/optimised both use the MIR optimiser with different pass sets.

---

## Step 8: Language server

Work:
1. `drachm lsp` using `tower-lsp` (or `lsp-server`), reusing the library from step 0.
2. The parser must recover from errors (keep going after mistakes) so half-typed
   code still yields an AST: synchronise on `;` and `}`.
3. Features in order: diagnostics on save/change; completion (bare enum variants
   from the expected type, required by the SPEC; methods after `::`/`->`;
   imports); go to definition; hover with types; inferred `self` borrow mode as
   an inlay hint.
4. A VS Code extension with a TextMate grammar for highlighting `.drh`.

VS Code first (decided).

**Progress (pulled forward, 2026-11-13).** Built: `drachm lsp` (`src/lsp/`;
`lsp-server` + `lsp-types`, a synchronous loop, no async runtime), diagnostics
from the open buffers (debounced 250 ms on change, at once on save; a buffer
overlay in `modules::with_overlay`, so no temp files) for single files and project
folders (a file of a project is checked as part of the project); parser recovery
(an unclosed `{` keeps the block and struct read so far, the CLI output is
unchanged); hover, go to definition and completion from a side table that the
checker fills while `typeck::index::collect` is running (`src/typeck/index.rs`,
hooks are one-liners at the places names and types are resolved); the VS Code
client. With syntax errors the checker still runs on the recovered tree, for the
table only (its errors are dropped). Completion patches the buffer under the
cursor (receiver kept with a placeholder member, or a placeholder name) and
analyses that. Not built: the inferred-`self` inlay hint, imports completion,
rename, references, formatting, builtin method signatures in completion.

**Open (ask owner):** highlighting colours/theme, if they care.

---

## Documentation site (cross-cutting)

Goal: a browsable site for people learning Drachm, separate from the design
documents. Lives in `site/`: a Next.js site built with Fumadocs and exported
as static files. Look: like the nextjs.org docs (minimal, monochrome, Geist
fonts, thin borders); only a slight nod to alloy-luau.github.io, which the
owner finds too busy.
Content is plain Markdown in `site/content/`; Drachm code is highlighted by
Shiki using the VS Code TextMate grammar in `editors/vscode/syntaxes/`, so the
editor and the site always agree.

Rules:
- Only document what the compiler accepts today. Designed-but-not-built
  features go on one "Coming later" page, marked by step.
- Every example on the site is a real program, checked by `cargo test`
  (examples live in `site/examples/*.drh` with expected output beside them).
- Update the site in the same commit as the feature it describes.

Pages: introduction and install, a tour (hello world through structs and
`match`), one reference page per area (types and numbers, strings, structs and
methods, control flow, errors/diagnostics, the CLI), and the error index
(generated from `src/diag/codes.rs`, matching `drachm explain`).

Decided (2026-11-11): repo stays private. 2026-11-12: the docs site is published
publicly at https://drachm-lang.github.io (built site pushed to the public repo
`drachm-lang/drachm-lang.github.io`, branch `gh-pages`) by
`.github/workflows/docs.yml` on every push to master that touches `site/`.

**Open (ask owner):** domain (docs.drachm.dev?) once it goes public.

---

**Step 5 progress, S5C (failures):** `failure` types are enums (kind `Failure`) whose variants all start with a hidden cause word;
a function that `fails` has `hir::Func.ret` = an outcome enum (`Ok(value)` tag 0, `Fail(failure)` tag 1, or a `FailSet` with one
variant per listed type), the written type stays in the checker's `Sig::ret`. `message`/`exit_code` are synthesised per failure and
set (`src/typeck/failures.rs`); a failing `main` is wrapped by a synthesised `drachm_main_wrapper` that calls `Report` and returns the
exit code (0-255, the failure's code wins over main's number). `x maybe v` is one syntax for optionals and failing calls (`FailMaybe`
in the HIR); `call fails` is `PassUp`; text `::to<T>()` is `TryParse` (status then value, `ConvertError` `Empty`/`NotANumber`/
`OutOfRange`, replacing the exit-101 interim). Interims: a `drop()` or constructor that `fails` is E0901; `a != b` with a failing `==`
is E0901; failure values hold numbers, bools and strings; `match`/`copy` on a failure and a failing function returning a borrow are
E0901; `fails with` is E0901 until S5D; a cause can only come from a failure value in a variable (no handlers yet).
Benchmark baseline (`bench/fails.*`): see the Performance section.

**Step 5 progress, S5D (handlers):** `call fails with (T e) => { }` chains, one handler for each failure type (single-type calls take the
failure whole), `fails with (any e)`, `otherwise (any e)` last (E0545/E0546/E0547/E0550/E0553, W0111, E0544 listing unhandled types),
`fails with none` and `ignore failure` (calls returning nothing). `hir::ExprKind::Handled` lowers like `maybe`: the failure is moved
out of the outcome, a `Switch` on the set tag picks the arm, `e` is a local owned by the arm; `maybe v;` is `Stmt::Resume` (assigns the
result temp, unwinds the arm's frames, jumps to the join); `fail e;` is `Stmt::Reraise`; an arm of a value call that can fall off its
end is E0547 (found while building, by reachability of the arm's last block). `${e}`, `e::message()`, `e::exit()`, `match (e)` and
`because e` work on a caught failure (also a set). `a != b` with a failing `==` takes `maybe`/`fails` (the fallback is the value of the
`!=`); handlers after it stay E0901. Failing constructors: signature gets an outcome; the shorthand `Port p = 8080 maybe ...` (single
value only) and `new Port of (...)` take the suffix after the value; `self` is declared so a `fail` drops the fields set so far. A
failing `drop()`: glue calls a synthesised wrapper (returns `none`) that reports the failure and ends the process at once with its exit
code (`ReportExit`, `_exit`, so live values are not freed). Interims: `e::cause()` and `copy` of a failure are E0901; the list shorthand
`Port p = 1, 2 maybe x` is not built; `Callee::Parse` was already replaced by S5C's status call.
**Open (owner):** what `e::cause()` returns; whether `maybe` after a list shorthand should apply to the whole list.

## Site redesign (cross-cutting, owner 2026-11-13)

A full re-look of the docs site so every page looks "super duper sexy" (owner's
words). Supersedes the earlier "minimal, like nextjs.org docs" brief as the
target look, once started. Scope to agree with the owner before starting: a
visual direction (mockups first), typography and colour, code blocks and the
showcase, lesson/reference/design page layouts, the landing page, light and
dark themes, phone layouts. Not scheduled; ask the owner when to slot it in.

## Performance (cross-cutting)

S5C baseline (`bench/fails.*`, four loops in one program: failing leaf 100M calls, 3-level chain 20M, string outcome 2M, plain control 100M; best of 5): drachm -O 609 ms, C struct-by-value 450 ms (135%), Rust Result 305 ms (199%), plain Cranelift build 891 ms. No gate; S5E may improve it.

S6Z benchmarks (`nice -n 19 env CARGO_BUILD_JOBS=3 sh bench/run.sh`, best of 5, 2026-11-13;
the machine was shared with other work, load 4-5, so treat the numbers as rough). Times in ms:

| bench | drachm -O | C | Rust | plain (Cranelift) | -O vs C | -O vs Rust |
|---|---|---|---|---|---|---|
| collatz | 419 | 424 | 387 | 763 | 98% | 108% |
| collections (sum, index, generic, map, dyn cases) | 427 | 348 | 410 | 406 | 122% | 104% |
| dyn | 131 | 199 | 369 | 251 | 65% | 35% |
| map | 1363 | 561 | 1065 | 1245 | 242% | 127% |
| fails | 438 | 346 | 286 | 752 | 126% | 153% |
| fib | 187 | 183 | 302 | 392 | 102% | 61% |
| mandel | 181 | 166 | 189 | 215 | 109% | 95% |
| primes | 1488 | 1994 | 1178 | 1488 | 74% | 126% |
| strings | 292 | 106 | 180 | 268 | 275% | 162% |

Against the overview's targets (reported, not gates): `collections` is within 1.3x
of C (122%) and of Rust (104%); `map` is within 2x of Rust's `HashMap` (127%, against
the C open-addressing twin it is 2.4x, so the `map` target is read against Rust);
`dyn` is within 1.5x of Rust (35%, faster). Not separated: the five `collections` cases
are one benchmark file, so the per-case rows the brief asked for are not split out here.
`map` and `strings` are the rows to look at next (outside step 6 scope to fix now).

Goal from the owner: benchmark speed on par with Rust. Measure with
`bench/run.sh` after every step that touches codegen, and add a benchmark
whenever a step adds something worth measuring (strings, structs, lists).

Decided (2026-11-10): **Cranelift for plain builds and scripts, generated C for
`-O`.** With `-O` the compiler emits C and hands it to the system C compiler
(`cc -O2` or better), which is already needed for linking. Rules:
- Both backends must give identical program behaviour; the
  `optimised_output_matches` golden test checks every run case both ways.
- The C must pin down Drachm semantics itself: wrapping overflow (do signed
  arithmetic through unsigned casts or use `-fwrapv`), checked division, exact
  integer widths (`<stdint.h>`, `__int128`), IEEE floats (no `-ffast-math`).
- Mark functions `static` so the C compiler can inline freely.

**Open (ask owner):** none.

---

## Later (unscheduled)

- **Concurrency:** `thread::spawn`, `Channel<T>`, `Mutex<T>`. Data-race freedom
  needs Send/Sync-like marker properties: decide with the owner how these
  appear (or stay hidden). Async: `async` functions become state machines,
  `::await` is postfix, and the runtime is built into the stdlib.
- **C header import:** `in * from c."SDL2/SDL.h" as sdl;` via libclang
  (`clang-sys`), mapping C types to Drachm types. See step 7.1.
- **Maths:** `math` basics; `Vector2/3/4`, `Matrix3/4`, quaternions; big
  integers, fractions, decimal/fixed-point; user-definable units of measure
  (`unit m; unit N = kg * m / s^2;`, `float64<m/s>`, literals `9.6<s>`),
  checked at compile time. Scaled units auto-convert to the first operand's
  unit; SI units come with `in si;` (done in step 4.5). Open: the maths type names and module.
- **256-bit integers and `float8`/`float16`:** runtime software arithmetic
  where Cranelift lacks native support.
- **`drachm fmt`**, the **Agora** registry (agora.drachm.dev), `drachm add`.
- **Targets:** AArch64, macOS. (Windows x86-64 works: clang with the MSVC linker and libraries.)
