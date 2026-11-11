import { readFileSync } from 'node:fs';
import path from 'node:path';
import { createHighlighter } from 'shiki';
import type { HighlighterGeneric, LanguageRegistration } from 'shiki';

// The Drachm grammar is read straight from the editor extension so the site
// and the editor always highlight the same way.
const GRAMMAR = path.resolve(
  process.cwd(),
  '../editors/vscode/syntaxes/drachm.tmLanguage.json',
);

export function drachmLang(): LanguageRegistration {
  const grammar = JSON.parse(readFileSync(GRAMMAR, 'utf8'));
  return { ...grammar, name: 'drachm', aliases: ['drh'] } as LanguageRegistration;
}

export const themes = { light: 'vitesse-light', dark: 'vitesse-dark' } as const;

let hl: Promise<HighlighterGeneric<any, any>> | undefined;
export async function highlight(code: string) {
  hl ??= createHighlighter({ themes: [themes.light, themes.dark], langs: [drachmLang()] });
  return (await hl).codeToHtml(code, { lang: 'drachm', themes, defaultColor: false });
}
