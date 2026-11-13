# Drachm for VS Code

Support for Drachm (`.drh` files, see the repository README): syntax highlighting, bracket and quote
auto-closing, comment toggling, indentation rules, snippets (`main`, `func`, `struct`, `if`,
`ifelse`, `while`, `for`, `foreach`, `match`, `test`, `enum`, `interface`, `failure`, `outln`), and
a small extension that runs the compiler.

## Features

- **Run**: a ▶ button in the editor title bar, a `▶ Run` CodeLens above `main`, and
  `Ctrl+Shift+Enter` (`Cmd+Shift+Enter` on macOS) run `drachm run <file>` in a terminal named
  "Drachm", reused between runs. The file is saved first. The dropdown next to ▶ has
  **Run Optimised (-O)**.
- **Build**, **Check** and **Explain Error Code** from the command palette (`Drachm: ...`).
  Build output and explanations go to the "Drachm" output channel.
- **Language server**: the extension starts `drachm lsp`. Errors and warnings appear in the
  Problems panel while you type (from the unsaved text, for the whole project), with the error code
  and the notes, help lines and secondary labels as related information. **Hover** shows types and
  signatures, **Go to Definition** (F12) works for locals, functions, structs, fields, methods and
  enum variants, and **completion** offers methods after `::`, fields after `.`, bare enum
  variants, names in scope and keywords. `Drachm: Restart Language Server` restarts it (do that
  after rebuilding the compiler).
- **Without the server** (`drachm.languageServer` off): on each save, `drachm check` runs and its
  errors and warnings appear in the Problems panel. Turn that off with `drachm.checkOnSave`.
- **Status bar**: "Drachm" shows the server is running; click it to restart. Without the server it
  shows the last check of the active file.
- **Tasks**: a `drachm` task type with `build`, `run` and `check` (see below), and a `$drachm`
  problem matcher for the compiler's error format.

## Settings

| Setting             | Default   | What it does                                              |
| ------------------- | --------- | --------------------------------------------------------- |
| `drachm.path`       | `drachm`  | The compiler to run: a name on PATH, or a full path.      |
| `drachm.languageServer`| `true` | Start `drachm lsp` (diagnostics, hover, definition, completion). |
| `drachm.checkOnSave`| `true`    | Only without the server: run `drachm check` on save.      |

## Tasks

```jsonc
// .vscode/tasks.json
{
  "version": "2.0.0",
  "tasks": [
    { "label": "Drachm: check", "type": "drachm", "command": "check", "file": "${file}" },
    { "label": "Drachm: build -O", "type": "drachm", "command": "build", "file": "${file}", "optimised": true, "group": "build" }
  ]
}
```

`command` is `build`, `run` or `check`. `file` defaults to the active editor's file.

## Try it without installing

From the repository root:

```sh
code --extensionDevelopmentPath="$PWD/editors/vscode"
```

Open a `.drh` file in that window. The `drachm` binary must be on PATH, or set `drachm.path`.

To install it for good, copy or symlink this folder into your extensions folder, after `npm install`
and `npm run compile`:

```sh
ln -s "$PWD" ~/.vscode/extensions/drachm-0.1.0        # run inside editors/vscode
```

(On Windows use `%USERPROFILE%\.vscode\extensions\`.)

## Package as a .vsix

```sh
cd editors/vscode
npm install
npx @vscode/vsce package          # runs the compile step, creates drachm-0.1.0.vsix
code --install-extension drachm-0.1.0.vsix
```

## Develop and test

```sh
cd editors/vscode
npm install
npm run compile                   # tsc, src/ to out/
npm run watch                     # or recompile on change
npm test                          # grammar tests, then the diagnostic parser tests
```

- `tests/*.drh` are grammar tests (`vscode-tmgrammar-test`). Test files use scope assertions
  (`// ^^^ scope.name`) under each source line. Negative assertions (`- scope`) pin down what must
  not be highlighted, for example `hex` as a variable name, `<` as less-than, and `*` as
  multiplication.
- `src/test/parse.test.ts` runs under `node --test` on the compiled parser (`src/parse.ts`). It reads
  real compiler output from `tests/compiler-output/*.txt`. Regenerate those after a change to the
  compiler's diagnostic format with `NO_COLOR=1 drachm check FILE.drh 2> FILE.txt`.

## Known gaps

- Variable and parameter names are not scoped (only definitions of functions, types and units).
- Array lengths in `T[4]` are not treated as part of the type, so `xs[4]` and `uint8[4]` look alike.
- Generic brackets are recognised only when the `<` follows a name directly and a type comes next
  (`List<string>`, `max<uint32>(5, 9)`); `a<b` stays a comparison.
- `%` placeholders are highlighted only in the first string argument of `outf`.
- Function names split across lines (return type on one line, name on the next) are not recognised.
- The extension checks the saved file on disk. Diagnostics are not refreshed while you type.
- Run sends the command to the "Drachm" terminal. If a program is still running there, the text
  goes to that program.
