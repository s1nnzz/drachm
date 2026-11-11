# Drachm site

Static Next.js + Fumadocs site for the Drachm docs.

```sh
npm install
npm run dev
npm run build        # static export to out/
```

- `NEXT_BASE_PATH=/drachm npm run build` for serving from a sub-path (GitHub Pages).
- `DRACHM_CONTENT_DIR=.sample-content npm run build` reads pages from another
  directory. Examples then come from `<that dir>/examples` (override with
  `DRACHM_EXAMPLES_DIR`). Default: `content/` and `examples/`.

## Content

`content/*.md`, each with front matter `title:` and `description:` (plain
`key: value` lines, no YAML quoting needed). The first `# H1` is dropped since
the title comes from front matter. Pages are plain Markdown (no JSX).

`content/order.json` sets the reading order and the sidebar sections:

```json
{ "sections": [ { "title": "Start", "pages": ["index", "install"] } ] }
```

Slugs are file names without `.md`. `index` is the docs home (`/docs`); the
others live at `/docs/<slug>`. A listed page that does not exist fails the
build; pages not listed are not published. The landing page's chapter list is
generated from the same file.

A line `<!-- example: NAME -->` becomes `examples/NAME.drh` as a `drachm` code
block, followed by `examples/NAME.out` as a `text` block titled "output".

### Status markers

The site shows the language as designed, so pages and sections about things the
compiler does not accept yet are marked:

- `status: not-implemented` (decided in `docs/SPEC.md`, not built) or
  `status: tentative` (still open, may change) in a page's front matter marks
  the whole page: a badge after the title and a small badge in the sidebar.
- A line `<!-- status: not-implemented -->` or `<!-- status: tentative -->`
  directly under a `##`/`###` heading marks that section with a badge after the
  heading. On a page that is already marked as a whole, use section markers only
  for exceptions (a tentative section on a not-implemented page).

A marker that is not directly under a heading, or has an unknown status, is
ignored with a build warning. Unbuilt features use untested inline `drachm`
blocks (no "run this" wording; output blocks are titled `intended output`).
Built features have tested examples in `examples/`. When a feature lands,
remove its marker in the same commit. Implemented in `lib/remark-status.ts`.

### Code in lessons

Each `##`/`###` section with code is prose (left) plus one code panel (right).
Fence meta: `title="hello.drh"` adds a label (`output` / `expected output`
render as a terminal strip). `solution` collapses the block behind "Show
solution". A paragraph starting with `<!-- focus: 3-5 -->` (or `3,7-8`) is
linked to those lines of the section's first code block: the paragraph nearest
the viewport middle, or hovered, highlights them. Headings starting "Your turn"
are exercises. `<!-- solution-of: NAME -->` renders nothing.

## Highlighting

Shiki, with the `drachm` language loaded at build time from
`../editors/vscode/syntaxes/drachm.tmLanguage.json`. Themes: vitesse-light and
vitesse-dark.

## Layout

`lib/content.ts` reads content and builds the Fumadocs page tree,
`lib/render.tsx` compiles Markdown, `lib/remark-example.ts` expands example
markers, `app/page.tsx` is the landing page. Search is a static index at
`/api/search`.
