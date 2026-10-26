# Working on the Kiln compiler

Instructions for coding agents (and people) changing this repository.

## Project direction

Read [`docs/vision.md`](docs/vision.md) for Kiln's long-term principles:
eliminate repetitive full-stack glue, keep deep customization and progressive
integration, support both directions of language interoperability, and allow
opt-in performance control without complicating normal GC-managed code.

For implementation choices, distinguish **implemented**, **roadmapped**, and
**exploratory** ideas. Follow the [roadmap](README.md#roadmap) and
[`docs/design.md`](docs/design.md) for phase scope and technical rules;
the vision is not permission to implement future milestones early or treat
proposed API spellings as current language syntax.

For Phase 3 work, follow [docs/plans/phase-3.md](docs/plans/phase-3.md): `Ui` is a
compiler-typed styling value, `ui` is a structured Kiln expression,
**Finishes are ordinary functions** returning style values/records,
themes are checked contracts, and native CSS/CSS Modules remain supported.
Keep SSR/styling separate from later Embers/WasmGC and Wares work. Target
syntax is not current syntax until implemented and tested; update the
reference docs and tests as the feature becomes real.

For Phase 4 Embers/WasmGC work, follow [docs/plans/phase-4.md](docs/plans/phase-4.md): keep SSR Glazes useful without client scripts, hydrate interactive islands selectively and secure RPC against untrusted inputs. **Do not depend on Phase 5 SQL/auth/admin**; use pure or ephemeral server functions and browser regression tests.

For the corrective Phase 4.1 source-model work, follow [docs/plans/phase-4.1.md](docs/plans/phase-4.1.md) **over any contradictory implicit-Ember wording in Phase 4**. `.ember` is the explicit browser-capable component file; `.glaze` stays server-only, and existing WasmGC/SSR/RPC behavior is preserved. The explicit source model is implemented; keep tests, docs and editor support aligned with the compiler.

For Phase 4.2 local-module/project work, follow [docs/plans/phase-4.2.md](docs/plans/phase-4.2.md) **before Phase 5**. Organize app code in imported folders with `pub` exports, optional minimal `kiln.toml`, module-aware Glazes/Embers/assets and cross-folder CLI/LSP. Do not pull Wares, registry, lockfile or public `kiln test` into this milestone.

For Phase 5 data/auth/admin work, follow [docs/plans/phase-5.md](docs/plans/phase-5.md), building on Phase 4.2 local modules and reusing Phase 4 RPC for authenticated ORM-backed calls and shared Ember/SSR validation. Keep SSR-only forms useful and enforce per-record authorization on the server.

For Phase 6 Wares/testing/tooling/MCP work, follow [docs/plans/phase-6.md](docs/plans/phase-6.md), which divides the ecosystem into four reviewable subphases. Local module resolution and a minimal optional manifest arrive in Phase 4.2; Phase 6 extends project dependency metadata. Preserve manifest-free programs and the global per-user Ware cache. Syntax in plans is prospective until implemented, tested and documented.

For Phase 6 testing, read [docs/plans/testing.md](docs/plans/testing.md): the public
`kiln test` tool must support unit, integration **and real-browser E2E**
tests in one CLI, including Ember hydration and typed RPC from its first
release. Do not invent a separate JavaScript testing ecosystem or a custom browser.

## Commands

```sh
cargo build                                  # build everything
cargo test                                   # all tests, including examples/
cargo fmt --all                              # format; CI rejects unformatted code
cargo clippy --all-targets -- -D warnings    # lint; CI rejects warnings
cargo run -p kiln_cli -- check examples      # run the CLI on the examples
cargo run -p kiln_cli -- run conformance/basics   # compile and run a program
cargo run -p kiln_cli -- build conformance/basics --emit-c out.c  # see the C
cargo run -p kiln_cli -- build conformance/basics --emit-ir out.ir  # see the IR
cargo run -p kiln_cli -- run --backend c conformance/basics   # through C instead
cargo run -p kiln_cli -- run site            # the docs site, on port 8080
cargo run -p kiln_cli -- run site/tools/content   # after changing docs/
```

`scripts/clean-check.sh path/to/kiln` runs the conformance programs with a
`kiln` binary inside an empty Docker container, as CI does for the static
release build (`cargo build --release --target x86_64-unknown-linux-musl
-p kiln_cli`). `scripts/windows-check.sh path/to/kiln.exe` does the same
for `kiln.exe` (`cargo build --release --target x86_64-pc-windows-gnu -p
kiln_cli`), on Windows or through Wine. With `KILN_WINE=wine`, `cargo test`
also runs every conformance program built with `--target windows`.

Building needs `musl-tools` and `gcc-mingw-w64-x86-64` (for the native
backend's Linux and Windows runtimes), Rust's standard library for
`x86_64-unknown-linux-musl` and `x86_64-pc-windows-gnu` (for the runtime's
Rust part; `rust-toolchain.toml` lists them, or `rustup target add` them)
and, for the C backend and its tests, a C compiler and `libgc-dev`.
`cargo deny check` (CI runs it) checks the dependencies for advisories and
licences.

Run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and
`cargo test` before every commit.

## Layout

- `crates/kiln_diagnostics`: `Span`, `SourceFile`, `Diagnostic`, and the error
  code registry in `src/codes.rs`.
- `crates/kiln_syntax`: `lexer.rs` (text to tokens), `parser.rs` (tokens to
  the tree in `ast.rs`). `lib.rs` exposes `parse(src)`. `glaze.rs` parses
  `.glaze` files with `parse_glaze(src)`, using the same lexer and parser for
  the code inside braces.
- `crates/kiln_check`: the type checker. `collect.rs` turns declarations into
  definitions, `checker.rs` checks bodies, `types.rs` holds `Ty`. `lib.rs`
  exposes `check(module)` and `check_app(modules, glazes)`, which checks a
  directory's `.kiln` files and glazes together; `glaze.rs` holds the glaze
  rules. The standard library's signatures are Kiln stubs in `std/`; their
  bodies are ignored.
- `crates/kiln_ir`: the typed IR between the checker and the backends.
  `lib.rs` defines it, `lower.rs` turns a checked program
  (`kiln_check::check_program`) into it with `lower(module, program)`, and
  `print.rs` gives its readable form. Names, closures' captures, default
  arguments and coercions are resolved here, and each generic function or
  method gets one copy per set of type arguments, so backends never look at
  the syntax tree. Lowering reports E0401 for language features with no IR
  form yet; the backend reports it for library calls it cannot build.
- `crates/kiln_codegen`: the C backend. `emit.rs` turns an IR program into
  C with `emit_c(&ir)`; `runtime/kiln.c` is prepended to it, and `kiln_rt`
  (built for the system by `build.rs`) is linked with it. Building needs
  a C compiler and the Boehm GC (`libgc-dev`).
- `crates/kiln_rt`: the runtime's Rust part, a static library linked into
  every program, which exports `k_*` functions with the C ABI. The
  network-facing parts of the runtime move here, onto maintained libraries
  (the plan and its rules are in `docs/plans/runtime-rust.md`); new runtime code
  goes here rather than in `kiln.c`. `staticlib.rs` builds it for a target
  with a nested cargo and the `runtime` profile; the build scripts of
  `kiln_native` and `kiln_codegen` use it. A function here is declared at
  the top of `kiln.c` so the C runtime can call it.
- `crates/kiln_native`: the native backend, the default where `kiln`
  carries the target's runtime. `gen.rs` turns an IR program into an object
  file with Cranelift (`compile(&ir, target)`, ELF for Linux and COFF for
  Windows), and `link(&object, target, out)` makes the executable: for
  Linux `link.rs` links it with the built-in wild linker, and for Windows
  `pe.rs` adds it to the prelinked runtime executable as new sections.
  `build.rs` compiles both runtimes: `runtime/native.c` (which includes the
  shared `kiln.c`), `kiln_rt` and the Boehm GC vendored in `vendor/bdwgc`,
  with musl's C library for Linux and mingw-w64 for Windows, all carried
  inside `kiln`.
  Code in `kiln.c` that differs on Windows goes under `#ifdef _WIN32`. `runtime.rs` lists the C signatures of the runtime functions the
  generated code calls. Generated code shows, compares, hashes and orders
  compound values through `k_desc_*` in `native.c`, which read a
  description of the type; the C backend writes the same descriptions
  (`desc` in `emit.rs`) for runtime code that works on any type, such as
  JSON in `kiln_rt`. The two backends must behave the same: the
  conformance tests run every program with both.
- `crates/kiln_wasm`: the WasmGC backend, for embers and `--target wasm`.
  `emit.rs` writes an IR program as WebAssembly text (`emit/browser.rs`
  for the `ember`, `browser` and `js` modules), which `wat` assembles and
  `wasmparser` validates. `runtime/kiln.wat` and `runtime/kiln.js` are its
  runtime (strings are JavaScript strings), and `runtime/ember.js` is the
  bridge a page with embers loads: hydration, events, the DOM morph and
  calls. The glaze compiler makes an ember's browser code with
  `kiln_ir::glaze::generate_client`; `tests/browser.rs` in `kiln_cli`
  drives it in Chromium (`tests/browser/*.mjs`, with Playwright).
- `crates/kiln_cli`: the `kiln` binary (`check`, `build`, `run`, `lsp`,
  `parse`, `explain`, `add`, `upgrade`). `npm.rs` is `kiln add npm:...`,
  `npm.lock` and the Kiln bindings made from TypeScript declarations. `lsp/mod.rs` is the language server; it
  reuses `check_sources` from `main.rs`, so editors and `kiln check` report
  the same errors. `lsp/features.rs` answers hover, definition, completion
  and symbol requests from `kiln_check::ide`, where the checker records what
  each name refers to (`Ref`) and what fits at `COMPLETION_MARKER`, which the
  server writes at the cursor. A checker change that resolves a new kind of
  name calls `refer` there too; `tests/lsp.rs` drives the server.
- `editors/vscode`: the VS Code extension. `syntaxes/` holds the TextMate
  grammars for Kiln (`source.kiln`) and Glaze (`text.html.glaze`), `icons/`
  copies of the brand icons, and `src/extension.js` starts `kiln lsp` and
  runs `kiln check`, `run` and `build` as tasks (plain JavaScript, which
  esbuild bundles into `dist/extension.js` for VS Code to load). It stays a thin client: anything
  other editors could use belongs in `kiln lsp`. `npm test` there tokenizes
  code with VS Code's TextMate engine; a syntax change updates the
  grammars and their tests too.
- `conformance/`: programs, one folder each, with expected output
  (`main.out`, and `main.err` when
  they fail). `crates/kiln_cli/tests/conformance.rs` runs each one; add one
  for every feature the compiler learns. A server program also has
  `main.requests` and `main.responses` (see `conformance/README.md`), and
  `scripts/lib/conformance.sh` sends them in the clean-system checks too.
  The `web` server's connections are hyper's, in `kiln_rt/src/server.rs`,
  which builds `Request` and reads `Response` as slot arrays in the field
  order of `std/web.kiln`; routing, handlers and error pages stay in
  `kiln.c`. It runs each connection on a task (the scheduler is in
  `kiln.c`), polling hyper and waiting through `k_task_wait`: runtime
  code that waits for a socket or for time calls `k_task_wait` or
  `k_task_sleep` instead of blocking, and a global that belongs to one
  request is saved in `k_task_switch`. `tests/server.rs` checks that
  requests run at the same time.
- `site/`: Kiln's documentation website, a Kiln program that uses every
  part of phase 3 (glazes, layouts, `ui`, Finishes, two themes in
  `theme.kiln`, global CSS, a CSS Module and scoped `<style>`).
  `content.kiln` carries the pages of `docs/` and is generated by
  `site/tools/content` (also Kiln): a change to `docs/` reruns it, or
  `tests/site.rs` fails. That test crawls the running site and checks
  every link, file and page.
- `install.sh` and `install.ps1`: the one-line installers in the README.
  They download a release by the asset names `release.yml` gives
  (`kiln-<tag>-<arch>-<os>.tar.gz` or `.zip`, with a `.sha256`), so a
  change to those names changes them too, and `kiln upgrade`
  (`crates/kiln_cli/src/upgrade.rs`). `installers.yml` runs the scripts
  against the real releases.
- `examples/`: Kiln programs. Tests in `crates/kiln_syntax/tests/examples.rs`
  and `crates/kiln_check/tests/examples.rs` parse and type-check every one of
  them, so they must stay valid.
- `assets/brand/`: the logo and vocabulary icons. Brand colours are terracotta
  `#C65A34`, charcoal `#1F232B` and off-white `#F7F3EC`.
- `docs/reference/syntax.md` and `docs/reference/glazes.md` document what the parsers accept; `docs/reference/stdlib.md` lists
  the standard library modules the compiler builds; `docs/design.md` is the
  language design.

## Conventions

- **Never stop at the first error.** The parser records a diagnostic and
  recovers (`recover_to_line_end`, `recover_to_item`). Every loop that parses
  a list must make progress even on bad input.
- **Every diagnostic has a code.** Add new codes to `codes.rs` with a title,
  an explanation, and a wrong and right example; `kiln explain` prints them.
  Codes are never reused. Ranges: `E00xx` lexer, `E01xx` parser, `E02xx`
  names and types, `E03xx` glazes, `E04xx` the compiler.
- **Give a fix when there is one.** Use `Diagnostic::with_suggestion` with
  the exact replacement text, so agents can apply it from `kiln check --json`.
- **Syntax changes update three places:** the parser, a test in
  `crates/kiln_syntax/tests/parser.rs` (or `glaze.rs`), and `docs/reference/syntax.md`
  (or `docs/reference/glazes.md`). New keywords and literals also go in the grammars
  in `editors/vscode/syntaxes`.
- **One mistake, one error.** After an error the checker uses `Ty::Unknown`,
  which is compatible with everything, so later uses are not reported again.
- **Standard library changes** go in `crates/kiln_check/std/*.kiln`. New
  modules are also listed in `STD_MODULES` in `checker.rs`. To compile a
  module's functions, see the steps in `docs/reference/stdlib.md`.
- **The compiler never guesses.** A program that type-checks but uses
  something the backend cannot build yet gets E0401 at that spot, never
  wrong code.
- **Both backends learn every feature.** A change to what the compiler
  builds goes in `kiln_codegen/src/emit.rs` and `kiln_native/src/gen.rs`;
  runtime functions are `K_API` in `kiln.c` or `#[no_mangle] extern "C"`
  in `kiln_rt`, so both can call them.
- Keep error messages short, specific, and in plain words: say what was
  expected and what was found.

## Vocabulary

Source files are `.kiln`. Server-only templates and components are glazes (`.glaze`),
packages are wares, browser-capable WebAssembly components are embers (`.ember`), and
`kiln fire` makes a release build.
