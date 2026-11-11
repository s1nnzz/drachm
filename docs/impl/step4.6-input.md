# Step 4.6: input (`io::inln`, `io::ended`, text `::to<T>()`)

Implementer: **standard model.** Plumbing through every layer (runtime, typeck,
HIR, MIR, both backends, the test harness, the site) with no new analysis. The
two places that need care are the runtime's reading (flushing, end of input,
UTF-8) and keeping the two backends identical.

Read first: "Input, 2026-11-11" in `owner-answers.md`; `wp6.md` (the backends
today) and `step3-ownership.md` 2.9 (how a runtime call is made in both
backends); `step4.5-units.md` Q11 (`::to<T>()` on numbers), and the code that
the existing `io::out` and `::text()` go through:
`src/typeck.rs` (`io_call`, `value_method`), `src/typeck/strings.rs`
(`string_method`), `src/hir.rs` (`ExprKind::Out`, `Text`, `StrFn`),
`src/mir/{mod,build,print,validate}.rs` (`Callee`), `src/codegen_mir.rs` and
`src/cgen_mir.rs` (the `Callee::Out` and `Callee::Text` arms),
`runtime/runtime.c`, `runtime/strings.h`, `runtime/strings/src/lib.rs`,
`tests/run.rs`. Where this brief differs from the owner's text, the owner's
text wins; where it fills a gap it says **interim** and the point is listed in
the language questions at the end.

## What the owner decided

- `io::inln()` returns the next line of standard input as a `string`, without
  the newline.
- At end of input it returns `""`; `io::ended()` (a `bool`) says whether input
  is finished: `while (!io::ended()) { string line = io::inln(); ... }`.
- `line::to<uint32>()` parses text. It can fail on bad text, so it becomes a
  failing call with step 5 (`line::to<uint32>() maybe 0`). Until then bad text
  stops the program with a runtime error, exit code 101, naming the text and the
  type.

## Parallelism with WP7, WP8, WP9 and the units work

Yes, it can run in parallel with all of them, and it can merge before or after
any of them. Where it touches the same files (resolve by rebasing; each hunk is
disjoint):

| File | Others touching it | Your hunk |
|---|---|---|
| `src/codegen_mir.rs`, `src/cgen_mir.rs` | WP7 step 1 (place addressing, `Rvalue` arms, struct glue) | three new `Callee` arms next to `Callee::Out` |
| `src/mir/mod.rs` | WP7 (`LivesSig`) | three new `Callee` variants |
| `src/mir/build.rs`, `validate.rs`, `print.rs` | WP8 (one line in `build.rs`) | new arms in the `Callee`/`ExprKind` matches |
| `src/typeck.rs` | WP7 (flag renames), WP9 (`address_expr`, `resolve`) | `io_call` only |
| `src/typeck/units.rs` | U5/U6 | the `ExprKind` child-walk (about line 1267) needs an arm for the new HIR node: two lines. Tell the units implementer, or rebase onto theirs |
| `tests/run.rs` | WP7 (adds one test function) | the helpers `run_program`, `run_case`, `site_example`, `optimised_output_matches` |
| `site/content/order.json`, `coming-later.md` | WP7/8/9 | one entry |

## Files you own

New: `tests/run/in_*.drh` + `.out` (+ `.in`, `.err`, `.code`), `site/content/reference-io.md`,
`site/examples/input-*.drh` + `.in` + `.out`, `tests/fail/in_*.drh` + `.err`.

Edit (listed above): the runtime, typeck (`io_call`, `string_method`), HIR,
MIR enum and its three printers, both backends, `tests/run.rs`,
`tests/backends.rs` (give its spawned programs a null stdin),
`site/lib/remark-example.ts` (show `NAME.in`), site pages below, `docs/SPEC.md`
(the part that lists `io` functions and `::to`; **not** section 4, which the
units work owns), `docs/PLAN.md` (the progress row for 4.6).

Do not touch `src/diag/codes.rs`: this step needs no new code. The messages use
existing ones (E0103 arguments, E0206 `io` not imported, E0201 unknown function,
E0101 wrong type, E0901 not supported yet).

## What to build

One commit per step; `cargo test` green after each.

### Step 1. The test harness learns stdin

Do this first: every later test needs it.

- Convention: **`NAME.in` next to a test or example is that program's standard
  input** (`tests/run/NAME.in`, `tests/examples/NAME.in`, `site/examples/NAME.in`).
  Without one the program's stdin is **null** (`Stdio::null()`): reading gives
  end of input at once and a test can never hang waiting for the terminal.
- `run_program(exe, stdin: Option<&Path>)`: `Command::new(exe).env("DRACHM_CHECK_LEAKS", "1")
  .stdin(match stdin { Some(p) => Stdio::from(File::open(p)?), None => Stdio::null() }).output()`.
  A file as stdin needs no writer thread and cannot deadlock.
- Use it in `run_case` (look for `expect("in")`), in `site_example` (both builds
  read the same file), and in `optimised_output_matches`. Update the doc comment
  at the top of `tests/run.rs` to describe `NAME.in`.
- `tests/backends.rs` and any other `Command::new(&exe)`: give them null stdin.
- A test with a `.in` file but whose program never reads is fine.
- Add a self-test of the harness: a run case that prints a line count of its
  `.in`, which is the first test of step 5.

### Step 2. Runtime (`runtime/runtime.c`, `runtime/strings.h`, `runtime/strings/src/lib.rs`)

Reading uses C stdio, in `runtime.c` (the same `stdin` for both backends; the
Rust library must not read standard input itself, or two buffers would disagree).

```c
// Reads one line of standard input into `out` (owned, counted), without the
// line ending ("\n" or "\r\n"). At end of input writes the empty not-owned string.
DRACHM_FN void drachm_in_line(drachm_str *out);
// 1 when no more input is left (peeks one byte).
DRACHM_FN uint8_t drachm_in_ended(void);
```

- Collect the raw bytes of the line (`getline` with `_POSIX_C_SOURCE 200809L`
  defined at the very top of `runtime.c`, or an `fgets` loop; verify the `-O`
  build, which pastes `runtime.c` at the top of the generated C, still compiles
  with whichever you pick; lines may contain NUL bytes and be arbitrarily long,
  so `fgets` needs a length-aware loop).
- Validity: a `string` is always valid UTF-8 (strings.h). Hand the bytes to a new
  Rust function `drachm_str_from_bytes(drachm_str *out, const uint8_t *p, uint64_t n)`
  in `runtime/strings/src/lib.rs` that allocates through the counted allocator
  and replaces each invalid sequence by U+FFFD (**interim**, language question 2).
  Pure ASCII lines take a fast path (one copy, no re-validation beyond a check).
- Line ending: remove a final `\n`, and then a `\r` directly before it
  (**interim**, language question 2). A last line without a newline is returned
  as a line. A trailing newline does not make an extra empty line: after
  `"a\nb\n"` the lines are `a`, `b`, and then `ended()` is true. Blank lines in
  the middle are lines (`""`).
- `drachm_in_ended`: `getc` then `ungetc`; `EOF` means ended. It may block on a
  terminal until the user types something; that is intended.
- **Prompts.** If standard input is a terminal (`isatty(0)`, tested once and
  cached), `fflush(stdout)` before every blocking read, so `io::out("name? ")`
  appears before the program waits. When input is not a terminal do not flush
  (a program reading a pipe and printing a lot must stay fast).
- Panics: `drachm_panic` already flushes stdout and exits 101.

Parsing numbers, in the Rust library (exact, locale-free, one implementation for
both backends):

```c
// Parse the whole of `s` as a number of the given kind. Returns 1 and writes
// the value, or 0 for bad text (nothing is written). Integers: the value is
// range-checked for `bits` and `is_signed`; the 64-bit result in *lo is the
// two's complement pattern (sign-extended for signed), *hi is the upper half for 128.
uint8_t drachm_str_parse_int(const drachm_str *s, uint8_t is_signed, uint32_t bits, uint64_t *lo, uint64_t *hi);
uint8_t drachm_str_parse_f64(const drachm_str *s, double *out);
uint8_t drachm_str_parse_f32(const drachm_str *s, float *out);
```

and in `runtime.c`:

```c
_Noreturn void drachm_parse_failed(const drachm_str *s, const char *type_name);
// panics with:  cannot read "abc" as uint32
```
The text is shown truncated to 40 bytes with `...` after it, control bytes as `?`.
Final output: `drachm: panic: cannot read "abc" as uint32`, exit code 101.

**Parsing rules, as first written** (superseded: question 1 was decided forgiving
on 2026-11-11, see "Language questions found"; the rules that now apply are in
`site/content/reference-io.md`):
- the whole text must be the number: no spaces before or after, no empty text;
- integers: decimal digits, with a leading `-` allowed only for signed types
  (`-0` is `0`), no `+`, no `_`, no `0x`;
- a value out of range for the type is bad text (`"256"` as `uint8`), never wraps;
- floats: optional `-`, digits with an optional `.` and fraction, optional
  exponent `e`/`E` with optional sign; no `+`, no `inf`, `nan`, `infinity`,
  no hex; a float whose digits overflow the type is bad text;
  implement by validating the shape yourself, then calling Rust's `str::parse`.

Unit tests in the Rust library for every rule above (boundaries of each width
including `u128::MAX` and `i128::MIN`, `"-0"`, `""`, `" 1"`, `"1 "`, `"+1"`,
`"1e3"` as integer (bad), `"1e3"` as float, `".5"`, `"5."` (decide and test: both
bad under the digits-then-optional-fraction rule; say which in the report)).

### Step 3. Front end

HIR (`src/hir.rs`): `ExprKind::InLine` (type `string`), `ExprKind::InEnded` (type
`bool`), `ExprKind::ParseText { text: Box<Expr>, to: Type }` (type `to`).

Typeck:
- `io_call`: add `"inln"` and `"ended"` to the names, take **no** arguments (E0103
  "`io::inln` takes no arguments" in the existing style, and no type arguments),
  update the unknown-function help to "it has `io::out`, `io::outln`, `io::outf`,
  `io::inln` and `io::ended`". They need `in io;` (E0206) like the others.
- `string_method` (`src/typeck/strings.rs`) and `method` in `typeck.rs`: today a
  `string` receiver with `to` falls through to `value_method`, which reports "`to`
  converts numbers". Route `to` on a `string` (and on a `&string` once borrows
  build) to a new `parse_text` with these rules:
  - exactly one type argument, else E0103 "`to` needs the type to convert to";
  - the target must be a plain number type (`uint8`..`uint128`, `sint8`..`sint128`,
    `float32`, `float64`): any other type is E0101 "`to` on text reads a number,
    not `string`" (use the existing E0101 style with the type span as the label);
  - `bool` is **E0901** "reading text as `bool` is not supported yet" and a unit
    type (`float64<m>`) is **E0901** "reading text as a number with a unit is not
    supported yet" (language question 3);
  - no arguments.
  Result type = target; the expression is `ParseText`.
- Pretty/exhaustive matches over `ExprKind` (`typeck/units.rs` children walk,
  `build.rs` name table near line 1060, any `hir` visitor): add the arms.

MIR:
- `Callee::InLine`, `Callee::InEnded`, `Callee::Parse(Type)` (`Type` is `Copy`, so
  `Callee` stays `Copy`). `build.rs`: `InLine`/`InEnded` are calls with no arguments
  writing a fresh temp; `ParseText` borrows its text (`&string` operand, same helper
  `Out` uses for its argument, so a temporary text is dropped at the end of the
  statement and an owned `line` is not moved).
- `print.rs`: `InLine`, `InEnded`, `Parse<uint32>`. `validate.rs`: signatures
  `(Some([]), Str)`, `(Some([]), Bool)`, `(Some([shared(Str)]), ty)`.
- The result of `InLine` is an owned string: it gets a `Drop` like any call result.
  `cleanup`/`drops` need no change; verify with the MIR test below.

### Step 4. Both backends

Cranelift (`codegen_mir.rs`) and C (`cgen_mir.rs`), arms next to `Callee::Out`:

- `InLine`: `drachm_in_line(&dest)`.
- `InEnded`: `dest = drachm_in_ended()` (a `bool` is one byte, 0 or 1).
- `Parse(ty)`: call the matching parser with a stack slot for the result, test the
  returned flag, on 0 call `drachm_parse_failed(&text, "uint32")` (a `_Noreturn`
  call: in Cranelift follow it with `trap`, in C it ends the block). Then narrow:
  for integers of at most 64 bits the value is `*lo` truncated to the type (the
  bits are already in range, so any narrowing is exact); 128-bit combines `lo`
  and `hi` the way the existing 128-bit helpers do (`halves`); `float32`/`float64`
  use the float parsers. The type name in the message is `Type`'s `Display`.
  Keep both backends calling the same runtime functions in the same order; the
  failure message must be byte-identical.
- No change to `mark_in_memory` or the drop elaboration is expected; if a string
  result written through `dest` misbehaves, mirror how `Callee::Text` does it.

### Step 5. Tests

Run (`tests/run`, each with `.in` where it reads; leak-free on both backends,
and the existing `optimised_output_matches` compares them):
- `in_lines`: `while (!io::ended()) { ... io::outln(...) }` echoing lines with a
  counter; `.in` with 3 lines.
- `in_blank_lines`: blank lines in the middle, final newline present: counts
  every line, including the empty ones, and no extra one at the end.
- `in_no_final_newline`: `.in` written without a trailing newline; the last line
  is still returned. (Create it with `printf 'a\nb' > in_no_final_newline.in`.)
- `in_crlf`: `.in` with `\r\n` endings; lines have no `\r` (interim rule).
- `in_empty`: no `.in` file: `io::ended()` is true at once and `io::inln()` returns `""`.
- `in_after_end`: `inln()` called again after the end keeps returning `""`.
- `in_sum`: reads numbers with `line::to<sint64>()`, sums, prints; negative
  numbers and a value per line; plus `float64` average with `outf`.
- `in_parse_types`: one line per type with the boundary values of `uint8`,
  `sint8`, `uint64`, `sint64`, `uint128` max, `sint128` min, `float32`, `float64`,
  parsed and printed back with `::text()`.
- `in_prompt`: `io::out("name? ")` then `io::inln()`; stdout of the test shows the
  prompt followed by the echo (a terminal is not involved here; it checks order,
  not flushing).
- `in_utf8`: a line with non-ASCII text echoed unchanged, and (as raw bytes in
  the `.in`) one invalid byte replaced by U+FFFD (interim rule).
- `in_long_line`: a 100 000 byte line, echoed by length with `::len()`.
- `in_parse_bad`: `abc` as `uint32`: `.code` 101, `.err` lines
  `cannot read "abc" as uint32`; `in_parse_range`: `256` as `uint8` likewise;
  `in_parse_space`: `" 5"` is 5 (forgiving, Q1); `in_parse_gap`: `"4 2"` bad; `in_parse_empty`: `""` bad.
  These panic, so they are exempt from the leak check.
- `in_mixed_with_out`: interleaved `io::out`/`io::inln` to check ordering of output.

Fail (`tests/fail`, `.err` lines): `in_args` (`io::inln(1)`: E0103), `in_no_import`
(`inln` without `in io;`: E0206), `in_unknown` (`io::input()`: E0201 with the help
line listing the new names), `in_to_bool` (E0901), `in_to_unit` (E0901),
`in_to_string` (E0101), `in_to_no_type` (E0103), and a MIR test `tests/mir/in_line.drh`
whose `.mir` shows `InLine`, a `Drop` of its result, and `Parse<uint32>` with a
shared borrow of the text.

Benchmark check: reading 1 million short lines (generate the file in the test
script, do not commit it) should take well under a second on both backends; run
once by hand and put the number in the report. `bench/run.sh` must not change.

## Site updates (same commit as the user-visible change; CLAUDE.md)

Only describe what the compiler accepts now: not `maybe`, not bool or unit
parsing, and the interim strictness stated as how it behaves today without
promising it.

- `site/content/reference-io.md` (new, `order.json` under Reference, before
  `reference-strings`): `io::out`, `io::outln`, `io::outf` (link to the strings page
  for formats), `io::inln`, `io::ended`, the line rules (no newline, `""` at the end,
  last line without newline, blank lines are lines), reading numbers with `::to<T>()`,
  what happens on bad text today (exit code 101 and the message, shown with real output),
  and a note that this changes when failing calls arrive in a later step.
- Examples (each has `.in`): `input-echo.drh`, `input-sum.drh`, `input-bad.drh`
  would exit 101 and site examples must exit 0, so show the failure as a text block
  with output pasted by hand from a real run, not as an example file.
- `site/lib/remark-example.ts`: when `examples/NAME.in` exists, show it as a
  ```text title="input" block between the program and its output.
- `site/content/lesson-5-strings.md`: a short section "Reading a line" with
  `io::inln()` and a two-line example, linking to the reference. **Lesson 3**
  (loops): one exercise "sum the numbers on standard input" with
  `site/examples/solution-3-N.drh` + `.in` + `.out` if the lesson's pattern allows
  (follow what its existing exercises do).
- `site/content/reference-strings.md`: the `to` method on strings in the method
  list.
- `site/content/coming-later.md`: add under step 5 "`::to<T>()` on text reporting
  bad text as an error you handle (`maybe`) instead of stopping the program".
- `site/content/reference-cli.md`: `drachm run` passes its standard input to the
  program (verify with a pipe: `printf '1\n2\n' | drachm run examples/x.drh`).
- `docs/SPEC.md`: the `io` function list and `::to` on text; keep the design doc
  link in CLAUDE.md in mind (the owner syncs the Claude Docs copy; say in the
  report which lines changed).
- `site/gen-errors.sh`: not needed (`codes.rs` unchanged).

## Done when

`cargo test` green (new tests included, both backends, leak check); the
programs above behave as described; `drachm run` of an input program works with a
pipe and on a terminal (prompt appears before the read; check by hand and say so);
`NAME.in` is documented in the test header; the language questions below are in
the report.

## Pitfalls

- Reading must go through C `stdin` only; flushing only when stdin is a terminal.
- `getline`'s buffer is `malloc`ed by libc and the string must come from the
  counted allocator (`drachm_str_alloc`/Rust), or the leak counter and
  double-free detection (WP6) will misreport. Free the libc buffer yourself and
  never hand it to a `drachm_str`.
- A test without `.in` must not inherit the terminal.
- `ended()` followed by `inln()` must not skip or repeat a byte (`ungetc`).
- The `bool` ABI: the Cranelift side loads one byte; C `uint8_t`. Same as `Text`.
- Panic text has no source position (no spans at run time yet), unlike the owner's
  eventual error type; keep it exactly `cannot read "<text>" as <type>`.
- `-O` pastes `runtime.c` into the C file with `DRACHM_INLINE`: new functions
  that other translation units must call (none here) would need to be non-static;
  all of yours are `DRACHM_FN`. `drachm_parse_failed` is `_Noreturn` and not hot.

## Language questions found

Interim behaviour is what the code above does; none of it was asked.

1. **How strict is number text?** `"  42"`, `"+5"`, `"1_000"`, `"0x10"`, `"3.0"` as
   an integer, a number too big for the type.
   - A. Strict (interim): the whole text is digits (and one `-`); anything else, and
     any out-of-range value, is bad text. Programs trim and clean the text themselves.
   - B. Forgiving: surrounding spaces are ignored, `+` allowed; out-of-range is still bad.
   - C. Original: say what is accepted in the call, so the program, not the language,
     decides: `line::to<uint32>(spaces: skip)`.

   **Decided (2026-11-11): B.** Surrounding whitespace is ignored, a leading `+` is
   allowed, and for floats `.5` and `5.` are fine (a lone `.` is still bad).
   Out-of-range values, `1_000`, `0x10`, `4 2` and empty text stay bad text, exit
   101. Implemented in `runtime/strings/src/lib.rs`; golden tests
   `in_parse_space`, `in_parse_forgiving`, `in_parse_gap`.
   ```cpp
   uint32 a = " 42 "::to<uint32>();          // A: stops the program   B: 42
   uint32 b = " 42 "::trim()::to<uint32>();  // 42 in both
   uint32 c = " 42 "::to<uint32>(spaces: skip); // C
   ```
2. **Line endings and bytes that are not text.** `io::inln()` removes `\n` and a
   `\r` before it, and replaces bytes that are not valid UTF-8 with U+FFFD (interim).
   - A. As interim.
   - B. Remove only `\n`; keep `\r`; invalid bytes stop the program with a runtime error.
   - C. Original: `io::inln()` is always strict (B), and a second function
     `io::inlnLossy()` / a flag on the call decides the lenient behaviour.
   ```cpp
   string a = io::inln();          // "hi"   (A, from "hi\r\n")
                                   // "hi\r" (B)
   ```
3. **Reading text as `bool` and as a number with a unit.** `"true"::to<bool>()`
   and `"5"::to<float64<m>>()` are E0901 now.
   - A. `bool` accepts `true` and `false` exactly; a unit type reads the number and
     attaches the unit (like `float64<m> d = 5;`, Q12b).
   - B. Neither: text becomes a plain number and the program writes `true`/`false`
     comparisons and units itself.
   - C. Original: a unit is read with the unit in the text: `"5 m"::to<float64<m>>()`
     and `"5 km"` converts to metres.
   ```cpp
   bool ok = line::to<bool>();               // A
   float64<m> d = line::to<float64<m>>();    // A: 5 m for "5"   C: needs "5 m"
   ```
