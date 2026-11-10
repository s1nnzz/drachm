# Drachm for VS Code

Syntax highlighting and basic editing support for [Drachm](../../README.md) (`.drh` files):
TextMate grammar, bracket/quote auto-closing, comment toggling, indentation rules, and snippets
(`main`, `func`, `struct`, `if`, `ifelse`, `while`, `for`, `foreach`, `match`, `test`, `enum`,
`interface`, `failure`, `outln`).

The grammar covers the whole designed language (`docs/SPEC.md`), including features the compiler
does not implement yet: `f"..."` and backtick interpolation (the `${expr}` is highlighted as code),
`"""` strings, `io::outf` format placeholders (only in the first string argument of `outf`),
number suffixes and word bases (`hex FF`), unit literals (`9.6<s>`), `failure` blocks,
`skip to next`, and `//--!$key: value` file flags.

## Install from the folder

Copy or symlink this directory into your VS Code extensions folder and restart VS Code:

```sh
ln -s "$PWD" ~/.vscode/extensions/drachm-0.1.0        # Linux / macOS, run inside editors/vscode
```

(On Windows use `%USERPROFILE%\.vscode\extensions\`.) For a quick try without installing:
`code --extensionDevelopmentPath=$PWD`.

## Package as a .vsix

```sh
cd editors/vscode
npx @vscode/vsce package          # creates drachm-0.1.0.vsix
code --install-extension drachm-0.1.0.vsix
```

## Tests

```sh
cd editors/vscode
npm install
npm test                          # vscode-tmgrammar-test over tests/*.drh
```

Test files use scope assertions (`// ^^^ scope.name`) under each source line. Negative
assertions (`- scope`) pin down the things that must not be highlighted, for example `hex` as a
variable name, `<` as less-than, and `*` as multiplication.

## Known gaps

- Variable and parameter names are not scoped (only definitions of functions, types and units).
- Array lengths in `T[4]` are not treated as part of the type, so `xs[4]` and `uint8[4]` look alike.
- Generic brackets are recognised only when the `<` follows a name directly and a type comes next
  (`List<string>`, `max<uint32>(5, 9)`); `a<b` stays a comparison.
- `%` placeholders are highlighted only in the first string argument of `outf`.
- Function names split across lines (return type on one line, name on the next) are not recognised.
