'use client';
import { useEffect } from 'react';

/** "3,7-8" -> Set{3,7,8} */
function parseLines(spec: string): Set<number> {
  const set = new Set<number>();
  for (const part of spec.split(',')) {
    const [a, b] = part.split('-').map(Number);
    if (!Number.isFinite(a)) continue;
    for (let n = a; n <= (Number.isFinite(b) ? b : a); n++) set.add(n);
  }
  return set;
}

/**
 * Links `p[data-focus]` paragraphs to lines of their section's first code block.
 * The paragraph nearest the vertical middle of the viewport (tracked with an
 * IntersectionObserver), or the hovered one, highlights its lines and dims the
 * rest. Renders nothing; mount once per page.
 */
export function Walkthrough() {
  useEffect(() => {
    const paras = Array.from(document.querySelectorAll<HTMLElement>('p[data-focus]'));
    if (!paras.length) return;

    const visible = new Set<HTMLElement>();
    let hovered: HTMLElement | null = null;
    let active: HTMLElement | null = null;
    let raf = 0;

    const codeOf = (p: HTMLElement) =>
      p.closest('.drh-step')?.querySelector<HTMLElement>(':scope > .drh-code-col pre:not(details *)') ?? null;

    const apply = (next: HTMLElement | null) => {
      if (next === active) return;
      if (active) {
        active.removeAttribute('data-active');
        const old = codeOf(active);
        old?.removeAttribute('data-focus-on');
        old?.querySelectorAll('.line[data-hl]').forEach((l) => l.removeAttribute('data-hl'));
      }
      active = next;
      if (!next) return;
      next.setAttribute('data-active', '');
      const code = codeOf(next);
      if (!code) return;
      const lines = parseLines(next.dataset.focus ?? '');
      code.setAttribute('data-focus-on', '');
      code.querySelectorAll('.line').forEach((l, i) => {
        if (lines.has(i + 1)) l.setAttribute('data-hl', '');
      });
    };

    const pick = () => {
      raf = 0;
      if (hovered) return apply(hovered);
      const mid = window.innerHeight / 2;
      let best: HTMLElement | null = null;
      let bestDist = Infinity;
      for (const p of visible) {
        const r = p.getBoundingClientRect();
        const d = mid >= r.top && mid <= r.bottom ? 0 : Math.min(Math.abs(r.top - mid), Math.abs(r.bottom - mid));
        if (d < bestDist) (best = p), (bestDist = d);
      }
      apply(best);
    };
    const schedule = () => {
      if (!raf) raf = requestAnimationFrame(pick);
    };

    // Only paragraphs inside the middle band are candidates.
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (e.isIntersecting) visible.add(e.target as HTMLElement);
          else visible.delete(e.target as HTMLElement);
        }
        schedule();
      },
      { rootMargin: '-25% 0px -25% 0px' },
    );
    const enter = (e: Event) => ((hovered = e.currentTarget as HTMLElement), schedule());
    const leave = () => ((hovered = null), schedule());
    for (const p of paras) {
      io.observe(p);
      p.addEventListener('mouseenter', enter);
      p.addEventListener('mouseleave', leave);
    }
    window.addEventListener('scroll', schedule, { passive: true });
    window.addEventListener('resize', schedule);
    return () => {
      io.disconnect();
      cancelAnimationFrame(raf);
      window.removeEventListener('scroll', schedule);
      window.removeEventListener('resize', schedule);
      for (const p of paras) {
        p.removeEventListener('mouseenter', enter);
        p.removeEventListener('mouseleave', leave);
      }
      apply(null);
    };
  }, []);
  return null;
}
