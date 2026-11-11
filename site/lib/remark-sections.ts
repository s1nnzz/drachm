import type { Root, RootContent } from 'mdast';
import type { VFile } from 'vfile';

/** A node that belongs in the code column: fenced code and expanded example blocks. */
function isCode(node: RootContent): boolean {
  if (node.type === 'code') return true;
  return (
    node.type === 'mdxJsxFlowElement' &&
    node.name === 'details' &&
    node.attributes.some((a) => a.type === 'mdxJsxAttribute' && a.name === 'className' && a.value === 'drh-output')
  );
}

const wrap = (name: string, className: string, children: RootContent[]): RootContent =>
  ({
    type: 'mdxJsxFlowElement',
    name,
    attributes: [{ type: 'mdxJsxAttribute', name: 'className', value: className }],
    children,
  }) as never;

/**
 * Splits each page into sections at `##` and `###` headings. A section with
 * code becomes `<section class="drh-step">` holding its prose (left) and its
 * code, in source order (right). Sections without code, and the intro before
 * the first heading, stay as they are (full width).
 *
 * Runs after the fumadocs plugins so the TOC still sees the headings. Sets
 * `data.drhSplit` on the tree when any section was split.
 */
export function remarkSections() {
  return (tree: Root, file: VFile) => {
    const groups: RootContent[][] = [[]];
    for (const node of tree.children) {
      if (node.type === 'heading' && (node.depth === 2 || node.depth === 3)) groups.push([]);
      groups[groups.length - 1].push(node);
    }

    const out: RootContent[] = [];
    let split = false;
    for (const group of groups) {
      if (!group.some(isCode)) {
        out.push(...group);
        continue;
      }
      split = true;
      out.push(
        wrap('section', 'drh-step', [
          wrap('div', 'drh-prose', group.filter((n) => !isCode(n))),
          wrap('div', 'drh-code-col', group.filter(isCode)),
        ]),
      );
    }
    tree.children = out;
    file.data.drhSplit = split;
  };
}
