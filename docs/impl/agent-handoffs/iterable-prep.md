# Iterable preparation handoff

The read-only Step 6 audit and implementation brief are complete. Generic interfaces and fresh-walker Iterable remain to be implemented. The brief is in the preceding chat, including proposed fixture sources and exact output or diagnostic expectations.

## Checkout and artifacts

- Audited /mnt/shared/50_Dev/drachm on master at 70b5b60 (Merge dynamic interface borrows). At handoff the same worktree was at 355299c (Split on an empty separator into characters). No separate worktree or branch was created for this audit.
- Owner follow-ups were recorded at 742b487. The visible maps worktree was at 23fa8a9 and codex/s6f-lambdas was at 70b5b60 when this note was written. Check their current status before integration.
- This uncommitted note is the only repository file changed by this task. The brief is in chat. The standalone Iterable fixture drafts and expectations are in /tmp/drachm-iter-fixtures. No commits or compiler changes were made.

## Tests

No compiler tests, builds, or benchmarks were run. The proposed outputs and diagnostics were reviewed against the draft source and remain unverified by execution. After integration, run these commands with `nice -n 19 env CARGO_BUILD_JOBS=2 DRACHM_TEST_JOBS=2`:

- `cargo test --test run run -- --exact`
- `cargo test --test run fail -- --exact`
- `cargo test --test run optimised_output_matches -- --exact`
- `cargo test --test run site_examples -- --exact`

The harness filters Cargo test names, so fixture stems such as `iter_` are unsuitable as Cargo filters. Coordinate compile slots with the main agent.

## Resume

1. Read docs/impl/owner-answers.md Q4 and Q19 alongside step6-B-interfaces.md and step6-G-iterable.md. The S6G brief's mutable-source and exhausted-second-loop assumptions were superseded. The current interface prelude is the Drachm PRELUDE string in src/typeck/interfaces.rs. The brief's src/prelude/core.drh path is absent.
2. Land generic interfaces first. The parser already collects the syntax. In src/typeck/interfaces.rs, intern resolved instances by template plus type arguments. Resolve generic parents in the child environment, retain the instantiated owner for inherited signatures, and use instance IDs for conformance and bounds. In src/typeck/generics.rs, resolve bounds and generic struct promises with the actual type environment and make placeholder interface instances participate in definition-pass rollback. Audit the dynamic interface consumers of IDs. The static loop path can land without generic vtable dispatch.
3. Add prerequisite goldens from the brief. iface_generic_instances exercises Cell<uint32> and Cell<string> via ReadChild<T> extends Read<T> and fetch<T, R with Read<T>>, with stdout 7 then hi. The independent failure mutations cover the wrong interface instance (E0607), wrong member signature (E0611), wrong arity (E0602), and a missing method on an uncalled constrained definition (E0606).
4. Settle the remaining protocol contract with the owner. The source creates a fresh walker per traversal and next() advances that walker. The public spelling for the concrete walker result and whether it may borrow the source remain unresolved. An owned Iterator<T> interface result conflicts with Q4. Also settle whether one source may implement multiple Iterable item types and how iter() selects the walker if so.
5. Add Iterable after the generic interface prerequisite. In src/typeck/control.rs, extend the non-range dispatch currently in for_text. Evaluate the source once, call iter() once, and give the walker a loop-scoped hidden mutable local. Reuse looped and the optional match payload-move machinery. Each iteration calls next() once and ends at none. Check drop behavior on exhaustion and jumps, and keep synthesized control flow out of the user-visible skip targets.
6. Add the iter_fresh, iter_temporary_once, iter_jumps, iter_owning_items, iter_wrong_item, iter_not_iterable, and iter_wrong_next fixtures from the brief. The fresh test traverses one immutable source twice, then calls a constrained sum. Its expected lines are 3, 2, 1, 3, 2, 1, 6. The temporary source should print its marker once. The owning-yield test breaks early and uses the existing leak-checked harness. Use E0630 for item mismatch, proposed E0660 for no applicable conformance, and E0611 for the wrong next signature.
7. Coordinate the non-range dispatch with maps, which already touches control.rs. Map walking depends on S6D, and its callback form also depends on S6F. Add runnable site examples and update feature markers and diagnostics in the feature commits. The S6Z sweep follows integration and measures benchmarks after the other compiling work is idle.

## Iterable fixture drafts

See /tmp/drachm-iter-fixtures/README.md. The directory mirrors the repo layout.

- `tests/run/iter_fresh.{drh,out}` covers two traversals and a constrained sum.
- `tests/run/iter_temporary_once.{drh,out}` checks evaluation and iter() each happen once.
- `tests/run/iter_owning_items.{drh,out}` retains a moved yield across break and exercises owning-yield cleanup through continue and exhaustion.
- `tests/run/iter_jumps.{drh,out}` covers skip, continue, break, continue 2 and break 2.
- `tests/fail/iter_wrong_item.{drh,err}`, `tests/fail/iter_not_iterable.{drh,err}`, and `tests/fail/iter_wrong_next.{drh,err}` cover E0630, proposed E0660 and E0611 respectively.

The provisional declarations assume an owned concrete walker. After the owner settles the result contract, adapt the `with Iterator<T>` and `with Iterable<T>` clauses, `iter()` return wiring, and constrained sum bound as needed. Keep the behavior assertions. Once the prerequisite and loop lowering land, review and copy each source with its expected-output or diagnostic file into the matching repository test directory, then run the harness commands above. The generic interface prerequisite fixture remains in the implementation brief in chat.
