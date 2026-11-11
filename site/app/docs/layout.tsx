import type { ReactNode } from 'react';
import { DocsLayout } from 'fumadocs-ui/layouts/docs';
import { baseOptions } from '@/lib/layout.shared';
import { source } from '@/lib/content';

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <DocsLayout {...baseOptions} tree={source.getPageTree()} tabs={false} sidebar={{ defaultOpenLevel: 1 }}>
      {children}
    </DocsLayout>
  );
}
