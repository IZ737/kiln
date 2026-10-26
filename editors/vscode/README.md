# Kiln for Visual Studio Code

Language support for [Kiln](https://github.com/IZ737/kiln): `.kiln` source
files and `.glaze` templates and components.

- **Highlighting** for Kiln, and for Glaze's HTML with the Kiln code in its
  `{...}` outputs, `{% ... %}` tags and attribute values. `<style>` and
  `<script>` contents are highlighted as CSS and JavaScript.
- **File icons** for `.kiln` and `.glaze` files, in light and dark versions.
  They show with any file icon theme that has no icon of its own for Kiln.
- **Language features from `kiln lsp`**: errors as you type with quick
  fixes, completion, hover with types and doc comments, go to definition,
  the outline, and Go to Symbol in Workspace. They come from the compiler,
  so they match what `kiln check` says.
- **Commands**: **Kiln: Check Program**, **Kiln: Run Program** (also the
  ▶ button in the editor title) and **Kiln: Build Program** run `kiln
  check`, `kiln run` and `kiln build` on the app root within the current workspace,
  which is the program, in the terminal. Errors from `check` and `build`
  appear in the Problems panel. The same commands are tasks (**Tasks: Run
  Task** → kiln), and can go in `tasks.json`:

  ```json
  { "type": "kiln", "command": "build", "program": "examples/hello" }
  ```

- Comment toggling, bracket matching, auto-closing pairs and indentation,
  including `{% %}` and `{# #}` in glazes.

## Settings

| Setting | Default | Meaning |
| --- | --- | --- |
| `kiln.path` | empty | The `kiln` executable. When empty, the extension uses `kiln` from your `PATH`, then `~/.kiln/bin/kiln` (where the [installers](https://github.com/IZ737/kiln#install) put it). |
| `kiln.trace.server` | `off` | Log the language server's messages in the Kiln output. |

If `kiln` cannot be found, the extension says so and offers to open the
install instructions or the setting; highlighting and icons still work.
**Kiln: Restart Language Server** restarts `kiln lsp`, for example after
upgrading Kiln, and **Kiln: Show Output** shows its log.

## Install

The extension is not on the Marketplace yet. Each
[Kiln release](https://github.com/IZ737/kiln/releases) has it as
`kiln-vscode-<version>.vsix`; download it, then run

```sh
code --install-extension kiln-vscode-<version>.vsix
```

or use **Extensions: Install from VSIX...** in VS Code. Install `kiln`
itself too (see the [install instructions](https://github.com/IZ737/kiln#install)).
To build the extension from this folder instead:

```sh
npm install
npm run package          # makes kiln-<version>.vsix
```

## Limitations

- No debugger yet: breakpoints and stepping are a later milestone.
- Find references, rename, formatting and deeper navigation between glazes
  and components come in phase 6.
- Go to definition stops at the standard library, which has no source file
  on disk; hover still shows its signatures and docs.
- Local folder modules support cross-folder hover, definition, export/import
  completion and diagnostics. The LSP uses unsaved source text and preserves
  original file locations; see the [project rules](../../docs/reference/projects.md).

## Develop

The extension is plain JavaScript in `src/`; `npm run bundle` puts it and
the libraries it uses into one file, `dist/extension.js`, which is what VS
Code loads (packaging runs it too). Open this folder in VS Code and press
F5: it bundles, then a second window opens with the extension loaded and the repository's `examples` folder open. After
changing a grammar, run **Developer: Reload Window** there, and use
**Developer: Inspect Editor Tokens and Scopes** to see the scopes a token
gets.

```sh
npm install
npm test                                      # fast tests, no VS Code needed
node test/lsp-smoke.js ../../target/debug/kiln        # kiln lsp as an editor sees it
KILN_PATH=../../target/debug/kiln npm run test:integration   # in a real VS Code
```

The integration test downloads VS Code the first time and needs a display
(CI runs it under `xvfb-run`). The fast tests tokenize code with the same
TextMate engine VS Code uses, check the scopes of the constructs in
`docs/reference/syntax.md` and `docs/reference/glazes.md`, check that every program in
`examples/` tokenizes with no rule left open, and activate the extension
against a stand-in for VS Code. They also check that the bundle loads with
nothing but VS Code around it, and that the package holds only what VS Code
loads. The icons in `icons/` are copies of
`assets/brand`; a test fails if they drift apart. The extension's version
is kiln's, and a test checks that too.

### Ember components

`.ember` files use the shared Glaze markup grammar with embedded Kiln code,
a distinct Ember icon, and the same Check, Run and Build commands. The language
server checks state/handlers and server/browser boundaries, completes props and
locals, and navigates component tags from `.glaze` to their actual `.ember`
source. `.glaze` files are server-only; rename interactive components to `.ember`.
