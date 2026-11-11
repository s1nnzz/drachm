import { notFound } from 'next/navigation';
import type { Metadata } from 'next';
import { DocsPage, DocsBody, DocsTitle, DocsDescription } from 'fumadocs-ui/layouts/docs/page';
import defaultMdxComponents from 'fumadocs-ui/mdx';
import { docBySlug, flatDocs } from '@/lib/content';
import { renderDoc } from '@/lib/render';

type Props = { params: Promise<{ slug?: string[] }> };

const slugOf = (slug?: string[]) => (slug?.length ? slug.join('/') : 'index');

export function generateStaticParams() {
  return flatDocs.map((d) => ({ slug: d.slug === 'index' ? [] : [d.slug] }));
}

export async function generateMetadata({ params }: Props): Promise<Metadata> {
  const doc = docBySlug.get(slugOf((await params).slug));
  if (!doc) return {};
  return { title: doc.title, description: doc.description };
}

export default async function Page({ params }: Props) {
  const doc = docBySlug.get(slugOf((await params).slug));
  if (!doc) notFound();
  const { Body, toc } = await renderDoc(doc);
  return (
    <DocsPage toc={toc}>
      <DocsTitle>{doc.title}</DocsTitle>
      <DocsDescription>{doc.description}</DocsDescription>
      <DocsBody>
        <Body components={defaultMdxComponents} />
      </DocsBody>
    </DocsPage>
  );
}
