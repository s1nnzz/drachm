# Step 4 leftovers: `match` on strings and `for` over a string's characters

Implementer: **standard model.** No new analysis. It is plumbing through
typeck, HIR, the runtime and both backends, plus two small compile-time
algorithms (glob compilation, glob coverage). Keep the two backends identical
and leak-free. It is independent of the rest of step 5 and is lane 2's first
package (see `step5-overview.md`, "Order").

Read first: PLAN step 4 ("Control flow done" notes, including the HIR shape of
`match` and range loops); SPEC section 3 (the pattern list and `skip` inside
`match`) and the strings parts of section 4; `owner-answers.md` ("Step 2/4
follow-ups"); and the code: `src/typeck/control.rs` (`match_stmt`, `for_each`,
`not_iterable`, `Coverage`, `Test`, `chain`), `src/typeck/strings.rs`,
`src/hir.rs` (`StrFn`, `ExprKind::StrCall`), `src/mir/build.rs` (how
`Callee::Str` operands are made: places are borrowed, not moved),
`src/codegen_mir.rs` and `src/cgen_mir.rs` (the `Callee::Str` arms),
`runtime/strings.h`, `runtime/strings/src/lib.rs` (`bytewise`, `char_offsets`,
the grapheme helpers), `tests/run.rs`.

## What the owner decided

- `match` arms are `case` blocks, no fallthrough. Patterns include strings and
  string globs with `*`: `case "/api/*"`, `case "*.png"`. `case _` is the
  catch-all. A missing case is a warning (W0110).
- `for (string name of names)` is the for-each form; "character" means a
  grapheme cluster, and `chars()` splits by grapheme.
- Open (PLAN step 4): how to match a literal `*`. Not decided; see the interim.

## Interim choices (listed in the language questions, `step5-overview.md`, Q1 and Q2)

- Every `*` in a `case` string is a wildcard. There is no way to match a
  literal `*` yet. The escape `\*` stays what it is today (E0008, invalid
  escape). The site page says so in one sentence.
- The loop variable of `for (string c of s)` is an ordinary owned `string`
  (it can be moved, stored, returned). Its value never points into `s`.

## Semantics

### `match` on `string`

```cpp
string route = io::inln();
match (route) {
    case "/"               { io::outln("home"); }
    case "/about", "/help" { io::outln("info"); }
    case "/api/*"          { io::outln("api"); }
    case "*.png", "*.jpg"  { io::outln("image"); }
    case "a*b*c"           { io::outln("abc"); }
    case _                 { io::outln("not found"); }
}
```

- A pattern is a string literal (any non-interpolating form: `"..."`, `"""..."""`;
  an `f"..."` or backtick string with `${}` is E0109 "a pattern must be a plain
  string"). A list `case a, b` mixes literals and globs.
- Without a `*`, a pattern matches when the value equals it, byte for byte
  (the same rule as `==` on strings).
- With `*`: `*` matches any run of characters, including none. The pattern must
  match the **whole** value. `"/api/*"` matches `"/api/"` and `"/api/x/y"`, not
  `"/api"`. `"*"` matches everything. Two `*` in a row mean the same as one.
  The text between the stars is compared bytewise, anywhere it fits, leftmost
  first, with the last segment anchored at the end (the classic linear glob
  algorithm; no backtracking blow-up: O(len x segments) worst case).
- The value is evaluated once into a hidden local, and **a `match` on a
  `string` place must not move or copy it**: `match (name)` leaves `name`
  usable afterwards. Use a hidden `&string` borrow of a place, or an owned
  hidden local for an rvalue (the temp is dropped right after the `match`
  value is no longer needed, exactly like the existing conditions' temporaries).
  Today's `match_stmt` assigns the value to a hidden local by value; for
  numbers that is a copy, for a string it would be a move out of `name`. Test it.
- `skip` and `skip to next` inside a string `match` count levels exactly as
  for numbers (the cases are an `If` chain in the HIR already).
- Exhaustiveness: only `case _` (or a case that is exactly `"*"`) makes a
  string `match` exhaustive. Otherwise W0110, with the usual wording, but no
  list of missing values: "text has too many possible values to list". The
  help is "add `case _ { }` to handle every other value".
- W0111 "this case can never run" fires when every pattern of a case is
  covered by earlier ones. Cover test, done at compile time: a literal is
  covered by an earlier literal with the same text, or by an earlier glob that
  matches it (run the same matcher on the literal text); a glob is covered
  only by an earlier glob with identical text or by an earlier `"*"`. Anything
  cleverer (does `"a*"` cover `"ab*"`?) is **not** attempted, so no false W0111;
  put that sentence in a comment. After `case _` everything is unreachable, as now.
- A pattern of another type is E0101 as for numbers ("a `case` of a `string`
  match must be a string"). Ranges on strings (`case "a".."f"`) are E0109 "a
  range pattern needs numbers".
- `_` alone in its `case` stays the rule (E0109).

### `for (string c of s)`

```cpp
for (string c of "naïve 👨‍👩‍👧") {
    io::outln(c);
}
```

- Runs once per grapheme cluster of the string, in order. An empty string runs
  zero times. `continue`, `break N`, `continue N`, `skip` inside work as in any
  loop. `for (mut string c of s)` is allowed (the variable may be reassigned in
  the body); the loop variable is declared with the type written, which must be
  `string`/`str` (anything else after `of` a string: E0101 "a `string` loop
  variable holds one character as a `string`, not `uint32`").
- `s` may be a place or any expression of type `string` (`for (string c of
  line::trim())`). The string is borrowed for the whole loop: changing it in
  the body is E0302 from the existing borrow checker, which is why the
  lowering below makes a real `&string` borrow. An rvalue lives in a temporary
  until the loop ends.
- Characters are produced by the runtime's own grapheme rule (`unicode-segmentation`,
  as `len()`, `reverse()` and `chars()`), so `for` and `len()` agree: the number
  of iterations equals `s::len()`.
- ASCII fast path: a single ASCII byte that cannot start a longer cluster
  (not followed by a non-ASCII byte, and not `\r` followed by `\n`) yields a
  `string` pointing at a static one-byte table (`cap == 0`, never freed, never
  dangling). Every other cluster allocates an owned copy. Both are plain `string`
  values to the language, and dropping either is correct.

## Lowering

Reuse what exists; add two runtime-backed string functions and no new MIR.

HIR for `for (string c of e) body`:

```
let text: &string = &e            // hidden local "text", a real shared borrow
let pos: uint64 = 0               // hidden local "position", a byte offset
loop {
    cond: pos < ByteLen(*text)    // Binary(Lt, ...)
    body: [
        Decl(c)
        c = NextChar(*text, pos)  // owned string
        pos = pos + ByteLen(c)    // before the body, so `continue` is safe
        ...user body...
    ]
}
```

- New `StrFn::ByteLen` (`[S] -> U64`): the string's byte length. Add it as a
  C function `drachm_str_byte_len(const drachm_str *s)` and let both backends
  call it (a load of the `len` field inline is an optional speed-up, only if it
  is a few lines in each backend and the tests still pass).
- New `StrFn::NextChar` (`[S, U64] -> S`): the cluster that starts at byte
  offset `pos`; the empty string if `pos >= len`. C:
  `void drachm_str_next_char(drachm_str *out, const drachm_str *s, uint64_t pos)`
  in `runtime/strings.h` and `runtime/strings/src/lib.rs`. It must **not** scan
  from the start (that would make the loop O(n squared)): take the slice from
  `pos` and call `graphemes(true).next()` on it (or a `GraphemeCursor`). A byte
  offset that is not a cluster boundary cannot occur, because `pos` only ever
  advances by `ByteLen` of what `NextChar` returned.
- The result's byte length is exactly the bytes consumed, so `pos + ByteLen(c)`
  is right for static one-byte results and owned copies alike.
- `c` is dropped at the end of each iteration by the existing scope-end drop.
  A `break` from the body drops it through the existing jump drop logic.
- `StrFn::params`/`ret`/`name`, the HIR printer, `mir/validate.rs` (the `Callee::Str`
  argument table) and both backends need one arm each. The C backend and
  Cranelift call the same runtime function; nothing else differs.
- Match lowering: a glob test is `StrCall(StrFn::Glob, [text, pattern])`
  (`[S, S] -> Bool`) with C function
  `uint8_t drachm_str_glob(const drachm_str *s, const drachm_str *pattern)`.
  A pattern without a `*` is `StrFn::Eq` (no new call). The pattern is a
  literal `string` constant (a not-owned literal), so the call allocates
  nothing. The runtime function fast-paths `"x*"` (`starts_with`) and `"*x"`
  (`ends_with`) before the general loop.

## Files you own

Edit: `src/typeck/control.rs` (`match_stmt`, `for_each`, `not_iterable`,
`Coverage` for text), `src/hir.rs` (two or three `StrFn` variants and their
tables), `src/typeck/strings.rs` only if `StrFn` arms are listed there,
`src/mir/{build,print,validate}.rs` (the `Callee::Str` tables), `src/codegen_mir.rs`
and `src/cgen_mir.rs` (one `StrFn` arm each, next to the others),
`runtime/strings.h`, `runtime/strings/src/lib.rs` (rebuild the runtime the way
the repository already does: look at `build.rs`/`link.rs`; do not add a
dependency), `src/diag/codes.rs` only if you touch an existing code's text
(you add no new code), the site pages below, `docs/PLAN.md` (progress row 4
and the "Still to do in this step" note), `docs/SPEC.md` (delete nothing; the
Patterns bullet may gain the sentence "every `*` is a wildcard; a literal
`*` cannot be matched yet").

New: `tests/run/match_text.drh`, `match_text_glob.drh`, `match_text_warn.drh`
(+ `.warn`), `for_chars.drh`, `for_chars_jumps.drh`; `tests/fail/match_text.drh`;
`site/examples/match-text.drh`, `for-chars.drh` (+ `.out`).

Do not touch: `src/mir/{borrowck,moves,drops}`, the parser (`match`/`for`
already parse any expression and any string pattern; verify with a test before
you decide anything is missing), and anything under `docs/impl/` other than
this file's report notes.

## What to build (one commit per step, `cargo test` green after each)

1. Runtime: `drachm_str_byte_len`, `drachm_str_next_char` (with the static
   one-byte table and the `\r\n` rule), `drachm_str_glob`. Rust unit tests in
   `runtime/strings/src/lib.rs`: globs (`*`, `a*`, `*a`, `a*b`, `a**b`, `*a*`,
   empty value, empty pattern, pattern longer than value, repeated segments such
   as `"*ab*ab"` against `"abab"` and `"aab"`), next_char over ASCII, `"\r\n"`,
   `"e\u{301}"`, an emoji family sequence, an empty string; loop to the end and
   require the count to equal `len`.
2. `for` over strings: HIR lowering above, E0101 for a wrong variable type,
   remove the E0901 in `not_iterable` for strings. Tests below.
3. `match` on strings: literals first (no globs), then globs, then coverage and
   warnings, then remove the E0901.
4. Site and docs.

## Tests

Run (both backends and leak-free automatically): `match_text` (literals, lists,
`case _`, a match inside a loop, `skip to next` inside a text case, the value
used after the match, a match on `io::inln()` via `.in`), `match_text_glob`
(each shape in "Semantics"), `for_chars` (ASCII, accents, a combining sequence,
emoji family, `\r\n`, an empty string, a long string, count equals `len()`,
`break`, `continue 1`, and `string kept = c;` stored outside the loop and printed
after it), `for_chars_jumps` (nested loops over two strings; `break 2`).

Warning: `match_text_warn` (`.warn` lines): W0110 for a text match without
`case _`, W0111 for a repeated literal and for a literal after `"/api/*"`.

Fail (`tests/fail/match_text.drh` + `.err`): a number pattern against a string
(E0101), `f"${x}"` as a pattern (E0109), `case "a".."f"` (E0109), `for
(uint32 c of "abc")` (E0101), a `for` body that changes the string (E0302).

Bench: add `bench/chars.drh` (+ `chars.c`, `chars.rs`) only if it is quick to
write: count the characters of a 50 MB ASCII string with `for`. Report the
number; the ASCII path should be within a small factor of a byte loop. Not a
gate.

## Site updates (CLAUDE.md: same commit; only what the compiler accepts today)

- `site/content/reference-control-flow.md`: in "match", a subsection "Matching
  text" (literal, list, glob rules above, the exhaustiveness/W0110 sentence,
  the limit that `*` is always a wildcard) with `site/examples/match-text.drh`
  + `.out`; in "while and for", a subsection "Looping over characters" with
  `site/examples/for-chars.drh` + `.out` and the grapheme sentence.
- `site/content/reference-strings.md`: one line under Methods pointing at both.
- `site/content/lesson-7-match-and-skip.md`: if it says text matching is coming,
  replace by a link and add one short example.
- `site/content/coming-later.md`: Progress row 4 becomes `done` (drop "string
  patterns, globs and `for` over strings left"). Remove any bullet about them.
- `site/content/design-more.md` (or wherever the pattern list is designed): remove
  any "Not implemented yet" badge on string patterns / `for` over text, and leave
  the badge on lists.
- `docs/PLAN.md`: step 4 "Done" line; the Open item about a literal `*` stays,
  marked "interim: always a wildcard".
- Both example files compile with no output and print exactly their `.out`.

## Done when

- `cargo test` green; `match (name) { ... }` leaves `name` usable (test); `for`
  over a string of N clusters runs N times and equals `len()`.
- `grep -rn "matching on text is not supported\|looping over a string" src` is empty.
- Both backends print the same bytes for every new run test.
- The report lists: whether the inline `ByteLen` was done, the chars bench
  number if measured, and any coverage case that gave a false W0111.

## Pitfalls

- A `match` on a `string` place that moves it (see above). The existing leak
  checker will report a double free or a leak; the cause is the hidden local.
- `NextChar` called with a stale `pos` after the string changed: impossible
  because the loop holds a `&string`; keep the hidden borrow a real borrow in
  the HIR (not a copy of the bits) so E0302 fires on a changing body.
- A cluster longer than one byte beginning with an ASCII byte (`e` + U+0301,
  `\r\n`): the fast path must look at the next byte.
- A pattern literal containing a NUL or a multi-byte character: bytewise, fine.
- Do not let `drachm_str_glob` allocate.
- `case` lists with more than one text and the `skip to next` counting: the
  `chain` helper already handles lists; do not special-case.

## Language questions found

Q1 (`*` literal) and Q2 (what the loop variable is) in `step5-overview.md`.
