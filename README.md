# Drachm

A statically typed, natively compiled programming language (files end in
`.drh`). The compiler is written in Rust. Plain builds go through
[Cranelift](https://cranelift.dev) and compile in milliseconds; `-O` builds
generate C for the system C compiler and run as fast as C or Rust.

## Usage

```sh
cargo build --release
./target/release/drachm run examples/hello.drh        # compile + run
./target/release/drachm build examples/hello.drh -O   # optimised ./examples/hello
./target/release/drachm check examples/hello.drh      # errors and warnings only
./target/release/drachm explain W0102                 # describe a code
```

Requires `cc` (gcc or clang) on `PATH`.

## The language so far

```cpp
in io;

uint64 fib(uint64 n) {
    if (n < 2) { return n; }
    return fib(n - 1) + fib(n - 2);
}

none main() {
    uint32 answer = 40 + 2;          // immutable; types are always written
    mut uint32 i = 0;                // `mut` allows changes
    while (i < 3) {
        io::outln(i::text());
        i++;
    }
    io::out("fib(20) = ");
    io::outln(fib(20)::text());
    io::outln((answer::to<float64>() / 4)::text());   // 10.5
}
```

- **Types:** `uint8` to `uint128`, `sint8` to `sint128`, `float32`, `float64`,
  `bool`, and `none` for functions that return nothing.
- **Functions** start with their return type; there is no `fn` keyword.
  `none main()` or `sint32 main()` (returns the exit code).
- **Bindings** are immutable unless `mut`. `uint32 later;` may be assigned once.
- **Numbers** convert automatically; lossy conversions warn (W0102) unless
  written `value::to<T>()`. In `a + b` the left side's type leads. Constant
  expressions are evaluated exactly (`0.1 + 0.2` is `0.3`).
- **Operators:** `+ - * / % **`, `== != < <= > >=`, `&& || !`,
  `& | ^ ~ << >>`, `++ --`, and compound assignment. Overflow wraps.
  Bitwise operators need parentheses next to arithmetic and comparisons.
- **Control flow:** `if (...) { } else if (...) { } else { }`, `while (...) { }`,
  `return`. Conditions are always parenthesised.
- **Loops:** C-style `for (mut uint32 i = 0; i < 10; i++) { }` and inclusive
  ranges `for (uint8 i of 0..255) { }` (safe at the type's ends).
  `break;` / `continue;` take a loop count: `break 2;`.
- **`skip`** leaves `if` blocks the way `break` leaves loops: `skip;` leaves the
  outermost one, `skip 2;` two levels, `skip to next;` goes on to the next
  `else if`. Loops are walls.
- **`match`** on numbers and bools: `case 0 { }`, `case 1, 2 { }`,
  `case 13..19 { }`, `case _ { }`. No fallthrough; missing values and
  unreachable cases are warnings (W0109, W0110).
- **Output:** `io::out` / `io::outln` print text; `::text()` turns a value into text.
- **Runtime safety:** whole-number division by zero panics with exit code 101.

`docs/SPEC.md` has the full design, most of which is still to be built;
`docs/PLAN.md` has the build plan and progress.

## Layout

| Path | Role |
|---|---|
| `src/lexer.rs`, `src/parser.rs`, `src/ast.rs` | source → tokens → syntax tree |
| `src/typeck.rs`, `src/consts.rs`, `src/types.rs` | checking; syntax tree → typed tree (`src/hir.rs`) |
| `src/codegen.rs` | plain builds: typed tree → Cranelift → object file |
| `src/cgen.rs` | `-O` builds: typed tree → C |
| `src/link.rs`, `runtime/runtime.c` | the C compiler driver, and the runtime every program includes |
| `src/diag/` | diagnostics: rendering and the error code table |
| `src/main.rs`, `src/lib.rs` | CLI, and the compiler as a library |
| `tests/` | golden tests: `cargo test` |
| `bench/` | benchmarks against C and Rust: `bench/run.sh` |
