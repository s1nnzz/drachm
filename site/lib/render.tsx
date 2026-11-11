import { createCompiler } from '@fumadocs/mdx-remote';
import { rehypeCodeDefaultOptions } from 'fumadocs-core/mdx-plugins';
import { remarkExample } from './remark-example';
import { drachmLang, themes } from './shiki';
import type { Doc } from './content';

const compiler = createCompiler({
  format: 'md',
  remarkPlugins: (v) => [remarkExample, ...v],
  rehypeCodeOptions: {
    ...rehypeCodeDefaultOptions,
    themes,
    langs: [drachmLang()],
  },
});

export async function renderDoc(doc: Doc) {
  const { body, toc } = await compiler.compile({ source: doc.body, filePath: doc.file || 'x.md' });
  return { Body: body, toc };
}
