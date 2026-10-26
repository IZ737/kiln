# Projects and local modules

A folder is a module. Its `.kiln` files share functions, types and constants;
imports belong to the source file that writes them. Child folders are separate
modules and enter the application only through explicit imports.

```kiln
import posts, posts.search

fn main() {
    print(posts.title())
    print(search.find())
}
```

`posts/` supplies `posts`; `posts/search/` supplies `search` (the last import
segment). Paths are always relative to the project root. A folder must contain
source; importing its parent does not import its children. Imports perform no
route registration or other initialization: call exported setup functions.

Use `pub fn`, `pub struct`, `pub enum` and `pub const` for exports. Unmarked
names are private to their folder. Public signatures, exported fields/variants
and inferred public constants cannot expose private nominal types. Types from
different modules remain distinct, even when their names and fields match.
Struct fields/constructors and enum variants retain the existing access rules.
Methods need `pub fn` to be callable from another folder.
Generic functions, methods, errors and typed server functions retain their
ordinary behavior across module boundaries.

Imports must be acyclic. E0230 reports local-module/project errors, including
private access, missing modules, cycles and manifest errors. Import aliases
cannot collide with other imports or declarations. Standard-library names are
reserved: rename a local `web/` rather than shadowing `import web`. Paths are
case-sensitive on every platform. Case-folded filename collisions and source
or module symlinks are rejected. Internal module symbol prefixes are reserved.

## Roots and entrypoints

`kiln run folder` and `kiln build folder` select that folder as the root. They
never capture a manifest above an explicitly selected directory. Without a
manifest, the root folder supplies the program's single `fn main()`; `main.kiln`
is the conventional filename. Multiple definitions of `main` are diagnosed.
A file target includes its sibling source files, preserving existing scripts
and same-folder programs. If a file is inside a manifest project, the nearest
ancestor manifest establishes its import root, while the explicit file selects
its entry folder.

`kiln check folder` checks the reachable project. Checking a collection of
independent programs (such as `kiln check examples`) still checks each program.
An explicit file check uses its siblings for context and reports that file's
errors plus errors in imported folders.

An optional root `kiln.toml` selects an entry folder:

```toml
[package]
name = "fieldnotes"
kind = "app"
entry = "app/main.kiln"
```

The entry must name an existing `.kiln` file within the root; its folder's
source files form the entry module. Explicit file targets override this default.
`name` and `kind` are optional strings; the only supported kind is `app`.
`entry` is required when a manifest exists. Absolute paths, parent traversal,
unknown keys, unsupported sections and wrong value types are errors with source
locations. This initial schema is unversioned and strict. Phase 6 will extend
this same file for dependencies; `[wares]`, lockfiles and workspaces are not
implemented here. No manifest or dependency installation is needed for local
modules.

## Components and assets

Inside `posts/`, `web.render("index.glaze", {})`, `<Editor />` and
`{% layout "layout.glaze" %}` refer to that module's components. A slash-bearing
render/layout path is explicitly project-root-relative, such as
`"accounts/index.glaze"`. A qualified tag such as `<posts.search.Result />`
names `posts/search/result.glaze` or `.ember` in a reachable module. Folder
segments retain their exact spelling; the final capitalized component name uses
the existing snake-case filename convention. Other modules' same-named files
cannot hijack local lookup.

A glaze has no imports of its own. It sees the folder modules that its
folder's `.kiln` files import, so in a folder whose routes import `shared`,
a page can take `{% props post: shared.Post %}` and call
`shared.may_edit(...)`.

`.glaze` remains server-only. `.ember` retains selective hydration and typed
RPC. A page without an Ember loads no Ember JS or Wasm. Stylesheet imports and
scoped CSS URLs resolve relative to the owning component; CSS asset paths remain
inside the project. Root `static/` files use `/static/...`; a module's files use
`/static/posts/...` (including nested module segments). Duplicate asset URLs are
errors. Only imported modules contribute assets.

The executable dogfood application is in
`crates/kiln_cli/tests/fieldnotes/`: `posts` and `accounts` register routes,
module-local CSS styles an Ember form, and a typed `server fn` validates its
submission. `/accounts` is an SSR-only route. Run it with:

```sh
kiln run crates/kiln_cli/tests/fieldnotes
```

A larger one is [`examples/journal`](../../examples/journal): `shared` holds
the models, the rules and the database, `accounts` signing up and in, and
`posts` the pages, with an ember that adds comments through a `server fn`.
Its migrations are committed, and its admin is generated:

```sh
kiln db migrate --url sqlite:journal.db --path examples/journal
JOURNAL_HTTP=1 JOURNAL_ADMIN=you@example.com kiln run examples/journal
```

The LSP follows the project graph with unsaved editor text, resolves public
exports for hover/definition/completion, and keeps diagnostics on original
source paths. VS Code Check/Run/Build actions select the app root within the
workspace and save dirty source files throughout the project.
