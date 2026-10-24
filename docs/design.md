# Kiln design overview

This is a summary of the language design. Decisions here can still change
until 1.0; open questions are listed at the end.

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

## Memory and concurrency

- A concurrent, non-moving, tri-color mark-and-sweep GC, as in Go. Phase 1
  links the Boehm collector; phase 6 replaces it with a precise collector.
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
| Cranelift | Fast debug builds | 4 |
| WasmGC | Browser code (embers) | 5 |
| LLVM | Optimized release builds | 6 |

## Web standard library

`web` (server, router, static files), `template` (glazes), `json`, `form`,
`validate`, `sql` (Postgres, MySQL, SQLite, migrations), `orm`, `admin`, `session`, `auth`
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

## Editor support

`kiln lsp` is a language server (LSP) for any editor. The first version, in
phase 1, reports the same diagnostics as `kiln check` as you type and offers
their suggested fixes as quick fixes. Phase 4 adds completion, hover types,
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
  confirmed before phase 5.
