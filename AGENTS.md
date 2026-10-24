# Working on the Kiln compiler

Instructions for coding agents (and people) changing this repository.

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
```

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
  arguments and coercions are resolved here, so backends never look at the
  syntax tree. Lowering reports E0401 for language features with no IR
  form yet; the backend reports it for library calls it cannot build.
- `crates/kiln_codegen`: the C backend. `emit.rs` turns an IR program into
  C with `emit_c(&ir)`; `runtime/kiln.c` is prepended to it. Building needs
  a C compiler and the Boehm GC (`libgc-dev`).
- `crates/kiln_cli`: the `kiln` binary (`check`, `build`, `run`, `lsp`,
  `parse`, `explain`). `lsp.rs` is the language server; it reuses
  `check_sources` from `main.rs`, so editors and `kiln check` report the same
  errors.
- `conformance/`: programs, one folder each, with expected output
  (`main.out`, and `main.err` when
  they fail). `crates/kiln_cli/tests/conformance.rs` runs each one; add one
  for every feature the compiler learns.
- `examples/`: Kiln programs. Tests in `crates/kiln_syntax/tests/examples.rs`
  and `crates/kiln_check/tests/examples.rs` parse and type-check every one of
  them, so they must stay valid.
- `assets/brand/`: the logo and vocabulary icons. Brand colours are terracotta
  `#C65A34`, charcoal `#1F232B` and off-white `#F7F3EC`.
- `docs/syntax.md` and `docs/glazes.md` document what the parsers accept; `docs/design.md` is the
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
  `crates/kiln_syntax/tests/parser.rs` (or `glaze.rs`), and `docs/syntax.md`
  (or `docs/glazes.md`).
- **One mistake, one error.** After an error the checker uses `Ty::Unknown`,
  which is compatible with everything, so later uses are not reported again.
- **Standard library changes** go in `crates/kiln_check/std/*.kiln`. New
  modules are also listed in `STD_MODULES` in `checker.rs`.
- **The compiler never guesses.** A program that type-checks but uses
  something the backend cannot build yet gets E0401 at that spot, never
  wrong C.
- Keep error messages short, specific, and in plain words: say what was
  expected and what was found.

## Vocabulary

Source files are `.kiln`. Templates and components are glazes (`.glaze`),
packages are wares, interactive WebAssembly components are embers, and
`kiln fire` makes a release build.
