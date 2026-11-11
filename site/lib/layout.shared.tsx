import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { Wordmark } from '@/components/logo';

export const GITHUB = 'https://github.com/s1nnzz/drachm';

export const baseOptions: BaseLayoutProps = {
  nav: { title: <Wordmark />, url: '/' },
  githubUrl: GITHUB,
  links: [{ text: 'Docs', url: '/docs', active: 'nested-url' }],
};
