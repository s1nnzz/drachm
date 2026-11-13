# Step 7 WP5 scripts lowering

[Owner answers](owner-answers.md) Q10 and the late Q10b follow-up govern
shared state and entry selection. Q10b supersedes the old interim in the
[module brief](step7-modules.md). [SPEC sections 2, 9 and 10](../SPEC.md)
supply the existing binding, module and script rules. [PLAN](../PLAN.md) and
[HANDOFF](../HANDOFF.md) describe the packages already in place.

## Front end and entry

1. Carry explicit project/script mode from `config::open` through the
   graph and checker. WP3 already finds the nearest config and supplies
   script defaults, entry, and dependency roots. Keep source-only compiler
   APIs and project/library entry validation on their existing paths.
2. Extend `parser.rs` and `ast::Program` to retain the entry file's runtime
   declarations and executable statements in one source-ordered sequence,
   with original spans. Reuse statement and declaration parsing. Preserve
   constant candidates until the existing constant checker classifies them.
   Recognise the first-line shebang in both the lexer and flag scanner
   without shifting offsets. Imported files still need their own file
   identity and settings even after WP2 flattens the graph.
3. In `resolve.rs`, reserve script binding identities before resolving
   function bodies and walk the executable sequence with the existing
   lexical scopes. Resolve shared reads, assignment targets and projected
   places to the same identity. Preserve WP2's import visibility and
   shadowing machinery. Coordinate the new declaration kind and AST walk
   with the WP2 owner. Exercise one-file and multi-file scripts because
   `front_project` currently bypasses the resolver for a one-file graph.
4. If the entry file declares `main`, reject executable top-level
   statements and label both the statement and the entry declaration.
   Runtime binding declarations are declarations for this check. If there
   is no explicit main, give the source-ordered sequence a synthetic
   entry body. Use an internal identity for the generated entry. Keep
   existing explicit main validation and exit conventions (`none` gives
   zero, `sint32` supplies the status, and the existing failure wrapper
   reports a failing main). Set `hir::Program.main` to the outer lifecycle
   wrapper so both native entry shims call it. Keep the user's main
   distinct from that wrapper so an ordinary call to main cannot rerun
   startup.

## Shared storage and cleanup

Use a typed script-state table in HIR with stable slot IDs, declaration
spans and binding capabilities. Carry shared places through MIR as typed
roots with the same field and index projections used for locals. Give each
run one storage instance. The first executable implementation can reserve
aligned private storage per slot in the object and equivalent private
typed storage in C. Use matching initialization bits for owning slots
where cleanup cannot prove liveness. Zeroed storage alone carries no
initialization guarantee. JIT must give each invocation fresh logical
state and clear it after teardown.

The entry wrapper owns startup and cleanup. Lower runtime initializers
using ordinary expression checking, with their file's settings. For the
script's startup, initialize all top-level declarations before the first
statement or explicit main. Use E0901 where initialization order among
declarations or initialization safety is unresolved.
Set the initialized bit only after storing the completed value. Perform
replacement with existing assignment and drop elaboration. Evaluate the
incoming value before dropping the old value, using the same move and
alias checks as for a local. All functions in the supported scope use
the same slot. Constants remain on the existing constant path and
runtime slots stay out of type expressions.

Add shared roots to MIR initialization, move, liveness and alias checks
before enabling owning values. Check transitive reads and writes at calls
against the caller's initialization and active loans. For a first slice,
compute conservative effects over direct call edges to a fixed point.
Include initializer calls and recursion. Use E0901 where indirect or
dynamic calls, escaping borrows, or moves out of a slot need effects
the implementation cannot prove. Keep replacement and final drop as
separate ownership events. Funnel supported exits through MIR cleanup,
dropping initialized slots once in reverse initialization order. A
failure during initialization must clean up the initialized prefix.
Capture the exit result before cleanup and propagate it through the
outer wrapper. Audit diverging runtime calls, including WP4's crash
path and existing panic and failing-drop paths. A direct process exit
cannot execute wrapper cleanup. Coordinate a cleanup-capable failure
path with WP4 before shipping owning state on that path. Preserve the
existing hard-stop contract pending owner clarification.

Both backends consume the same checked, drop-elaborated MIR. In
`codegen_mir.rs`, map a shared root to module data plus existing
projection addressing. In `cgen_mir.rs`, emit private typed slots and
use the existing place and drop emission. Keep initialization,
replacement and cleanup decisions in MIR. The existing native shims
(`emit_c_main` and `main_fn`) call the selected HIR entry in each
backend. Compare output, stderr, exit status and observable drop order.

## Dependencies and landing order

- **Independent front-end work.** Land shebang recognition and internal
  mode plumbing with targeted lexer/config checks. Land ordered script
  syntax and entry validation behind E0901 where lowering is pending.
  WP3 supplies config and dependencies. Coordinate local import tests
  against the WP2 handoff and use its resolver for dependency imports.
- **Executable slice.** Land generated entry and explicit-main dispatch
  on both backends with ordinary calls and locals. Add proven initialized
  scalar state and direct calls as a separate commit. Use a temporary
  executable for plain run and `-O` under the E6 interim. Cover bare
  scripts, configured scripts, and a local helper import. Restrict
  early slices with E0901 wherever accepting a script would silently
  use the wrong policy.
- **WP4 integration.** Config defaults are already stored, but storing
  them does not implement crash on unhandled failure. Consume WP4's
  effective settings for source functions and generated initializer or
  statement code. Keep synthesized lifecycle operations policy-neutral.
  Share WP4's failure reporting and exit code path. Apply warning
  promotion before display filtering once WP4 supplies the policy, and
  keep errors visible. Full script behaviour waits for these interfaces.
  WP5 has no need to implement WP4 settings itself.
- **Owning state and follow-ons.** Land checked replacement, cleanup,
  and supported borrowing after parity and leak checks. Auto-import only
  implemented core modules, retaining qualified names per Q14b. Inline
  registry dependencies remain E0901 under Q12. WP5b replaces temporary
  execution with Cranelift JIT and must preserve the same entry and state
  lifecycle, including repeated runs and runtime symbol setup.

## Owner questions and interims

- Writes to script-wide variables require `mut`. Preserve named constants.
- Confirm initialization order among declarations and visibility before
  initialization, including reads through startup calls, forward references,
  cycles and declarations without an initializer. Use E0901 where
  initialization cannot be established.
- Does script-wide state include imported helpers reading entry bindings,
  shared declarations in helper files, or exported state reached by
  multiple imports? Ask about module startup order if helpers can own
  state. Keep the first slice within the entry file and gate cross-file
  state with E0901 when requested.
- Can a top-level statement return a status or propagate failure from the
  generated body? Can callers take ownership from a slot or keep a borrow
  across calls? Confirm teardown order where custom drops make it visible,
  and cleanup expectations for panic and a failing destructor. Use E0901
  for unsupported forms and retain ordinary ownership errors for proven
  misuse. Confirm warning promotion precedence if WP4 has no ruling.

## Acceptance and documentation

Add a script matrix covering mutation from two functions, recursive direct
calls, constant names in types, local shadowing, invalid entry mixtures,
explicit exit status, startup reads through calls, and project/library
regressions. Add owning string or collection replacement and a custom
drop trace. Check successful teardown, failure during startup, and failure
after a write with both backends and the leak harness. Check diagnostics
against original source spans, including after a shebang and in imports.

Each user-visible landing updates `site/content/design-scripts.md` and
the relevant guide or reference, adds runnable `site/examples/` fixtures
with expected output, and removes only the status markers for shipped
behaviour. Regenerate the error index with `site/gen-errors.sh` if the
diagnostic table changes. Extend the `examples/showcase/` project in the
same commit with a companion script and wire it into showcase coverage.
Run it as its own script input so the project retains its entry and mode.
Use targeted tests with `nice -n 19 env CARGO_BUILD_JOBS=3 DRACHM_TEST_JOBS=2`
during implementation, followed by one full foreground `cargo test`.
