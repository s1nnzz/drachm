import { test } from "node:test";
import * as assert from "node:assert/strict";
import * as fs from "node:fs";
import * as path from "node:path";
import { charIndex, parseCheckOutput } from "../parse";

// Compiled to out/test/, so the repo's extension folder is two levels up.
const root = path.join(__dirname, "..", "..");
const fixtures = path.join(root, "tests", "compiler-output");

function load(name: string): string {
    return fs.readFileSync(path.join(fixtures, `${name}.txt`), "utf8");
}

test("parses every error in immutable.drh", () => {
    const { diagnostics, other } = parseCheckOutput(load("immutable"));
    assert.equal(diagnostics.length, 6);
    assert.deepEqual(other, []);
    assert.ok(diagnostics.every((d) => d.severity === "error" && d.code === "E0205"));

    const first = diagnostics[0];
    assert.equal(first.message, "cannot change parameter `n`: it is not `mut`");
    assert.equal(first.file, "immutable.drh");
    assert.equal(first.line, 2);
    assert.equal(first.column, 5);
    assert.deepEqual(first.labels[0], { line: 2, col: 4, len: 1, text: "changed here", primary: true });
    assert.deepEqual(first.labels[1], {
        line: 1,
        col: 12,
        len: 8,
        text: "declared here without `mut`",
        primary: false,
    });
    assert.deepEqual(first.help, ["declare it as `mut uint32 n` to allow changes"]);
});

test("a diagnostic without a location keeps no line", () => {
    const { diagnostics, other } = parseCheckOutput(load("nomain"));
    assert.equal(diagnostics.length, 9);
    assert.deepEqual(other, []);

    const noMain = diagnostics.find((d) => d.code === "E0204");
    assert.ok(noMain);
    assert.equal(noMain.file, "nomain.drh");
    assert.equal(noMain.line, undefined);
    assert.equal(noMain.column, undefined);
    assert.deepEqual(noMain.labels, []);
    assert.deepEqual(noMain.help, ["add `none main() { }`; it is where the program starts"]);
});

test("reads the primary label of a single-error file", () => {
    const { diagnostics } = parseCheckOutput(load("noimport"));
    assert.equal(diagnostics.length, 2);
    const [first] = diagnostics;
    assert.equal(first.code, "E0206");
    assert.equal(first.line, 2);
    assert.deepEqual(first.labels, [{ line: 2, col: 4, len: 2, text: "not imported", primary: true }]);
    assert.deepEqual(first.help, ["add `in io;` at the top of the file"]);
});

test("warnings are told apart from errors and gutters of different widths parse", () => {
    const { diagnostics, other } = parseCheckOutput(load("structdecl"));
    assert.deepEqual(other, []);
    const warning = diagnostics.find((d) => d.code === "W0208");
    assert.ok(warning);
    assert.equal(warning.severity, "warning");
    // Line 10 has a two-digit gutter; the label still lands on the right line.
    assert.equal(warning.line, 10);
    assert.equal(warning.labels[0].line, 10);
    assert.equal(diagnostics.filter((d) => d.severity === "error").length, diagnostics.length - 1);
});

test("the summary line and ANSI colour codes are not diagnostics", () => {
    const coloured = "\x1b[1;31merror[E0201]\x1b[0m: unknown name `x`\n  --> a.drh:1:1\n";
    const { diagnostics, other } = parseCheckOutput(coloured + "a.drh: 1 error\n");
    assert.equal(diagnostics.length, 1);
    assert.equal(diagnostics[0].code, "E0201");
    assert.deepEqual(other, []);
});

test("lines that are not diagnostics are returned as other", () => {
    const { diagnostics, other } = parseCheckOutput("drachm: no such file\n");
    assert.deepEqual(diagnostics, []);
    assert.deepEqual(other, ["drachm: no such file"]);
});

test("display columns count tabs as four and map to string indexes", () => {
    assert.equal(charIndex("\tbb", 4), 1);
    assert.equal(charIndex("\tbb", 5), 2);
    assert.equal(charIndex("ab", 5), 2);
    assert.equal(charIndex("", 3), 0);
});

test("the $drachm problem matcher finds the same diagnostics as the parser", () => {
    const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
    const matcher = pkg.contributes.problemMatchers.find((m: { name: string }) => m.name === "drachm");
    assert.ok(matcher, "problem matcher `drachm` is contributed");
    const [header, location] = matcher.pattern.map((p: { regexp: string }) => new RegExp(p.regexp));

    for (const name of ["immutable", "noimport", "structdecl"]) {
        const lines = load(name).split(/\r?\n/);
        let matched = 0;
        for (let i = 0; i + 1 < lines.length; i++) {
            if (header.test(lines[i]) && location.test(lines[i + 1])) matched++;
        }
        const expected = parseCheckOutput(load(name)).diagnostics.filter((d) => d.line !== undefined).length;
        assert.equal(matched, expected, name);
    }
});
