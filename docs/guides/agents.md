# Kiln for coding agents

A compact guide for AI coding agents (and people in a hurry) writing Kiln.
`kiln docs --llm` prints this page for the installed version, and
`kiln mcp` serves it as the `guide` tool. When this guide and the compiler
disagree, the compiler is right: run `kiln check`.

## The loop

1. Read the project: `kiln check --json .` lists every error with a code,
   a position and often an exact fix (`suggestion.replacement` for
   `suggestion.span`); `kiln explain E0201` explains a code with a wrong and
   a right example. With MCP, `project_graph` gives the folders, imports and
   `pub` signatures.
2. Change code, then `kiln check` until it reports no errors. Apply the
   suggested fixes rather than guessing.
3. `kiln test --json` runs unit, integration and browser tests in one
   command; `--filter NAME` runs one. Exit code 0 passed, 1 a test failed,
   2 build or usage problem, 3 the runner itself failed.
4. `kiln fmt` before you finish. There is one layout and no options.

Never edit `kiln.lock` or files in the Ware cache by hand; use `kiln add`,
`kiln update` and `kiln remove`. Never use `--allow-capabilities` unless
the person asked for it.

## A minimal app

```kiln
// main.kiln
import web, notes

fn main() ! {
    let app = web.app()
    app.get("/", fn(req) {
        return web.render("home.glaze", { items: notes.all() })
    })
    app.get("/notes/{id: Int}", fn(req, id) {
        return web.json(notes.find(id) ?? throw web.error(404, "no such note"))
    })
    try app.listen("127.0.0.1:8080")
}
```

```kiln
// notes/notes.kiln: the folder `notes/` is the module `notes`
pub struct Note {
    id: Int
    title: String
    done: Bool = false
}

pub fn all() -> List<Note> {
    return [Note(id: 1, title: "Read the guide"), Note(id: 2, title: "Write a page")]
}

pub fn find(id: Int) -> Note? {
    return all().find(fn(n) { n.id == id })
}
```

```glaze
{# home.glaze: server-rendered HTML; values in {...} are escaped #}
{% props items: List<notes.Note> %}
<!doctype html>
<html lang="en"><head><meta charset="utf-8"><title>Notes</title></head>
<body>
<ul>
  {% for note in items %}
  <li>{note.title}{% if note.done %} (done){% end %}</li>
  {% empty %}
  <li>No notes.</li>
  {% end %}
</ul>
</body></html>
```

`kiln run .` builds and serves it; `kiln new NAME` writes a starter like
this with tests. `kiln fire --static` writes the same app as a static
site in `dist/`: every `GET` route without parameters, and the pages a
route with parameters lists with `.static(fn() { [...] })`.

## Language essentials

- No semicolons; a newline ends a statement. No `null`: optional values
  are `T?`, and the missing value is `none`.
- `let` is immutable, `var` mutable. Parameters always have types;
  closure parameters may omit them: `fn(n) { n.done }`.
- A function returns its last expression or uses `return`. `.title` is
  short for `fn(x) { x.title }`.
- `and`, `or`, `not`; `==` and `!=` compare by value. Strings interpolate
  with `"${name}"`. Durations are literals: `250ms`, `2s`.
- A `{` in expression position always starts a record: `{ title: "x" }`.
  `if` and `match` are expressions: `let n = if a { 1 } else { 2 }`.
- Errors: a function that can fail has `!` after its type
  (`fn load(id: Int) -> Post ! { ... }`). Call it with `try`, which passes
  the error up (so the caller needs `!` too), or handle it with
  `expr catch err { fallback }`, where `err.message` is the text.
  `value ?? fallback` unwraps an optional; `?? throw web.error(404, "...")`
  fails instead.
- Patterns: `match x { some(v) => ..., none => ..., _ => ... }` and
  `if found is some(post) and post.published { ... }`.
- Unwrap or leave with let-else: `let some(post) = found else { return
  web.not_found() }`; the `else` block must `return`, `break`,
  `continue` or `throw`.
- Where the enum is known, a variant can drop its name: `.Accent`,
  `.Custom(name: "x")`, and `.Accent =>` in a `match`.
- `let` stops rebinding, not changes: structs, records, lists and maps
  are shared references, so a function that pushes to a list it was given
  changes the caller's list. Copy with `Post(..post, title: "New")` or
  `{ ..record, x: 0 }`. A `const` cannot be changed at all (E0207).
- Named arguments and defaults: `web.json(post, status: 201)`.
- Generic calls name the type: `req.form<{ title: String }>()`,
  `orm.find<Post>(db)`, `json.decode<Post>(text)`.
- A generic function that calls methods on `T` bounds it with an
  interface: `interface Named { fn name(self) -> String }` and
  `fn label<T: Named>(x: T)`. Any type with those methods fits; an
  interface is never a parameter's type itself.
- `String` is text and `Bytes` is binary data: `text.bytes()`,
  `data.text()` (can fail), `fs.read_bytes`, `web.bytes(data,
  content_type: "image/png")`, `req.bytes()`, and `Bytes` form fields for
  uploads.
- Map literals: `["Ada": 36, "Grace": 45]`. Empty collections need a
  type: `let tags: List<String> = []`, `let form: Map<String, String> = Map()`.
- Multi-line text is a `"""` string: its lines start on the line after the
  opening quotes, and the closing `"""` line's indentation is removed.

## Modules

A folder is a module; its `.kiln` files share declarations. `import posts`
and `import posts.search` name folders from the project root (`search` is
the last segment). Only `pub fn`, `pub struct`, `pub enum` and `pub const`
are visible to other folders, and a method needs `pub fn` too. Imports
must not form a cycle, and a local folder may not shadow a standard
module (`web`, `sql`, ...). Imports do nothing on their own: export a
`routes(app, ...)` function and call it from `main`.

A glaze has no imports. It sees the modules its folder's `.kiln` files
import, so a page in `posts/` can take `{% props post: shared.Post %}`.
`web.render("index.glaze", {...})` finds the glaze in the calling
folder; a path with `/` is from the project root.

## Glazes, styles and embers

- `.glaze` is server-only HTML: `{% props ... %}`, `{% if %}...{% else %}...{% end %}`,
  `{% for x in xs %}...{% empty %}...{% end %}`, `{% let v = ... %}`,
  `{% layout "shared/layout.glaze" title: "Posts" %}` with `{% slot %}` in the layout.
- Components are capitalized tags: `<CommentBox post={id} />` renders
  `comment_box.glaze` or `.ember`; `<ui.Button>` is from the module `ui`.
- `<style>` in a glaze is scoped to it; `{% css "site.css" %}` is global.
- Typed styles: `ui={{ p: space.md, bg: color.surface, hover: { shadow: shadow.lg } }}`.
  Tokens are CSS custom properties your stylesheet declares (`--space-md`).
  Values are written in place and may be chosen with `if`/`match`, never computed.
- `.ember` files run in the browser too (WebAssembly): `{% state count = 0 %}`,
  `on:click={ count += 1 }`. They call the server through a typed
  `server fn` declared in Kiln code; validate its arguments on the server,
  as for any request. Prefer plain forms in glazes; use an ember only for
  interaction that needs it.
- State several embers share is a `store` in a `.kiln` module:
  `pub store cart: Cart = Cart()`, read and changed by name from any ember
  (`shop.cart.lines.push(x)`); no props or provider. Each request has its
  own, and its value is sent to the page, so keep secrets out of stores.
  `client:navigate` on a `<nav>` keeps stores across pages; `persist "key"`
  keeps one across reloads.

## Data and auth

```kiln
import sql, orm, auth

pub struct Post {
    id: Int = 0             // the primary key; 0 until inserted
    user_id: Int
    title: String
    published: Bool = false
    user: User? = none      // a relation: loaded with .include(.user)
}

let db = try sql.open(env("DATABASE_URL") ?? "sqlite:app.db")
let posts = try orm.find<Post>(db).where(.published).order_by(.id, descending: true).limit(20).all()
let post = try orm.find<Post>(db).where(fn(p) { p.id == id }).first()     // Post?
let saved = try orm.insert(db, Post(user_id: 1, title: "Hi"))
```

`kiln db diff --name add_posts` writes a migration from the models (read
it, commit it); `kiln db migrate --url sqlite:app.db` applies it. Raw SQL
uses placeholders, never string building: `db.query<Post>("select * from
posts where id = $1", id)`.

Sessions: `let sessions = try auth.sessions(db)`; `sessions.sign_in(res, subject)`,
`sessions.current(req)` (an `Identity?`), `sessions.require(req)` (fails
with 401), `sessions.csrf(req)` for a form's hidden `csrf` field and
`sessions.check(req, form.csrf)` when it comes back. Hash passwords with
`auth.hash_password` and check them with `auth.verify_password`. Check
**on the server** that the signed-in person may touch each record:
hiding a link is not authorization. `validate.check([...])` collects
form problems; return the page again with status 422 and the messages.

## Wares

`kiln add publisher/name` adds a Ware to `kiln.toml` and pins it in
`kiln.lock`; `import name` uses it. Commit both files. `kiln tree` shows
what is locked and why, with each Ware's capabilities (`filesystem`,
`network`, `database`, `environment`, `foreign`). `kiln ui add button`
copies a kiln/ui component into `components/` to own and change.
`kiln build --locked --offline` builds exactly what the lock says.

## Tests

Files named `test_*.kiln` with `fn test_*()` functions. Unit tests sit in
the module they test and may call its private functions; tests in
`tests/integration/` start the app (`test.get`, `test.post_form`) with a
fresh SQLite database and the migrations applied; tests in
`tests/browser/` drive Chromium (`test.page()`, then
`page.role("button", name: "Save").click()`,
`page.label("Title").fill("Hi")`, `page.css(".problem").expect_text("...")`).
Assertions: `test.equal`, `test.not_equal`, `test.is_true`, `test.is_false`,
`test.contains`, `test.present`, `test.fails`, `test.fail`. Each test runs
in its own process. `[test] env` in `kiln.toml` sets the app's variables
during tests.

## Habits that are errors

| Written | Write instead | Code |
| --- | --- | --- |
| `null`, `nil`, `undefined` | `none` | E0106 |
| `;` at line end | nothing | E0107 |
| `function`, `def`, `func`, `let mut`, `elif` | `fn`, `fn`, `fn`, `var`, `else if` | E0108 |
| `===`, `!==` | `==`, `!=` | E0109 |
| `&&`, `\|\|`, `!x` | `and`, `or`, `not x` | E0110 |
| Calling a function that can fail without `try` or `catch` | `try f()` | E0203 |
| `try` on a call that cannot fail | `f()` | E0204 |
| Using another folder's private name | mark it `pub` | E0230 |

Also: `return` cannot be a `match` arm's value (E0102); write
`let some(v) = x else { return ... }` instead.

## More

- [Your first Kiln site](first-site.md): one path from install to a tested app
- [Syntax](../reference/syntax.md), [types](../reference/types.md),
  [projects](../reference/projects.md), [glazes](../reference/glazes.md),
  [standard library](../reference/stdlib.md), [data](../reference/data.md),
  [Wares](../reference/wares.md), [testing](../reference/testing.md)
- [Formatting, live reload and `kiln mcp`](tooling.md)
