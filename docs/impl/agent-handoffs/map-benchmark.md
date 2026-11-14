# Step 6 map benchmark handoff

The three source drafts are complete. They are outside the checkout, and this handoff is the only repository file added by this task. There is no benchmark commit.

## Worktree and revisions

- Main worktree: `/mnt/shared/50_Dev/drachm`, branch `master`. HEAD was `70b5b60` when drafting began and `355299c9c3a8a3695046b54dd808aa5d21a97edb` when this note was written.
- Map API reference: `.claude/worktrees/agent-a425e048db0a7e84b`, branch `worktree-agent-a425e048db0a7e84b`, at `23fa8a9aac1179b7a85603ea15bc2c293b3e0c7b`. The map implementation commit in that history is `c5d8d6f`.
- The draft uses the empty mutable `Map<uint64, uint64>`, runtime `::set` and optional `::get` shown in that branch's collection reference and map examples.

## Drafts and checks

| Draft | Repository destination |
| --- | --- |
| `/tmp/drachm-map-bench/map.drh` | `bench/map.drh` |
| `/tmp/drachm-map-bench/map.c` | `bench/map.c` |
| `/tmp/drachm-map-bench/map.rs` | `bench/map.rs` |

Each program starts with an empty map, inserts 5,000,000 distinct keys with value `3 * key + 1`, and performs 5,000,000 successful lookups. A stride of 7,919 visits each key once. C uses a growing linear-probing table. Rust uses the standard `HashMap` hasher by default and accepts `--fixed` for a separate measurement. C and the fixed Rust mode use the map runtime's SplitMix64 finalizer.

The only execution check was the arithmetic reference below. It passed and printed `37499997500000`, the expected single stdout line from each program. No builds, test suites, or benchmark runs were performed under the CPU restriction.

```sh
python3 -c 'from math import gcd; n = 5_000_000; assert gcd(7919, n) == 1; print(n + 3*n*(n-1)//2)'
```

## Resume

1. Once the map implementation is in the integration checkout, copy the three drafts into its `bench/` directory. Preserve the drafts from `/tmp` before any cleanup.
2. When CPU is available, run `nice -n 19 env CARGO_BUILD_JOBS=2 bench/run.sh` on a quiet machine. It discovers the `map` row, checks stdout equality for both Drachm backends, C and default Rust, and reports best-of-five wall times. Check the map checksum against the expected line above.
3. Use the Rust executable produced by the runner for the fixed-hasher check: `target/bench/map-rust --fixed`. Expect the same checksum. Time that mode separately using the runner's best-of-five method with stdout redirected.
4. Record the results for the Step 6 sweep. The map target is within 2× of Rust's `HashMap`. If it is slower than that, the map brief calls for profiling hash versus probing cost.
