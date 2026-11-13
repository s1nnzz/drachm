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
- **In flight (2026-11-13):** a `drachm-builder` agent in worktree
  `.claude/worktrees/agent-a7ff068dde4ee3b46` (branch
  `worktree-agent-a7ff068dde4ee3b46`) building the decided step 6 leftovers.
  Committed: `a not same as b`; `same as` on maps/structs of borrows is E0633;
  `T as unit` + E0605 help names a free letter; `take xs[i] leaving v`. In
  progress (a "WIP" commit after a usage cutoff): ranges `0..10 by 3`. Not
  started: array length parameters `struct Buffer<T, uint32 N>` (Q12). If the
  agent is gone: commit any WIP, then either resume it (SendMessage to its id,
  if this session's agents survive) or start a new `drachm-builder` in that
  worktree for the rest, then merge (full `cargo test`, site build, push) and
  remove the worktree.
- Still undesigned: `exclusive &Shape`, `unit U with X`. Not built: map-walk
  views, interface methods with their own type parameters, borrowing a generic
  interface.
- **Next after that: step 7** (modules, config, scripts, stdlib): needs a spec
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
- `examples/stupid.drh` is the owner's untracked scratch file: never add,
  edit or delete it (it makes three `examples` test cases fail; that's expected).
