import { createCompiler } from '@fumadocs/mdx-remote';
import { rehypeCodeDefaultOptions } from 'fumadocs-core/mdx-plugins';
import { remarkExample } from './remark-example';
import { remarkSections } from './remark-sections';
import { remarkStatus } from './remark-status';
import { remarkLinks } from './remark-links';
import { drachmLang, themes } from './shiki';
import type { Doc } from './content';

const compiler = createCompiler({
  format: 'md',
  remarkPlugins: (v) => [remarkExample, remarkLinks, ...v, remarkStatus, remarkSections],
  rehypeCodeOptions: {
    ...rehypeCodeDefaultOptions,
    themes,
    langs: [drachmLang()],
  },
});

export async function renderDoc(doc: Doc) {
  const { body, toc, vfile } = await compiler.compile({ source: doc.body, filePath: doc.file || 'x.md' });
  // True when remarkSections split at least one section: the page then uses the wide two-column layout.
  const split = vfile.data.drhSplit === true;
  return { Body: body, toc, split };
}
