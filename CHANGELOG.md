# Changelog

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
  code; see [docs/editors.md](docs/editors.md).

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
  backends, on Linux and Windows. See [docs/stdlib.md](docs/stdlib.md#web).
- **JSON:** `json.encode(value)` and `json.decode<T>(text)`, derived from
  types at compile time, with errors that say where a value went wrong.
  `web.json(value)` responds with JSON and `req.json<T>()` reads a request
  body; a body that does not decode gets 400. See
  [docs/stdlib.md](docs/stdlib.md#json).
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
  and `log`. See [docs/stdlib.md](docs/stdlib.md).
- **Glazes:** `.glaze` templates parse and type-check; rendering them comes
  in phase 3.

Not yet: the `web`, `sql`, `auth` and `json` modules type-check but do not
build. See [what phase 1 leaves for later](docs/design.md#what-phase-1-leaves-for-later).
