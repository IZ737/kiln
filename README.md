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

> **Status: phase 0 (spec and prototype).** The parsers and type checker work:
> `kiln check` reports syntax and type errors in `.kiln` files and glazes. Nothing compiles or runs yet. See the [roadmap](#roadmap).

## Highlights

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

You need a recent stable Rust toolchain.

```sh
cargo run -p kiln_cli -- check examples          # check every .kiln file
cargo run -p kiln_cli -- check --json examples   # machine-readable output
cargo run -p kiln_cli -- parse examples/hello.kiln
cargo run -p kiln_cli -- explain E0106
```

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/kiln_diagnostics` | Spans, error codes, human and JSON rendering |
| `crates/kiln_syntax` | Lexer, parser and syntax tree |
| `crates/kiln_check` | Type checker, and standard library signatures in `std/` |
| `crates/kiln_cli` | The `kiln` command |
| `examples/` | Example programs and glazes; every one is checked in CI |
| `docs/syntax.md` | The syntax the parser accepts |
| `docs/glazes.md` | Glaze (template and component) syntax |
| `docs/types.md` | The type rules the checker enforces |
| `docs/design.md` | Language design overview and roadmap |

## Roadmap

| Phase | Scope | Done when |
| --- | --- | --- |
| 0 | Spec and prototype: grammar, parser, diagnostics, type rules | The sample blog app parses and type-checks |
| 1 | Core compiler: type checker, IR, C backend, Boehm GC, a first language server (`kiln lsp`: diagnostics and quick fixes) | Compiled CLI programs pass a conformance suite |
| 2 | Runtime and HTTP server: scheduler, network poller, HTTP/1.1 and 2, TLS | A hello-world server holds up under load next to Go's net/http |
| 3 | Web standard library: router, glazes, JSON, forms, SQL, ORM with schema import, generated admin UI, sessions, auth | A blog with login, ORM models pulled from an existing Postgres database and a customized admin ships as one binary |
| 4 | Tooling: full language server (completion, hover, go to definition, rename, glaze support), static site generation (`kiln fire --static`), formatter, wares, live reload, MCP server | Debug rebuilds finish in under a second |
| 5 | Frontend: embers on WasmGC, server functions, JS interop | An ember hydrates and calls a server function |
| 6 | Performance and 1.0: precise concurrent GC, LLVM release backend | GC pauses under 1 ms; syntax frozen |
| 7 | Self-hosting (optional) | The compiler builds itself |

## License

MIT. See [LICENSE](LICENSE).
