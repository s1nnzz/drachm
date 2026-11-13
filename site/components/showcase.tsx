'use client';
import { Children, useEffect, useRef, useState } from 'react';
import type { KeyboardEvent, PointerEvent, ReactNode } from 'react';

// The showcase block: the program beside its output. Each pane scrolls on its
// own. A vertical splitter sets the source/output ratio, a bottom handle sets
// the height, and Expand turns the block into a full-screen overlay.
// Drag and keyboard changes write one CSS variable per frame; React state is
// only touched when a drag ends, so reflow stays cheap.

const KEY = 'drh-showcase';
const DEFAULT_SPLIT = 0.58;
const MIN_SPLIT = 0.2;
const MAX_SPLIT = 0.8;
const MIN_HEIGHT = 192; // px, 12rem

const clamp = (n: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, n));
const maxHeight = () => Math.round(window.innerHeight * 0.95);

type Saved = { split: number; height: number | null };

function load(): Saved {
  try {
    const raw = JSON.parse(window.localStorage.getItem(KEY) || 'null');
    const split = typeof raw?.split === 'number' ? clamp(raw.split, MIN_SPLIT, MAX_SPLIT) : DEFAULT_SPLIT;
    const height = typeof raw?.height === 'number' ? raw.height : null;
    return { split, height };
  } catch {
    return { split: DEFAULT_SPLIT, height: null };
  }
}

function save(s: Saved) {
  try {
    window.localStorage.setItem(KEY, JSON.stringify(s));
  } catch {
    /* storage blocked: the layout just resets on reload */
  }
}

export function Showcase({ children }: { children: ReactNode }) {
  const [source, output] = Children.toArray(children);
  const root = useRef<HTMLElement>(null);
  const slot = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const expandBtn = useRef<HTMLButtonElement>(null);
  const current = useRef<Saved>({ split: DEFAULT_SPLIT, height: null });
  const [expanded, setExpanded] = useState(false);
  const [split, setSplit] = useState(DEFAULT_SPLIT);
  const [height, setHeight] = useState(0);

  const applySplit = (v: number) => root.current?.style.setProperty('--sc-split', `${(v * 100).toFixed(2)}%`);
  const applyHeight = (px: number) => root.current?.style.setProperty('--sc-h', `${Math.round(px)}px`);

  // Restore the saved layout after mount (the server render uses the defaults).
  useEffect(() => {
    const saved = load();
    current.current = saved;
    applySplit(saved.split);
    if (saved.height !== null) applyHeight(clamp(saved.height, MIN_HEIGHT, maxHeight()));
    setSplit(saved.split);
    setHeight(body.current?.getBoundingClientRect().height ?? 0);
  }, []);

  // Esc closes the overlay; the page is not scrollable behind it.
  useEffect(() => {
    if (!expanded) return;
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === 'Escape') setExpanded(false);
    };
    document.addEventListener('keydown', onKey);
    const was = document.body.style.overflow;
    document.body.style.overflow = 'hidden';
    return () => {
      document.removeEventListener('keydown', onKey);
      document.body.style.overflow = was;
    };
  }, [expanded]);

  // Keep the slot's height while the block is in the overlay, so the page does not jump.
  const toggle = (open: boolean) => {
    if (open && slot.current) slot.current.style.height = `${slot.current.offsetHeight}px`;
    if (!open && slot.current) slot.current.style.height = '';
    setExpanded(open);
    if (!open) requestAnimationFrame(() => expandBtn.current?.focus());
  };

  // Vertical splitter between source and output (wide layout only).
  const dragSplit = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !body.current) return;
    e.preventDefault();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    const rect = body.current.getBoundingClientRect();
    let next = current.current.split;
    let frame = 0;
    const move = (ev: globalThis.PointerEvent) => {
      next = clamp((ev.clientX - rect.left) / rect.width, MIN_SPLIT, MAX_SPLIT);
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          applySplit(next);
        });
    };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.removeEventListener('pointercancel', up);
      current.current = { ...current.current, split: next };
      setSplit(next);
      save(current.current);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
    handle.addEventListener('pointercancel', up);
  };

  const keySplit = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 0.1 : 0.02;
    let next: number | null = null;
    if (e.key === 'ArrowLeft') next = split - step;
    else if (e.key === 'ArrowRight') next = split + step;
    else if (e.key === 'Home') next = MIN_SPLIT;
    else if (e.key === 'End') next = MAX_SPLIT;
    if (next === null) return;
    e.preventDefault();
    const v = clamp(next, MIN_SPLIT, MAX_SPLIT);
    applySplit(v);
    setSplit(v);
    current.current = { ...current.current, split: v };
    save(current.current);
  };

  // Bottom handle: height of the block (wide layout only).
  const dragHeight = (e: PointerEvent<HTMLDivElement>) => {
    if (e.button !== 0 || !body.current) return;
    e.preventDefault();
    const handle = e.currentTarget;
    handle.setPointerCapture(e.pointerId);
    const startY = e.clientY;
    const startH = body.current.getBoundingClientRect().height;
    let next = startH;
    let frame = 0;
    const move = (ev: globalThis.PointerEvent) => {
      next = clamp(startH + ev.clientY - startY, MIN_HEIGHT, maxHeight());
      if (!frame)
        frame = requestAnimationFrame(() => {
          frame = 0;
          applyHeight(next);
        });
    };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.removeEventListener('pointercancel', up);
      current.current = { ...current.current, height: Math.round(next) };
      setHeight(Math.round(next));
      save(current.current);
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
    handle.addEventListener('pointercancel', up);
  };

  const keyHeight = (e: KeyboardEvent<HTMLDivElement>) => {
    const step = e.shiftKey ? 96 : 24;
    const now = body.current?.getBoundingClientRect().height ?? height;
    let next: number | null = null;
    if (e.key === 'ArrowUp') next = now - step;
    else if (e.key === 'ArrowDown') next = now + step;
    else if (e.key === 'Home') next = MIN_HEIGHT;
    else if (e.key === 'End') next = maxHeight();
    if (next === null) return;
    e.preventDefault();
    const px = Math.round(clamp(next, MIN_HEIGHT, maxHeight()));
    applyHeight(px);
    setHeight(px);
    current.current = { ...current.current, height: px };
    save(current.current);
  };

  return (
    <div className="drh-showcase-slot" ref={slot}>
      <section
        ref={root}
        className="drh-showcase"
        data-expanded={expanded ? '' : undefined}
        role={expanded ? 'dialog' : undefined}
        aria-modal={expanded ? true : undefined}
        aria-label={expanded ? 'Showcase, full view' : undefined}
      >
        <div className="drh-sc-bar">
          <button
            ref={expandBtn}
            type="button"
            className="drh-sc-btn"
            onClick={() => toggle(!expanded)}
            aria-label={expanded ? 'Close full view' : 'Expand to full view'}
          >
            {expanded ? 'Close' : 'Expand'}
          </button>
        </div>
        <div className="drh-sc-frame">
          <div className="drh-sc-body" ref={body}>
            {source}
            <div
              className="drh-sc-split"
              role="separator"
              aria-orientation="vertical"
              aria-label="Resize source and output"
              aria-valuemin={Math.round(MIN_SPLIT * 100)}
              aria-valuemax={Math.round(MAX_SPLIT * 100)}
              aria-valuenow={Math.round(split * 100)}
              tabIndex={0}
              onPointerDown={dragSplit}
              onKeyDown={keySplit}
            />
            {output}
          </div>
          <div
            className="drh-sc-vert"
            role="separator"
            aria-orientation="horizontal"
            aria-label="Resize block height"
            aria-valuemin={MIN_HEIGHT}
            aria-valuenow={height}
            tabIndex={0}
            onPointerDown={dragHeight}
            onKeyDown={keyHeight}
          />
        </div>
      </section>
    </div>
  );
}
