import './global.css';
import './lesson.css';
import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import { GeistSans } from 'geist/font/sans';
import { GeistMono } from 'geist/font/mono';
import { RootProvider } from 'fumadocs-ui/provider/next';

const base = process.env.NEXT_BASE_PATH || '';

export const metadata: Metadata = {
  title: { default: 'Drachm', template: '%s | Drachm' },
  description:
    'Drachm is a statically typed, natively compiled language with wordy English keywords, ownership checked at compile time and no garbage collector.',
  // The site has its own dark theme; ask Dark Reader not to restyle it (its
  // injected attributes also caused hydration warnings).
  other: { 'darkreader-lock': 'true' },
};

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="en" className={`${GeistSans.variable} ${GeistMono.variable}`} suppressHydrationWarning>
      <body className="flex min-h-screen flex-col font-sans">
        <RootProvider search={{ options: { type: 'static', api: `${base}/api/search` } }}>
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
