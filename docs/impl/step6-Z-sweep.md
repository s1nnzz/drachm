# S6Z: closing sweep for step 6

Implementer: standard model. No language change, no new feature. It runs after
every other package has merged. Its job is to make the site, the plan and the
benchmarks tell the truth, and to remove the badges that earlier packages were
not allowed to remove.

Read first: `step6-overview.md` sections 4, 5; CLAUDE.md "Keeping the docs
site current"; the six `status:`/marker locations: `site/content/{design-generics,
design-collections,lesson-15-generics,lesson-16-collections,guide-generics}.md`
and `coming-later.md`; `reference-generics.md` and `reference-collections.md`
(created by S6A and S6C); each package's final report.

## Goal

1. **Lesson 15** (`lesson-15-generics.md`): every non-tentative code block
   compiles and the exercises' starters fail (or print) as the text says and
   the solutions print their `intended output`. Remove `status: not-implemented`
   from the front matter and the sentence "The compiler can't run this lesson
   yet; the code shows the planned design." Keep the `<!-- status: tentative -->`
   marker on "Your own iterables" (Q19) and on any unit-generics text (Q2).
   Add `solution-15-N.drh` + `.out` for each exercise, by the `solution-10-N`
   convention (the files already exist from S6A/S6E for exercises 1 and 2;
   check them). If a block does not build because a Q interim rejects it, do
   **not** change the block silently: list it in the report with the
   diagnostic, leave the badge on that section as `<!-- status: not-implemented -->`,
   and let the owner decide.
2. **Lesson 16**: the same for collections (arrays, lists, indexing, changing a
   list, walking, splitting); the map-walking and map-changing text stays
   marked if Q9/Q16 are open. Page-level badge removed only if nothing but
   marked sections is unbuilt.
3. **`guide-generics.md`**: same; "Generic over units" stays tentative (Q2)
   unless A.3 and Q2 are settled.
4. **`design-generics.md`, `design-collections.md`**: remove sections whose
   content moved to the reference pages (leave a link each); remove the
   page-level `status:` when nothing unbuilt remains outside markers; rewrite
   the lead paragraph ("the compiler does not accept any of it yet") to be
   true. If both pages end up as marker-only remainders, keep them as short
   "still to come" pages rather than deleting them (the language-question
   options are the owner's to settle).
5. **Reference pages**: `reference-generics.md` and `reference-collections.md`
   read well as a whole (order, cross-links, every error section shows the real
   compiler output), `reference-types.md` lists every new type, `order.json`
   has the new pages, `coming-later.md` row 6 and bullets are accurate,
   `reference-diagnostics.md` lists E0601 to E0662 and W0601 (after
   `site/gen-errors.sh`).
6. **`docs/PLAN.md`**: tick step 6 (with the interims listed, each pointing at
   its Q number), update "Where things stand" (the file table gains
   `typeck/{generics,interfaces,lists,maps,dynamic,lambdas}.rs`,
   `mir/vtable.rs`, `runtime/lists.h`), the Performance section with the
   `collections` benchmark table. Move newly decided answers (none until the
   owner answers) into SPEC only when answered; add to SPEC "Collections"
   and "Generics" a "Built (S6x)" note in the style of the optionals note.
   `docs/SPEC.md` section 10's code-range list gets `06xx`.
7. **Benchmarks**: run `bench/run.sh` once on a quiet machine (`nice -n 19`),
   paste the `collections` rows (sum, index, generic, map, dyn) against the
   targets in overview 4 into PLAN's Performance section and the report.
8. **Question list for the owner**: re-read overview section 8, and for each
   question add what the packages learned (the "Q data" items from their
   reports) as a short "Data:" line; do not decide anything. Produce
   `docs/impl/step6-questions-for-owner.md` only if the owner asks (otherwise
   the list lives in the overview).

Out of scope: any compiler change except a typo-level fix of a diagnostic
(report it); thread/async text; modules; config; anything marked tentative.

## Files you own

`site/content/*` as listed, `site/examples/solution-15-*`, `solution-16-*`,
`docs/PLAN.md`, `docs/SPEC.md` (range list and "Built" notes only),
`docs/impl/step6-overview.md` (add "Data:" lines only), `bench/` (results
only).

## Method

For each page: run `cargo test` once (the site examples are tests), then for
each code block ask: is it in `site/examples/` with a `.out` (built), is it
marked, or is it bare design? Anything bare that the compiler now accepts gets
an example; anything the compiler rejects gets its marker restored. A script
that extracts the `drachm` fences from the five pages and feeds each through
`drachm check` (run it once, not in a loop) is a good way to find the bare
ones; keep the script out of the repo.

## Done when

- `cargo test` green; `grep -rn "status: not-implemented" site/content` lists
  only pages the owner has not settled (report the list); lessons 15 and 16
  no longer say the compiler can't run them, or the report says exactly which
  blocks block them and why.
- The PLAN progress table row 6 and the "Where things stand" text match the
  tree.
- Report: the lists above, benchmark rows, and the Q data lines added.

## Pitfalls

- A lesson block that only compiles inside a wrapper (`none main() { ... }`)
  must stay shown as in the lesson; the examples directory holds the full
  program and the lesson links the shown excerpt to it by the existing
  convention (check how lesson 13 does it).
- Do not "fix" a lesson by changing its code to avoid an interim: the site shows
  the language as designed; the badge is the honest signal.
- Do not remove a page-level badge because most of the page builds.
