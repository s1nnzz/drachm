# Step 7 system modules

[HANDOFF](../HANDOFF.md) records module loading, projects and stateless scripts as
built. [SPEC](../SPEC.md) and [PLAN](../PLAN.md) list the system library work,
with environment and process originally listed separately. The owner has now
selected this split:

- `fs` reads, writes and creates files. Write replaces the contents. Create is
  exclusive and fails if the destination exists. It takes a path and optional
  initial text and returns the shared `File` used by reader and writer APIs
  for line reads and writes. Cursor and opening modes are pending.
- Failures are typed by cause, such as `FileMissing` and `AccessDenied`.
- `os` exposes platform, current working directory and environment lookup.
  `os::getEnv` returns `string?` with `none` for a missing variable.
- Process execution belongs to a separate module. If later approved, pass the
  executable and argument vector directly to the OS. Shell syntax, expansion
  and interpolation require an explicit shell invocation.

## FS first

Use `src/modules.rs` and `src/resolve.rs` for per-source-file import checks.
Follow the existing `time` path for standalone checker support and test project
and script entry points. Keep builtin names and failure types consistent through
resolution, checker lookup, diagnostic formatting and the LSP index.

The native helper belongs in `runtime/strings/src/fs.rs`, linked from the same
Rust static library on both backends via `build.rs` and `src/link.rs`. Have the
main agent coordinate ownership of this file and agree the ABI before parallel
edits. Helpers borrow length-delimited Drachm strings and return an explicit
status. A read writes one owned Drachm string to an output slot on success.
Write needs no output slot. Create needs a separate owned-handle ABI. Keep the status mapping synchronized
between the runtime, MIR and `runtime/strings.h`. Map host errors centrally by
cause, including missing paths, permission denial and existing destinations.
Reserve a distinct status for unmapped errors and settle its public failure
type with the owner.

Use a single host operation for each call. Exclusive creation must use an
exclusive open, so two competing creators cannot both succeed. Replacing
contents can use a truncating open if the owner confirms those semantics.
Keep handles under RAII and initialize the read slot only after successful
decoding. Carry input lengths across the ABI so embedded NUL cannot silently
truncate a path. For a text-path API, convert valid UTF-8 through the host path
API, which can perform native encoding on Windows. Keep non-Unicode native
paths and invalid file text explicit in the contract. Avoid lossy conversion
that could redirect a file operation.

## Shared failure lowering

`src/typeck/failures.rs` already declares typed failures, builds outcome and
failure-set types, and checks propagation and handlers. `TryParse` in
`src/mir/build.rs` demonstrates runtime status conversion, but its specialized
paths split status and value acquisition. An FS call must execute once, even
when a handler resumes or the result is ignored.

Lower a system operation to one shared MIR call and branch on its status.
Construct `Ok` or a typed `Fail` using the existing enum layout, including the
hidden cause word and the correct failure-set tag. Have `outcome_of` feed that
value into existing `maybe`, propagation and handler lowering. Keep the ABI
out-slot unowned until the success branch transfers it into the outcome. Reuse
ordinary MIR cleanup for all owned strings and failure payloads. Add matching
validation and printing in `src/mir/`. The C and Cranelift implementations in
`src/cgen_mir.rs` and `src/codegen_mir.rs` should only marshal arguments and
call the common helper. Later `os` calls can reuse the status-to-outcome code.

## OS second

Land whole-file read and write first. Platform and environment lookup can
land before the shared File API or cwd.
Read the platform from the compiled runtime target and query cwd and the
environment in the running executable. Use the same owned string convention
for returned text. Keep absence separate from an empty environment value and
retain enough information to distinguish invalid names and non-Unicode values
once their public behavior is decided.

## Owner questions for main

- Does write create a missing file? Is replacing contents a truncating write,
  or must replacement be atomic? Decide the shared File cursor and opening
  modes before exposing create or the line APIs.
- Confirm text and path encoding and the public cause names for existing
  destinations, invalid paths, invalid file text and other host errors. Should
  failure values include the path or a platform error detail?
- Confirm OS function names and types, platform spelling, cwd failure types,
  and how invalid names and non-Unicode environment values are exposed.
  Keep unresolved compiler entry points at E0901 and report each interim.

## Verification and landing

Use disposable working directories for mutation tests. Check create and
readback, shorter writes replacing longer contents, repeated exclusive create
preserving the existing file, concurrent creation, Unicode paths, missing
files, permission denial under an unprivileged account, invalid encoding, and
handler, fallback, propagation and ignored-result cleanup. For OS, inject a
controlled environment and cwd and test missing and empty values separately.
Run the same assertions through plain and `-O`, including repeated operations
on the leak harness.

Update the relevant site pages, add real `site/examples/*.drh` and `.out`
fixtures, remove implementation markers for landed features, and update
`examples/showcase/` and its expected output in the same feature commit.
Retain **Not implemented yet** or **Tentative** markers for the remaining
surface. Regenerate site errors with `site/gen-errors.sh` if diagnostic codes
change. Keep `examples/stupid.drh` untouched.

Commit each completed step with a short plain message. Use
`nice -n 19 env CARGO_BUILD_JOBS=3 DRACHM_TEST_JOBS=2` for targeted tests and
the final foreground `cargo test`. No push or monitors.
