# S6F lambdas handoff

## Status

Core S6F and list/array `each` are implemented and verified on both backends.
The branch is clean. The handoff is the only file written in the root checkout.
There was no push or merge.

- Worktree: `/mnt/shared/50_Dev/drachm/.claude/worktrees/codex-s6f-lambdas`
- Branch: `codex/s6f-lambdas`, based on `70b5b60`
- `5e582e5` Add function types and named callbacks
- `a0e2d9f` Lift lambda expressions into functions
- `5dbc8a0` Borrow and move lambda captures
- `8e71f19` Run each callbacks on lists and arrays

Git writes needed escalation because the shared worktree index lives under the
read-only `.git` directory. All four commits succeeded after escalation.

## Changed areas

- `src/typeck/lambdas.rs` resolves function signatures, builds named-function
  thunks, lifts bodies, and infers capture mutations using the method receiver
  analysis. `src/typeck/lists.rs` synthesizes a loop per collection and callback
  type. It evaluates the receiver and callback once, visits items in order,
  and makes a per-item copy for owning by-value parameters.
- `src/ast.rs`, `src/parser.rs`, `src/types.rs`, and `src/hir.rs` carry
  arrow signatures and lambda bodies. The MIR, its validation and analysis,
  and both backends carry function construction and indirect calls.
- `docs/PLAN.md`, `docs/SPEC.md`, and the site's reference and roadmap pages
  cover implemented features and pending forms. The generated diagnostic
  index was refreshed. Runnable site examples are `site/examples/lambda-*`.
  Golden fixtures are `tests/{run,fail,mir,borrowck}/lambda_*`.
- See `git diff --name-only 70b5b60..8e71f19` for the full file list.

A callable is a 16-byte `{code, env}` pair. The target receives the hidden
environment pointer before the written parameters. Cranelift passes the pair
in memory and puts its normal hidden memory-result pointer ahead of the
environment when needed. C passes the pair by value and casts the code
pointer to the matching C signature. Named functions get a forwarding thunk
and a null environment. Capture-free lambdas also use a null environment.

Captured places use ordinary shared or mutable loans and captured owned
values use ordinary moves and drop glue. The new borrow-checker work is one
function-value region and a `MakeFn` loan on the environment that keeps its
capture regions live (see `src/mir/borrowck/constraints.rs:162`). The MIR and
borrow-checker fixtures cover loan expiry after the final call. Named
environments use the containing block for storage, and call-argument
environments use the statement. E0652 enforces the Q17 downward-only rule
for return and storage. Q18 mutable captures work, including calls on
captured `self`. The old brief's E0653 read-only interim was superseded.

## Verification

Run from the worktree:

```sh
nice -n 19 env CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2 cargo build --offline
nice -n 19 env CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2 CARGO_NET_OFFLINE=true bash site/gen-errors.sh
nice -n 19 env CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2 cargo test --offline --test run --test backends --test leaks -- --test-threads=1
git diff --check 70b5b60..HEAD
```

All passed. The integration command ran 11 harness tests, including runtime
fixtures, backend parity, compiler failures, MIR, borrow checking, leaks and
all site examples. Direct smoke runs of the new fixtures used
`DRACHM_CHECK_LEAKS=1` with plain and `-O` builds. Full unfiltered
`cargo test` remains for the main merge.

The standalone `-O` benchmark summed 100 million `uint64` items, by walking
a one-million-item list 100 times. Both variants printed `50000050000000`.
Across seven alternating runs with leak checking and list setup included,
median wall time was 41.5 ms for `each` and 30.8 ms for `for` (1.35× on this
shared machine). Temporary reproducer files are
`/tmp/drachm-s6f-{each,for}-bench.drh` and
`/tmp/drachm-s6f-benchmark.py`.

## Outstanding and next action

- Mutable item callbacks to `each` give E0901 at
  `src/typeck/lambdas.rs:145` pending the callback and collection borrow
  rules. A mutable capture in a value or shared-view callback is supported.
- E0901 also covers mutation of fixed bindings captured by `move`
  (`src/typeck/lambdas.rs:427`), capturing function values, borrowing
  existing borrows or holders into a closure, moving a holder or `self`,
  and capturing during fill (`src/typeck/lambdas.rs:318–328`). Moving a
  plain `&mut` borrow into a closure works. Borrow-returning callables are
  deferred (`src/typeck/lambdas.rs:30,123,238`). Callback failure signatures,
  unsafe named callbacks, and function types as generic arguments are
  deferred (`src/parser.rs:190,242`, `src/typeck/lambdas.rs:115`).
- The living design doc still needs the S6F owner decisions reflected there.
  The branch includes the updated repository SPEC snapshot.
- Main agent: merge `codex/s6f-lambdas` through `8e71f19`, then run the
  full suite and the site's build on the merged revision. A read-only
  merge preview against master `355299c` showed no conflict markers.
  Preserve the newer empty-separator split wording in
  `site/content/reference-collections.md` alongside the new `each` link.
