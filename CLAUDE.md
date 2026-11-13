# Drachm

Drachm (pronounced /drɑxm/, files `.drh`) is a statically typed, natively
compiled language designed by this repo's owner. The compiler is written in
Rust. Plain builds use Cranelift; `-O` builds generate C for the system C
compiler (the owner's choice, for benchmark speed).

## Read first

- `docs/HANDOFF.md`: current state, how work is run (agents, roles, CPU and
  push rules), the docs site and the owner's tools. Start here.
- `docs/SPEC.md`: every design decision so far (a snapshot of the design doc).
- `docs/PLAN.md`: the step-by-step build plan, progress table, and the **Open**
  questions for each step.
- Living design doc (Claude Docs): https://claude.ai/artifact/DxosrpAV2x2TzpfNbaFw8i.
  Keep it, `docs/SPEC.md` and the code in sync when decisions change.

## Working with the owner

- **The owner makes the language design decisions.** Ask; don't decide
  language behaviour yourself. Interview a few questions at a time with
  `AskUserQuestion`, then write down exactly what they chose. Read their free-text
  notes carefully: they often refine or override the option they picked.
- **Don't copy Rust (or any one language).** Every option set includes at least
  one original idea, and you don't default-mark another language's answer as
  "Recommended". Drachm's own flavour: wordy English keywords (`with`, `of`,
  `lives`, `maybe`, `fails with`, `in`/`out`, `skip`).
- Put Drachm code in option previews in ```cpp fences so it gets highlighted.
- Engineering choices (data structures, crates, algorithms) are yours; mention
  any that change what users see.
- Commits: no AI attribution lines (no Co-Authored-By etc.), short plain messages like a human programmer. Pushing master after a verified merge is allowed; ask before anything else.

## Keeping the docs site current

`site/` is the user docs (Next.js; content in `site/content/*.md`, runnable
examples in `site/examples/`). Every change that makes a feature usable, or
adds/changes a diagnostic, updates the site **in the same commit**:
- add or edit the relevant tour/reference page, with a real example in
  `site/examples/NAME.drh` + `.out` (checked by `cargo test`);
- remove the feature's "Not implemented yet" / "Tentative" marker once it builds;
- rerun `site/gen-errors.sh` when `src/diag/codes.rs` changes.
The site shows the language **as designed** (owner, 2026-11-11: "for showoff
purposes"), not only what builds. Every page or section about something the
compiler does not accept yet carries a status marker: **Not implemented yet**
(decided in `docs/SPEC.md`, not built) or **Tentative** (still open, may
change). Built features must have tested examples; marked ones may show
untested code. When a feature lands, remove its marker in the same commit.
Agent briefs must say this.

## The showcase

`examples/showcase.drh` is one program that shows off everything the compiler
accepts today, written to impress (owner, 2026-11-13: "EVERYTHING the
language can do, at max awesomeness"). It is a tested example (expected output
in `tests/examples/showcase.out`, run on both backends by `cargo test`). Every
change that makes a feature usable extends the showcase **in the same commit**,
weaving the feature into the program rather than appending a toy. Agent briefs
must say this.

## Commands

```sh
cargo build
cargo test                                   # unit tests + golden tests (tests/run.rs)
DRACHM_BLESS=1 cargo test                    # rewrite expected .out/.code files
bench/run.sh                                 # speed vs C and Rust
target/debug/drachm run examples/hello.drh       # plain build (Cranelift)
target/debug/drachm run examples/hello.drh -O    # optimised build (via C)
```

See `docs/PLAN.md` for progress (steps 0-5 done). The two backends must always
behave identically; `cargo test` checks every run case both ways.
