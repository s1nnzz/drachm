import { createSearchAPI } from 'fumadocs-core/search/server';
import { flatDocs, docUrl } from '@/lib/content';

export const dynamic = 'force-static';

const api = createSearchAPI('simple', {
  indexes: flatDocs.map((d) => ({
    title: d.title,
    description: d.description,
    content: d.body,
    url: docUrl(d),
  })),
});

export const GET = api.staticGET;
