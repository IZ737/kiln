# Editor setup

`kiln lsp` is a language server (LSP) that talks over standard input and
output. It reports the same errors as `kiln check` while you type, including
unsaved changes, and offers each error's suggested fix as a quick fix. A file
is checked together with the other `.kiln` and `.glaze` files in its
directory, so glazes see your types and pages see your glazes' props.

Completion, hover, go to definition and rename arrive in phase 4, together
with a VS Code extension that uses the icons in `assets/brand/`.

Install the `kiln` binary first:

```sh
cargo install --path crates/kiln_cli
```

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
