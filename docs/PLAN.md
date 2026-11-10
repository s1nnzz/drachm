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
| 0 | Foundations: lib split, test harness | not started |
| 1 | Migrate v0.1 to Drachm syntax | not started |
| 2 | Strings and structs | not started |
| 3 | Ownership and borrow checking | not started |
| 4 | `for`, `skip`, `match` | not started |
| 5 | Constructors, enums, `fails` / `maybe` | not started |
| 6 | Generics, interfaces, collections | not started |
| 7 | Modules, config, scripts, stdlib | not started |
| 8 | Language server | not started |

Later, unscheduled: concurrency (threads, async, lambdas beyond step 6),
C header import, `drachm fmt`, the Agora registry.

---

## Where things stand (v0.1)

v0.1 works end to end but uses the OLD syntax (`fn name() -> i64`, `let`/`var`,
`i64`, `print`, `.dr` files, no parentheses on conditions).

| File | Role |
|---|---|
| `src/lexer.rs` | hand-written lexer → `Vec<Token>` with byte spans |
| `src/parser.rs` | recursive descent + precedence climbing → AST (`src/ast.rs`) |
| `src/typeck.rs` | types `i64`/`bool`/`()`, mutability, all-paths-return check |
| `src/codegen.rs` | AST → Cranelift IR → object file (`cranelift-object` 0.137) |
| `src/error.rs` | `Span`, `Error`, `file:line:col` rendering with a caret line |
| `src/main.rs` | CLI `build`/`run`/`check`, `-O`; links object + runtime with `cc` |
| `runtime/runtime.c` | `drachm_print_i64`, `drachm_print_bool`, `drachm_div_zero` (exit 101) |

Codegen notes that still apply:
- User functions are mangled `drachm_<name>`; a generated C `main` calls `drachm_main`.
- Cranelift 0.137: `FunctionBuilder::finalize(cfg)` takes `module.isa().frontend_config()`;
  `declare_var(ty)` returns the `Variable`; use `bxor_imm_u`.
- After `return`, emission continues in a fresh unreachable block (`ensure_open`).
- `&&`/`||` short-circuit via a merge block with a block parameter.
- Division by zero checks the divisor and calls the runtime panic.

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

**Open (ask owner):** none.

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

**Open (ask owner):**
- Confirm the string method signatures in SPEC section 4 (they are a draft).
- `io::outf`: literal percent sign (assumed `%%`), width/padding/hex options.

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
9. Remove step 2's deep-copy-on-assign workaround.

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

**Open (ask owner):** how to match a literal `*` in a string pattern (assumed `\*`).

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

**Open (ask owner):** none.

---

## Step 6: Generics, interfaces, collections

Work:
1. Generic functions and structs by monomorphisation; type-argument inference
   from arguments (unification). `<T with Shape>` checks the constraint at the
   definition (only Shape's methods are usable) and at instantiation.
2. Interfaces: `interface X { sigs }`, `struct S with X`, conformance checks
   (missing or mismatched methods get clear errors). Static dispatch for
   generics. Dynamic dispatch via vtables once its syntax is decided.
3. Collections in the stdlib, written in Drachm where possible: `Array<T, N>` /
   `T[N]`, `List<T>` / `T[?]` (push, len, get, indexing, iteration), and
   `Map<K, V>` with `{ k: v }` literals, `[]` returning `V?` and `::get`.
   Bounds checks panic by default.
4. Lambdas `(T x) => expr` and `{ block }`, captures by borrow, `move`. Needed
   for `names::each`.
5. The `chars`/`split`/`lines` return types become real `string[?]`.

**Done when:** generic `max` and `Stack<T>`, an interface with two structs, and
list/map programs from the SPEC all run; constraint violations give clear errors.

Decided (2026-11-10): `&Shape` dynamic dispatch (vtables), `with A and B`,
interface static methods / required fields / `extends` / `Self` (no default
bodies), arrow function types, unspecified map order plus `OrderedMap`,
`Iterable<T>` with `T? next()`, automatic `==`/hashing with `==` override
(SPEC sections 4-5, 8). Interfaces with required fields need the field at a
known offset or an accessor in the vtable: pick one and note it.

Map keys: a `key()` method defines what keys compare and hash by.

**Open (ask owner):** a type with both `key()` and a custom `==`: allowed? which wins?

---

## Step 7: Modules, config, scripts, stdlib

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

**Open (ask owner):** the full `[warnings]` name list (grows as warnings are added).

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

**Open (ask owner):** highlighting colours/theme, if they care.

---

## Later (unscheduled)

- **Concurrency:** `thread::spawn`, `Channel<T>`, `Mutex<T>`. Data-race freedom
  needs Send/Sync-like marker properties: decide with the owner how these
  appear (or stay hidden). Async: `async` functions become state machines,
  `::await` is postfix, and the runtime is built into the stdlib.
- **C header import:** `in * from c."SDL2/SDL.h" as sdl;` via libclang
  (`clang-sys`), mapping C types to Drachm types, all calls unsafe.
- **Maths:** `math` basics; `Vector2/3/4`, `Matrix3/4`, quaternions; big
  integers, fractions, decimal/fixed-point; user-definable units of measure
  (`unit m; unit N = kg * m / s^2;`, `float64<m/s>`, literals `9.6<s>`),
  checked at compile time. Scaled units auto-convert to the first operand's
  unit; SI units ship in std. Open: the maths type names and module.
- **256-bit integers and `float8`/`float16`:** runtime software arithmetic
  where Cranelift lacks native support.
- **`drachm fmt`**, the **Agora** registry (agora.drachm.dev), `drachm add`.
- **Targets:** AArch64, macOS, Windows.
