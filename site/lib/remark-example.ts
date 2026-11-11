import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import type { Root } from 'mdast';
import { examplesDir } from './content';

const MARKER = /^\s*<!--\s*example:\s*([\w./-]+?)\s*-->\s*$/;

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
