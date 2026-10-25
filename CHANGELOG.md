# Changelog

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
