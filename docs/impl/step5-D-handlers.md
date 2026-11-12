# S5D: handlers, `otherwise`, `because`, `${e}`, failing `::to<T>()`, failing constructors

Implementer: **standard model.** It builds on S5C's lowering (the outcome
branch already exists); this package adds the handler arms, the cause chain
and three more places where failing calls come from. The only delicate
parts are ownership of the bound failure `e` (it is moved into the handler and
dropped or consumed by `because`) and the rewrite of the input tests. Needs
S5C and S5B.1 merged (failing constructors); S5A.2 is not required.

Read first: `step5-overview.md` 2.3 to 2.5, 5, 6, Q11 to Q17; SPEC section 7;
`step5-C-failures.md` (what you extend; read its "Pitfalls"); `step4.6-input.md`
(what changes) and "Input" in `owner-answers.md`; the code: everything S5C added
(`src/typeck/failures.rs`, the `Maybe`/`PassUp` lowering in `src/mir/build.rs`),
`src/typeck/strings.rs` (`parse_text`), `src/mir/build.rs` (`Callee::Parse`),
`src/codegen_mir.rs` / `src/cgen_mir.rs` (the `Callee::Parse` arms),
`runtime/runtime.c` and `runtime/strings/src/lib.rs` (`parse_int`, `parse_f64`),
`tests/run/in_*.drh`, `site/examples/input-*.drh`, `site/content/reference-io.md`,
`site/content/design-errors.md`, `site/content/design-constructors.md`
("A constructor that can fail").

## Goal

1. `call fails with (E e) => { ... }` with one handler per failure type, chained:
   `fails with (A a) => { ... } fails with (B b) => { ... }`; a handler block
   ends by giving a value (`maybe v;`), leaving (`return`, `fail`, `break`,
   `continue`) or, for calls returning `none`, simply ending.
2. `fails with (any e) => { }` and `otherwise (any e) => { }` after specific
   handlers.
3. `fail X because e;` and `${e}` (message only) in user code; the report of an
   escaping failure prints the chain.
4. `line::to<uint32>()` is a failing call (`ConvertError`); the exit-101 path is
   removed and every test/example/page that used it is rewritten.
5. Constructors may fail: `Port(uint32 n) fails BadPort { ... }`;
   `Port p = 8080 maybe Port::default();`, `new Port of (8080) fails`.

## Files you own

New: `src/prelude/core.drh` (the `ConvertError` failure; loaded like `si.drh`),
`tests/run/handler_*.drh`, `cause_*.drh`, `to_fails_*.drh`, `ctor_fails_*.drh`,
`tests/fail/handler_*.drh`, `site/examples/fails-handlers.drh`, `fails-cause.drh`,
`input-parse.drh` (+ `.in`, `.out`).

Edit: `src/parser.rs`, `src/ast.rs`, `src/hir.rs` (handler nodes),
`src/typeck/failures.rs` (handlers, `because`), `src/typeck/strings.rs`
(`parse_text` becomes failing), `src/typeck.rs` (prelude load, constructor
`fails`), `src/typeck/construct.rs` (failing constructors), `src/mir/build.rs`
(handler lowering, `Callee::Parse` returns an outcome), `src/mir/{mod,print,
validate}.rs` and both backends' `Parse` arms, `runtime/runtime.c` (cause
helpers already from S5C; add `drachm_cause_message` if needed), `src/diag/
codes.rs` (E0545, E0546, E0547, E0549, E0550, E0553), the tests and pages
below, `docs/PLAN.md`, `docs/SPEC.md` (the `::to` sentence only, and the
exit-101 sentence in section 10).

Do not touch: backends beyond `Callee::Parse`, `typeck/control.rs`, borrowck.

## LOOK FIRST

1. S5C's `Maybe` lowering: the handlers are the same shape with more arms and a
   bound variable; reuse the helper that evaluates the call into an outcome temp
   and branches, and the result-temp join.
2. How the failing set is switched on: for a single-type set there is no
   `FailSet`; for a set, the failure arm `Switch`es on the `FailSet` tag
   (`Discriminant`) and each arm moves the member failure out as `e` (the whole
   member value: allowed as the compiler's own unwrapping of a temporary).
3. `Callee::Parse(Type)` today: panics on bad text (`drachm_panic`, the message
   with the text and the type). The new form: `Callee::Parse` still calls the
   runtime function that returns `ok` and the value; the **MIR builder** wraps it
   (`If ok { Ok(v) } else { Fail(BadText(copy text, "uint32")) }`), so the
   backends only drop the panic path. Decide whether to keep a panic-free
   `Callee::ParseTry(Type)` that returns the pair through an out parameter, or
   to give the runtime functions an `ok` out flag: choose the smaller change
   and say which in the report.
4. The rest of the input tests: which of them depended on exit 101.
5. The constructor lowering of S5B: a failing constructor returns an outcome
   whose Ok value is the finished `self`; `fail` inside a constructor leaves
   `self` partly filled, dropped field by field (the init analysis handles it).

## What to build (one commit per step)

### Step 1. Handler chains

- Parser: after a call, one or more `fails with (Type name) => { block }`
  followed optionally by `otherwise (any name) => { block }`, ending at the
  statement's `;` (the SPEC examples end the whole expression with `;`).
  `(any e)` is a catch-all handler written in place of a type. A handler on a
  statement-level call returning `none` is fine. Precedence per Q15 interim
  (they bind to the preceding call; the whole chain is one postfix).
- Typeck:
  - Each handler type must be in the call's failure set (E0545 otherwise);
    once per type (E0546); `any` and `otherwise` handle everything not named,
    `otherwise` must be last and single (E0553), `any` as the *only* handler
    is allowed. After all types are handled, an `otherwise` is allowed (it is
    unreachable but harmless: W0111, "every failure is handled above").
  - Types not named and no catch-all: E0544 ("`parse` can fail with
    `FileMissing` here and nothing handles it"), listing the unhandled types in
    the message.
  - Inside the block: `e` is a new owned local of the failure type (an `any e`
    has the whole `FailSet` type, or the single type), dropped at the end of the
    block unless moved. `maybe v;` as a statement (a `Stmt::Resume`) gives the
    handler's value (type converts to `T`, E0101/W0102 as usual) and jumps to the
    end of the whole call expression. It is E0550 outside a handler block (and
    inside a nested function-level block of a *different* handler: the nearest
    enclosing one is used). `return` leaves the outer function (value checked
    against its return type, wrapped as Ok if it fails); `fail` raises in the
    outer function (must be listed: E0541/E0542); `break`/`continue` target the
    enclosing loop of the call. A block that can reach its end without any of
    these, for a call that returns a value, is E0547.
  - Handler order of evaluation: the call, then the matching handler only.
  - `${e}` in a handler is the message; `e` cannot be used otherwise except
    `because e` (Q14: E0901 for field access, for passing `e` on, for `match
    (e)` until Q3 is answered).
- HIR/MIR: `ExprKind::Handled { call, arms: Vec<Arm>, result: LocalId, label }`,
  `Stmt::Resume(label, Option<Expr>)`. The lowering is S5C's with a `Switch`
  on the `FailSet` tag (or none for one type) and one block per arm.
- Tests (run): `handler_basic` (the SPEC `parse(input) fails with (ParseError e)
  => { io::outln(f"bad input: ${e}"); maybe 0; }`), `handler_chain` (two types,
  each arm taken), `handler_return` (return in a handler drops the caller's
  strings), `handler_fail` (re-fail a different type), `handler_loop`
  (`continue` inside), `handler_any` (`any e` alone, and `otherwise`),
  `handler_none_call` (call returning `none`, handler block ends normally),
  `handler_nested` (a handler containing another failing call with its own
  handler), `handler_order` (prints which arms ran). Fail: `handler_impossible`
  (E0545), `handler_twice` (E0546), `handler_no_value` (E0547),
  `handler_maybe_outside` (E0550), `handler_otherwise_first` (E0553),
  `handler_missing_type` (E0544 listing the type).

### Step 2. `because`, `${e}`, the report

- `fail LoadFailed because e;` (and `fail BadConfig("x") because e;`): `e` must
  be a failure value (E0549), it is moved. Lowering per overview 2.4: render
  `message(&e)` into an owned string, take `e`'s cause chain, `drachm_cause_new`,
  store in the new failure's cause word, then drop what remains of `e`
  (payload strings) **without** freeing the chain that moved. Make a tiny
  runtime helper if the glue cannot express "drop but keep the cause"
  (`drachm_cause_take(void **cause) -> void*` that returns the pointer and
  nulls the slot, called before `e`'s drop).
- `${e}` and `e` in `f"..."`, backtick strings and `io::outf("%", e)`: the
  message. `e::text()` is not added (Q12 asks first). Outside a handler, a
  failure value you built yourself works the same way.
- The report format is S5C's; test the chain (`.err` lines) with one, two and
  three links.
- Tests: `cause_one`, `cause_chain` (A because B because C: `.code` and
  `.err` lines of the SPEC example), `cause_leaks` (the chain is freed on a
  handled failure: leak check), `cause_moved` (using `e` after `because e` is
  E0301). Fail: `cause_not_failure` (E0549).

### Step 3. `::to<T>()` fails; the input tests move

- `src/prelude/core.drh` with `ConvertError` (overview 2.5), loaded for every
  program before the user's items. A user declaration of `ConvertError` is E0202
  with help "`ConvertError` is built in".
- `parse_text` returns a call that `fails ConvertError`. `::to<T>()` on numbers
  (conversion between number types, units) is **not** failing (it follows
  `[behaviour] overflow`); only the text receiver is. The error message names
  both: `"abc" is not a valid uint32`.
- Rewrite, in one commit: `tests/run/in_parse_*`, `tests/fail/in_*` where needed,
  `site/examples/input-*.drh`, `solution-3-3.drh` and the lesson text that read
  numbers, `site/content/reference-io.md` ("bad text stops the program" becomes
  the failure), `docs/SPEC.md` (the `::to` sentence), `docs/impl/step4.6-input.md`
  (add "superseded by S5D"). Keep one test whose bad text escapes `main` so the
  report and exit code 1 are covered (`in_parse_escape`). Forgiving-parse rules
  from `owner-answers.md` (spaces, `+`, `.5`, `5.`) are unchanged.
- Remove the panic path in `Callee::Parse` in both backends; the runtime
  functions stay.
- Tests: `to_fails_maybe` (`line::to<uint32>() maybe 0` over several inputs
  with `.in`), `to_fails_handler` (message printed through `${e}`),
  `to_fails_escape` (`.code` 1, `.err`), `to_fails_propagate`. Fail:
  `to_unhandled` (E0544 on a bare `line::to<uint32>()`), `to_user_name` (E0202).

### Step 4. Failing constructors

- `Port(uint32 n) fails BadPort { if (n == 0) { fail Zero; } self.n = n; }`.
  Parser: `fails` on a constructor (S5C kept E0901 there). Typeck: the
  constructor's ABI return is the outcome with Ok = the struct. The shorthand
  forms: `Port p = 8080 maybe Port::default();`, `Port p = 8080 fails;`,
  `Port p = 8080 fails with (BadPort e) => { ... };`, and
  `new Port of (8080) maybe ...`, `new Port of (0) fails`. The suffix goes after
  the value list in the shorthand (that is why `maybe` here takes the whole
  initialiser: Q15 interim, a single operand fallback).
  A shorthand declaration of a failing constructor with no suffix is E0544.
- `fail` inside a constructor: `self` is partly filled and dropped field by
  field; completeness (E0504) is required only on paths that return Ok.
- Tests (run): `ctor_fails_maybe`, `ctor_fails_handler`, `ctor_fails_propagate`
  (a function that `fails` because its constructor does), `ctor_fails_drops` (a
  `string` field set before the `fail`: leak-free). Fail: `ctor_fails_unhandled`
  (E0544).

### Step 5. Site, docs

See the checklist.

## Site updates (same commit as each step; only what builds)

- `design-errors.md`: remove the remaining badges and move "Handling a failure
  at the call" (`fails with`), "Several handlers", "Catch-alls", "Chaining a
  cause" to `reference-errors.md` with examples `fails-handlers.drh`,
  `fails-catchall.drh`, `fails-cause.drh` (runnable, exit 0: handle the failure
  and print the chain with `${e}`; show the escaping report as plain text from
  a `tests/run` case). The `crash` mode paragraph keeps its marker (step 7).
  The page's lead says what is true. Once only the `crash` paragraph is unbuilt
  the page-level `status:` goes.
- `design-constructors.md`: "A constructor that can fail" moves to
  `reference-structs.md` (example `ctor-fails.drh`).
- `reference-io.md`, `lesson-3-decisions-and-loops.md` / the lesson that reads
  numbers, `input-*.drh`: use `maybe`/handlers; the page no longer says exit 101
  for bad text.
- `lesson-11-failures.md`: add handlers, `because`, reading numbers.
- `coming-later.md` (row 5 done except the `crash` mode: write that), the
  error index (`site/gen-errors.sh`), `reference-diagnostics.md`.
- `docs/PLAN.md`: step 5 done note with the list of interims and the open
  questions; step 7's `crash` note stays.

## Done when

- `cargo test` green on both backends, leak-free; all rewritten tests pass; no
  test or page mentions exit 101 for bad text.
- `grep -rn "exit 101\|exit code 101" docs/SPEC.md site/content` mentions it
  only for runtime faults (division by zero, bounds, overflow panic).
- Report: how `Callee::Parse` was changed (choice in LOOK FIRST 3), a list of
  every file rewritten for the input change, interims taken (Q12 to Q15), and
  what a handler cannot do yet (Q14).

## Pitfalls

- `e` moved into a handler: the outcome temp must be considered moved on that
  path (the compiler's own unwrapping rule) or the failure is dropped twice.
- Leaks on the `because` path: the old `e` payload strings are dropped, the
  cause chain is not (moved); the new failure owns the chain exactly once.
- A handler that `return`s inside a loop inside a function with live borrows:
  the builder's return path handles it; test with a `for` over a range.
- `maybe v;` inside a nested `if` in the handler: a jump to the call's join
  block from inside nested scopes drops those scopes first (use the same
  mechanism as `break N`).
- `fails with (any e)` binding type: the whole `FailSet` (or the single type).
  `${e}` dispatches through the set's synthesised `message`.
- `ConvertError` shadowing: a user struct named `ConvertError` is the same
  clash as any other duplicate type (E0202).
- Rewriting the input tests: `.code` files that said 101 become 1 (escape) or
  disappear (handled). Read each diff before blessing.

## Language questions found

Q11 to Q18 in `step5-overview.md`; add any more you meet to your report, not to
this file.
