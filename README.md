# Drachm

A statically typed, natively compiled programming language. The compiler is
written in Rust and uses [Cranelift](https://cranelift.dev) to produce machine
code, linked into a standalone executable with the system C compiler.

## Usage

```sh
cargo build --release
./target/release/drachm run examples/hello.dr        # compile + run
./target/release/drachm build examples/hello.dr -O   # produce ./examples/hello
./target/release/drachm check examples/hello.dr      # type-check only
./target/release/drachm explain E0101                # describe an error code
```

Requires `cc` (gcc or clang) on `PATH` for linking.

## The language (v0.1)

```drachm
fn fib(n: i64) -> i64 {
    if n < 2 { return n; }
    return fib(n - 1) + fib(n - 2);
}

fn main() -> i64 {
    let x: i64 = fib(20);   // immutable binding; types are always explicit
    var i: i64 = 0;         // mutable binding
    while i < 3 {
        print(i);
        i = i + 1;
    }
    print(x > 1000 && true);
    return 0;               // becomes the process exit code
}
```

- **Types:** `i64`, `bool`, `()` (no value). Every binding and parameter is annotated.
- **Bindings:** `let` (immutable), `var` (mutable), block-scoped, shadowing allowed.
- **Control flow:** `if` / `else if` / `else`, `while`, `return`.
- **Operators:** `+ - * / %`, `== != < <= > >=`, `&& || !` (short-circuiting), unary `-`.
- **Functions:** `fn name(a: T, ...) -> T { ... }`; omit `-> T` for `()`.
  Non-`()` functions must return on every path (checked at compile time).
- **Entry point:** `fn main()` or `fn main() -> i64`.
- **Builtins:** `print(i64 | bool)`.
- **Runtime safety:** division by zero panics with exit code 101.
- Comments: `// ...`

## Layout

| File | Role |
|---|---|
| `src/lib.rs` | the compiler as a library |
| `src/lexer.rs` | source → tokens |
| `src/parser.rs` | tokens → AST (recursive descent, precedence climbing) |
| `src/typeck.rs` | type checking, mutability, return-path analysis |
| `src/codegen.rs` | AST → Cranelift IR → object file |
| `src/diag/` | diagnostics: rendering and the error code table |
| `src/link.rs`, `runtime/runtime.c` | linking, and the builtins linked into every program |
| `src/main.rs` | CLI driver |
| `tests/` | golden tests: `cargo test` |
| `bench/` | benchmarks against C and Rust: `bench/run.sh` |

v0.1 above predates the full design. See `docs/SPEC.md` for the language as designed and `docs/PLAN.md` for the build plan.
