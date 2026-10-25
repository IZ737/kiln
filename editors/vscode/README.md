# Kiln for Visual Studio Code

Language support for [Kiln](https://github.com/IZ737/kiln): `.kiln` source
files and `.glaze` templates and components.

- Syntax highlighting for Kiln, and for Glaze's HTML with the Kiln code in
  its `{...}` outputs, `{% ... %}` tags and attribute values. `<style>` and
  `<script>` contents are highlighted as CSS and JavaScript.
- File icons for `.kiln` and `.glaze` files, in light and dark versions.
  They show with any file icon theme that has no icon of its own for Kiln.
- Comment toggling, bracket matching, auto-closing pairs and indentation,
  including `{% %}` and `{# #}` in glazes.

Errors, completion, hover, go to definition and the Check, Run and Build
commands come from `kiln lsp` and the `kiln` command; this extension will
start them in a later release. Until then, any LSP client can run
`kiln lsp` (see [docs/editors.md](../../docs/editors.md)).

## Install from source

The extension is not on the Marketplace yet. From this folder:

```sh
npx @vscode/vsce package          # makes kiln-0.1.0.vsix
code --install-extension kiln-0.1.0.vsix
```

## Develop

Open this folder in VS Code and press F5: a second window opens with the
extension loaded and the repository's `examples` folder open. After
changing a grammar, run **Developer: Reload Window** there, and use
**Developer: Inspect Editor Tokens and Scopes** to see the scopes a token
gets.

```sh
npm install
npm test        # grammar and manifest tests
```

The tests tokenize code with the same TextMate engine VS Code uses, check
the scopes of the constructs in `docs/syntax.md` and `docs/glazes.md`, and
check that every program in `examples/` tokenizes with no rule left open.
The icons in `icons/` are copies of `assets/brand`; a test fails if they
drift apart.
