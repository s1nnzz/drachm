# S5C: failures: `failure`, `fails`, `fail`, bare `fails`, `maybe`, outcomes

Implementer: **needs a strong model.** This changes the calling convention of
every failing function in both backends (through enums), adds control flow in
the middle of expressions in the MIR builder, and must keep ownership exact on
the failure paths (a leaked string on `fail`, or a double drop when `maybe`
takes the Ok arm, is silent until the leak check). It depends on S5A.1 (enums,
`Variant`, `Discriminant`, `Switch`, glue) and must not start before it has
merged.

Read first: `step5-overview.md` sections 1 to 4 and 6 (all of 2.3 and 2.4
twice), Q11 to Q18; SPEC section 7 (the whole section) and the `unhandled_fail`
config line in section 10; PLAN step 5 items 5, 6 and the Decided paragraphs;
`step3-ownership.md` 2.3 to 2.9; `step5-A-enums.md` (what you build on);
`wp6.md`; the code: `src/parser.rs` (`func_rest` and its `fails` arm, `stmt`,
`postfix`, `primary`), `src/typeck.rs` (`check_body`, `args`, `static_call`,
`struct_method`, `io_call`, `auto_eq`/`eq_func` as the pattern for synthesised
functions), `src/hir.rs`, `src/mir/build.rs` (how `Call`, `Logic` and `Return`
are lowered, scope drops on `Return`), `src/codegen_common.rs`
(`emit_c_main`, `in_memory`), `src/cgen_common.rs` (`main_fn`),
`runtime/runtime.c`, `site/content/design-errors.md`, `bench/*`.

## Goal

1. `failure` declarations (variants, messages with `${name}`, optional `exit N`),
   usable as types in `fails` lists and as values.
2. `T f(...) fails A, B { ... fail X; ... }`: signatures with failure lists,
   `fail X;`, `fail X(args);`, `fail Type::X;` (E0543 when ambiguous), `return`
   in a failing function.
3. Calls: `call maybe v` (lazy fallback), bare `call fails`, in expressions and
   statements and in all positions of a call (argument, operand,
   initialiser, `return` value).
4. `unhandled_fail = error` (E0544); `main` may fail; an escaping failure prints
   its message and exits with its code (default 1); `none main() fails X`.
5. The lowering in overview 2.3 with the baseline ABI (outcome enum returned in
   memory). Both backends identical, leak-free on every path. A benchmark
   baseline recorded (overview section 4).

Out of scope here (S5D): `fails with` chains, `otherwise`, `because`, `${e}` in
user code, failing `::to<T>()`, failing constructors. A handler suffix or
`because` met by the parser stays E0901 until S5D.

## Files you own

New: `src/typeck/failures.rs` (declaration, signatures, `fail`, call
suffixes), `bench/fails.drh` + `fails.c` + `fails.rs`, `tests/run/fail_*.drh`,
`tests/fail/fail_*.drh`, `tests/mir/fail_*.drh`, `site/examples/fails-*.drh`,
`site/content/reference-errors.md`, `site/content/lesson-11-failures.md`
(S5D completes it).

Edit: `src/lexer.rs` only if a keyword needs context handling; `src/ast.rs`,
`src/parser.rs`, `src/hir.rs` (`Func.fails`, `Stmt::Fail`, `ExprKind::Maybe`,
`ExprKind::PassUp`), `src/typeck.rs` (hunks; synthesised
`message`/`exit_code` functions via `failures.rs`), `src/mir/build.rs`
(outcome construction, the call-result branch, the failure arms),
`src/mir/{mod,print,validate}.rs` (nothing new if A.1 is complete; add tests),
`src/codegen_common.rs` + `src/cgen_common.rs` (the `main` wrapper only),
`runtime/runtime.c` + a declaration block for generated C (the cause API and
`drachm_unhandled`), `src/diag/codes.rs` (E054x, E0548, E0551, E0552, E0554),
site pages, `docs/PLAN.md`.

Do not touch: the call/return conventions in `codegen_mir.rs` / `cgen_mir.rs`
beyond what enums already need (S5E owns changes there), borrowck,
`typeck/control.rs`, strings runtime.

## LOOK FIRST

1. How `Return` lowers in `build.rs`: which scopes drop, in what order. A
   `fail` path is exactly a `return` whose value is the Fail outcome; reuse the
   path, do not write a second drop sequence.
2. How a call expression is lowered in operand position (`call`, `operands`)
   and how short-circuit `Logic` creates blocks inside an expression: your
   `Maybe`/`PassUp` lowering uses the same technique (evaluate the call into an
   outcome temp, `If` on the tag, one arm assigns the result temp, the other
   runs the fallback or returns; join). The result is a temp read by the rest of
   the expression.
3. `auto_eq`/`eq_func`: how a synthesised HIR function is produced and
   registered, so `message` and `exit_code` can be made the same way.
4. `emit_c_main` and `main_fn`: the wrapper around user `main`.
5. How a callee's `Func.ret` type is read by the callers, `validate.rs`
   (`Callee::Func` argument/result types), `drops::elaborate`: the MIR body of a
   failing function has `_0` of the outcome type; the HIR `Func.ret` stays the
   source type `T`. Decide once, in one helper `hir::Func::abi_ret(&Program) ->
   Type`, and use it in every place that needs the callee's result type
   (typeck keeps `T` for expressions; MIR/backends use `abi_ret`).
6. Which of the compiler's own unwrappings is allowed to `Move` a `Downcast`
   field (overview 2.2): the Ok arm of a temporary outcome.

## What to build (one commit per step; `cargo test` green after each)

### Step 1. `failure` declarations

- Parser: `failure Name { Variant "message" exit N, Variant(Type v, ...) "msg ${v}",
  }`, trailing comma, `exit` optional, message required (E0003 otherwise,
  "a failure variant needs a message"). The message is parsed as an
  interpolating string whatever its quotes (Q11): `${name}` must name one of the
  variant's values (E0551); an expression other than a bare name is E0901
  (keep the first version to names; the owner's examples are all names).
- Declaration pass (`failures.rs`): an `EnumDef` of `EnumKind::Failure` with
  `has_cause`; duplicate variant E0520 (reuse), empty failure E0554, exit not in
  1..=255 E0552. A failure type's name must not clash with a struct/enum/
  failure (E0202). Payload types: numbers, bools, `string`, Copy structs
  (E0901 for borrows, as enums). Messages and exit codes are compiled to the
  two synthesised functions of overview 2.4: `string message(&F)` and
  `uint8 exit_code(&F)`, registered under hidden names (`ToolError::message`),
  never user-callable. `ExprKind::Payload` for the interpolated names.
- Failure values can be built (`NoInput`, `BadConfig("x")`, `ToolError::NoInput`)
  as expressions with the same rules as enum variants. They are not Copy.
  A failure value used on its own is legal but useless until S5D's handlers;
  that is fine for tests of layout and glue.
- The cause word (overview 2.4): in the layout and in glue; the runtime
  functions `drachm_cause_new/drop` and `drachm_unhandled`. Rust-free C in
  `runtime.c`; messages are `drachm_str` through the counted allocator.
- Tests (run): `fail_decl` (declare, construct, move a failure value, drop one
  that owns a `string` payload: leak check), `fail_msg` is S5D's. Fail:
  `fail_decl_bad` (E0551, E0552, E0554, E0520).

### Step 2. `fails` in signatures; `fail`; outcomes

- Parser: `fails A, B` after the parameters in functions and methods (either
  order with `lives`: already free-order). Constructors with `fails`: still
  E0901 (S5D). Operator methods with `fails`: E0901 (Q17). `drop()` with
  `fails`: E0328.
- Typeck: each name after `fails` must be a failure type (E0540, with "no such
  type" vs "not a failure" variants), no repeats (E0554). `Func.fails:
  Vec<EnumId>`. The outcome and (for two or more types) `FailSet` enums are
  created and interned (overview 2.3); `abi_ret` returns the outcome.
- `fail X;`: resolution as SPEC: search every listed failure type for variant
  `X`; one match wins; several is E0543; none is E0541 (or E0542 when the
  function has no `fails`). `fail Type::X;` is exact. `fail X(args);` for
  payload variants, with the usual conversions and E0522 arity. `fail` ends the
  function: statements after it in the same block are unreachable (existing
  flow analysis treats it like `return`: add it to that code; the "function
  may end without return" check E0105 accepts a block ending in `fail`).
- `return v;` in a failing function: wrapped as Ok. `return;` in a failing
  `none` function: Ok. Falling off the end of a failing `none` function: Ok.
  A failing function whose body can end without returning a value: E0105 as
  today.
- MIR builder: `Return` and `Fail` as in overview 2.3. Validate: `_0`'s type is
  the outcome; V-rules for `Variant`.
- At this step a call to a failing function is E0544 unless S5C step 3 is in:
  keep an internal escape hatch for the tests of this step only (a
  `#[cfg(test)]` HIR check), or write step 2's tests as MIR-level tests in
  `tests/mir`. Do not land a user-visible way to ignore a failure.
- Tests (mir): `fail_return.drh` + `.mir`: the printed MIR of a failing
  function with an `fail` and a `return`, showing the two `Variant`s and the
  drops before `Return`.

### Step 3. Call suffixes `maybe` and `fails`; unhandled calls; propagation

- Parser: postfix `maybe EXPR` and `fails` after a call expression, binding
  to the immediately preceding call (`a::b()->c() maybe 0` attaches to `c()`).
  Fallback extent per Q15 interim: a single operand (a literal, a name, a
  parenthesised expression, a call chain) and anything that continues as an
  operator is E0003 "put the fallback in parentheses" with the fix-it. `fails`
  followed by `with` is S5D's (E0901 here). A statement-level `call maybe;` is
  E0003 (Q16). The suffix never attaches to a non-call.
- Typeck: `call maybe v`: `call` fails with set S; `v` converts to `T` (lossy
  W0102 as usual; `v` for a `none`-returning call: E0003); result type `T`. A
  fallback expression is checked in the caller's scope; it is evaluated only on
  failure. `call fails`: every type of S must be in the caller's list (E0548
  naming the first missing one); result type `T`.
- Unhandled: every call (user function, method, static, constructor shorthand
  once S5D lands, `::to<T>()` once S5D lands) whose callee has a non-empty
  `fails` and that carries no suffix is E0544. The same rule inside the
  fallback and inside handler blocks. A statement that is just a failing call
  needs a suffix too (`save(p) fails;`).
- HIR: `ExprKind::Maybe { call: Box<Expr>, fallback: Box<Expr> }`,
  `ExprKind::PassUp { call: Box<Expr> }`. MIR builder per overview 2.3:
  the call into an outcome temp (type `abi_ret`), `If Discriminant != 0`:
  - Maybe: failure arm drops the outcome (glue), evaluates the fallback, assigns
    the result temp; Ok arm moves/copies the Ok value into the result temp.
  - PassUp: failure arm converts to the caller's failure type (move, or the
    synthesised re-wrap function when the caller's set is a larger `FailSet`),
    builds `_0 = Fail`, runs the normal return path.
  - The result temp joins; the rest of the statement continues. Drop flags and
    scope drops of enclosing temporaries must remain correct on the early
    return of PassUp (it is a `Return` with live temps: the builder's existing
    return path drops them).
- `main`: may be `fails`. The entry wrapper (both backends): call, test the
  outcome, on Fail call `message`, `exit_code` and `drachm_unhandled`; on Ok
  return 0 (or the returned number). `drachm_unhandled` prints per overview 2.4
  and `_Exit`s. A `main` with parameters or non-`none`/number return keeps its
  current error (`tests/fail/badmain`).
- Tests (run): `fail_maybe` (numbers, `string` result, lazy fallback: a fallback
  with a visible side effect that must not run on success), `fail_propagate`
  (propagation through two and three functions; a three-type `FailSet` where
  the middle function lists a subset), `fail_two_types` (`fail ParseError::Empty`
  disambiguation), `fail_drops` (owned strings in the failing function and in
  the caller alive across the call: leak-free on success and failure),
  `fail_in_expr` (`1 + parse(a) maybe 0 * 2` is E0003; with parentheses it runs;
  a failing call as an argument with its own suffix; evaluation order printed),
  `fail_loop` (`continue`/`break` around failing calls), `fail_main_ok`,
  `fail_main_escape` (`.code` is the `exit` of the variant, `.err` the message),
  `fail_main_default` (no `exit`: code 1), `fail_main_number` (returns a number,
  succeeds). Fail: `fail_unhandled` (E0544, the overview rendering), `fail_not_declared`
  (E0548), `fail_unlisted` (E0541), `fail_nofails` (E0542), `fail_ambiguous`
  (E0543), `fail_not_failure` (E0540), `fail_maybe_paren` (E0003),
  `fail_handler_later` (`fails with` is E0901 until S5D).

### Step 4. Benchmark baseline, docs, site

- `bench/fails.drh`, `fails.c`, `fails.rs` per overview section 4 (four loops).
  Run `bench/run.sh` once with `nice -n 19`. Paste the table in the report and
  in `docs/PLAN.md` "Performance". No gate.
- Site per the checklist.

## Site updates (same commit as the step; only what builds)

- `design-errors.md`: S5C moves "A function that can fail", "Failure types",
  the `maybe` and bare `fails` parts of "Handling a failure at the call", and
  "When nothing is said" (only the **error** default; the `crash` mode text
  keeps `<!-- status: not-implemented -->` with "step 7") to
  `reference-errors.md` (new; add to `order.json` after `reference-types`/
  `reference-strings`: choose a sensible slot, after `reference-ownership`), with
  examples `fails-basic.drh`, `fails-maybe.drh`, `fails-propagate.drh`,
  `fails-main.drh` (+ `.out`; the escape example is documented as a transcript,
  not a runnable `.out`, because the site examples must exit 0: make the exit
  case a `tests/run` case and show it in the page as plain output).
  "Several handlers", "Catch-alls", "Chaining a cause" keep their badges
  (S5D). Update the page lead paragraph.
- `reference-ownership.md`: a short "Failures and drops" note (a failure drops
  what is alive like a return).
- `lesson-11-failures.md`: write it now with `fail`, `fails`, `maybe`, `fails`
  propagation, the E0544 error; S5D adds handlers.
- `coming-later.md`, `reference-diagnostics.md`, `errors.md` (regenerate).
- `docs/PLAN.md` progress + interims; `docs/SPEC.md` section 10 range list.

## Done when

- `cargo test` green on both backends, leak-free; the escape tests give the
  right exit codes and messages.
- `grep -rn "\`fails\` is not supported yet" src` leaves only the
  constructor/operator cases.
- `bench/fails.*` exist and the baseline table is in the report.
- Report: the `abi_ret` helper and every place that uses it; the size of the
  outcome for `leaf` (print it); interims taken (Q11 to Q18); everything the
  SPEC example in section 7 could not do yet and why.

## Pitfalls

- The failing function's `Func.ret` is `T` for typeck and the outcome for the
  ABI. A caller that reads the wrong one is a type confusion that only shows
  in the backends. Assert in `validate.rs` that a `Callee::Func` call's `dest`
  type equals the callee's ABI return type.
- `maybe`'s fallback must be evaluated only on failure and must see the same
  scope as the call; a `string` fallback is moved into the result, and the
  failure value is dropped (glue frees the cause chain) **before or after**
  evaluating it: pick one (before) and test with a side-effecting fallback.
- Result temp joins with two assignments (Ok arm and fallback arm): drop
  elaboration must see one initialisation per path.
- `PassUp` in the middle of an expression: the enclosing statement's other
  temporaries are live at the `Return`; they are dropped by the return path.
  Test `f(make_string(), parse(x) fails)`.
- A failing call in a loop condition: the suffix is required, and `break`
  inside a fallback is a jump out of the expression; the builder must support
  jumps from a fallback (tests: `while (parse(s) maybe 0 > 0)`).
- `FailSet` tag order = order written in the signature; the re-wrap function
  between a callee set and a caller set is generated once per pair.
- The cause word makes every failure value 8 bytes bigger and non-Copy; keep
  `copy` of a failure value an error (E0901 "failures cannot be copied yet").
- `exit_code` returns `uint8` in the generated code; the process exit status is
  that value, never 0 (E0552 guarantees it).
- Message templates run during the unhandled report with the failure value
  alive; the function takes `&F`.

## Language questions found

Q11 to Q18 in `step5-overview.md`.
