# Working on the Kiln compiler

Instructions for coding agents (and people) changing this repository.

## Commands

```sh
cargo build                                  # build everything
cargo test                                   # all tests, including examples/
cargo fmt --all                              # format; CI rejects unformatted code
cargo clippy --all-targets -- -D warnings    # lint; CI rejects warnings
cargo run -p kiln_cli -- check examples      # run the CLI on the examples
```

Run `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and
`cargo test` before every commit.

## Layout

- `crates/kiln_diagnostics`: `Span`, `SourceFile`, `Diagnostic`, and the error
  code registry in `src/codes.rs`.
- `crates/kiln_syntax`: `lexer.rs` (text to tokens), `parser.rs` (tokens to
  the tree in `ast.rs`). `lib.rs` exposes `parse(src)`.
- `crates/kiln_cli`: the `kiln` binary (`check`, `parse`, `explain`).
- `examples/`: Kiln programs. `crates/kiln_syntax/tests/examples.rs` parses
  every one of them, so they must stay valid.
- `docs/syntax.md` documents what the parser accepts; `docs/design.md` is the
  language design.

## Conventions

- **Never stop at the first error.** The parser records a diagnostic and
  recovers (`recover_to_line_end`, `recover_to_item`). Every loop that parses
  a list must make progress even on bad input.
- **Every diagnostic has a code.** Add new codes to `codes.rs` with a title,
  an explanation, and a wrong and right example; `kiln explain` prints them.
  Codes are never reused. Ranges: `E00xx` lexer, `E01xx` parser.
- **Give a fix when there is one.** Use `Diagnostic::with_suggestion` with
  the exact replacement text, so agents can apply it from `kiln check --json`.
- **Syntax changes update three places:** the parser, a test in
  `crates/kiln_syntax/tests/parser.rs`, and `docs/syntax.md`.
- Keep error messages short, specific, and in plain words: say what was
  expected and what was found.

## Vocabulary

Source files are `.kiln`. Templates and components are glazes (`.glaze`),
packages are wares, interactive WebAssembly components are embers, and
`kiln fire` makes a release build.
