import { existsSync, readdirSync, readFileSync, statSync } from 'node:fs';
import path from 'node:path';
import type { Root } from 'mdast';
import { examplesDir } from './content';

const MARKER = /^\s*<!--\s*example:\s*([\w./-]+?)\s*-->\s*$/;
const SHOWCASE = /^\s*<!--\s*showcase\s*-->\s*$/;

// The showcase is the compiler's own tested example (a project folder,
// `examples/showcase/`), read straight from the repo so the page can never
// drift from it.
const repoRoot = path.resolve(process.cwd(), process.env.DRACHM_REPO_DIR || '..');

/** Every `.drh` file below `dir`, as paths relative to `base`, main.drh first. */
function projectFiles(dir: string, base: string): string[] {
  const found: string[] = [];
  for (const name of readdirSync(dir).sort()) {
    const full = path.join(dir, name);
    if (statSync(full).isDirectory()) found.push(...projectFiles(full, base));
    else if (name.endsWith('.drh')) found.push(path.relative(base, full).split(path.sep).join('/'));
  }
  return found;
}

/**
 * Replaces `<!-- example: NAME -->` with the contents of examples/NAME.drh as a
 * ```drachm block (or, for a folder examples/NAME/ that is a project, one block
 * per file and its expect.out). If examples/NAME.in exists, it follows as a ```text
 * title="input" block (the program's standard input), and then examples/NAME.out
 * as a ```text title="output" block.
 */
export function remarkExample() {
  return (tree: Root) => {
    const out: Root['children'] = [];
    for (const node of tree.children) {
      if (node.type === 'html' && SHOWCASE.test(node.value)) {
        const project = path.join(repoRoot, 'examples/showcase');
        const expected = path.join(project, 'expect.out');
        if (!existsSync(project) || !existsSync(expected)) throw new Error(`showcase: ${project} or ${expected} not found`);
        // The entry file first, then the others in path order, one block each.
        const files = projectFiles(project, project).sort((a, b) => (a === 'src/main.drh' ? -1 : b === 'src/main.drh' ? 1 : a < b ? -1 : 1));
        const code = (lang: string, title: string, file: string) =>
          ({ type: 'code', lang, meta: `title="${title}"`, value: readFileSync(file, 'utf8').replace(/\n+$/, '') }) as never;
        out.push({
          type: 'mdxJsxFlowElement',
          name: 'DrhShowcase',
          attributes: [{ type: 'mdxJsxAttribute', name: 'files', value: JSON.stringify(files) }],
          children: [...files.map((f) => code('drachm', f, path.join(project, f))), code('text', 'output', expected)],
        } as never);
        continue;
      }
      const m = node.type === 'html' ? MARKER.exec(node.value) : null;
      if (!m) {
        out.push(node);
        continue;
      }
      const name = m[1];
      // A folder is a project of several files: one block per file, then the output.
      const folder = path.join(examplesDir, name);
      if (existsSync(folder) && statSync(folder).isDirectory()) {
        const files = projectFiles(folder, folder).sort((a, b) => (a === 'src/main.drh' ? -1 : b === 'src/main.drh' ? 1 : a < b ? -1 : 1));
        for (const f of files) {
          out.push({ type: 'code', lang: 'drachm', meta: `title="${f}"`, value: readFileSync(path.join(folder, f), 'utf8').replace(/\n+$/, '') });
        }
        const expected = path.join(folder, 'expect.out');
        if (existsSync(expected)) {
          out.push({ type: 'code', lang: 'text', meta: 'title="output"', value: readFileSync(expected, 'utf8').replace(/\n+$/, '') });
        }
        continue;
      }
      const drh = path.join(examplesDir, `${name}.drh`);
      if (!existsSync(drh)) throw new Error(`example "${name}": ${drh} not found`);
      out.push({ type: 'code', lang: 'drachm', meta: `title="${name}.drh"`, value: readFileSync(drh, 'utf8').replace(/\n+$/, '') });
      const inFile = path.join(examplesDir, `${name}.in`);
      if (existsSync(inFile)) {
        out.push({ type: 'code', lang: 'text', meta: 'title="input"', value: readFileSync(inFile, 'utf8').replace(/\n+$/, '') });
      }
      const outFile = path.join(examplesDir, `${name}.out`);
      if (existsSync(outFile)) {
        out.push({ type: 'code', lang: 'text', meta: 'title="output"', value: readFileSync(outFile, 'utf8').replace(/\n+$/, '') });
      }
    }
    tree.children = out;
  };
}
