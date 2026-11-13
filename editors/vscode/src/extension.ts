import * as cp from "node:child_process";
import * as path from "node:path";
import * as vscode from "vscode";
import { LanguageClient, LanguageClientOptions, ServerOptions } from "vscode-languageclient/node";
import { charIndex, ParsedDiagnostic, parseCheckOutput } from "./parse";

const LANGUAGE = "drachm";
const TERMINAL_NAME = "Drachm";
const OUTPUT_NAME = "Drachm";
const DEBOUNCE_MS = 300;
// `main` at the start of a line, after a return type: `none main() {`, `sint32 main() {`.
const MAIN_DECL = /^[A-Za-z_][\w[\]?<>]*\s+main\s*\(/;

type TaskCommand = "build" | "run" | "check";

interface DrachmTaskDefinition extends vscode.TaskDefinition {
    command: TaskCommand;
    file?: string;
    optimised?: boolean;
}

interface Captured {
    code: number | null;
    stdout: string;
    stderr: string;
    error?: NodeJS.ErrnoException;
}

interface CheckState {
    kind: "running" | "ok" | "warnings" | "errors" | "failed" | "unknown";
    errors: number;
    warnings: number;
    detail?: string;
}

let output: vscode.OutputChannel;
let problems: vscode.DiagnosticCollection;
let statusItem: vscode.StatusBarItem;
let checker: Checker;
let client: LanguageClient | undefined;
let warnedMissing = false;
const states = new Map<string, CheckState>();

export function activate(context: vscode.ExtensionContext): void {
    output = vscode.window.createOutputChannel(OUTPUT_NAME);
    problems = vscode.languages.createDiagnosticCollection(LANGUAGE);
    statusItem = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    statusItem.command = "drachm.check";
    checker = new Checker();

    context.subscriptions.push(
        output,
        problems,
        statusItem,
        checker,
        vscode.commands.registerCommand("drachm.run", () => runFile(false)),
        vscode.commands.registerCommand("drachm.runOptimised", () => runFile(true)),
        vscode.commands.registerCommand("drachm.build", build),
        vscode.commands.registerCommand("drachm.check", checkActive),
        vscode.commands.registerCommand("drachm.explain", explain),
        vscode.commands.registerCommand("drachm.restartServer", restartServer),
        vscode.languages.registerCodeLensProvider({ language: LANGUAGE }, new MainLensProvider()),
        vscode.tasks.registerTaskProvider(LANGUAGE, taskProvider),
        vscode.workspace.onDidSaveTextDocument((doc) => {
            // With the language server on, it checks on its own.
            if (doc.languageId === LANGUAGE && !serverEnabled() && config<boolean>("checkOnSave", true)) checker.schedule(doc);
        }),
        vscode.window.onDidChangeActiveTextEditor(updateStatus),
        vscode.workspace.onDidChangeConfiguration((e) => {
            if (e.affectsConfiguration("drachm.path")) warnedMissing = false;
            if (e.affectsConfiguration("drachm.path") || e.affectsConfiguration("drachm.languageServer")) void restartServer();
        }),
    );
    updateStatus();
    void startServer();
}

export function deactivate(): Thenable<void> | undefined {
    return client?.stop();
}

// ---------------------------------------------------------------- language server

function serverEnabled(): boolean {
    return config<boolean>("languageServer", true);
}

/** Start `drachm lsp`: diagnostics as you type, hover, go to definition, completion. */
async function startServer(): Promise<void> {
    if (!serverEnabled() || client) return;
    const serverOptions: ServerOptions = {
        command: drachmPath(),
        args: ["lsp"],
        options: { env: { ...process.env, NO_COLOR: "1" } },
    };
    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: "file", language: LANGUAGE }],
        outputChannel: output,
    };
    const c = new LanguageClient("drachm", "Drachm", serverOptions, clientOptions);
    client = c;
    try {
        await c.start();
    } catch (error) {
        client = undefined;
        output.appendLine(`Could not start "${drachmPath()} lsp": ${String(error)}`);
        reportMissingBinary(error as NodeJS.ErrnoException);
    }
    updateStatus();
}

async function restartServer(): Promise<void> {
    const old = client;
    client = undefined;
    try {
        await old?.stop();
    } catch {
        // The server was already gone.
    }
    await startServer();
}

function config<T>(key: string, fallback: T): T {
    return vscode.workspace.getConfiguration("drachm").get<T>(key, fallback);
}

function drachmPath(): string {
    return config<string>("path", "drachm") || "drachm";
}

function activeDrachmDoc(): vscode.TextDocument | undefined {
    const doc = vscode.window.activeTextEditor?.document;
    if (doc?.languageId === LANGUAGE) return doc;
    void vscode.window.showWarningMessage("Open a Drachm (.drh) file first.");
    return undefined;
}

async function saveFirst(doc: vscode.TextDocument): Promise<boolean> {
    if (!doc.isDirty && !doc.isUntitled) return true;
    return doc.save();
}

/** Run drachm and collect its output. Never rejects. */
function capture(args: string[], cwd?: string, onSpawn?: (p: cp.ChildProcess) => void): Promise<Captured> {
    return new Promise((resolve) => {
        let stdout = "";
        let stderr = "";
        const proc = cp.spawn(drachmPath(), args, {
            cwd,
            env: { ...process.env, NO_COLOR: "1" },
            windowsHide: true,
        });
        onSpawn?.(proc);
        proc.stdout.on("data", (chunk: Buffer) => (stdout += chunk.toString()));
        proc.stderr.on("data", (chunk: Buffer) => (stderr += chunk.toString()));
        proc.on("error", (error: NodeJS.ErrnoException) => resolve({ code: null, stdout, stderr, error }));
        proc.on("close", (code) => resolve({ code, stdout, stderr }));
    });
}

function reportMissingBinary(error: NodeJS.ErrnoException): void {
    output.appendLine(`Could not start "${drachmPath()}": ${error.message}`);
    if (warnedMissing) return;
    warnedMissing = true;
    void vscode.window
        .showErrorMessage(`Drachm: could not run "${drachmPath()}". Set drachm.path to the compiler.`, "Open Settings")
        .then((choice) => {
            if (choice) void vscode.commands.executeCommand("workbench.action.openSettings", "drachm.path");
        });
}

// ---------------------------------------------------------------- run / build / explain

/** Quote one shell argument for the integrated terminal. */
function shellQuote(s: string): string {
    if (!/[\s"'$`\\&|;<>()*?!#]/.test(s)) return s;
    return process.platform === "win32" ? `"${s.replace(/"/g, '""')}"` : `'${s.replace(/'/g, "'\\''")}'`;
}

function runInTerminal(args: string[]): void {
    const terminal =
        vscode.window.terminals.find((t) => t.name === TERMINAL_NAME) ??
        vscode.window.createTerminal({ name: TERMINAL_NAME });
    terminal.show(true);
    terminal.sendText([shellQuote(drachmPath()), ...args.map(shellQuote)].join(" "));
}

async function runFile(optimised: boolean): Promise<void> {
    const doc = activeDrachmDoc();
    if (!doc || !(await saveFirst(doc))) return;
    const args = ["run", doc.fileName];
    if (optimised) args.push("-O");
    runInTerminal(args);
}

async function build(): Promise<void> {
    const doc = activeDrachmDoc();
    if (!doc || !(await saveFirst(doc))) return;
    const file = doc.fileName;
    output.show(true);
    output.appendLine(`$ drachm build ${file}`);
    const result = await capture(["build", file]);
    if (result.error) return reportMissingBinary(result.error);
    output.append(result.stdout + result.stderr);
    if (result.code === 0) {
        void vscode.window.showInformationMessage(`Built ${path.basename(file, path.extname(file))}.`);
    } else {
        void vscode.window.showErrorMessage("Drachm build failed. See the Drachm output.");
    }
}

async function explain(code?: string): Promise<void> {
    code ??= await vscode.window.showInputBox({
        prompt: "Error code to explain",
        placeHolder: "E0205",
        validateInput: (v) => (/^\s*[EW]\d+\s*$/i.test(v) ? undefined : "Expected a code such as E0205"),
    });
    if (!code?.trim()) return;
    code = code.trim();
    const result = await capture(["explain", code]);
    if (result.error) return reportMissingBinary(result.error);
    output.show(true);
    output.appendLine(`--- drachm explain ${code} ---`);
    output.appendLine((result.stdout + result.stderr).trimEnd());
    output.appendLine("");
}

// ---------------------------------------------------------------- check and diagnostics

/** Link for the error code in the Problems panel: runs `drachm.explain` with the code. */
function explainLink(code: string): vscode.Uri {
    return vscode.Uri.parse(`command:drachm.explain?${encodeURIComponent(JSON.stringify([code]))}`);
}

/** Range of `len` display columns at `displayCol` on 1-based `line`. */
function rangeOf(doc: vscode.TextDocument, line: number, displayCol: number, len: number): vscode.Range {
    const lineNo = Math.min(Math.max(line - 1, 0), doc.lineCount - 1);
    const text = doc.lineAt(lineNo).text;
    const start = charIndex(text, displayCol);
    const end = charIndex(text, displayCol + len);
    return new vscode.Range(lineNo, start, lineNo, Math.max(start, end));
}

function toDiagnostic(doc: vscode.TextDocument, p: ParsedDiagnostic): vscode.Diagnostic {
    const primary = p.labels.find((l) => l.primary);
    let range: vscode.Range;
    if (primary) {
        range = rangeOf(doc, primary.line, primary.col, primary.len);
    } else if (p.line !== undefined) {
        range = rangeOf(doc, p.line, (p.column ?? 1) - 1, 1);
    } else {
        range = new vscode.Range(0, 0, 0, 0);
    }

    const severity =
        p.severity === "error" ? vscode.DiagnosticSeverity.Error : vscode.DiagnosticSeverity.Warning;
    const d = new vscode.Diagnostic(range, p.message, severity);
    d.source = "drachm";
    d.code = { value: p.code, target: explainLink(p.code) };

    const related: vscode.DiagnosticRelatedInformation[] = [];
    for (const label of p.labels) {
        if (label.primary) continue;
        const where = rangeOf(doc, label.line, label.col, label.len);
        related.push(
            new vscode.DiagnosticRelatedInformation(
                new vscode.Location(doc.uri, where),
                label.text || "also here",
            ),
        );
    }
    for (const note of p.notes) {
        related.push(new vscode.DiagnosticRelatedInformation(new vscode.Location(doc.uri, range), `note: ${note}`));
    }
    for (const help of p.help) {
        related.push(new vscode.DiagnosticRelatedInformation(new vscode.Location(doc.uri, range), `help: ${help}`));
    }
    d.relatedInformation = related;
    return d;
}

class Checker implements vscode.Disposable {
    private timers = new Map<string, NodeJS.Timeout>();
    private running = new Map<string, cp.ChildProcess>();
    /** Counts checks per file, so a killed check cannot write results after a newer one started. */
    private generation = new Map<string, number>();

    /** Check after a short pause, so a burst of saves runs one check. */
    schedule(doc: vscode.TextDocument): void {
        const key = doc.uri.toString();
        clearTimeout(this.timers.get(key));
        this.timers.set(
            key,
            setTimeout(() => {
                this.timers.delete(key);
                void this.check(doc);
            }, DEBOUNCE_MS),
        );
    }

    /** Run `drachm check` on the file's saved contents and publish the result. */
    async check(doc: vscode.TextDocument): Promise<void> {
        const key = doc.uri.toString();
        clearTimeout(this.timers.get(key));
        this.timers.delete(key);
        // A newer check supersedes one still running for the same file.
        this.running.get(key)?.kill();
        const generation = (this.generation.get(key) ?? 0) + 1;
        this.generation.set(key, generation);

        states.set(key, { kind: "running", errors: 0, warnings: 0 });
        updateStatus();

        const dir = path.dirname(doc.fileName);
        const base = path.basename(doc.fileName);
        const result = await capture(["check", base], dir, (p) => this.running.set(key, p));
        if (this.generation.get(key) !== generation) return;
        this.running.delete(key);

        if (result.error) {
            problems.delete(doc.uri);
            states.set(key, { kind: "failed", errors: 0, warnings: 0, detail: result.error.message });
            reportMissingBinary(result.error);
            updateStatus();
            return;
        }

        const text = result.stderr + result.stdout;
        const parsed = parseCheckOutput(text);
        if (parsed.diagnostics.length === 0 && result.code !== 0) {
            // Not a diagnostic: the compiler could not check the file at all.
            problems.delete(doc.uri);
            output.appendLine(text.trimEnd());
            states.set(key, { kind: "failed", errors: 0, warnings: 0, detail: text.trim() });
            updateStatus();
            return;
        }

        const errors = parsed.diagnostics.filter((d) => d.severity === "error").length;
        const warnings = parsed.diagnostics.length - errors;
        problems.set(
            doc.uri,
            parsed.diagnostics.map((d) => toDiagnostic(doc, d)),
        );
        states.set(key, {
            kind: errors > 0 ? "errors" : warnings > 0 ? "warnings" : "ok",
            errors,
            warnings,
        });
        updateStatus();
    }

    dispose(): void {
        for (const timer of this.timers.values()) clearTimeout(timer);
        for (const proc of this.running.values()) proc.kill();
        this.timers.clear();
        this.running.clear();
    }
}

async function checkActive(): Promise<void> {
    const doc = activeDrachmDoc();
    if (!doc || !(await saveFirst(doc))) return;
    // The language server checks on save by itself.
    if (client) return;
    await checker.check(doc);
}

function updateStatus(): void {
    const doc = vscode.window.activeTextEditor?.document;
    if (!doc || doc.languageId !== LANGUAGE) {
        statusItem.hide();
        return;
    }
    if (client) {
        statusItem.text = "$(check) Drachm";
        statusItem.tooltip = "Drachm: language server running. Click to restart it.";
        statusItem.command = "drachm.restartServer";
        statusItem.show();
        return;
    }
    statusItem.command = "drachm.check";
    const state: CheckState = states.get(doc.uri.toString()) ?? { kind: "unknown", errors: 0, warnings: 0 };
    const summary = (): string => {
        const parts: string[] = [];
        if (state.errors) parts.push(`${state.errors} error${state.errors === 1 ? "" : "s"}`);
        if (state.warnings) parts.push(`${state.warnings} warning${state.warnings === 1 ? "" : "s"}`);
        return parts.length ? parts.join(", ") : "no errors";
    };
    switch (state.kind) {
        case "running":
            statusItem.text = "$(sync~spin) Drachm";
            statusItem.tooltip = "Drachm: checking...";
            break;
        case "ok":
            statusItem.text = "$(check) Drachm";
            statusItem.tooltip = "Drachm: last check found no errors. Click to check again.";
            break;
        case "warnings":
            statusItem.text = "$(check) Drachm";
            statusItem.tooltip = `Drachm: last check found ${summary()}. Click to check again.`;
            break;
        case "errors":
            statusItem.text = "$(error) Drachm";
            statusItem.tooltip = `Drachm: last check found ${summary()}. Click to check again.`;
            break;
        case "failed":
            statusItem.text = "$(error) Drachm";
            statusItem.tooltip = `Drachm: the check could not run. ${state.detail ?? ""}`.trim();
            break;
        default:
            statusItem.text = "$(circle-large-outline) Drachm";
            statusItem.tooltip = "Drachm: not checked yet. Click to check.";
    }
    statusItem.show();
}

// ---------------------------------------------------------------- CodeLens

class MainLensProvider implements vscode.CodeLensProvider {
    provideCodeLenses(doc: vscode.TextDocument): vscode.CodeLens[] {
        const lenses: vscode.CodeLens[] = [];
        for (let i = 0; i < doc.lineCount; i++) {
            const text = doc.lineAt(i).text;
            if (!MAIN_DECL.test(text) || /^return\b/.test(text)) continue;
            lenses.push(
                new vscode.CodeLens(new vscode.Range(i, 0, i, 0), {
                    title: "▶ Run",
                    tooltip: "Run this Drachm program",
                    command: "drachm.run",
                }),
            );
        }
        return lenses;
    }
}

// ---------------------------------------------------------------- tasks

function makeTask(
    definition: DrachmTaskDefinition,
    file: string,
    name: string,
    scope: vscode.TaskScope | vscode.WorkspaceFolder = vscode.TaskScope.Workspace,
): vscode.Task {
    const args = [definition.command, file];
    if (definition.optimised && definition.command !== "check") args.push("-O");
    const execution = new vscode.ShellExecution(drachmPath(), args, { env: { NO_COLOR: "1" } });
    const task = new vscode.Task(definition, scope, name, "drachm", execution, "$drachm");
    if (definition.command === "build") task.group = vscode.TaskGroup.Build;
    return task;
}

const taskProvider: vscode.TaskProvider = {
    provideTasks(): vscode.Task[] {
        const doc = vscode.window.activeTextEditor?.document;
        if (doc?.languageId !== LANGUAGE) return [];
        return (["build", "run", "check"] as TaskCommand[]).map((command) =>
            makeTask(
                { type: LANGUAGE, command, file: doc.fileName },
                doc.fileName,
                `drachm ${command} ${path.basename(doc.fileName)}`,
            ),
        );
    },
    resolveTask(task: vscode.Task): vscode.Task | undefined {
        const definition = task.definition as DrachmTaskDefinition;
        const file = definition.file ?? vscode.window.activeTextEditor?.document.fileName;
        if (!file || !["build", "run", "check"].includes(definition.command)) return undefined;
        return makeTask(definition, file, task.name, task.scope);
    },
};
