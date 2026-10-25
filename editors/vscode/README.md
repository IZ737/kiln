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
  check`, `kiln run` and `kiln build` on the folder of the current file,
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

## Install from source

The extension is not on the Marketplace yet. From this folder:

```sh
npm install
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
npm test        # grammar, manifest and kiln-finding tests
```

The tests tokenize code with the same TextMate engine VS Code uses, check
the scopes of the constructs in `docs/syntax.md` and `docs/glazes.md`, and
check that every program in `examples/` tokenizes with no rule left open.
The icons in `icons/` are copies of `assets/brand`; a test fails if they
drift apart.
