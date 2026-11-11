import type { ReactNode } from 'react';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import type { Node } from 'fumadocs-core/page-tree';
import { baseOptions } from '@/lib/layout.shared';
import { source, statusByUrl, STATUS_SHORT } from '@/lib/content';

// Sidebar entries of marked pages get a small badge after the title.
function badge(nodes: Node[]): Node[] {
  return nodes.map((n) => {
    if (n.type === 'page') {
      const st = statusByUrl.get(n.url);
      if (!st) return n;
      return {
        ...n,
        name: (
          <>
            {n.name}
            <span className={`drh-status drh-status-${st} drh-status-side`}>{STATUS_SHORT[st]}</span>
          </>
        ),
      };
    }
    if (n.type === 'folder') return { ...n, children: badge(n.children) };
    return n;
  });
}

export default function Layout({ children }: { children: ReactNode }) {
  const tree = source.getPageTree();
  const marked = { ...tree, children: badge(tree.children) };
  return (
    <DocsLayout {...baseOptions} tree={marked} tabs={false} sidebar={{ defaultOpenLevel: 1 }}>
      {children}
    </DocsLayout>
  );
}
