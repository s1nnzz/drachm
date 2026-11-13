import { notFound } from 'next/navigation';
import type { Metadata } from 'next';
import { DocsPage, DocsBody, DocsTitle, DocsDescription } from 'fumadocs-ui/layouts/docs/page';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import { getContent, STATUS_LABEL } from '@/lib/content';
import { renderDoc } from '@/lib/render';
import { CodeBlock } from '@/components/code-block';
import { Walkthrough } from '@/components/walkthrough';
import { Showcase } from '@/components/showcase';

type Props = { params: Promise<{ slug?: string[] }> };

const slugOf = (slug?: string[]) => (slug?.length ? slug.join('/') : 'index');

export function generateStaticParams() {
  return getContent().flatDocs.map((d) => ({ slug: d.slug === 'index' ? [] : [d.slug] }));
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const doc = getContent().docBySlug.get(slugOf((await params).slug));
  if (!doc) return {};
  return { title: doc.title, description: doc.description };
}

export default async function Page({ params }: Props) {
  const doc = getContent().docBySlug.get(slugOf((await params).slug));
  if (!doc) notFound();
  const { Body, toc, split } = await renderDoc(doc);
  // Pages with prose/code sections use the wide layout and drop the right-hand TOC
  // (`full` hides it and widens the article).
  return (
    <DocsPage toc={toc} full={split}>
      <DocsTitle>
        {doc.title}
        {doc.status && <span className={`drh-status drh-status-${doc.status}`}>{STATUS_LABEL[doc.status]}</span>}
      </DocsTitle>
      <DocsDescription>{doc.description}</DocsDescription>
      <DocsBody>
        <Body components={{ ...defaultMdxComponents, pre: CodeBlock, DrhShowcase: Showcase }} />
        <Walkthrough key={doc.slug} />
      </DocsBody>
    </DocsPage>
  );
}
