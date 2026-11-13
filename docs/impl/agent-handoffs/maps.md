# S6D maps handoff

## Status and history

The isolated checkout is `/mnt/shared/50_Dev/drachm/.claude/worktrees/agent-a425e048db0a7e84b` on branch `worktree-agent-a425e048db0a7e84b`.

Existing commits, oldest first:

- `3bbfc4cb8305507344a2cd55ed709b6f2336a619` — Map and OrderedMap types, key rules and layout.
- `c5d8d6fb5a954da38598176e22b2262bd7c7b9e8` — literals, lookup, set, remove, length and both backends' ownership glue.
- `23fa8a9aac1179b7a85603ea15bc2c293b3e0c7b` — HEAD, OrderedMap test, iteration interim and map reference.

Takeover edits are present as tracked edits and untracked files in that checkout. No new commit or push. `git add` exited 128 because `.git/worktrees/agent-a425e048db0a7e84b/index.lock` is on a read-only filesystem in this session. This note is the only change made in the main checkout by this task.

## What works and what changed

- Hashable today: signed and unsigned integers through 128 bits, bool, string, float32 and float64. Floats compare by bits (Q14). Decimal literal negation preserves the sign of zero. Constant duplicate detection compares converted float bits and uses a hash table. Lists, maps, borrowed keys and raw pointers give E0626. Composite keys currently give E0901.
- Both backends emit a static operations table shared by Map and OrderedMap for each key/value pair. Lookup borrows the key and probes once, constructing an owned optional. The copy can allocate for strings and nested collections. A borrowing lookup could avoid this cost and would need a separate language decision.
- Map names are reserved for built-ins, with E0202 on conflicting declarations and E0602 on wrong arity. Undecided methods give E0201 with a note. The decided iteration method `each` has an E0901 interim.
- The existing runtime supports copy, move, lookup, replacement, removal and insertion order. No runtime changes were needed. Existing mutation syntax suffices for benchmarking, so the temporary compiler flag in the earlier brief was unnecessary.

Files in the isolated checkout:

- Compiler: `src/{cgen_mir,codegen_common,codegen_mir,consts,lexer,parser,typeck}.rs`, `src/mir/{build,mod,validate}.rs`, `src/typeck/{control,enums,generics,maps,overload,units}.rs`.
- Docs: `docs/{PLAN,SPEC}.md`, `site/content/{coming-later,design-collections,lesson-16-collections,reference-collections,reference-diagnostics}.md`, and `.drh`/`.out` pairs for `site/examples/{map-key,map-optional,solution-16-2}`. Pending features retain badges. Lesson 16 retains its overall badge for S6Z. The SPEC snapshot records existing owner answers. The living design doc was left untouched.
- Regression coverage: `.drh`/`.err` pairs for `tests/fail/map_{arity,duplicate_float,reserved_name}`, and `.drh`/`.out` pairs for `tests/run/map_{nan,positions}`. The latter covers map moves through a parameter and return, a copied map after mutation, a generic holder with a map argument, deep lookup through nested maps, maps of lists, lists of maps, enum payloads and reuse of a string lookup key.
- Benchmark: `bench/map.{drh,c,rs}` uses the separate benchmark agent's drafts, with the Drachm text conversion corrected. `bench/run.sh` discovers the separate row, which keeps map timings independent from list timings.

## Verification

All compiles, tests and benchmarks used `nice -n 19 env CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2`.

- `cargo build --offline` passed.
- `cargo test --offline --lib --test leaks --test backends` passed, 134 unit tests and two integration tests.
- `bash /tmp/drachm-s6d-check.sh` passed. To reproduce without the script, build each `tests/run/map_*.drh`, `site/examples/map-*.drh` and `site/examples/solution-16-2.drh` with `target/debug/drachm build SOURCE -o EXE` and again with `-O`. Execute with `DRACHM_CHECK_LEAKS=1`, compare stdout to the sibling `.out` and check that stderr is empty. The script checked 5 run cases and 4 site examples on each backend. It also checked 11 `tests/fail/map_*.drh` against their sibling `.err` files.
- Generated a temporary 1,000-entry literal at `/tmp/drachm-s6d-many.drh`. Both backends printed `1000` for length and `1000` for the last value, with leak checks enabled.
- `git diff --check` passed. This was the initial targeted pass. The full-suite reruns are recorded under merge readiness.

## Benchmark

Five timed runs per executable via `bash /tmp/drachm-s6d-bench.sh`, using the debug compiler driver to build plain and `-O` native executables, `cc -O3` for C, and `rustc -C opt-level=3` for Rust. Each executable printed `37499997500000`. Times varied under load. Best wall times:

| Cranelift | Drachm -O | C | Rust default | Rust fixed SplitMix64 |
| --- | --- | --- | --- | --- |
| 1.215 s | 0.986 s | 0.573 s | 0.883 s | 0.438 s |

Drachm -O took 1.12× the time of the default Rust HashMap and 2.25× the time of the fixed-hasher comparison. No direct hash-versus-probing profile was taken. Repeat on a quiet machine using the map row in `bench/run.sh` and time `target/bench/map-rust --fixed` separately.

## Outstanding work and resume

1. Stage and commit the tested edits together with their site documentation and examples when the worktree index is writable. Use `git status --short` to include the untracked tests, benchmark and site examples. Re-run the targeted checks before integrating onto master.
2. Implement composite hash/equality for structs, enums, optionals and arrays, including structural diagnostics for unhashable fields. Current key classification is `src/typeck/maps.rs:39`. Implement and validate `key()` and E0627/E0628. Apply Q13 so `key()` wins for maps while ordinary `==` keeps the user's override. Use the brief's conservative rejection for custom equality without `key()` unless the owner resolves it differently. Add both-backend coverage for variant tags, optional none vs empty string, and owning struct keys.
3. Complete Q9 compound updates (`src/typeck/maps.rs:239`), preserving single evaluation of receiver/key and stopping on a missing key. Add success and exit-101 tests. Keep the matching site badge until it works.
4. Implement Q16 two-binding traversal (parser interim at `src/parser.rs:1467`) and test OrderedMap order through replacement and removal. The `each` method interim is in `src/typeck/maps.rs:187` and depends on lambdas. Map-as-Iterable belongs with the iteration work.
5. Implement Q3 borrow-carrying map values with lifetime inference. Explicit and inferred value checks in `src/typeck/maps.rs` currently reject direct borrows and known holders; verify indirect holders as part of that work. Borrowed keys remain unhashable.
6. Full merge verification and docs site build belong to the integrating agent.

## Merge readiness against master 355299c

Reviewed master `355299c9c3a8a3695046b54dd808aa5d21a97edb` (empty-separator split after the S6E merge). The merge base is `d012e155cc0d610510ffe38e1cf0069e1bf5da18`. The maps branch still points to `23fa8a9`. Only this handoff note changed in the main checkout. Its Git metadata was untouched. The temporary clone was used for merge objects and a rehearsal only.

Read-only to the shared repo: made a synthetic maps snapshot in a disposable clone using the three existing commits, the tracked diff, and every listed untracked file. `git merge-tree --write-tree --name-only 355299c SNAPSHOT` reported these seven conflicts with the final patch:

| Path | Resolution rehearsed |
| --- | --- |
| `docs/PLAN.md` | Keep master's S6E status and append S6D partial status. |
| `docs/SPEC.md` | Keep the complete dynamic-borrow paragraph and the maps branch's hashable-fields, Q13 and Q14 wording. Keep the maps mutation and iteration answers in Collections. |
| `site/content/coming-later.md` | Keep working interface borrows and add working scalar-key map features and the pending maps items. |
| `src/codegen_common.rs` | Keep the `Type::Dyn` arm, classify `Type::Map` as in-memory, and retain `is_fat()` in the same predicate. |
| `src/codegen_mir.rs` | Keep both vtable and map operation data definitions and generator fields. Accept both map and fat-borrow arguments as in-memory. Keep both `MapNew` and `Unsize` out of scalar lowering. |
| `src/typeck/layout.rs` | Keep both layouts: fat borrows 16/8 and maps 48/8. |
| `src/types.rs` | Keep both `Dyn` and `Map` variants and the associated methods. |

The split change merges without conflicts. Preserve master's runtime implementation, the `list_split_empty` expected output and removed panic expectations, and the new split example. Maps of lists rely on the usual list copy/drop glue. `site/content/errors.md` auto-merges, and generation in the rehearsal matched the merged file. Map and dynamic borrow MIR arms coexist through analysis, validation and both backends.

Review found and fixed a real interaction in `src/typeck.rs` and `src/typeck/maps.rs`: quiet resolution used for generic arguments bypassed map value validation. Before the fix, `Holder<Map<string, &Counter>>` compiled on the combined checkout even though the direct value type was rejected. Explicit, quiet and inferred resolution now share the map value check, including rejection of `none` and known borrow holders. The isolated branch regression uses `&uint32`; the combined probe uses a fat `&Counter`.

### Cross-feature verification

The combined program `map-dyn-split.drh` in the rehearsal directory passed plain and `-O` builds with `DRACHM_CHECK_LEAKS=1`. It called an `&mut Counter` method that changes a map field, called an `&Counter` method that reads it, and retrieved a deep copy of an owned list made by `split("")`. The list contained the two graphemes `á` and `b`. Separate combined probes gave E0626 for an interface borrow as the key and E0901 for both a direct interface-borrow value and the generic-nested case. The diagnostic index regenerated byte-for-byte, and the staged rehearsal patch passed `git diff --cached --check`.

### Full-suite verification

- Combined rehearsal: `nice -n 19 env CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2 cargo test --offline` passed in the disposable checkout with its own target directory. This includes 134 unit tests and all 9 golden-test groups, including optimized output comparison and the site examples. Log: `/tmp/drachm-maps-review.FZrQch/integration-final.log`.
- An initial maps-worktree full suite passed before the generic-argument fix. During the final rerun I reused that target directory for an early rehearsal, which left master's runtime archive in the maps-only build. The isolated `list_split_empty` case then observed master's successful split instead of its branch's expected panic. The fresh combined run above used a separate target. For the maps-only retest I ran `cargo clean -p drachm` in the isolated worktree to remove the stale generated archive before rebuilding.

- Clean maps-worktree rerun: `nice -n 19 env CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2 cargo test --offline` passed with the final patch, including all 9 golden-test groups. The plain split fixture also confirmed the maps branch uses its original runtime after cleaning. Log: `/tmp/drachm-s6d-full-clean.log`.

- Combined Next.js static export passed with `npm run build -- --webpack` in the disposable checkout, using a copy of the main checkout's installed dependencies after confirming the lockfiles matched. Webpack compiled, passed TypeScript checks and generated all 72 static pages. Log: `/tmp/drachm-maps-review.FZrQch/site-webpack-final.log`. The default Turbopack build stalled during compile and was stopped, so the default build command still needs confirmation in the integration environment.

### Exact staging inventory

Run from the maps worktree in a session that can write its Git index. The 25 tracked paths and 19 new paths belong together in one commit with the docs and tested examples:

```sh
git add --pathspec-from-file=- <<'PATHS'
docs/PLAN.md
docs/SPEC.md
site/content/coming-later.md
site/content/design-collections.md
site/content/lesson-16-collections.md
site/content/reference-collections.md
site/content/reference-diagnostics.md
site/examples/map-key.drh
site/examples/map-key.out
src/cgen_mir.rs
src/codegen_common.rs
src/codegen_mir.rs
src/consts.rs
src/lexer.rs
src/mir/build.rs
src/mir/mod.rs
src/mir/validate.rs
src/parser.rs
src/typeck.rs
src/typeck/control.rs
src/typeck/enums.rs
src/typeck/generics.rs
src/typeck/maps.rs
src/typeck/overload.rs
src/typeck/units.rs
bench/map.c
bench/map.drh
bench/map.rs
site/examples/map-optional.drh
site/examples/map-optional.out
site/examples/solution-16-2.drh
site/examples/solution-16-2.out
tests/fail/map_arity.drh
tests/fail/map_arity.err
tests/fail/map_duplicate_float.drh
tests/fail/map_duplicate_float.err
tests/fail/map_generic_borrow.drh
tests/fail/map_generic_borrow.err
tests/fail/map_reserved_name.drh
tests/fail/map_reserved_name.err
tests/run/map_nan.drh
tests/run/map_nan.out
tests/run/map_positions.drh
tests/run/map_positions.out
PATHS
git diff --cached --check
git diff --cached --stat
git commit -m "Complete scalar map support"
```

### Integration steps

1. Stage and commit the 44 paths above in the maps worktree. Include all three existing maps commits by merging that branch, preserving its history.
2. Recheck the integration base. On `355299c`, run `git merge --no-commit --no-ff worktree-agent-a425e048db0a7e84b` in the authorized integration checkout. Resolve the seven paths as above. If master has advanced, rerun merge-tree and review any additional conflicts first.
3. The rehearsal files are in `/tmp/drachm-maps-review.FZrQch/repo` and the complete reviewed patch against `355299c` is `/tmp/drachm-maps-review.FZrQch/maps-on-355299c.patch`. For the identical base and maps commit contents, use the seven resolved files from that checkout, then stage them. If /tmp has been cleaned, use the table above. Compare `git diff --cached --binary HEAD` with the rehearsal patch when available.
4. Verify the index has no unresolved paths and `git diff --cached --check` passes. With the repository CPU limits, regenerate the diagnostic index using `nice -n 19 env CARGO_NET_OFFLINE=true CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2 bash site/gen-errors.sh`, and run `nice -n 19 env CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2 cargo test --offline`. Run the default Next.js site build before finalizing the integration commit. The rehearsed Webpack export passed.
5. Commit the integration once verified. Keep S6D marked partial for composite keys, compound updates, iteration and borrowed values. Do not include this main-checkout handoff in the maps commit.
