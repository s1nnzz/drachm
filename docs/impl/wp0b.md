# WP0b: file flags

Implementer: **a cheaper model is fine** (self-contained, about 150 lines of
Rust). Runs in parallel with WP0a and WP1 phase A in its own worktree.
Background: `step3-ownership.md` 3.9 and section 4 (E0318, W0319).

## Goal

Read `//--!$key: value` lines at the top of a source file, report bad ones, and
hand `unsafe_allowed` to the type checker. Nothing else changes.

## Files you own (and may edit)

- new `src/flags.rs`
- `src/lib.rs`: **only** the `pub mod flags;` line (put it between `pub mod
  diag;` and `pub mod hir;`) and the body of `check()`.
- `src/ast.rs`: add one field, exactly as written here, as the last field of
  `pub struct Program` (WP0a adds the identical line; identical text merges
  cleanly):

  ```rust
      /// The file flag `//--!$unsafe: true` is present (set by `lib::check`).
      pub unsafe_allowed: bool,
  ```

  and in `src/parser.rs` the one place that builds `Program { .. }` (line 12) gets `unsafe_allowed: false,`.
- `src/diag/codes.rs`: add `E0318` and `W0319` **after the `E0901` entry** at
  the very end of the `codes!` block (WP0a inserts its codes before `E0901`,
  so the hunks do not touch).
- new tests (below).

Do not touch `typeck*`, `hir.rs`, backends, `main.rs`, `tests/run.rs`.

## Build

```rust
// src/flags.rs
pub struct FileFlags { pub entries: Vec<FileFlag>, pub unsafe_allowed: bool }
pub struct FileFlag { pub key: String, pub value: String, pub span: Span }
/// Scans `src`; pushes E0318 / W0319 into `diags`. Never fails.
pub fn parse(src: &str, diags: &mut Vec<Diagnostic>) -> FileFlags;
```

Rules (spans are byte offsets into `src`, like the lexer's):

1. If the first line starts with `#!`, skip it (step 7 will need this).
2. From there, every consecutive line that starts with `//--!$` is a flag line:
   `key: value`; `key` matches `[a-z_][a-z0-9_]*`; the value is the rest of the
   line, trimmed. The first line that is not a flag line (a blank line counts)
   ends the block. A flag line with no `:` or a bad key: E0318 "malformed file
   flag" at the line.
3. Any later line that starts with `//--!$` (anywhere after the block): E0318
   "file flags must be at the very top of the file", label "this flag comes
   after code", help "move it above the first line of code, with the other
   flags". It is not applied.
4. Known key `unsafe`: value `true` / `false`, else E0318 "`unsafe` must be
   `true` or `false`, found `yes`" (label on the value). `unsafe_allowed` is
   true only for a valid `true`. Duplicate keys: the last valid one wins.
5. Unknown key: warning W0319 "unknown file flag `unsfe`"; help "did you mean
   `unsafe`?" when a known key is within edit distance 2 (a `similar()` helper
   exists in `typeck.rs`; copy the idea, do not import it).
6. Texts and `explain` strings for E0318 / W0319: `step3-ownership.md` section 4.

`lib::check` becomes: `let flags = flags::parse(src, &mut diags);` first;
after `parser::parse`, `program.unsafe_allowed = flags.unsafe_allowed;`. Flag
errors count as errors (the existing `has_errors` early return applies).

## Tests

Unit tests in `src/flags.rs` (`#[cfg(test)]`): shebang then flag; three flags in
a row; blank line ends the block; flag after code (E0318); `unsafe: yes`
(E0318); `unsfe: true` (W0319 with the help); no flags at all (empty entries,
`unsafe_allowed == false`); spans point at the right bytes.

Golden tests:

- `tests/fail/own_flag_value.drh`
  ```
  //--!$unsafe: yes
  none main() { }
  ```
  `.err`: ``error[E0318]: `unsafe` must be `true` or `false`, found `yes` ``
- `tests/fail/own_flag_late.drh`
  ```
  none main() { }
  //--!$unsafe: true
  ```
  `.err`: `error[E0318]: file flags must be at the very top of the file`
- `tests/run/own_flag_ok.drh` with `.out` = `ok`, `.warn` =
  ``warning[W0319]: unknown file flag `unsfe` ``
  ```
  //--!$unsafe: false
  //--!$unsfe: true
  in io;
  none main() { io::outln("ok"); }
  ```

## Done when

`cargo test` is green (all existing tests unchanged); the three golden tests and
the unit tests pass; `drachm explain E0318` and `W0319` print text; both
backends are untouched, so `optimised_output_matches` needs no attention.

## Pitfalls

- The lexer already treats `//--!$` lines as comments; do not change it.
- CRLF files: compute spans from byte offsets, and trim `\r` from values.
- A blank first line means no flags at all (the block must start at line 1,
  after an optional shebang).
- Do not make a flag warning/error depend on `unsafe` being used: that check is
  WP0a's E0317.
