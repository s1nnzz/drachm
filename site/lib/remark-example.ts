import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import type { Root } from 'mdast';
import { examplesDir } from './content';

const MARKER = /^\s*<!--\s*example:\s*([\w./-]+?)\s*-->\s*$/;

/**
 * Replaces `<!-- example: NAME -->` with the contents of examples/NAME.drh as a
 * ```drachm block, followed by examples/NAME.out in a collapsed "Output" block.
 */
export function remarkExample() {
  return (tree: Root) => {
    const out: Root['children'] = [];
    for (const node of tree.children) {
      const m = node.type === 'html' ? MARKER.exec(node.value) : null;
      if (!m) {
        out.push(node);
        continue;
      }
      const name = m[1];
      const drh = path.join(examplesDir, `${name}.drh`);
      if (!existsSync(drh)) throw new Error(`example "${name}": ${drh} not found`);
      out.push({ type: 'code', lang: 'drachm', meta: `title="${name}.drh"`, value: readFileSync(drh, 'utf8').replace(/\n+$/, '') });
      const outFile = path.join(examplesDir, `${name}.out`);
      if (existsSync(outFile)) {
        out.push({
          type: 'mdxJsxFlowElement',
          name: 'details',
          attributes: [{ type: 'mdxJsxAttribute', name: 'className', value: 'drh-output' }],
          children: [
            { type: 'mdxJsxFlowElement', name: 'summary', attributes: [], children: [{ type: 'text', value: 'Output' }] },
            { type: 'code', lang: 'text', value: readFileSync(outFile, 'utf8').replace(/\n+$/, '') },
          ],
        } as never);
      }
    }
    tree.children = out;
  };
}
