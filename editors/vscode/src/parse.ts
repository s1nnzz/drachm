// Parser for the text `drachm check` prints (see src/diag/render.rs in the
// compiler). Pure TypeScript with no `vscode` import, so the tests can run it
// under plain node.

export interface Label {
    /** 1-based line in the checked file. */
    line: number;
    /** Display column, 0-based. Tabs count as four columns, as the compiler prints them. */
    col: number;
    /** Underline width in display columns. */
    len: number;
    text: string;
    primary: boolean;
}

export interface ParsedDiagnostic {
    severity: "error" | "warning";
    code: string;
    message: string;
    /** File name as the compiler printed it. */
    file?: string;
    /** 1-based line and column (in characters) from the `-->` line. */
    line?: number;
    column?: number;
    /** The primary label comes first, then secondary ones, in source order. */
    labels: Label[];
    notes: string[];
    help: string[];
}

export interface ParseResult {
    diagnostics: ParsedDiagnostic[];
    /** Non-blank lines that are not part of any diagnostic (e.g. `drachm: no such file`). */
    other: string[];
}

const HEADER = /^(error|warning)\[([EW]\d+)\]: (.*)$/;
const LOCATION = /^\s*--> (.+?)(?::(\d+):(\d+))?$/;
const SOURCE = /^ *(\d+) \| ?(.*)$/;
const MARK = /^\s*\| ( *)(\^+|-+)(?: (.*))?$/;
const SEPARATOR = /^\s*\|\s*$/;
const NOTE = /^\s?note: (.*)$/;
const HELP = /^\s?help: (.*)$/;
const ELLIPSIS = /^\s*\.\.\.\s*$/;
const SUMMARY = /^.*: \d+ (?:errors?|warnings?)(?:, \d+ (?:errors?|warnings?))?$/;
const ANSI = /\x1b\[[0-9;]*m/g;

export function stripAnsi(text: string): string {
    return text.replace(ANSI, "");
}

export function parseCheckOutput(output: string): ParseResult {
    const diagnostics: ParsedDiagnostic[] = [];
    const other: string[] = [];
    let cur: ParsedDiagnostic | undefined;
    // Line of the most recent source row; the underline rows after it belong to it.
    let sourceLine: number | undefined;

    for (const raw of stripAnsi(output).split(/\r?\n/)) {
        const line = raw.trimEnd();
        if (line === "") continue;

        const header = HEADER.exec(line);
        if (header) {
            cur = {
                severity: header[1] === "error" ? "error" : "warning",
                code: header[2],
                message: header[3],
                labels: [],
                notes: [],
                help: [],
            };
            diagnostics.push(cur);
            sourceLine = undefined;
            continue;
        }
        if (!cur) {
            if (!SUMMARY.test(line)) other.push(line);
            continue;
        }

        const location = LOCATION.exec(line);
        if (location) {
            cur.file = location[1];
            if (location[2] !== undefined) {
                cur.line = Number(location[2]);
                cur.column = Number(location[3]);
            }
            continue;
        }
        const source = SOURCE.exec(line);
        if (source) {
            sourceLine = Number(source[1]);
            continue;
        }
        const mark = MARK.exec(line);
        if (mark && sourceLine !== undefined) {
            cur.labels.push({
                line: sourceLine,
                col: mark[1].length,
                len: mark[2].length,
                text: mark[3] ?? "",
                primary: mark[2].startsWith("^"),
            });
            continue;
        }
        const note = NOTE.exec(line);
        if (note) {
            cur.notes.push(note[1]);
            continue;
        }
        const help = HELP.exec(line);
        if (help) {
            cur.help.push(help[1]);
            continue;
        }
        if (SEPARATOR.test(line) || ELLIPSIS.test(line) || SUMMARY.test(line)) continue;
        other.push(line);
    }

    // Primary label first, then the rest in source order.
    for (const d of diagnostics) {
        d.labels.sort((a, b) => Number(b.primary) - Number(a.primary) || a.line - b.line || a.col - b.col);
    }
    return { diagnostics, other };
}

/**
 * Turns a display column (tabs = 4) on `lineText` into a string index, which
 * is what VS Code positions use.
 */
export function charIndex(lineText: string, displayCol: number): number {
    let display = 0;
    let index = 0;
    for (const ch of lineText) {
        if (display >= displayCol) break;
        display += ch === "\t" ? 4 : 1;
        index += ch.length;
    }
    return index;
}
