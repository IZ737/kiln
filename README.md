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

> **Status: phases 0 and 1 are done, and phase 1.5 (self-contained compiler
> and executables) is done for Linux x86-64 and Windows x86-64; phase 2
> (HTTP) is in progress.**
> `kiln check` reports syntax and type errors in `.kiln` files and glazes,
> `kiln lsp` shows them in your editor, and `kiln run` compiles command-line
> programs with the `time`, `fs`, `os` and `io` modules, and HTTP servers
> with `web`: the example above runs. `kiln` compiles to
> machine code with Cranelift and makes Linux and Windows x86-64
> executables itself (`--target linux` or `--target windows`, from either
> system), so it needs no C compiler, linker or libraries; the C backend
> (`--backend c`) remains for other platforms. JSON works in both
> directions (`json`, `web.json`, `req.json<T>()`); templates, `sql` and
> `auth` arrive in later phases. See the [roadmap](#roadmap) and
> [what phase 1 leaves for later](docs/design.md#what-phase-1-leaves-for-later).

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

To build the compiler from source you need a recent stable Rust toolchain
and the C compilers that build the runtimes `kiln` carries inside it:
`musl-gcc` for Linux executables (`musl-tools` on Debian and Ubuntu,
`musl-gcc` on Fedora) and mingw-w64 for Windows ones
(`gcc-mingw-w64-x86-64` on Debian and Ubuntu, `mingw64-gcc` on Fedora). The
`kiln` you get then compiles programs with no other tools: its Linux
executables are static and run on any Linux x86-64 system, and its Windows
`.exe` files use only DLLs that come with Windows. Without either compiler,
or with `--backend c`, `kiln` compiles through C and needs a C compiler and
the Boehm garbage collector (`libgc-dev` on Debian and Ubuntu, `gc-devel`
on Fedora, `bdw-gc` on Homebrew).

Releases after 0.1.0 also ship a prebuilt `kiln` for Linux x86-64 and a
`kiln.exe` for Windows x86-64 on the
[releases page](https://github.com/IZ737/kiln/releases): one executable
that needs nothing else installed.

```sh
tar xzf kiln-v*-x86_64-linux.tar.gz
./kiln-v*-x86_64-linux/kiln run examples/hello
./kiln-v*-x86_64-linux/kiln build examples/hello --target windows   # hello.exe
```

```sh
cargo run -p kiln_cli -- run examples/hello     # compile and run a program
cargo run -p kiln_cli -- build examples/hello -o hello
cargo run -p kiln_cli -- check examples          # check every .kiln file
cargo run -p kiln_cli -- check --json examples   # machine-readable output
cargo run -p kiln_cli -- parse examples/hello/main.kiln
cargo run -p kiln_cli -- explain E0106
```

For errors and quick fixes in your editor, see [docs/editors.md](docs/editors.md).

## Repository layout

| Path | Contents |
| --- | --- |
| `crates/kiln_diagnostics` | Spans, error codes, human and JSON rendering |
| `crates/kiln_syntax` | Lexer, parser and syntax tree |
| `crates/kiln_check` | Type checker, and standard library signatures in `std/` |
| `crates/kiln_ir` | The typed IR between the checker and the backends |
| `crates/kiln_codegen` | The C backend, and the runtime in `runtime/kiln.c` |
| `crates/kiln_native` | The native backend: Cranelift code generation, built-in linking for Linux and Windows, and the bundled runtime, Boehm GC (`vendor/bdwgc`) and C libraries |
| `crates/kiln_cli` | The `kiln` command |
| `examples/` | Example programs and glazes; every one is checked in CI |
| `conformance/` | Programs with their expected output; every one is compiled and run in CI |
| `assets/brand/` | Logo, symbol and the vocabulary icons (`.kiln`, glaze, ware, ember, `kiln fire`), as SVG and PNG |
| `docs/syntax.md` | The syntax the parser accepts |
| `docs/glazes.md` | Glaze (template and component) syntax |
| `docs/types.md` | The type rules the checker enforces |
| `docs/stdlib.md` | The standard library modules the compiler builds |
| `docs/editors.md` | Setting up `kiln lsp` in Neovim, Helix and other editors |
| `docs/design.md` | Language design overview and roadmap |

## Roadmap

| Phase | Focus | Done when |
| --- | --- | --- |
| 0 | Language specification, grammar, parser, diagnostics, type rules | The blog example parses and type-checks (done) |
| 1 | Core compiler, C backend, Boehm GC, basic LSP (`kiln lsp`: diagnostics and quick fixes), conformance tests | Native CLI programs compile and run correctly (done) |
| 1.5 | Self-contained toolchain and executables: Cranelift native backend, integrated linking, statically bundled runtime and Boehm GC (Linux x86-64 first); keep the C backend | Download one `kiln` executable and compile supported CLI programs without installing a C compiler, linker or runtime libraries; run the resulting native executables on compatible clean systems without installing dependencies |
| Windows target (post-1.5, non-blocking) | Windows x86-64 support: standalone `kiln.exe` compiler and native Windows `.exe` outputs, with a bundled linker, statically linked Kiln runtime and GC, and Windows CI conformance | Download `kiln.exe`, compile a supported CLI app and run its `.exe` on a clean compatible Windows system without installing a C compiler, linker, Rust or a separate GC/runtime package |
| 2 | Core runtime, HTTP server, typed routing, JSON, error handling | A real HTTP API runs as a native binary |
| 3 | Glazes, server-rendered components, layouts, CSS, scoped styling, design tokens | The Kiln documentation website is built in Kiln |
| 4 | SQL (SQLite first, for local development with no database server, then Postgres and MySQL), ORM with schema import (`kiln db pull`), migrations, forms, validation, sessions and auth, generated admin UI | A database-backed application with authentication works |
| 5 | Developer tooling: full LSP (completion, hover, go to definition, rename, glaze support), formatter, hot reload, testing, wares (package management), MCP server and agent guide | A complete application can be developed and tested comfortably |
| 6 | Embers on WasmGC, reactive UI, server functions, generated RPC, JS interoperability | A full-stack interactive Kiln application works end-to-end |
| 7 | Production hardening, static site generation (`kiln fire --static`), deployment, performance (precise GC, LLVM release builds), stability | Kiln 1.0: stable and ready for personal production projects |
| 8+ | Installable PWAs with local-first data that syncs with the server, advanced optimization, interoperability improvements, desktop apps, additional targets, request arenas, self-hosting | Independent milestones after 1.0 |

## License

MIT. See [LICENSE](LICENSE).
