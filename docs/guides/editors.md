# Editor setup

`kiln lsp` is a language server (LSP) that talks over standard input and
output. A file is understood together with the other `.kiln`, `.glaze` and `.ember`
files in its module and the folders it imports, so glazes see local types and
pages see their components' props. Project discovery respects workspace bounds
and optional manifests; the editor's unsaved text is used for open files.
See [projects](../reference/projects.md) for module paths and visibility. It gives:

- **Diagnostics**: the same errors as `kiln check`, while you type, with
  each error's suggested fix as a quick fix.
- **Completion**: fields and methods after `value.`, functions and types
  after `module.`, variants after `Enum.`, the locals in scope and the
  program's functions, types and constants, types where a type is written,
  and standard library modules not imported yet (choosing one adds the
  `import`). It works in glazes' `{...}` code too. After `import ` it
  offers the project's folder modules, and after `import posts.` the
  folders inside `posts/`. In a glaze or ember, `<posts.` offers the
  components in `posts/` and the folders inside it, and the first string
  of `web.render(...)` or `{% layout %}` offers component paths as the
  compiler finds them (same-folder files by name, others from the
  project root).
- **Signature help**: inside a call, the function's signature with the
  argument being typed marked, and its doc comment.
- **Hover**: the type of a name or expression, a function's signature, a
  type's fields or variants, and the doc comment (`//` lines just above the
  declaration), including for the standard library.
- **Go to definition**: for locals, functions, types, fields, methods and
  variants, across the files and folders of the program; from a module in
  an `import` to its folder's main file; and from a component tag, a
  `web.render(...)` path or a `{% layout %}` path to the component.
  Nothing is cached between requests, so new, renamed, deleted and unsaved
  files are seen at once.
- **Styles**: in a `ui(...)` or `ui={{ ... }}` record, completion offers
  the style keys (with the CSS properties each sets), breakpoints inside
  `at`, the design tokens after `space.` or `color.`, the keywords after
  `.` and the themes after `theme: .`; CSS Modules' class names complete
  after the module's name. Hover shows what a key sets and a token's
  custom property and value, and go to definition opens the stylesheet
  where a token or theme is declared.
- **Symbols**: the outline of a file (document symbols), and the
  declarations of every `.kiln` file in the workspace (workspace symbols).

- **Find references and rename**: every use of a function, type, field,
  method, variant or constant across the project's folders, and renaming
  one everywhere at once. Names that come from a cached Ware cannot be
  renamed, since their source is not yours to change.
- **Formatting**: Format Document uses [`kiln fmt`](tooling.md#kiln-fmt).

Code that does not parse yet still gets answers: whatever the parser
recovered is analyzed, so completing `post.` on an unfinished line works.
A debugger is not part of Kiln yet. See [the phase 2.5 plan](../plans/phase-2.5.md).

Install the `kiln` binary first:

```sh
cargo install --path crates/kiln_cli
```

## VS Code

The extension in [editors/vscode](../../editors/vscode) starts `kiln lsp` for
you, highlights `.kiln`, `.glaze` and `.ember` files, gives them Kiln's file icons,
and adds **Kiln: Check Program**, **Run Program**, **Build Program**,
**Test Program**, **Start Dev Server** and **Format Program** commands and
tasks. It finds `kiln` on your `PATH` or in `~/.kiln/bin`, or
where the `kiln.path` setting says; its README shows how to install it. Its
TextMate grammars (`editors/vscode/syntaxes`) also work in other editors
that read them, such as Sublime Text.

## Neovim (0.11 and later)

```lua
vim.filetype.add({ extension = { kiln = "kiln", glaze = "glaze", ember = "ember" } })
vim.lsp.config("kiln", {
  cmd = { "kiln", "lsp" },
  filetypes = { "kiln", "glaze", "ember" },
  root_markers = { ".git" },
})
vim.lsp.enable("kiln")
```

With Neovim's default mappings, quick fixes are under `gra`, hover under
`K`, go to definition under `<C-]>`, the outline under `gO` and completion
under `<C-x><C-o>`.

## Helix

In `~/.config/helix/languages.toml`:

```toml
[language-server.kiln]
command = "kiln"
args = ["lsp"]

[[language]]
name = "kiln"
scope = "source.kiln"
file-types = ["kiln", "glaze", "ember"]
comment-token = "//"
language-servers = ["kiln"]
```

## Zed, Sublime Text, Emacs and others

Any editor with a generic LSP client works: run `kiln lsp` for files ending
in `.kiln`, `.glaze` and `.ember`. For Emacs `eglot`:

```elisp
(define-derived-mode kiln-mode prog-mode "Kiln")
(add-to-list 'auto-mode-alist '("\\.\\(kiln\\|glaze\\|ember\\)\\'" . kiln-mode))
(add-to-list 'eglot-server-programs '(kiln-mode . ("kiln" "lsp")))
```
