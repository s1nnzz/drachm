import type { Root } from 'mdast';

const MD_LINK = /^(?:\.\/)?([\w.-]+)\.md(#[\w-]*)?$/;

/**
 * Content pages link to each other by file name (`[Units](reference-units.md#scaled)`),
 * which also works when reading the Markdown on GitHub. On the site those become
 * `/docs/reference-units#scaled` (`index.md` is `/docs`).
 */
export function remarkLinks() {
  return (tree: Root) => {
    const walk = (node: any) => {
      if ((node.type === 'link' || node.type === 'definition') && typeof node.url === 'string') {
        const m = MD_LINK.exec(node.url);
        if (m) node.url = (m[1] === 'index' ? '/docs' : `/docs/${m[1]}`) + (m[2] ?? '');
      }
      node.children?.forEach(walk);
    };
    walk(tree);
  };
}
