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
block, followed by `examples/NAME.out` in a collapsed "Output" block.

## Highlighting

Shiki, with the `drachm` language loaded at build time from
`../editors/vscode/syntaxes/drachm.tmLanguage.json`. Themes: vitesse-light and
vitesse-dark.

## Layout

`lib/content.ts` reads content and builds the Fumadocs page tree,
`lib/render.tsx` compiles Markdown, `lib/remark-example.ts` expands example
markers, `app/page.tsx` is the landing page. Search is a static index at
`/api/search`.
