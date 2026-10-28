# Changelog

## Unreleased

- The notice that a newer kiln is out works for commands that end with
  Ctrl-C (#68). The check used to start only after a command finished, so
  `kiln dev` and servers under `kiln run` never started it; it now starts
  first, and those commands say what it found before they run. `kiln
  --version` says so too, and a failed check records why in
  `~/.kiln/update-check`. On Windows the check leaves the terminal's job
  and process group, so closing an editor's terminal does not stop it.
- `kiln upgrade` no longer offers to "upgrade" a kiln built from source
  that is newer than the latest release.

## 0.15.0 (2026-10-27)

Phase 6.7: pages as files. A site can keep its pages in a folder and let
their paths be their URLs, mixed freely with routes written in code.

- `app.file_routes("pages", at: "/", ignore: ["_components"])` adds a
  route for each `.glaze` page in a project folder when the program is
  compiled: `about.glaze` answers `GET /about`, `index.glaze` its folder's
  URL, and a `[slug]` file or folder is a URL parameter, typed from the
  page's props or the handlers that take it. The routes are ordinary
  `app.get`/`post`/... routes, tried literal segments first, so hooks,
  error pages, tests, embers, pre-rendering and static export work as
  before.
- A `route.kiln` beside an `index.glaze` gives the page its props
  (`pub fn load`), answers methods (`pub fn get`, `post`, `put`, `patch`,
  `delete`), lists its static pages (`pub fn static_params`), and sets
  `pub const static_refresh` and `pub const dynamic`. A folder with only a
  `route.kiln` is an endpoint.
- Mistakes are errors at the file they are about: mount arguments
  (E0238), two routes for the same URL or overlapping in part, or on
  `/_kiln` (E0239), and a page or `route.kiln` that does not fit its
  URL, such as a missing prop or a parameter of two types (E0240).
  Catch-alls, optional segments and route groups are errors that say what
  to do instead. A filesystem route that an earlier explicit route
  shadows is warning E0241: `kiln check` now reports warnings, which do
  not fail a build.
- `kiln fire --static` writes the pages, and an API-only `route.kiln`
  must say `pub const dynamic = true` rather than be left out silently.
- A running server no longer keeps a `.static` page that reads the
  request (a cookie, a header, the query or the body): it logs an error
  and renders that page on each request.
- In the editor, hovering `app.file_routes` lists its routes, and go to
  definition opens their files.
- `kiln new --pages` makes an app whose pages are files, and
  `examples/garden` is a complete one. New guide:
  `docs/guides/file-routes.md`.

## 0.14.0 (2026-10-27)

Phase 6.6: static sites. The same Kiln web app can be written as plain
files for any static host, or mix prerendered pages with dynamic ones.

- `kiln fire --static` builds the program and runs it so that
  `app.listen` writes its pages to `dist/` (or `-o`) instead of serving
  them: every `GET` route without parameters, with its styles, `static/`
  files, ember code and a `404.html` from `app.on_error`. No port is
  opened. `/about` is written as `about/index.html`, a file such as
  `/feed.xml` keeps its name, and a redirect becomes a page that sends the
  browser on.
- Route methods return a `web.Route`. `.static(fn() { [...] })` lists the
  pages of a route with parameters, typed from its pattern (`List<Int>`
  for `/posts/{id: Int}`, a record for more parameters), and `.dynamic()`
  leaves a `GET` route out of a static site.
- In a running server, `.static` pages are rendered once when it starts
  and answered from memory; `.static(refresh: 10m)` renders them again
  after that long.
- A site that would be broken on its host is a build error at its source:
  a route with parameters and no list (E0237), a page that reads the
  query string, headers, cookies or body, fails, or is not HTML at a path
  without an extension (E0404), an ember on a written page that calls a
  `server fn` (E0405), and a link, form, image, script or stylesheet
  `url()` to a page the site does not have (E0406).
- `kiln fire` without `--static` builds an executable as `kiln build` does,
  until phase 7's optimized release builds.
- New example `examples/static_blog`, and a guide: `docs/guides/static-sites.md`.
- The docs site renders its documentation pages once when it starts.

## 0.13.0 (2026-10-27)

Phase 6.5.1: embers share state through stores.

- `store name: Type = value` declares application state in an ordinary
  module. Any ember, handler or server code reads and changes it by name,
  with no props or provider: `shop.cart.lines.push("tea")`,
  `shop.cart = shop.Cart()`.
- After each event, the islands that read a store that changed render
  again, once and in page order, keeping their own `{% state %}`, focus
  and typed text. Deep changes count, through any alias.
- The server gives each request fresh stores, and a page with embers
  carries the ones its request used as JSON for hydration.
  `stores.scope(fn() { ... })` gives tests stores of their own.
- `client:navigate` on a link, or an element around links, loads the
  next page into the document and keeps the stores; back and forward work.
  A reload starts the stores again.
- `persist "key"` after a store keeps it in the browser's `localStorage`
  across reloads. Nothing else is kept.
- New errors: E0236 (a store without a written type, or a bad or reused
  key). A glaze can now read a folder module's `pub const`.
- `{% state %}` is unchanged and stays the way to declare an ember's own
  state.

## 0.12.0 (2026-10-27)

Phase 6.5: a review of Kiln as a language against its vision, using the
programs in this repository as evidence. Every decision, with its reason
and what it breaks, is in `docs/decisions.md`; the review is recorded in
`docs/plans/phase-6.5.md`. This release has breaking changes, listed
first.

**Breaking changes**

- `env(name)` returns `String?`; write `env("PORT") ?? "8080"`. `os.env`
  is removed.
- The server answers 403 to a `POST`, `PUT`, `PATCH` or `DELETE` that a
  browser sent from another site, unless `web.app(cross_site: [...])`
  lists the path's prefix. Hand-written `same_origin` checks can go.
- `fn rpc_guard` is removed: use `app.before("/_kiln/rpc", hook)`.
- In embers, the submitted form's map is `event.fields`.
- `ui` keys and words are snake_case: `max_w`, `text_size`,
  `focus_visible`, `.row_reverse`, `.not_allowed`. The old names are
  errors whose fix is the new name.
- `base64.encode` takes `Bytes` and `base64.decode` returns `Bytes`;
  encode text with `base64.encode(text.bytes())`.
- `Decimal` is removed. `Any` and `Rest` are reserved for the standard
  library (E0233), a `const` cannot be changed (E0207), and the standard
  library's internals cannot be used (E0230).
- `req.cookie` decodes `%XX` escapes, and a form's `String` field rejects
  data that is not UTF-8.

**Language**

- `let some(post) = found else { return web.not_found() }`.
- `.Variant` where the enum is known, in values and `match` arms.
- Copies with changes: `Post(..post, title: "New")`, `{ ..record, x: 0 }`.
- Map literals `["Ada": 36]` and `"""` multi-line strings.
- Interfaces as bounds on generic functions:
  `interface Named { fn name(self) -> String }` and
  `fn label<T: Named>(x: T)`. A type fits by having the methods, built-in
  types included; calls are compiled per type.
- `Bytes`: `text.bytes()`, `data.text()`, `len`, `slice`,
  `fs.read_bytes`, `fs.write_bytes`; Base64 in JSON and blobs in SQL.
- `docs/reference/semantics.md` describes evaluation and sharing.

**Web**

- `status:` and `headers:` on every response function, `web.render`
  included; `web.bytes(data, content_type:)`, `req.bytes()`, and file
  fields read into `Bytes`.
- `res.set_cookie(name, value, ...)` with `HttpOnly`, `Secure` and
  `SameSite=Lax` by default, and several cookies on one response.
- `app.before(prefix, hook)` runs ahead of the routes under a prefix.
- A glaze's `request: web.Request` prop is filled in automatically.
- `event.form<T>()` reads an ember's form as a struct.

## 0.11.0 (2026-10-27)

Phase 6: the developer ecosystem. Projects use versioned Wares from a
registry, Git or a folder, pinned by `kiln.lock` and kept once per user;
`kiln test` runs unit, integration and real-browser tests written in
Kiln; `kiln fmt`, `kiln dev`, references and rename make editing safe; and
`kiln mcp` gives coding agents the compiler's own answers.

- **Wares.** `[dependencies]` in `kiln.toml` (registry, `path` or `git`
  with a pinned commit), SemVer resolution with one release per Ware,
  a deterministic `kiln.lock`, `--locked` and `--offline` builds, and a
  per-user content-addressed cache (`kiln cache dir/stats/verify/prune`).
  `kiln add`, `remove`, `update`, `tree [--why]`, `fetch`, `publish`,
  `login`, `yank` and `ware verify`. Wares declare capabilities that kiln
  checks against their imports, and `[policy]` limits publishers,
  sources, capabilities and trust tiers across the graph.
- **A registry written in Kiln** (`registry/`), speaking registry protocol
  version 1: accounts, publish tokens, immutable releases, yanking, and
  an admin to verify publishers and record reviews.
- **kiln/ui**, the first official Ware: accessible Glaze components styled
  by design tokens. `kiln ui add` copies components into a project to own;
  `kiln new` makes a new app with tests.
- **`kiln test`**: `test_*` functions in `test_*.kiln` files; unit tests
  beside the code, integration tests that start the app with a database
  of their own, and browser tests that drive Chromium through kiln itself
  (role, label and text locators, hydrated embers, keyboard, screenshots on
  failure). `--filter`, `--watch`, `--json`, `--junit`, `--retries`,
  `--jobs` and `--install-browser`; each test runs in its own process.
- **`kiln fmt`**: one layout for Kiln code and glaze tags, which never
  changes tokens or comments. **`kiln dev`**: rebuilds and restarts on
  save, keeps the last good build running and reloads open pages.
- **Editors**: find references, rename across folders and document
  formatting in `kiln lsp`; Test, Dev Server and Format commands and tasks
  in VS Code.
- **Agents**: `kiln mcp`, a Model Context Protocol server with the
  project graph, structured diagnostics, explanations, Wares, formatting
  and tests (running code needs `--allow-exec`, writing `--allow-write`);
  `kiln docs --llm` and `llms.txt` give the agent guide; `bench/agents`
  holds seven benchmark tasks with hidden checks and a runner.
- **Docs**: [Wares](docs/reference/wares.md), [Testing](docs/reference/testing.md),
  [Your first Kiln site](docs/guides/first-site.md), [tooling](docs/guides/tooling.md)
  and [the agent guide](docs/guides/agents.md); `examples/guestbook` is the
  first-site guide's finished app.
- `app.listen` uses `KILN_LISTEN` when it is set, and `auth.sha256`
  hashes text.
- Fixed: `json.encode(xs.map(f))` and `web.json(xs.map(f))` encoded every
  item as `null` when the call was passed straight in.

## 0.10.0 (2026-10-26)

Phase 5: data, auth and admin. Programs reach SQLite, PostgreSQL and
MySQL through `sql`, query models through `orm` (checked and turned into
SQL at build time), keep schemas in step with reviewed `kiln db`
migrations, read forms with typed validation, sign people in with `auth`,
and get admin pages from `admin.pages<T>`.

- `sql`: parameterised queries into typed records, transactions and a
  shared pool. SQLite is built in; PostgreSQL and MySQL/MariaDB clients
  are part of the runtime and wait on Kiln's tasks, so a slow query holds
  up only its own request. TLS to a database is not supported yet.
- `orm`: models are structs; `where`, `order`, `limit`, inserts, updates
  and deletes are checked against the model and compiled to SQL for the
  database the program opens.
- `kiln db diff/migrate/status/verify/pull`: per-database migration
  folders with a neutral `schema.json`, SHA-256 ledger, drift reports, and
  models read from an existing database. Destructive changes need
  `--allow-destructive`.
- `req.form` and `validate`: typed form records with field errors that SSR
  forms and Embers share.
- `auth`: Argon2id passwords, database sessions, CSRF checks and
  same-origin guards. A `server fn` can take `web.Request` first, so RPC
  is authenticated and authorised per record on the server.
- `admin.pages<T>`: list, search, sort, page, create, edit and delete
  pages for a model, behind an authorisation function.
- `examples/journal`: a multi-folder app with sign-in, posts, an Ember
  calling authenticated ORM-backed RPC, and admin, tested in a real
  browser on all three databases.

## 0.9.0 (2026-10-26)

- **Multi-folder projects (Phase 4.2).** Explicit local folder imports, nested
  modules, public exports, private implementation types and deterministic
  project graphs. Optional strict `kiln.toml` selects the entry folder.
- Module-local Glazes, Embers, CSS and static assets; qualified component tags;
  cross-folder LSP navigation/completion and VS Code project tasks.
- Add a Fieldnotes SSR/Ember/RPC dogfood app, module conformance and browser,
  editor, visibility, manifest and source-location regressions.

- **Explicit Ember files (Phase 4.1).** Browser-capable components now use
  `.ember`; `.glaze` stays server-only. Rename reactive Glazes to `.ember`
  and update explicit filename references. Component tag names stay the same.
- Diagnose state/handlers in Glazes and hydration hints on Glaze targets
  (E0314), duplicate Glaze/Ember names (E0315), and Ember layouts (E0316).
- Discover Ember files in the CLI and LSP, with completion, hover, component
  navigation, symbols, highlighting and distinct VS Code icons.
- Preserve WasmGC, SSR, hydration, typed RPC and browser/npm interop; omit
  unreferenced components from browser assets. Static routes load no runtime.


## 0.8.0 (2026-10-26)

Phase 4: embers. Glazes with state and event handlers render on the
server and hydrate in the browser as WasmGC, call `server fn`s through
generated, checked RPC, and use browser APIs and npm packages through
typed bindings, with no JavaScript to write.

- **A WasmGC backend.** `kiln_wasm` compiles checked Kiln to WebAssembly
  that uses the browser's garbage collector, with the native runtime's
  semantics for strings, errors, JSON and collections.
  `kiln build --target wasm` writes a Node.js script, and the conformance
  tests run every program that can run in a browser as a third target.
- **Embers.** A glaze with `{% state %}` or an `on:` handler is an ember.
  The server renders it inside `<kiln-island>` with its props and state
  as JSON; a page with one loads one content-addressed script, and a page
  without loads none. `client:load`, `client:idle`, `client:visible` and
  `client:only` choose when it hydrates. After each event the ember
  renders again and the DOM is morphed in place, keeping focus, typed
  text, `key=` items, nested embers' state and the caller's slot content.
  Removed embers are disposed. Without WasmGC the page stays as rendered,
  and `<html>` gets `data-k-embers="unsupported"`.
- **Typed server calls.** A `server fn` called in a handler is a call to
  `POST /_kiln/rpc/<name>`; the statements after it run with the reply.
  The call takes a `catch`. The routes accept only same-origin JSON up to
  1 MiB from a matching build (409 otherwise), decode arguments as
  untrusted input, and call `rpc_guard(req, name)` first when the program
  has one. Pending elements get `aria-busy`; a repeated event is ignored
  while its call waits unless the handler is `.latest`, which cancels the
  older call. Calls time out and end with the page. `.prevent` and `.stop`
  are the other modifiers; unknown ones are E0313.
- **The browser and JavaScript.** `browser` (title, location, storage,
  console, focus, confirm, `fetch`, `copy`) and `js.Value`, with checked
  conversions, for what typed APIs do not reach. They run only in
  handlers (E0223).
- **npm packages.** `kiln add npm:name@version` downloads a package,
  checks its integrity, keeps only its browser module and what it imports,
  and records file checksums in `npm.lock`; it runs no scripts. Its
  TypeScript declarations become a typed Kiln module; promise-returning
  functions wait like server calls, and exports Kiln cannot map are left
  out with why.
- **What crosses is plain data.** Ember props and state and `server fn`
  parameters and results must have a JSON form (E0224).
- **The docs site uses embers**: a search box that asks the server as you
  type (a form that loads `/search` without WebAssembly) and copy buttons
  for the install commands.

## 0.7.0 (2026-10-25)

Phase 3: glazes render, with styles, typed `ui`, Finishes and themes, and
Kiln's documentation site is a Kiln program.

- **Glazes render.** `web.render("page.glaze", { props })` runs the glaze
  and answers with HTML. Glazes are compiled into the program on both
  backends: values, props with defaults, attributes, `{% let %}`,
  `{% if %}`, `{% for %}` with `where` and `{% empty %}`, `{% match %}`,
  components with children, and layouts. Values are escaped for HTML,
  values that decide a link's scheme must be relative or `http`, `https`,
  `mailto` or `tel`, and values in `on...`, `style` (except numbers) and
  `srcdoc` attributes are an error (E0309). Lines holding only tags render
  as nothing.
- **Named slots fill.** `{% fill name %} ... {% end %}` gives a layout's
  or component's `{% slot name %}` its content (E0311 when misplaced). A
  glaze that renders itself without end, through layouts or components, is
  E0310. Slot content is placed without its surrounding whitespace.
- **Glazes have CSS.** A glaze's `<style>` is scoped to its own elements,
  `{% css "site.css" %}` adds a global stylesheet and
  `{% css module card from "card.module.css" %}` gives typed, renamed
  class names (`card.card_title`; an unknown class is a type error). It is
  all built into one stylesheet, in fixed cascade layers, which every
  `<head>` links. The program serves it, the files CSS refers to and the
  `static/` folder from inside itself. CSS is read with Lightning CSS;
  CSS errors and missing files are E0312.
- **Typed styles.** `ui({ p: space.md, bg: color.surface, hover: {...},
  at: { tablet: {...} } })` and `ui={{ ... }}` in glazes are checked
  against a fixed set of keys and compiled into atomic CSS classes, with
  `ui.merge` (later styles win per property) and Finishes as ordinary
  functions returning `Ui`. Design tokens come from the `--color-*`,
  `--space-*`, ... custom properties of global stylesheets, and
  breakpoints from `--breakpoint-*`. Wrong values are E0220 and unknown
  tokens E0221.
- **Themes.** A theme is a struct of Finishes, checked like any value,
  plus a `.theme-<name>` rule in a global stylesheet that sets the token
  values; `ui({ theme: .name })` applies it. Every theme must set the
  same tokens, and token values must suit their category (E0222).
- **Editors know styles.** `kiln lsp` completes style keys, breakpoints,
  tokens, keywords and themes in `ui` records and CSS Module class names,
  hovers keys and tokens with their CSS, and goes to the stylesheet that
  declares a token or theme.
- **The documentation site is a Kiln program.** `site/` serves the
  landing, install and getting started pages and every page of `docs/`,
  with glazes, layouts, typed styles, Finishes, a CSS Module and two
  themes; changing `active_theme()` switches between them, layout
  included. `kiln run site/tools/content` regenerates its pages from
  `docs/`, and a test crawls every page and file it serves.
- The new `markdown` module has `markdown.to_html`, on the pulldown-cmark
  crate: HTML in the Markdown stays text, unsafe links are dropped and
  headings get ids.
- The new `html` module has `html.escape` and `html.safe_url`, on the
  html-escape crate, and the new `base64` module has `base64.encode` and
  `base64.decode`.

## 0.6.0 (2026-10-25)

The runtime's network-facing parts move from hand-written C to maintained
Rust libraries ([docs/plans/runtime-rust.md](docs/plans/runtime-rust.md)).

- **The web server runs on hyper.** Reading requests, chunked bodies,
  `expect: 100-continue`, keep-alive and writing responses are hyper's,
  driven on Kiln's own tasks, so handlers and concurrency work as before.
  Query strings, cookies and path segments are decoded with the
  `form_urlencoded`, `cookie` and `percent-encoding` crates. What clients
  may notice: a request whose head cannot be read gets a bare 400, 414 or
  431 with no body, HTTP/1.0 requests get an HTTP/1.0 status line, a
  request may have at most 100 headers, response header names are lower
  case, and decoded request data that is not UTF-8 gets U+FFFD. A server
  program is about 300 KB larger.
- **JSON is serde_json.** `json.encode`, `json.decode` and `req.json` use
  it, with the same error messages and paths. Text must be UTF-8 with
  well-formed escapes, and typed values nest at most 128 deep.
- `kiln_rt`, the runtime's Rust part, is linked into every program on
  both backends and both targets. Building kiln needs Rust's standard
  library for `x86_64-unknown-linux-musl` and `x86_64-pc-windows-gnu`.
- CI checks the dependencies for advisories and licences with
  `cargo deny`, and Dependabot keeps them current. `kiln upgrade` uses
  zip 9, which fixes advisories in the version before.

## 0.5.5 (2026-10-25)

- The notice that a newer kiln is out now asks GitHub once an hour, not
  once a day. `kiln upgrade` records a check, so a release that came out
  soon after an upgrade went unnoticed until the next day.

## 0.5.4 (2026-10-25)

- **Completion as you type in VS Code.** The extension told VS Code that
  only the letters `p` and `L` make up words, so VS Code did not start
  completion as you typed a name, and did not narrow the list to what
  you had typed. Completions such as a function declared further down,
  or `String` after `->`, now appear as you type.

## 0.5.3 (2026-10-25)

- kiln says when a newer release is out, after a command finishes, and to
  run `kiln upgrade` to install it. Commands never wait for the network:
  they read what the last check found from `~/.kiln/update-check` (or
  `$KILN_HOME`), and once a day a check runs in the background for the
  next command. The notice goes to standard error and only to a terminal,
  so it never shows in CI, editors, pipes, `kiln check --json` or `kiln
  lsp`. `KILN_NO_UPDATE_CHECK=1` turns it off.

## 0.5.2 (2026-10-25)

- `kiln lsp` no longer lists a declaration whose name is not typed yet,
  such as `fn ` on its own. VS Code rejected the file's whole outline
  ("name must not be falsy") while one was there.

## 0.5.1 (2026-10-25)

- The VS Code extension is bundled: its code and the libraries it uses are
  one file, so the `.vsix` holds 15 files (96 KB) instead of 331, and VS
  Code loads it faster. A test checks that the package holds only what VS
  Code loads.

## 0.5.0 (2026-10-25)

The end of phase 2.5: Kiln has a VS Code extension, and `kiln lsp` knows
the program, not just its errors.

- **VS Code extension** ([editors/vscode](editors/vscode)), attached to
  each release as `kiln-vscode-<version>.vsix`. It highlights `.kiln`
  files and glazes (with the HTML, CSS and JavaScript in glazes), gives
  them the Kiln icons, starts `kiln lsp`, and has **Kiln: Check**, **Run**
  and **Build** commands and tasks whose errors go to the Problems panel.
  It finds `kiln` through the `kiln.path` setting, your PATH or
  `~/.kiln/bin`.
- **Completion:** locals, functions, types, constants and keywords; fields
  and methods after `value.`; functions and types after `module.`;
  variants after `Type.`. Choosing a standard library module adds its
  `import`.
- **Hover** shows the type of a name or expression, or the signature of a
  function, with its doc comment.
- **Go to definition** for locals, functions, types, fields, methods,
  variants and constants, across the files of a program.
- **Symbols:** the outline of a file, and the program's functions and
  types by name (**Go to Symbol in Workspace**).
- These work in any editor with an LSP client, on unsaved and unfinished
  code; see [docs/guides/editors.md](docs/guides/editors.md).

## 0.4.0 (2026-10-25)

Installing and upgrading: kiln installs with one command and keeps itself
up to date.

- **One-line installers:** `install.sh` for Linux and `install.ps1` for
  Windows download the latest release (or `--version`), check its
  checksum, install `kiln` to `~/.kiln/bin` and add it to PATH. See
  [Install](README.md#install).
- **`kiln upgrade`:** replaces kiln with the latest release (or
  `--version`), checking its checksum; `--check` only says whether there
  is one. It works behind proxies (`HTTPS_PROXY`) and trusts the system's
  certificates. On Windows it moves the running `kiln.exe` aside to
  replace it.
- `kiln --version` reports the release's version; 0.2.0 and 0.3.0 said
  0.1.0.

## 0.3.0 (2026-10-25)

The end of phase 2: Kiln programs are HTTP APIs that run as native
binaries. [examples/api](examples/api/main.kiln) is a small JSON API.

- **Errors and settings for real deployments:** `throw web.error(404,
  "no post 7")` answers with that status and message, and other errors
  get 500 without their message; error responses are JSON for clients
  that speak JSON, and `app.on_error` can make them instead, such as an
  HTML error page. `web.app(max_body:, max_header:, timeout:, keep_alive:,
  access_log:)` has defaults for production, and the server logs a line
  per request (`info GET /posts/7 200 1.2ms`).

## 0.2.0 (2026-10-25)

Phase 1.5 for Linux and Windows x86-64: a self-contained compiler and
executables, and most of phase 2: HTTP servers.

- **HTTP server:** `web.app()`, routes with typed parameters
  (`app.get("/posts/{id: Int}", fn(req, id) { ... })`), `web.text`,
  `web.html`, `web.redirect` and `web.not_found`, request headers, query
  parameters, cookies and bodies, and `app.listen`. A handler that fails or
  panics gets a 500 response and the server keeps going. It works with both
  backends, on Linux and Windows. See [docs/reference/stdlib.md](docs/reference/stdlib.md#web).
- **JSON:** `json.encode(value)` and `json.decode<T>(text)`, derived from
  types at compile time, with errors that say where a value went wrong.
  `web.json(value)` responds with JSON and `req.json<T>()` reads a request
  body; a body that does not decode gets 400. See
  [docs/reference/stdlib.md](docs/reference/stdlib.md#json).
- **Concurrent requests:** the server runs each connection on a
  lightweight task, so a slow handler or client holds up only its own
  request, and keeps connections alive between requests. Tasks share one
  core and switch where they wait for the network or `time.sleep`.
- **Default arguments of standard library functions** are filled in when
  they are left out, as in `web.text("hi")`.

- **Native backend:** `kiln build` and `kiln run` compile to machine code
  with Cranelift and link with a linker built into `kiln`, against the Kiln
  runtime, the Boehm garbage collector and musl's C library, which `kiln`
  carries. Building needs no C compiler, linker or libraries, and the
  executables are static. It is the default on Linux x86-64; `--backend c`
  compiles through C as before.
- **Standalone `kiln`:** releases ship one static `kiln` executable for
  Linux x86-64, checked by running every conformance program in an empty
  container.
- **Windows:** `kiln build --target windows` makes Windows x86-64 `.exe`
  files with no linker: `kiln` adds the program to a Windows runtime it
  carries. They need only DLLs that come with Windows, and file names,
  arguments and console output are UTF-8. Releases also ship `kiln.exe`,
  which builds for Windows and Linux; CI runs the conformance programs with
  it on Windows. `fs.remove` removes empty directories there too, and
  `fs.rename` replaces an existing file as on Linux.

## 0.1.0 (2026-10-24)

The first tagged release: phases 0 and 1 of the [roadmap](README.md#roadmap).
Kiln compiles command-line programs to native executables.

- **Language:** structs, enums, `impl` methods, generics (compiled once per
  type), closures, optionals (`T?`, `??`, `is some(x)`), anonymous records and
  record types, `match` with guards, errors with `! E`, `try`, `catch` and
  `throw`, string interpolation (`"${x}"`), and a folder of `.kiln` files as
  one program.
- **Compiler:** `kiln check`, `build`, `run`, `explain`, `parse` and `lsp`.
  Every error has a code, and many come with a fix (`kiln check --json`).
  Programs compile through C with the Boehm garbage collector.
- **Standard library:** the prelude (`print`, and methods on `Int`, `Float`,
  `String`, `Duration`, `List`, `Map` and `Set`), `time`, `fs`, `os`, `io`
  and `log`. See [docs/reference/stdlib.md](docs/reference/stdlib.md).
- **Glazes:** `.glaze` templates parse and type-check; rendering them comes
  in phase 3.

Not yet: the `web`, `sql`, `auth` and `json` modules type-check but do not
build. See [what phase 1 leaves for later](docs/design.md#what-phase-1-leaves-for-later).
