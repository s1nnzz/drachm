# Handoff: how this project is being run

For a new session with no memory of earlier conversations. Read this, then
`CLAUDE.md`, `docs/PLAN.md` (Progress table) and the newest sections of
`docs/impl/owner-answers.md`.

## State (2026-11-13)

- Steps 0-6 are done, plus 4.5 (units) and 4.6 (input). See the Progress
  table in `docs/PLAN.md`. Every step 6 decision is in
  `docs/impl/owner-answers.md` (sections from "Step 6 questions" down), in
  `docs/SPEC.md`, and in the living design doc ("Generics, interfaces and
  collections (step 6)" under Later decisions).
- Step 6 leftovers merged (2026-11-13): `not same as`, `same as` on
  maps/structs is E0633, `T as unit`, `take xs[i] leaving v`, ranges `by`.
  Value (length) parameters (Q12) are built (`Type::Val` is the stand-in for
  a value argument; see `typeck/generics.rs` `value_targ`).
- **Leftovers of 2026-11-13, built** (owner answers "Step 6 leftover
  follow-ups"): the SI unit is `tesla`; a type parameter named like a unit is
  W0618, and `X as unit` names the unit anywhere; `by` takes a negative step
  (counts down), a run-time zero step stops with `range step is zero` (exit
  101), a literal zero is E0115; float ranges with `by` (value k is
  `start + k * step`).
- **Built** (owner-answers "Step 7 batch D ... Q12 follow-ups"): named
  constants (`src/typeck/constants.rs`: non-`mut` top-level number with a
  constant value, usable in types; optional `fixed`, E0637/E0638) and constant
  arithmetic on value parameters in types (`T[N + 1]`, `Buffer<T, N * 2>`).
  Top-level values of the run (non-constant, `mut`) are E0901 until scripts.
- **To build (small)**: an unsigned loop with a literal negative `by` counts
  down with a warning (today W0102 and wraps). Answers in owner-answers
  "Follow-ups, 2026-11-13 (late)".
- **Design doc not synced yet** with the 2026-11-13 answers (tesla, unit-named
  parameters, `by` signs, float `by`, Q12 details, named constants, step 7
  batches A-D): the Docs connector was timing out. Do it next session.
- **Early language server in flight (2026-11-13)**: owner finds editing .drh
  without one painful, so step 8's core (diagnostics, recovery, hover,
  definition, completion, VS Code client) was pulled forward as its own package.
- **Step 7 WP1 merged 2026-11-13.** Step 7 spec: `docs/impl/step7-modules.md`; batches A-D answered
  (owner-answers), batch E and some sub-questions (9b, 13b, 13c, 15c, re-export,
  climbing a folder up) still open.
- **Owner questions pending (constants)**: `uint8 BIG = 300;` wraps with W0102,
  hard error instead?; a local named like a constant hides it silently, warn
  like Q4?; inference solves `[N]`, `[N +- k]`, `[k - N]`, `[N * k]` only.
- Still undesigned: `exclusive &Shape`, `unit U with X`. Not built: map-walk
  views, interface methods with their own type parameters, borrowing a generic
  interface.
- **Step 7 started** (WP1 built, see PLAN "Step 7 progress"): the module graph
  loads files; names across files are WP2. Spec: `docs/impl/step7-modules.md`.
  Originally: modules, config, scripts, stdlib needed a spec
  (drachm-architect) and an owner question round first. Today only `io` and
  `si` exist (hardcoded in `src/typeck.rs` `check`; `si` is
  `src/prelude/si.drh`).
- Agent types: use `drachm-architect` / `drachm-builder` / `drachm-writer`
  (see How work is done). Haiku by default; owner watches usage.
- `examples/showcase.drh` shows everything that builds (CLAUDE.md "The
  showcase"); its site page reads it at build time.
- Site redesign is on the roadmap (PLAN row D2), unscheduled.
- `AGENTS.md` and `docs/impl/agent-handoffs/` are untracked files from a Codex
  trial run; leave them.
- Then step 7 (modules, config, scripts, stdlib), 7.5 (MIR optimiser, own
  passes on the MIR, Cranelift stays for codegen), 8 (language server).
- Small known leftovers (E0901 today): `e::cause()`; a handler after a failing
  `!=`; the shorthand `Port p = 1, 2 maybe x`; constructor delegation; the
  per-character copy skip in `for (string c of s)` (behaviour is right, only an
  optimisation). Benchmarks were last measured under load: rerun `bench/run.sh`
  on a quiet machine (collatz looked slower after WP7, unconfirmed).
- Diagnostic wording: the owner lets me write it and will review it all in one
  pass (tables in `docs/impl/step3-ownership.md` section 4 and the step 5/6
  overviews). Not reviewed yet.

## How work is done

- **Roles** (owner's choice): the main session (Opus) orchestrates and talks to
  the owner; **Sonnet writes implementation specs and the hard implementation
  packages** (borrow checker, MIR, failures, merges with real conflicts),
  **Haiku implements standard packages** from a reviewed brief. Specs live in
  `docs/impl/`.
- **The owner decides all language behaviour.** Collect open questions from a
  spec, ask in batches of 3-4 with `AskUserQuestion` (```cpp previews, one
  original option each, no "recommended" on another language's answer), read
  the free-text notes (they often change the pick), and append the answers to
  `docs/impl/owner-answers.md` with the date, marking anything that is my
  reading rather than their words.
- **Agent types** (owner wants named agents, not "general-purpose"): `.claude/agents/`
  defines `drachm-architect` (Sonnet: hard packages, specs), `drachm-builder`
  (Haiku: standard packages) and `drachm-writer` (Haiku: docs, lessons, the
  showcase). Owner, 2026-11-13: Sonnet is costly, so Haiku takes every
  package except borrow-checker/MIR/phase-restructuring work; track Haiku
  fix-up cost per package and move a kind of work back to Sonnet if fixing
  costs more than it saved. `.claude/` is git-ignored, so these live only on this machine.
- **Agents run in git worktrees** (`isolation: worktree`), commit after every
  step (a usage limit once wiped an uncommitted agent's work), and are merged
  into master by the main session, which runs the full suite once per merge.
  Start a dependent package as soon as the commit it needs exists.
- **No watchers:** agent briefs say to run long test suites in the foreground
  (with a timeout) and not to start Monitor watchers (they piled up, owner
  noticed, 2026-11-13).
- **CPU:** the owner uses the machine. Every agent prompt says
  `nice -n 19 env CARGO_BUILD_JOBS=2..3 DRACHM_TEST_JOBS=2`, targeted tests
  while working, the expensive targets (`--test run run`,
  `optimised_output_matches`, `site_examples`, `--test leaks`, `--test backends`)
  once at the end. At most ~2-3 compiling agents at a time.
- **Usage limits** hit a few times a day. When an agent is cut off, commit its
  worktree as "WIP", then resume it with SendMessage after the reset.
- **Worktrees** pile up (~1-2 GB each): remove merged ones and their branches
  (`git worktree remove --force --force`, `git branch -D`).
- **Commits:** short plain messages, no AI attribution. **Push:** the owner
  allows pushing master after each verified merge (tests + site build green);
  ask before anything unusual (force push, new repos, making things public).

## The docs site

- `site/` (Next.js + Fumadocs, Markdown in `site/content/`, tested examples in
  `site/examples/`). Shows the **whole designed language**; unbuilt parts carry
  `status: not-implemented|tentative` badges (see `site/README.md`).
- Published at **https://drachm-lang.github.io** by `.github/workflows/docs.yml`
  on every push to master touching `site/` (the built site is pushed with a
  deploy key into the public repo `drachm-lang/drachm-lang.github.io`, branch
  `gh-pages`; the code repo `s1nnzz/drachm` stays private).
- Local: `cd site && npm run dev` (port 3000).
- Every feature change updates the site in the same commit (CLAUDE.md).

## Owner's tools

- `drachm` on PATH: `~/.local/bin/drachm` -> `target/debug/drachm`.
- VS Code: `~/.vscode/extensions/drachm-0.1.0` -> `editors/vscode` (run
  `npm run compile` there after TypeScript changes); vscode-icons custom icon in
  `~/.config/Code/User/vsicons-custom-icons/file_type_drachm.svg`.
- `examples/stupid.drh` is the owner's scratch file, now tracked as a real
  example (`tests/examples/stupid.out`): all tests pass, no expected failures.
  If the owner edits it, rebless its `.out` (ask first).
