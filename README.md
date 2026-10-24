<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="assets/brand/logo/kiln-horizontal-dark.svg">
    <img src="assets/brand/logo/kiln-horizontal.svg" alt="Kiln" width="320">
  </picture>
</p>

# Kiln

Kiln is a compiled, garbage-collected language for building web servers and
websites. Think PHP's "write a file, get a page" speed with Go's single-binary
deployment and a stronger type system than either, plus a standard library
that covers HTTP, routing, templates, JSON, SQL, sessions and auth out of the
box.

```kiln
import web

fn main() ! {
    let app = web.app()
    app.get("/hello/{name}", fn(req, name) {
        web.text("Hello, ${name}!")
    })
    try app.listen(":8080")
}
```

> **Status: phase 1 (core compiler).** `kiln check` reports syntax and type
> errors in `.kiln` files and glazes, `kiln lsp` shows them in your editor,
> and `kiln run` compiles command-line
> programs to native code through C. The web standard library (`web`, `sql`,
> `json`, `auth`) arrives in later phases. See the [roadmap](#roadmap).

## Highlights

- **Convention over configuration.** Routing, templates, the ORM, auth, the
  admin, builds and deployment ship with the language and already fit
  together, so a project needs no glue code. Any piece can still be
  configured in depth, or replaced, where it is used.
- **Progressive integration.** The same experience whether Kiln runs one
  component, one layer or a whole full-stack app. Full-stack apps get the
  deepest automatic integration; external systems connect through typed
  contracts and adapters, so no feature needs you to own the entire stack.
- **One binary deploys.** `kiln fire` will produce a static executable with
  templates and assets embedded.
- **Errors at compile time.** Routes, templates, SQL results and JSON shapes
  are type-checked. No `null`: optionals are `T?`, missing is `none`.
- **No async/await split.** Each request runs on a lightweight thread, as in Go.
- **Database to admin in minutes.** A built-in ORM can read an existing
  database's schema into typed models, and a generated, fully customizable
  admin UI (like Django's) manages the data.
- **Static when you want it.** The same app can be exported as a static site,
  or mix prerendered pages with dynamic ones.
- **Server-rendered by default.** Components (glazes) render to HTML on the
  server; only interactive ones (embers) compile to WebAssembly.
- **Built for coding agents.** Familiar syntax, one way to do things, error
  messages with machine-applicable fixes (`kiln check --json`), and an
  explanation for every error code (`kiln explain E0106`).

## Vocabulary

| Term | Meaning |
| --- | --- |
| `.kiln` | Source file |
| Glaze (`.glaze`) | A template or component |
| Ware | A package |
| Ember | An interactive component compiled to WebAssembly |
| `kiln fire` | Make a release build |

## Try it

You need a recent stable Rust toolchain, a C compiler, and the Boehm garbage
collector (`libgc-dev` on Debian and Ubuntu, `gc-devel` on Fedora, `bdw-gc`
on Homebrew).

```sh
cargo run -p kiln_cli -- run examples/hello.kiln # compile and run a program
cargo run -p kiln_cli -- build examples/hello.kiln -o hello
cargo run -p kiln_cli -- check examples          # check every .kiln file
cargo run -p kiln_cli -- check --json examples   # machine-readable output
cargo run -p kiln_cli -- parse examples/hello.kiln
cargo run -p kiln_cli -- explain E0106
```

For errors and quick fixes in your editor, see [docs/editors.md](docs/editors.md).

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/kiln_diagnostics` | Spans, error codes, human and JSON rendering |
| `crates/kiln_syntax` | Lexer, parser and syntax tree |
| `crates/kiln_check` | Type checker, and standard library signatures in `std/` |
| `crates/kiln_codegen` | The C backend, and the runtime in `runtime/kiln.c` |
| `crates/kiln_cli` | The `kiln` command |
| `examples/` | Example programs and glazes; every one is checked in CI |
| `conformance/` | Programs with their expected output; every one is compiled and run in CI |
| `assets/brand/` | Logo, symbol and the vocabulary icons (`.kiln`, glaze, ware, ember, `kiln fire`), as SVG and PNG |
| `docs/syntax.md` | The syntax the parser accepts |
| `docs/glazes.md` | Glaze (template and component) syntax |
| `docs/types.md` | The type rules the checker enforces |
| `docs/editors.md` | Setting up `kiln lsp` in Neovim, Helix and other editors |
| `docs/design.md` | Language design overview and roadmap |

## Roadmap

| Phase | Focus | Done when |
| --- | --- | --- |
| 0 | Language specification, grammar, parser, diagnostics, type rules | The blog example parses and type-checks |
| 1 | Core compiler, C backend, Boehm GC, basic LSP (`kiln lsp`: diagnostics and quick fixes), conformance tests | Native CLI programs compile and run correctly |
| 2 | Core runtime, HTTP server, typed routing, JSON, error handling | A real HTTP API runs as a native binary |
| 3 | Glazes, server-rendered components, layouts, CSS, scoped styling, design tokens | The Kiln documentation website is built in Kiln |
| 4 | SQL (SQLite first, for local development with no database server, then Postgres and MySQL), ORM with schema import (`kiln db pull`), migrations, forms, validation, sessions and auth, generated admin UI | A database-backed application with authentication works |
| 5 | Developer tooling: full LSP (completion, hover, go to definition, rename, glaze support), formatter, hot reload, testing, wares (package management), MCP server and agent guide | A complete application can be developed and tested comfortably |
| 6 | Embers on WasmGC, reactive UI, server functions, generated RPC, JS interoperability | A full-stack interactive Kiln application works end-to-end |
| 7 | Production hardening, static site generation (`kiln fire --static`), deployment, performance (precise GC, LLVM release builds), stability | Kiln 1.0: stable and ready for personal production projects |
| 8+ | Installable PWAs with local-first data that syncs with the server, advanced optimization, interoperability improvements, desktop apps, additional targets, request arenas, self-hosting | Independent milestones after 1.0 |

## License

MIT. See [LICENSE](LICENSE).
