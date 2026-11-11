import { existsSync, readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { loader } from 'fumadocs-core/source';

export const contentDir = path.resolve(
  process.cwd(),
  process.env.DRACHM_CONTENT_DIR || 'content',
);
// Examples live in site/examples, or in <DRACHM_CONTENT_DIR>/examples when the
// content dir is overridden (DRACHM_EXAMPLES_DIR overrides both).
export const examplesDir = path.resolve(
  process.cwd(),
  process.env.DRACHM_EXAMPLES_DIR ||
    (process.env.DRACHM_CONTENT_DIR ? path.join(contentDir, 'examples') : 'examples'),
);

export interface Doc {
  slug: string;
  title: string;
  description: string;
  body: string; // markdown, front matter removed
  file: string;
}
export interface Section { title: string; docs: Doc[] }

// Front matter is simple `key: value` lines. Parsed by hand (not as YAML) so a
// description may contain colons without quoting.
function frontMatter(text: string): { data: Record<string, string>; content: string } {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?/.exec(text);
  if (!m) return { data: {}, content: text };
  const data: Record<string, string> = {};
  for (const line of m[1].split(/\r?\n/)) {
    const kv = /^([\w-]+):\s*(.*)$/.exec(line);
    if (!kv) continue;
    data[kv[1]] = kv[2].replace(/^(["'])(.*)\1$/, '$2');
  }
  return { data, content: text.slice(m[0].length) };
}

function readDocs(): Map<string, Doc> {
  const docs = new Map<string, Doc>();
  if (!existsSync(contentDir)) return docs;
  for (const f of readdirSync(contentDir)) {
    if (!f.endsWith('.md')) continue;
    const file = path.join(contentDir, f);
    const { data, content } = frontMatter(readFileSync(file, 'utf8'));
    const slug = f.slice(0, -3);
    docs.set(slug, {
      slug,
      title: String(data.title ?? slug),
      description: String(data.description ?? ''),
      // The page title is rendered from front matter, so drop a duplicate H1.
      body: content.replace(/^\s*# .*\n/, ''),
      file,
    });
  }
  return docs;
}

function readSections(docs: Map<string, Doc>): Section[] {
  const orderFile = path.join(contentDir, 'order.json');
  if (existsSync(orderFile)) {
    const order = JSON.parse(readFileSync(orderFile, 'utf8')) as {
      sections: { title: string; pages: string[] }[];
    };
    return order.sections.map((s) => ({
      title: s.title,
      docs: s.pages.map((slug) => {
        const d = docs.get(slug);
        if (!d) throw new Error(`order.json lists "${slug}" but ${slug}.md does not exist in ${contentDir}`);
        return d;
      }),
    }));
  }
  const all = [...docs.values()].sort((a, b) => a.slug.localeCompare(b.slug));
  return all.length ? [{ title: 'Documentation', docs: all }] : [];
}

const docs = readDocs();
let sections = readSections(docs);
if (sections.length === 0) {
  const placeholder: Doc = {
    slug: 'index',
    title: 'Coming soon',
    description: 'The documentation is being written.',
    body: 'The documentation is being written. Check back soon.',
    file: '',
  };
  sections = [{ title: 'Documentation', docs: [placeholder] }];
}
export { sections };
export const flatDocs: Doc[] = sections.flatMap((s) => s.docs);
export const docBySlug = new Map(flatDocs.map((d) => [d.slug, d]));

export const source = loader({
  baseUrl: '/docs',
  source: {
    files: [
      ...flatDocs.map((d) => ({
        type: 'page' as const,
        path: `${d.slug}.md`,
        data: { title: d.title, description: d.description },
      })),
      {
        type: 'meta' as const,
        path: 'meta.json',
        data: {
          pages: sections.flatMap((s) => [`---${s.title}---`, ...s.docs.map((d) => d.slug)]),
        },
      },
    ],
  },
});

/** URL of a doc (index.md is the /docs landing page). */
export function docUrl(d: Doc) {
  return d.slug === 'index' ? '/docs' : `/docs/${d.slug}`;
}
