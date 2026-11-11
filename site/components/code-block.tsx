'use client';
import { useRef, useState } from 'react';
import type { ComponentProps } from 'react';

const OUTPUT = /^(expected )?output$/i;

/**
 * One code block: an optional small label, a hover copy button and the
 * highlighted lines. No border or background of its own; the surrounding
 * `.drh-code-col` panel (or the page, for blocks outside a section) is the surface.
 */
export function CodeBlock({ title, children, className }: ComponentProps<'pre'> & { title?: string }) {
  const pre = useRef<HTMLPreElement>(null);
  const [copied, setCopied] = useState(false);
  const output = typeof title === 'string' && OUTPUT.test(title.trim());

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(pre.current?.textContent ?? '');
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    } catch {
      /* clipboard unavailable */
    }
  };

  return (
    <figure className={`drh-block not-prose${output ? ' drh-term' : ''}`}>
      {title ? <figcaption className="drh-label">{title}</figcaption> : null}
      <button type="button" className="drh-copy" onClick={copy} aria-label="Copy code">
        {copied ? 'Copied' : 'Copy'}
      </button>
      <div className="drh-scroll" tabIndex={0} role="region" aria-label={title || 'Code'}>
        <pre ref={pre} className={className}>
          {children}
        </pre>
      </div>
    </figure>
  );
}
