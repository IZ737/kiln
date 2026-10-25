# Editor setup

`kiln lsp` is a language server (LSP) that talks over standard input and
output. It reports the same errors as `kiln check` while you type, including
unsaved changes, and offers each error's suggested fix as a quick fix. A file
is checked together with the other `.kiln` and `.glaze` files in its
directory, so glazes see your types and pages see your glazes' props.

Phase 2.5 adds completion, hover, go to definition and symbol navigation
to the shared Rust LSP, alongside a dedicated VS Code extension with syntax
highlighting, Kiln file icons and Check/Build/Run commands. Find references,
rename and deeper Glaze semantics remain in phase 5; a debugger is not part
of phase 2.5. See [the phase 2.5 plan](phase-2.5.md). So far the extension
has highlighting and icons; the LSP features are still to come.

Install the `kiln` binary first:

```sh
cargo install --path crates/kiln_cli
```

## VS Code

The extension in [editors/vscode](../editors/vscode) highlights `.kiln` and
`.glaze` files and gives them Kiln's file icons; its README shows how to
install it. Its TextMate grammars (`editors/vscode/syntaxes`) also work in
other editors that read them, such as Sublime Text.

## Neovim (0.11 and later)

```lua
vim.filetype.add({ extension = { kiln = "kiln", glaze = "glaze" } })
vim.lsp.config("kiln", {
  cmd = { "kiln", "lsp" },
  filetypes = { "kiln", "glaze" },
  root_markers = { ".git" },
})
vim.lsp.enable("kiln")
```

Quick fixes are under `vim.lsp.buf.code_action()` (`gra` by default).

## Helix

In `~/.config/helix/languages.toml`:

```toml
[language-server.kiln]
command = "kiln"
args = ["lsp"]

[[language]]
name = "kiln"
scope = "source.kiln"
file-types = ["kiln", "glaze"]
comment-token = "//"
language-servers = ["kiln"]
```

## Zed, Sublime Text, Emacs and others

Any editor with a generic LSP client works: run `kiln lsp` for files ending
in `.kiln` and `.glaze`. For Emacs `eglot`:

```elisp
(define-derived-mode kiln-mode prog-mode "Kiln")
(add-to-list 'auto-mode-alist '("\\.\\(kiln\\|glaze\\)\\'" . kiln-mode))
(add-to-list 'eglot-server-programs '(kiln-mode . ("kiln" "lsp")))
```
