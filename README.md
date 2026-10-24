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

> **Status: phase 0 (spec and prototype).** The parser and the `kiln check`
> command work. Nothing compiles or runs yet. See the [roadmap](#roadmap).

## Highlights

- **One binary deploys.** `kiln fire` will produce a static executable with
  templates and assets embedded.
- **Errors at compile time.** Routes, templates, SQL results and JSON shapes
  are type-checked. No `null`: optionals are `T?`, missing is `none`.
- **No async/await split.** Each request runs on a lightweight thread, as in Go.
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
| `crates/kiln_cli` | The `kiln` command |
| `examples/` | Example programs; every one is parsed in CI |
| `docs/syntax.md` | The syntax the parser accepts |
| `docs/design.md` | Language design overview and roadmap |

## Roadmap

| Phase | Scope | Done when |
| --- | --- | --- |
| 0 | Spec and prototype: grammar, parser, diagnostics, type rules | The sample blog app parses and type-checks |
| 1 | Core compiler: type checker, IR, C backend, Boehm GC | Compiled CLI programs pass a conformance suite |
| 2 | Runtime and HTTP server: scheduler, network poller, HTTP/1.1 and 2, TLS | A hello-world server holds up under load next to Go's net/http |
| 3 | Web standard library: router, glazes, JSON, forms, SQL, sessions, auth | A blog with login and Postgres ships as one binary |
| 4 | Tooling: formatter, language server, wares, live reload, MCP server | Debug rebuilds finish in under a second |
| 5 | Frontend: embers on WasmGC, server functions, JS interop | An ember hydrates and calls a server function |
| 6 | Performance and 1.0: precise concurrent GC, LLVM release backend | GC pauses under 1 ms; syntax frozen |
| 7 | Self-hosting (optional) | The compiler builds itself |

## License

MIT. See [LICENSE](LICENSE).
