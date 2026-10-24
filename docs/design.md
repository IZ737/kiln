# Kiln design overview

This is a summary of the language design. Decisions here can still change
until 1.0; open questions are listed at the end.

## Core principle

Most web development today is gluing together problems that were solved
long ago (routing, templates, the ORM, auth, forms, the admin, asset builds,
deployment) and sometimes reinventing one. Kiln removes that work: every one
of those pieces ships with the language and they already fit together, so a
project that follows the conventions needs no configuration and no glue
code. The conventions are defaults, not walls. Any piece can be configured
in depth, or replaced, where it is used. Every design decision is checked
against this rule.

## Progressive integration

Kiln provides a consistent developer experience whether used for an
individual component, one application layer or an entire full-stack
application.

The compiler uses all available information to provide type checking, code
generation and optimization. Full-stack Kiln applications benefit from
deeper automatic integration, while applications containing external systems
use explicit, typed contracts and adapters to unlock as many of the same
capabilities as possible.

No application feature should unnecessarily require ownership of the entire
technology stack.

## Goals

1. **One binary deploys**, with templates and assets embedded.
2. **Batteries included for the web**: HTTP/1.1 and 2, TLS, routing,
   templating, JSON, forms, SQL with an ORM, a generated admin UI, sessions,
   auth and background jobs.
3. **Errors at compile time**: routes, template variables, SQL results and
   JSON shapes are type-checked.
4. **Safe by default**: auto-escaping templates, parameterized SQL, signed
   cookies, CSRF protection on.
5. **Fast edit loop**: sub-second debug builds with live reload.
6. **No async/await split**: every request runs on a lightweight thread.
7. **One language for server and browser**: components render on the server
   and become interactive through WebAssembly only where needed.
8. **Easy for coding agents**: an LLM that has never seen Kiln can write
   correct code from the material shipped with the compiler.

Non-goals for 1.0: systems programming, an interpreter, and reimplementing
the JavaScript ecosystem (browser code calls npm packages through generated
bindings).

## Types

- Static types with local inference. Function signatures are always typed.
- Primitives: `Int` (64-bit), `Float`, `Bool`, `String` (UTF-8), `Bytes`,
  `Decimal` for money, `Duration` (`250ms`, `2s`).
- `List<T>`, `Map<K, V>`, `Set<T>`; structs with defaults; enums with payloads
  and exhaustive `match`; structural interfaces; generics by monomorphization.
- No null. `T?` is optional and the missing value is `none`.
- Errors are values: `fn f() -> T ! E`, propagated with `try`, handled with
  `catch`. A bare `!` uses the standard `Error` and accepts any error; `! E`
  only passes on errors of type `E`. See [types.md](types.md). Panics are for bugs only, and the HTTP server recovers them per
  request.
- JSON, form and SQL encoding are derived at compile time, with no runtime
  reflection.

## Programs and folders

A folder is one program, as in Go. All of its `.kiln` files share one
namespace, so splitting a file in two never needs an import, and the
folder's glazes see the same types and functions. Imports stay per file.
A subfolder will be a module, imported by its name, with `pub` items as its
interface; that arrives with wares.

## Memory and concurrency

- A concurrent, non-moving, tri-color mark-and-sweep GC, as in Go. Phase 1
  links the Boehm collector; phase 7 replaces it with a precise collector.
- Escape analysis, and later request-scoped arenas for garbage that dies when
  a response is sent.
- An M:N scheduler of lightweight tasks; blocking I/O parks the task. Tasks
  spawned in a request are cancelled when it ends. Values shared between tasks
  must be immutable or wrapped in `Mutex`, `Atomic` or a channel.

## Compilation

The compiler is written in Rust. Source is parsed, type-checked and lowered
to a typed mid-level IR, which feeds a backend:

| Backend | Use | Phase |
| --- | --- | --- |
| C (via clang, gcc or `zig cc`) | First backend; free cross-compilation | 1 |
| Cranelift | Fast debug builds | 5 |
| WasmGC | Browser code (embers) | 6 |
| LLVM | Optimized release builds | 7 |

The IR (`crates/kiln_ir`) is a typed tree: names are resolved, closures and
field shorthands are lifted into functions with their captures listed (a
captured `var` lives in a heap cell), call arguments are in parameter order
with defaults filled in, and every place a value changes representation,
such as an `Int` passed for a generic `T`, is an explicit coercion. It stays
a tree of blocks, loops and `if`s rather than a control-flow graph, so the
C backend maps it almost line for line; later backends can flatten it.
`kiln build --emit-ir` writes its readable form.

The phase 1 C backend (`crates/kiln_codegen`) emits one C file with the
runtime (`runtime/kiln.c`) in front, and builds it with `$CC` or `cc`
against the Boehm collector. Values keep their natural C types (`int64_t`,
`double`, `bool`, pointers); generic code, list items, enum fields and
closure arguments use a uniform 64-bit slot, so generic functions are
compiled once rather than monomorphized for now. A call that fails sets a
thread-local flag that `try` and `catch` check. Parts of the language the
backend does not support yet are reported as E0401.

## Web standard library

`web` (server, router, static files), `template` (glazes), `json`, `form`,
`validate`, `sql` (SQLite first, so local development needs no database
server, then Postgres and MySQL; migrations), `orm`, `admin`, `session`, `auth`
(Argon2, CSRF, rate limiting, JWT, OAuth 2/OIDC), `http.client`, `log`,
`metrics`, `trace`, `cache`, `jobs`, `ws`, `sse`, `mail`, `crypto`, `time`,
`fs`, `os`, `env`, `io`, `text`, `test`.

### ORM

The `orm` module ships with the standard library, so a database-backed site
needs no third-party package.

- **Models are ordinary structs.** Table and column names are derived
  (`Post` maps to `posts`, `author_id` to a foreign key) and can be
  overridden. Relations are typed fields: `author: User` and
  `comments: List<Comment>` load on request, never implicitly in a loop.
- **Typed queries.** `orm.find<Post>(db).where(.published).order_by(.created).limit(20)`
  is checked at compile time: a misspelled field or a comparison of the
  wrong type is a build error. Raw SQL stays available through `sql.query<T>`.
- **Migrations from models.** `kiln db diff` compares models with the database
  and writes a migration; `kiln db migrate` applies it.
- **Existing databases.** `kiln db pull` reads the schema of an existing
  Postgres, MySQL or SQLite database, including keys, foreign keys,
  indexes, defaults and enums, and generates `.kiln` model files with their
  relations. The generated models are normal code to edit; later pulls
  report drift instead of overwriting changes.

### Admin UI

The `admin` module generates a Django-style admin site from ORM models, in
one line:

```kiln
app.mount("/admin", admin.site(db, models: [Post, User, Comment]))
```

- List, search, filter, create, edit and delete pages for every model, with
  forms and validation derived from field types and relations.
- Protected by `auth`, with per-model permissions.
- **Fully customizable, in layers:** per-model options in code (columns,
  search fields, filters, read-only fields, custom actions); any field
  widget replaced by a component; and any page replaced by placing a glaze
  at the matching path, such as `admin/post/edit.glaze`. The admin is built
  from glazes, so overrides use the same component system as the rest of the
  site.

## Glazes and embers

Glazes (`.glaze`) are templates compiled to typed functions, with
context-aware escaping, typed props, layouts, components and reverse routing.
Every component renders on the server. A component that declares `state` or
handles events becomes an ember: it is compiled to WebAssembly (WasmGC) and
hydrates the server-rendered HTML. `server fn` declarations are callable from
embers as typed RPC; server-only modules such as `sql` cannot be referenced
from browser code.

### Static site generation

The same app can be built as a static site, for blogs, docs and marketing
pages that need no server. Planned for phase 7:

- `kiln fire --static` renders every route to HTML files, with assets, ready
  for any static host or CDN. Routes with parameters list their pages:
  `app.get("/posts/{id: Int}", show_post).static(fn() { all_post_ids() })`.
- Mixed apps prerender the routes marked `.static(...)` at build time and
  serve the rest dynamically from the same binary, optionally re-rendering a
  static page on a schedule (`.static(..., refresh: 10m)`).
- Embers still work on static pages: they hydrate in the browser as usual.
  Server functions need a server, so a fully static export reports any ember
  that calls one as a build error.

### Installable apps (PWAs)

Planned for phase 8+, after 1.0. Marking an app installable
(`app.pwa(name: "Notes", icon: "icon.png")`) makes `kiln fire` generate the
web manifest, icons at every size and a service worker that caches the app
shell and assets, versioned with each release. A standard library `local`
module gives embers a typed store in the browser (IndexedDB underneath) for
local-first data: changes queue while offline, and a `sync` layer pushes
them to the server and pulls other changes when the connection returns, with
a conflict rule chosen per model (last write wins, merge fields, or a custom
function). The same ORM models describe both sides. Push notifications and
background sync come with it.

## Editor support

`kiln lsp` is a language server (LSP) for any editor. The first version, in
phase 1, reports the same diagnostics as `kiln check` as you type and offers
their suggested fixes as quick fixes; [editors.md](editors.md) shows how to
set it up. Phase 5 adds completion, hover types,
go to definition, find references, rename, and the same support inside
glazes, including props and component attributes.

## Built for coding agents

- Familiar syntax and one way to do each thing, so model priors transfer.
- Targeted errors for habits from other languages (`null`, `;`, `&&`,
  `function`), each with a machine-applicable fix.
- `kiln check --json` and `kiln explain <code>` from phase 0; later an agent
  guide (`kiln docs --llm`, `llms.txt`), a generated `AGENTS.md` in new
  projects, an MCP server (`kiln mcp`), task examples tested in CI, and an
  agent benchmark that gates releases.

## Open questions

- Should a `.glaze` page with inline code be able to serve a route directly
  (PHP style), or should every route go through a handler?
- Whether the exact WasmGC browser support matches the plan should be
  confirmed before phase 6.
