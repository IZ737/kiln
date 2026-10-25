# Changelog

## Unreleased

Phase 1.5 for Linux and Windows x86-64: a self-contained compiler and
executables, and the start of phase 2: HTTP servers.

- **HTTP server:** `web.app()`, routes with typed parameters
  (`app.get("/posts/{id: Int}", fn(req, id) { ... })`), `web.text`,
  `web.html`, `web.redirect` and `web.not_found`, request headers, query
  parameters, cookies and bodies, and `app.listen`. A handler that fails or
  panics gets a 500 response and the server keeps going. It works with both
  backends, on Linux and Windows. See [docs/stdlib.md](docs/stdlib.md#web).
- **JSON:** `json.encode(value)` and `json.decode<T>(text)`, derived from
  types at compile time, with errors that say where a value went wrong.
  `web.json(value)` responds with JSON and `req.json<T>()` reads a request
  body; a body that does not decode gets 400. See
  [docs/stdlib.md](docs/stdlib.md#json).
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
  and `log`. See [docs/stdlib.md](docs/stdlib.md).
- **Glazes:** `.glaze` templates parse and type-check; rendering them comes
  in phase 3.

Not yet: the `web`, `sql`, `auth` and `json` modules type-check but do not
build. See [what phase 1 leaves for later](docs/design.md#what-phase-1-leaves-for-later).
