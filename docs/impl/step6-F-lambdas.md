# S6F: lambdas, function types, captures, `move`

Implementer: **needs a strong model.** A lambda is a synthesised function, a
synthesised struct and a borrower in the borrow checker's eyes; the function
value is a second kind of fat pair. Needs S6E (fat pair mechanics, `Unsize`
style rvalue) and S6C.1 (`each` on lists) merged.

Read first: `step6-overview.md` 2.2 (the `(A, B) => R` row), 2.5 (function types
in the borrow checker), 6 (E0650 to E0654) and Q17, Q18; SPEC section 8
("Lambdas: `(params) => expr` or `(params) => { block }`. Captures borrow by
default; `move` takes ownership. Function types mirror lambdas: `(string) =>
none`") and the `each` example; PLAN step 6 item 4; `step3-ownership.md` on
holders and `lives` (a lambda's environment is a holder), `owner-answers.md`
(Q10 struct `lives`, Q3 `lend`, the WP8 questions); `step6-E-dyn.md`; the
code: `src/mir/lives.rs`, `src/mir/borrowck/{regions,constraints,conflicts}.rs`,
`src/typeck.rs` (`Checker`, scopes: `lookup`, `declare`, `LocalInfo`; how
`check_body` builds a `hir::Func`; `args`), `src/hir.rs` (`Func`, `Local`),
`src/parser.rs` (expression parsing, `move` keyword is reserved, `=>` symbol),
backends (calls, signatures).

## Goal

1. Lambdas: `(string n) => io::outln(n)`, `(uint32 a, uint32 b) => a + b`,
   `(string n) => { io::outln(n); }` with parameter types written, the result
   type taken from the expression or from the `return`s in the block (all must
   agree; `none` when the block has none).
2. Function types: `(string) => none`, `(uint32, uint32) => uint32`, as
   parameter and local types; calling `f(x)`; passing a lambda where a function
   type is wanted (the lambda's parameter and result types must match: E0650,
   E0651); passing a named function `none show(string s)` where `(string) =>
   none` is wanted (`each(show)`), by name.
3. Captures: read access to enclosing locals by borrow (shared), checked by the
   borrow checker (the lambda holds a loan on each captured variable for as
   long as the function value lives; assigning a captured variable while the
   function value is alive is E0302 with the existing wording). `move
   (string n) => ...` takes the captured values (owning ones are moved: the
   variable is unusable afterwards, E0301 as usual) and the lambda owns them.
4. `names::each((string n) => io::outln(n));` on `string[?]` (and `uint32[?]`,
   arrays): the SPEC example. `each` takes a `(T) => none` and calls it per
   element in order, borrowing each element for the call (it gets a view, so
   an owning `T` parameter would be a copy: the lambda parameter type must be
   `&T` for owning `T`: `(&string n) => ...`; `(string n)` is E0650 with the fix
   `(&string n)`: report the usability as Q8 data).
5. Function values are passed down only (Q17 interim): usable as a parameter,
   a local and called; stored in a struct field, list element, map, returned,
   or captured by a lambda that escapes is E0652.
6. A lambda that changes a captured variable is E0653 (Q18 interim).

Out of scope (E0901): lambdas in generic *type arguments* (`List<(A) => B>`:
Q17), generic lambdas (parameter types are always written), recursion
through a lambda, closures returned, `async`/`thread::spawn` (step later; the
SPEC `thread::spawn(move () => ...)` example is not buildable), lambdas with
`fails` (`(S) => R fails X` types: after S5C a function type has no failure
list yet: calling a failing function from a lambda body needs `maybe` or
`fails with` inside the body as everywhere), operator-style lambdas, default
parameter values, point-free/partial application, overloaded function
types, `&mut` captures (Q18).

## Files you own

New: `src/typeck/lambdas.rs` (parsing hooks' typeck side, capture analysis,
function-type resolution and conversion), `tests/run/lambda_*.drh`,
`tests/fail/lambda_*.drh`, `tests/mir/lambda_*.drh` + `.mir`,
`tests/borrowck/lambda_*.drh` + `.bck`, `site/examples/lambda-*.drh`.
Edit: `src/types.rs` (`Type::Func { id }` with an interned signature table:
params and result; `Display` `(string) => none`), `src/ast.rs`
(`ExprKind::Lambda { moves, params, body }`, `TypeName` gains
`func: Option<Box<FuncType>>`), `src/parser.rs` (function-type syntax in
`ty()`: `(` types `)` `=>` type, only where a type is expected; lambda syntax
in `primary()`: `[move] ( params ) => expr-or-block`; disambiguating `(a) => ...`
from a parenthesised expression by the lookahead `... ) =>`), `src/hir.rs`
(`ExprKind::Lambda`, `ExprKind::FnCall`, `ExprKind::FuncRef`), `src/typeck.rs`
(small hunks: scopes need to know which locals belong to the enclosing
function for capture; calling a local of function type), `src/typeck/lists.rs`
(`each`), `src/mir/{mod,build,validate,print,ty}.rs` (`Rvalue::MakeFn`,
`Callee::Fn`), `src/mir/borrowck/{regions,constraints}.rs` (one region slot
for `Func`, outlives constraints at `MakeFn`), the four backend files,
`src/diag/codes.rs` (E0650 to E0654), site pages, `docs/PLAN.md`.

## LOOK FIRST

1. How `Checker` scopes map names to locals of the **current function**; a
   lambda body is checked with a child checker whose lookup falls through to
   the parent: record every parent local it touches (captures), with the first
   use span.
2. How holders are modelled (`StructDef::lives`, `lives.rs`): the environment
   struct is a holder with all borrow fields in `lives`; check that a holder
   created by `Aggregate` of `Ref` operands propagates loans (it does for
   `Pair` today: `tests/borrowck/holder.drh`).
3. How a `Ref` to a **temporary** is kept alive (owner Q13: borrowing a
   temporary in a declaration keeps it alive to the end of the block): a lambda
   passed as an argument creates the environment in a temporary that lives to
   the end of the call statement.
4. The calling convention for a function pointer in each backend: Cranelift
   `call_indirect` with the signature of the lifted function plus the hidden
   env pointer first; C: a function-pointer cast.

## What to build (one commit per step, `cargo test` green after each)

### Step 1. Function types and named functions

`Type::Func`; the signature table (`FuncSig { params, ret }`, id by equality);
type syntax; `FuncRef`: a top-level non-generic, non-method function used by
name where a function type is expected converts to `{code = a thunk, env =
NULL}`; the thunk has the env parameter and forwards (so all function values
share one ABI: `ret f(void *env, args...)`). Calling a local/param of function
type: `Callee::Fn(operand)`: load code and env from the pair, call. Borrowck:
one region slot per `Func` (like `&`): a pair made from `FuncRef` has no
loans. Tests: `lambda_ref` (pass `show` to a function taking `(string) =>
none`), call twice, wrong arity/type (E0650/E0651).

### Step 2. Lambdas without captures

Parse lambdas and `move`; typeck lifts the body into a synthesised
`hir::Func` (symbol `drachm_l_<mangle enclosing>_<n>`) with the env pointer
as hidden first parameter, parameters typed as written; result type inference
(expression; block: unify the `return`s). No captures: env = NULL. Tests:
immediately passed lambdas, lambdas in locals (`(uint32) => uint32 sq = (uint32
x) => x * x;`), block bodies, nested lambdas not capturing, a lambda calling a
generic function (instance created inside the lifted body).

### Step 3. Captures by borrow and `move`

Capture analysis: every enclosing local used in the body is captured (by
shared borrow unless the lambda is `move`). The environment is a synthesised
struct `Env#n { &T a; &U b; }` (for `move`: the values) with `lives` listing
every borrow field; inside the body a captured name is read through the env
(`env.a`, auto-deref as for any `&`). MIR: `env = Aggregate(Env#n, [&a, &b])`
in a temporary (or into a hidden local when the lambda is bound to a name),
`f = MakeFn(code, &env)`. Borrowck: the `Ref`s to the captured variables are
ordinary loans, held by the env; the function value borrows `env`; an
`Outlives` constraint from the function value's region to each captured
loan's region is what `Aggregate` of a holder already generates, so verify
with `tests/borrowck` rather than adding code. Moving a captured variable into
a `move` lambda: ordinary `Move` operands into the env aggregate; the env
temporary's glue (ordinary struct glue) drops the moved values at the end of
the statement (or of the block when named). A captured variable assigned while
the function value is live: E0302 (existing text with a note "captured here").
Captured variables that the body would change: E0653. Captured `self`
inside a method: allowed as a shared borrow of `self` (a method that creates a
lambda reading `self` stays `Shared`; one that would change it is E0653).
Tests: `lambda_capture_read`, `lambda_capture_two`, `lambda_move_string`
(leak-checked, the original variable use is E0301), `lambda_capture_conflict`
(E0302), `lambda_in_method`, `lambda_escape` (E0652 for return/store), a
captured variable's drop order versus the env temporary.

### Step 4. `each` and friends

`each` on lists and arrays (and strings? no: `for` covers text) as a Builtin
the typeck expands into a `Loop` calling the function value per element (HIR,
no runtime function): same iteration lowering as C.2's `for`, so the element
view/copy rules are the same (the callback gets `&T` for owning `T`, `T` for
Copy `T`: the declared function-type parameter decides, E0650 otherwise). The
SPEC example runs: `names::each((string n) => io::outln(n))` **fails** for an
owning `T` under the rules just stated (it needs `(&string n)`): this is a
real tension between SPEC and the ownership rules; do not decide, keep the
interim (E0650 with the `&string` fix) and report it as Q8/Q17 data with the
exact message. Tests: `lambda_each_numbers`, `lambda_each_view`, `lambda_each_array`.

### Step 5. Cost

The `-O` C backend: a call through a function value with a known lambda is an
indirect call the C compiler may inline when the lambda is a `static`
function and the pair is a local; do not add special cases. Measure `each`
summing 100 million `uint64` against a `for` loop and report the factor (no
target; this feeds step 7.5's inliner).

## Site updates (same commit as each step; only what builds)

- `reference-generics.md`, section "Function types and lambdas": function types,
  lambdas, captures, `move`, `each`, the "passed down only" rule and the
  captured-change rule, each as a **Not implemented yet** note where Q17/Q18
  apply (for example "A lambda that changes what it captured" and "Storing or
  returning a function"), with examples `lambda-basic.drh`, `lambda-capture.drh`,
  `lambda-move.drh`, `lambda-each.drh` (+ `.out`) and the real E0302, E0650,
  E0652, E0653 output.
- `design-generics.md` has no lambda section; `design-concurrency.md` shows
  `thread::spawn(move () => ...)` and stays marked (concurrency is later).
  `reference-types.md`: function types. `reference-ownership.md`: a lambda
  borrows or owns what it captures. `lesson-17` / others: untouched.
- `coming-later.md`: remove the lambda bullet if there is one; `docs/PLAN.md`
  row 6 note ("lambdas beyond step 6" in "Later" becomes: threads and async only).
- `reference-diagnostics.md`, `errors.md`.

## Done when

- `cargo test` green; the lambda and `each` snippets from SPEC section 8 that
  do not need threads run on both backends, leak-free (or are reported as
  blocked by Q8/Q17 with the exact diagnostic).
- Report: the function-value ABI as emitted, the borrowck cases that needed
  new code versus none, the `each` cost factor, every interim with file and
  line.

## Pitfalls

- A lambda's synthesised function must not appear in `main`'s ordering or in
  user-visible lists (`drachm check` output); name it so it cannot clash with
  user functions.
- Captured variable declared after the lambda in source order is not captured
  (lookup is by scope at the lambda's position).
- A lambda capturing a loop variable of a `for`: the env borrows it for one
  iteration only; the borrow checker's loop handling must accept it (test).
- Shadowing: a lambda parameter with the same name as an enclosing local
  shadows it (no capture, no warning change).
- A lambda inside a generic function body is lifted per instance; the lifted
  function's name includes the instance symbol.
- `move` of a Copy variable copies it (no E0301 later); `move` of a `&mut`
  moves the borrow (owner Q15).

## Language questions found

Q17, Q18 in `step6-overview.md`; Q8 (the `each` view rule).
