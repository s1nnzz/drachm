import Link from 'next/link';
import { HomeLayout } from 'fumadocs-ui/layouts/home';
import { baseOptions, GITHUB } from '@/lib/layout.shared';
import { highlight } from '@/lib/shiki';
import { sections, docUrl, flatDocs } from '@/lib/content';

const PROGRAM = `none main() {
    uint32 hp = 100;
    hp = hp - 10;
}
`;

async function Code({ code }: { code: string }) {
  const html = await highlight(code);
  return <div className="drh-code" dangerouslySetInnerHTML={{ __html: html }} />;
}

function Panel({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div className="flex min-w-0 flex-col">
      <div className="border-b border-fd-border px-5 py-2.5 font-mono text-xs text-fd-muted-foreground">{title}</div>
      {children}
    </div>
  );
}

const features = [
  {
    title: 'Reads like English',
    body: 'Functions start with their return type, and keywords are plain words: with, of, lives, maybe, fails with, skip.',
  },
  {
    title: 'Two backends, one behaviour',
    body: 'Plain builds go through Cranelift and compile in milliseconds. -O builds generate C and run at C speed. The test suite checks they agree.',
  },
  {
    title: 'Ownership, no garbage collector',
    body: 'Moves and borrows are checked at compile time, so there is nothing to pause and nothing to leak.',
  },
  {
    title: 'Diagnostics that show their work',
    body: 'Every error has a code, the lines involved and a suggested fix. drachm explain describes any code.',
  },
];

const btn =
  'inline-flex h-10 items-center justify-center rounded-md px-5 text-sm font-medium transition-colors';

export default function Home() {
  const first = flatDocs.find((d) => d.slug === 'install') ?? flatDocs[0];
  return (
    <HomeLayout {...baseOptions}>
      <main className="mx-auto w-full max-w-5xl flex-1 px-6">
        <section className="flex flex-col items-center py-20 text-center sm:py-28">
          <h1 className="max-w-3xl text-balance text-4xl font-semibold tracking-tight sm:text-6xl sm:leading-[1.05]">
            A fast language that reads like English.
          </h1>
          <p className="mt-6 max-w-xl text-balance text-lg text-fd-muted-foreground">
            Statically typed and natively compiled. Cranelift for builds in milliseconds, C for <code className="font-mono text-base text-fd-foreground">-O</code> speed.
          </p>
          <div className="mt-9 flex gap-3">
            <Link href="/docs" className={`${btn} bg-fd-foreground text-fd-background hover:opacity-85`}>
              Get started
            </Link>
            <a href={GITHUB} className={`${btn} border border-fd-border hover:bg-fd-secondary`}>
              GitHub
            </a>
          </div>
        </section>

        <section className="overflow-hidden rounded-lg border border-fd-border bg-fd-card">
          <div className="grid md:grid-cols-2 md:divide-x divide-y md:divide-y-0 divide-fd-border">
            <Panel title="main.drh">
              <Code code={PROGRAM} />
            </Panel>
            <Panel title="$ drachm check main.drh">
              <pre className="m-0 overflow-x-auto px-5 py-4 font-mono text-[13px] leading-[1.7] text-fd-muted-foreground">
                <span className="font-semibold text-red-600 dark:text-red-400">error[E0205]</span>
                <span className="font-semibold text-fd-foreground">: cannot change `hp`: it is not `mut`</span>
                {'\n'}
                {'  '}<span className="text-sky-600 dark:text-sky-400">--&gt;</span>{' main.drh:3:5\n'}
                {'   |\n'}
                {' 2 |     uint32 hp = 100;\n'}
                {'   |            '}<span className="text-sky-600 dark:text-sky-400">--</span>{' declared here without `mut`\n'}
                {' 3 |     hp = hp - 10;\n'}
                {'   |     '}<span className="text-red-600 dark:text-red-400">^^</span>{' assigned a second time here\n'}
                {'   |\n'}
                <span className="text-emerald-600 dark:text-emerald-400">{' help'}</span>
                {': declare it as `mut uint32 hp` to allow changes\n\n'}
                {'main.drh: 1 error\n'}
              </pre>
            </Panel>
          </div>
        </section>

        <section className="grid gap-x-12 gap-y-10 py-24 sm:grid-cols-2">
          {features.map((f) => (
            <div key={f.title}>
              <h3 className="text-base font-semibold tracking-tight">{f.title}</h3>
              <p className="mt-2 text-[15px] leading-relaxed text-fd-muted-foreground">{f.body}</p>
            </div>
          ))}
        </section>

        <section className="border-t border-fd-border py-16">
          <div className="mb-8 flex items-baseline justify-between">
            <h2 className="text-2xl font-semibold tracking-tight">Documentation</h2>
            <Link href={docUrl(first)} className="text-sm text-fd-muted-foreground hover:text-fd-foreground">
              Install Drachm
            </Link>
          </div>
          <div className="grid gap-x-12 gap-y-8 sm:grid-cols-2 lg:grid-cols-4">
            {sections.map((s) => (
              <div key={s.title}>
                <h3 className="mb-3 font-mono text-xs uppercase tracking-wider text-fd-muted-foreground">{s.title}</h3>
                <ul className="space-y-2 text-[15px]">
                  {s.docs.map((d) => (
                    <li key={d.slug}>
                      <Link href={docUrl(d)} className="hover:text-drh">{d.title}</Link>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </section>
      </main>
      <footer className="border-t border-fd-border py-8 text-center text-sm text-fd-muted-foreground">
        Drachm, a language named for an ancient Greek coin.
      </footer>
    </HomeLayout>
  );
}
