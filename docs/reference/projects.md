# Projects and local modules

A folder is a module. Its `.kiln` files share functions, types and constants;
imports belong to the source file that writes them. Child folders are separate
modules and enter the application only through explicit imports, or for
components, by being named (see [Components and assets](#components-and-assets)).

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

## Starting a project

`kiln new my-app` makes the folder `my-app` with a project in it, and `kiln
new .` makes one in the current folder, named after it (`My Site` gives
`my-site`; `--name` sets another). In a terminal it asks what to make:

| Template | What it writes |
| --- | --- |
| `web` | `main.kiln` with the routes, a layout, a page, `site.css`, a `notes/` module, unit and integration tests |
| `pages` | the same app with its pages as files in `pages/`, served by `app.file_routes` and exported by `kiln fire --static` |
| `cli` | a command-line program: `main.kiln`, a `greet/` module and a unit test, with no web code |

A web app can also get an Ember, `components/counter.ember`, on its home
page. Scripts and agents choose with flags: `--template web|pages|cli` (or
`--kind`), `--ember`, and `--yes` to ask nothing and take the defaults
(`web`, no Ember). Every file is written into the folder with a
`kiln.toml`, and `.gitignore` too unless one is there. Nothing is fetched
or run, and the output says how to run, test and build the project.

`kiln new` never overwrites a file. Before writing anything it checks
every file it would write, and if one is already there it names them all
and writes nothing. A folder that already has other files (a fresh Git
repository, say) gets the project beside them only after a yes: in a
terminal it asks, and otherwise it needs `--yes`.

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
source files form the entry module. Explicit file targets override this
default, and without `entry` the root folder is the entry module. `name`,
`kind`, `version`, `description` and `license` are optional for an app; the
kinds are `app` and `ware`. Absolute paths, parent traversal, unknown keys,
unsupported sections and wrong value types are errors with source
locations.

The same file declares the project's Wares in `[dependencies]`, the
registry they come from, a `[policy]` limiting them, and `[test]` settings
for `kiln test`, and `[aliases]` for paths in glazes (see
[Imports in glazes](#imports-in-glazes)); `kiln.lock` pins the exact Wares. See [Wares](wares.md)
and [Testing](testing.md). No manifest or dependency installation is
needed for local modules.

## Components and assets

Inside `posts/`, `web.render("index.glaze", {})`, `<Editor />` and
`{% layout "layout.glaze" %}` refer to that module's components. A slash-bearing
render/layout path is explicitly project-root-relative, such as
`"accounts/index.glaze"`. A qualified tag such as `<posts.search.Result />`
names `posts/search/result.glaze` or `.ember` in a reachable module. Folder
segments retain their exact spelling; the final capitalized component name uses
the existing snake-case filename convention. Other modules' same-named files
cannot hijack local lookup.

A slash-bearing path or a qualified tag brings the folder of the component it
names into the program, as an import would, so components can live anywhere
in the project: a `components/` or `layouts/` folder needs no `.kiln` file,
and nothing has to import it. Only string literals and tags count, since the
folders are found when the program is compiled. Naming a component is not an
import, so a folder whose component names a page in a folder that imports it
is no cycle. A missing component says where it was looked for, such as "no
file named `ui/missing.glaze` in the project".

A glaze sees the folder modules that its folder's `.kiln` files import, so
in a folder whose routes import `shared`, a page can take
`{% props post: shared.Post %}` and call `shared.may_edit(...)`.

### Imports in glazes

A glaze or ember can also import what it uses itself, by path, at its top
level:

```glaze
{% import "../components/card.glaze" %}
{% import "@ui/like_button.ember" as Like %}
{% import "@/shared" as s %}
{% layout "@/layouts/base.glaze" %}
{% css "@/styles/site.css" %}
<Card title={s.title(post)} />
<Like n={post.likes} />
```

A path to a `.glaze` or `.ember` file makes its component a tag, named in
PascalCase from the file name (`like_button.ember` is `<LikeButton />`) or
by `as`. A path to a folder imports that module, named after the folder or
by `as`, as `import` does in a `.kiln` file: it brings the folder into the
program, and its imports must be acyclic. The import is the file's own;
other glazes in the folder do not see it.

A path starts with `./` or `../` (from the file's folder), `@/` (from the
project root, or a Ware's root inside a Ware) or `@name/` (from a folder
`[aliases]` names). It is written with `/` and stays inside the project.
`@/` and `@name/` paths work in `web.render(...)`, `{% layout %}` and
`{% css %}` too, so a file can move without its paths changing.

Aliases are in the root `kiln.toml`:

```toml
[aliases]
ui = "components/ui"
```

An alias is a lowercase name, not the name of a dependency, for a folder in
the project. Each mistake is an E0230 error at the path or key: a path with
no `./`, `../` or `@`, one that leaves the project, an alias that is not
declared, a file or folder that is not there and an alias whose folder is
missing.

`.glaze` remains server-only. `.ember` retains selective hydration and typed
RPC. A page without an Ember loads no Ember JS or Wasm. Stylesheet imports and
scoped CSS URLs resolve relative to the owning component; CSS asset paths remain
inside the project. Root `static/` files use `/static/...`; a module's files use
`/static/posts/...` (including nested module segments). Duplicate asset URLs are
errors. Only folders in the program (imported, mounted or named by a
component) contribute assets.

A folder that `app.file_routes("pages")` mounts is part of the project too,
with its subfolders, whether or not anything imports it: its `.glaze` pages
become routes, each `route.kiln` is a module of its folder, and its
components are named like any others (`<pages._components.Card />`,
`<pages.plants.[slug].Card />` in a folder named with brackets). See
[Pages as files](../guides/file-routes.md).

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
source paths. Inside the first string of `web.render(...)` or `{% layout %}`
it completes component paths as the compiler resolves them: same-folder
files by name and the rest of the project from its root. In
`{% import "..." %}` it completes components and folders from the file's
folder, or from the root after `@`, and goes to what a path names. VS Code Check/Run/Build actions select the app root within the
workspace and save dirty source files throughout the project.
