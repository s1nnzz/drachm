# S5E: fast return of outcomes (optional, backends only)

Implementer: **needs a strong model.** It changes the calling convention for
failing functions in both backends and must keep them byte-for-byte identical
in behaviour. No language change, no new diagnostics, no site change except the
performance note. Do it only if the S5C baseline (`bench/fails.*`) shows the
plain lowering is slower than the targets in `step5-overview.md` section 4 (the
owner decides; report the numbers and stop if they are within target). Needs
S5C merged; independent of S5D (runs in lane 1 beside it).

Read first: `step5-overview.md` 2.1, 2.3 and 4; `step5-C-failures.md` (the
baseline); `wp6.md` and `step3-ownership.md` 2.9 (calling convention in both
backends); the code: `src/codegen_mir.rs` (`signature`, `ret_ptr`, the
`Callee::Func` arm, the `Return` terminator), `src/cgen_mir.rs` (the same in C),
`src/codegen_common.rs` (`in_memory`, `signature`), `src/cgen_common.rs`
(`c_type`, function prototypes), `bench/fails.*`.

## Idea

Today an outcome is an enum, so a failing function returns it through the
hidden first pointer: the callee writes tag and value to memory, the caller
reads them back. Split the convention instead (backends only; the MIR is
unchanged):

- A failing function whose Ok type is `none` or a scalar (number, bool, borrow,
  pointer) returns **two register values**: `status` (the outcome tag, `0` =
  Ok) and `value` (the Ok scalar, undefined when status is 1).
- Failure data does not travel in registers. The caller passes one extra hidden
  pointer, `fail_slot`, which is **the address of the Fail payload area of the
  outcome local that receives the result** (the `dest`). On failure the callee
  writes the failure value (tag, payload, cause word) there; on success it does
  not touch it.
- After the call the caller stores `status` into the destination's tag and, for
  Ok, `value` into its Ok payload. Nothing else changes: the MIR still has
  `dest: Outcome`, `Discriminant`, `If`, `Switch`.
- A failing function whose Ok type is not a scalar (a `string`, a struct, an
  enum) keeps the baseline convention (hidden result pointer = the whole outcome
  local). It could be extended later (decision in the report).

Cranelift: two return values in the signature, `fail_slot` as an extra
parameter after the hidden result pointer (none for the split convention).
C: return `struct { uint8_t status; T value; }` by value (the platform ABI puts
it in two registers on x86-64 SysV, and Win64 returns it in memory: measure
Windows only if a Windows machine is available; otherwise leave Windows on the
baseline convention behind `cfg(windows)` and say so).

## Files you own

Edit: `src/codegen_mir.rs`, `src/cgen_mir.rs`, `src/codegen_common.rs`,
`src/cgen_common.rs`, `bench/fails.*` (more variants), `docs/PLAN.md`
("Performance" table). One helper shared by both: `fn abi_kind(hir, func) ->
Abi { Plain, Outcome, SplitOutcome { value: Option<Type> } }` in
`codegen_common.rs`, so the two backends cannot disagree about which functions
are split.

Do not touch: HIR, typeck, MIR, runtime, the site.

## What to build

1. `abi_kind` and tests of it (unit tests: which signatures are split).
2. Cranelift: signature, prologue (the `Return` terminator writes `status`
   and `value` from `_0`'s tag/payload, or stores the failure into `fail_slot`
   when the function builds `_0 = Fail` — see the hazard), call site.
3. C backend: the same, using the struct return and an extra `T *` parameter.
4. `-O` and plain outputs identical on the whole suite; leak-free; the
   `bench/fails.*` table before/after pasted in the report.

Hazard: inside the callee `_0` is still an Outcome local in memory (the MIR is
unchanged) while the convention returns `(status, value)`. The simplest
correct version keeps `_0` in a stack slot and, at `Return`, loads
`status`/`value` from it (and, for Fail, **copies** the failure part to
`fail_slot` with one memcpy of the Fail payload size). That still saves the
caller side's memory traffic on the success path but not the callee's. The fast
version avoids the slot entirely: when `_0`'s only assignments are `Variant(Ok,
v)` and `Variant(Fail, f)`, build them directly into registers / `fail_slot`.
Do the simple version first, measure, then the direct one only if the
measurement justifies it (the optimiser in step 7.5 may do it more generally).
Never let the `fail_slot` alias a live local the callee can read.

## Tests

No new language tests. `cargo test` unchanged and green; add backend tests in
`tests/backends.rs`-style (a `(source, expected)` list) for: a split function
returning `none`, `uint64`, `bool`, `&string`; a failure with a `string` payload
and a cause through the slot; a chain of three split functions each propagating;
a split function called in a loop 10 000 times with 1% failures (counts printed);
a non-split (`string` Ok) failing function next to split ones. Each runs on both
backends with the leak check.

## Done when

- `cargo test` green; the benchmark table before/after is in the report and in
  `docs/PLAN.md`; the targets in the overview are met or the report says why not.
- `abi_kind` is the only place that decides the convention.
- Report: which variant (simple or direct) was built, numbers, Windows status.

## Pitfalls

- The two backends disagreeing on which functions are split (hence one helper).
- `fail_slot` for a call whose `dest` is a field of a larger local or an
  enum payload place: pass the address of the destination's Fail payload area
  (compute it from the place; if the destination is not addressable as an
  outcome local, use a temporary and copy).
- Function pointers / indirect calls do not exist yet; when they do (step 6+),
  they need the same convention; leave a comment.
- Struct-return ABIs differ between platforms in C; check the generated C with
  `-O` on x86-64 Linux only, and keep everything else on the baseline.
