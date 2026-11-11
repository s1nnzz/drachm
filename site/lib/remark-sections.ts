import type { Root, RootContent } from 'mdast';
import type { VFile } from 'vfile';

const FOCUS = /^\s*<!--\s*focus:\s*([\d,\s-]+?)\s*-->\s*$/;
const SOLUTION_OF = /^\s*<!--\s*solution-of:[^>]*-->\s*$/;

const hasClass = (node: RootContent, cls: string) =>
  node.type === 'mdxJsxFlowElement' &&
  node.attributes.some((a) => a.type === 'mdxJsxAttribute' && a.name === 'className' && a.value === cls);

/** A node that belongs in the code column: fenced code and collapsed solutions. */
const isCode = (node: RootContent): boolean => node.type === 'code' || hasClass(node, 'drh-solution');

const el = (name: string, className: string, children: RootContent[]): RootContent =>
  ({
    type: 'mdxJsxFlowElement',
    name,
    attributes: [{ type: 'mdxJsxAttribute', name: 'className', value: className }],
    children,
  }) as never;

const text = (value: string): RootContent => ({ type: 'text', value }) as never;

function headingText(node: RootContent): string {
  if (node.type !== 'heading') return '';
  return node.children.map((c) => ('value' in c ? String(c.value) : '')).join('');
}

/**
 * Prepares a page's top-level nodes:
 * - `<!-- focus: 3-5 -->` marks the next paragraph with `data-focus`
 *   (the client script links it to lines of its section's first code block);
 * - `<!-- solution-of: x -->` is dropped;
 * - a fence whose meta has the word `solution` becomes a collapsed
 *   `<details class="drh-solution">`.
 */
function prepare(nodes: RootContent[]): RootContent[] {
  const out: RootContent[] = [];
  let focus: string | null = null;
  for (const node of nodes) {
    if (node.type === 'html') {
      const f = FOCUS.exec(node.value);
      if (f) {
        focus = f[1].replace(/\s+/g, '');
        continue;
      }
      if (SOLUTION_OF.test(node.value)) continue;
    }
    if (node.type === 'paragraph' && focus) {
      node.data = { ...node.data, hProperties: { 'data-focus': focus, className: ['drh-focus'] } };
      focus = null;
    } else if (node.type !== 'html') {
      focus = null;
    }
    if (node.type === 'code' && /(^|\s)solution(\s|$)/.test(node.meta ?? '')) {
      out.push(el('details', 'drh-solution', [el('summary', 'drh-solution-toggle', [text('solution')]), node]));
      continue;
    }
    out.push(node);
  }
  return out;
}

/**
 * Splits each page into sections at `##` and `###` headings. A section with
 * code becomes `<section class="drh-step">` holding its prose (left) and its
 * code, in source order (right). Sections without code, and the intro before
 * the first heading, stay as they are (full width). A section whose heading
 * starts with "Your turn" is an exercise (`drh-exercise`, with a label).
 *
 * Runs after the fumadocs plugins so the TOC still sees the headings. Sets
 * `data.drhSplit` on the tree when any section was split.
 */
export function remarkSections() {
  return (tree: Root, file: VFile) => {
    const groups: RootContent[][] = [[]];
    for (const node of prepare(tree.children)) {
      if (node.type === 'heading' && (node.depth === 2 || node.depth === 3)) groups.push([]);
      groups[groups.length - 1].push(node);
    }

    const out: RootContent[] = [];
    let split = false;
    for (const group of groups) {
      const exercise = /^your turn\b/i.test(headingText(group[0]));
      const hasCode = group.some(isCode);
      if (!hasCode && !exercise) {
        out.push(...group);
        continue;
      }
      const prose = group.filter((n) => !isCode(n));
      if (exercise) prose.unshift(el('div', 'drh-exercise-label', [text('Exercise')]));
      const children = [el('div', 'drh-prose', prose)];
      if (hasCode) {
        split = true;
        children.push(el('div', 'drh-code-col', group.filter(isCode)));
      }
      out.push(el('section', `drh-step${exercise ? ' drh-exercise' : ''}${hasCode ? '' : ' drh-nocode'}`, children));
    }
    tree.children = out;
    file.data.drhSplit = split;
  };
}
