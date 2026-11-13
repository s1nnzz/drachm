import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import type { Root } from 'mdast';
import { examplesDir } from './content';

const MARKER = /^\s*<!--\s*example:\s*([\w./-]+?)\s*-->\s*$/;
const SHOWCASE = /^\s*<!--\s*showcase\s*-->\s*$/;

// The showcase is the compiler's own tested example, read straight from the
// repo so the page can never drift from `examples/showcase.drh`.
const repoRoot = path.resolve(process.cwd(), process.env.DRACHM_REPO_DIR || '..');

/**
 * Replaces `<!-- example: NAME -->` with the contents of examples/NAME.drh as a
 * ```drachm block. If examples/NAME.in exists, it follows as a ```text
 * title="input" block (the program's standard input), and then examples/NAME.out
 * as a ```text title="output" block.
 */
export function remarkExample() {
  return (tree: Root) => {
    const out: Root['children'] = [];
    for (const node of tree.children) {
      if (node.type === 'html' && SHOWCASE.test(node.value)) {
        const src = path.join(repoRoot, 'examples/showcase.drh');
        const expected = path.join(repoRoot, 'tests/examples/showcase.out');
        if (!existsSync(src) || !existsSync(expected)) throw new Error(`showcase: ${src} or ${expected} not found`);
        const code = (lang: string, title: string, file: string) =>
          ({ type: 'code', lang, meta: `title="${title}"`, value: readFileSync(file, 'utf8').replace(/\n+$/, '') }) as never;
        // <DrhShowcase> is the client component (components/showcase.tsx) that
        // lays out the two panes with their resize handles and expand button.
        const pane = (children: unknown[]) =>
          ({
            type: 'mdxJsxFlowElement',
            name: 'div',
            attributes: [{ type: 'mdxJsxAttribute', name: 'className', value: 'drh-sc-pane' }],
            children,
          }) as never;
        out.push({
          type: 'mdxJsxFlowElement',
          name: 'DrhShowcase',
          attributes: [],
          children: [pane([code('drachm', 'showcase.drh', src)]), pane([code('text', 'output', expected)])],
        } as never);
        continue;
      }
      const m = node.type === 'html' ? MARKER.exec(node.value) : null;
      if (!m) {
        out.push(node);
        continue;
      }
      const name = m[1];
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
