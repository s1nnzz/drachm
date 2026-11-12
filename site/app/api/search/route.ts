import { createSearchAPI } from 'fumadocs-core/search/server';
import { getContent, docUrl } from '@/lib/content';

export const dynamic = 'force-static';

const make = () =>
  createSearchAPI('simple', {
    indexes: getContent().flatDocs.map((d) => ({
      title: d.title,
      description: d.description,
      content: d.body,
      url: docUrl(d),
    })),
  });

// Built once for the static export; rebuilt per request in dev.
const prod = process.env.NODE_ENV === 'development' ? undefined : make();

export const GET = () => (prod ?? make()).staticGET();
