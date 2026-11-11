import type { Root, RootContent } from 'mdast';

const MARKER = /^\s*<!--\s*status:\s*([\w-]+)\s*-->\s*$/;
const LABEL: Record<string, string> = {
  'not-implemented': 'Not implemented yet',
  tentative: 'Tentative: may change',
};

/**
 * A line `<!-- status: not-implemented -->` or `<!-- status: tentative -->`
 * directly under a heading marks that section: the comment is removed and a
 * badge is appended to the heading. A marker that is not directly under a
 * heading, or names an unknown status, is dropped with a warning (it never
 * breaks the page). Pages without headings or markers pass through untouched.
 * Must run before `remarkSections`, which looks at heading children only by text.
 */
export function remarkStatus() {
  return (tree: Root) => {
    const out: RootContent[] = [];
    for (const node of tree.children) {
      const m = node.type === 'html' ? MARKER.exec(node.value) : null;
      if (!m) {
        out.push(node);
        continue;
      }
      const prev = out[out.length - 1];
      const label = LABEL[m[1]];
      if (prev?.type === 'heading' && label) {
        prev.children.push({
          type: 'mdxJsxTextElement',
          name: 'span',
          attributes: [{ type: 'mdxJsxAttribute', name: 'className', value: `drh-status drh-status-${m[1]}` }],
          children: [{ type: 'text', value: label }],
        } as never);
      } else {
        console.warn(`status marker "${m[1]}" is not directly under a heading (or unknown); ignored`);
      }
    }
    tree.children = out;
  };
}
