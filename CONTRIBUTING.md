# Contributing to Kiln

Thanks for your interest in Kiln! Bug reports, reproducible examples, documentation fixes, tests, and code contributions are all welcome. You do not need to be a compiler expert to help.

Kiln is an **experimental, pre-1.0** language under active development. Its interfaces and internals can change, and the project is currently **maintainer-led**. Please read [GOVERNANCE.md](GOVERNANCE.md) for how technical and roadmap decisions are made.

## Before you start

- **Small fixes** (typos, documentation, tests, clearly scoped bugs): you can usually open a pull request directly.
- **New syntax, public APIs, runtime behaviour, breaking changes, large refactors, or roadmap additions**: [open an issue](https://github.com/IZ737/kiln/issues/new/choose) first. Describe the problem, proposed behaviour, alternatives, and compatibility impact; wait for scope agreement before investing in an implementation.
- **Something already being worked on?** Search [issues](https://github.com/IZ737/kiln/issues) and [pull requests](https://github.com/IZ737/kiln/pulls) to avoid duplicated effort.
- **A security concern?** Follow [SECURITY.md](SECURITY.md), not a public bug report.
- **A usage question?** Read [SUPPORT.md](SUPPORT.md).

Feature ideas are welcome, but an item in [the vision](docs/vision.md) or an implementation plan is not necessarily accepted for the current release. Consult the [roadmap](README.md#roadmap), [language design](docs/design.md), and [current reference](docs/README.md). The source of truth for implemented behaviour is the compiler and its tests.

## Build from source

The repository is a Rust workspace. It pins no fixed Rust version in `rust-toolchain.toml`, but currently declares a minimum Rust version of **1.97** in `Cargo.toml`. Use a recent stable toolchain with `rustfmt`, `clippy`, and the Linux/Windows cross-compilation targets listed in `rust-toolchain.toml`.

On Debian/Ubuntu, the native toolchain/runtime build also needs:

```sh
sudo apt-get install musl-tools gcc-mingw-w64-x86-64 libgc-dev
```

You may also need a C compiler. Other distributions and Windows have corresponding toolchains; see [README.md](README.md#try-it) and [AGENTS.md](AGENTS.md#commands). A released `kiln` executable does **not** require this contributor build setup.

```sh
git clone https://github.com/IZ737/kiln.git
cd kiln
cargo build
cargo run -p kiln_cli -- run examples/hello
cargo run -p kiln_cli -- check examples
```

For work on the VS Code extension, install **Node.js 22**, then:

```sh
cd editors/vscode
npm ci
npm test
```

Some integration tests use VS Code, an X display, or a browser. See [the CI workflow](.github/workflows/ci.yml) for the full Linux setup (including Playwright/Chromium), rather than assuming `cargo test` alone exercises browser hydration.

## Repository orientation

| Location | What you'll find |
| --- | --- |
| `crates/kiln_syntax` | Lexer, parser, syntax trees |
| `crates/kiln_check` | Type checker and stdlib signatures |
| `crates/kiln_ir` | Typed intermediate representation |
| `crates/kiln_native`, `crates/kiln_codegen` | Native and C compilation |
| `crates/kiln_wasm` | WasmGC/Ember output and browser bridge |
| `crates/kiln_rt` | Runtime and HTTP support |
| `crates/kiln_cli` | CLI, LSP, and end-to-end tests |
| `conformance/` | Input programs plus checked expected output |
| `examples/` | Example Kiln apps |
| `site/` and `docs/` | Docs website and reference content |
| `editors/vscode/` | VS Code extension |

For detailed architecture, test commands and coding conventions, see [AGENTS.md](AGENTS.md). Despite its name, it is also useful for human contributors.

## Make and test a change

Create a topic branch in your fork (or your own writable branch). Keep a PR focused; unrelated changes should be separate PRs. Add or adjust tests for any behaviour change.

Before opening a code PR, run the applicable checks:

```sh
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo run -p kiln_cli -- check examples
```

The repository's CI runs additional Linux and Windows builds, conformance checks, dependency checks, VS Code tests, and real-browser tests. Some need Docker, Wine, Node.js, or Playwright locally; a contributor can explain in the PR when a platform-specific check could not be run.

### Compiler and language changes

- **Parser or syntax:** update parser tests and the relevant pages under `docs/reference/`. Update VS Code grammars/tests for changed tokens or keywords.
- **Type rules:** add positive and negative cases, with clear error codes and diagnostics. Keep errors actionable; do not silently accept something the backend cannot compile.
- **Code generation/runtime:** cover the native backend and C backend where applicable, and consider Linux/Windows portability.
- **Glaze or Ember behaviour:** check server rendering, browser hydration, and RPC boundaries when relevant; a server-only Glaze must not silently become a client component.
- **New language behaviour:** add a case in `conformance/` with expected `main.out`, `main.err`, or HTTP fixtures as appropriate. See [conformance/README.md](conformance/README.md). Review fixture changes carefully; `KILN_BLESS=1` is not a substitute for checking expected output.
- **Docs website:** when editing `docs/`, regenerate the site's content with `cargo run -p kiln_cli -- run site/tools/content` and include any generated `site/` changes required by its tests.

The planned public `kiln test` command is not the contributor test harness yet; use `cargo test` and the existing conformance suite.

### Documentation and examples

Keep examples runnable against the code in the PR. Distinguish **implemented behaviour** from **planned features** and **long-term experiments**; do not present plan-only syntax as released. Favour practical, minimal examples and plain-language error explanations.

## Open a pull request

1. Link the relevant issue if there is one; for substantial changes, get scope agreement before implementation.
2. Explain **what changed and why**, the user-visible behaviour, any compatibility/security impact, and relevant tests.
3. Include documentation, fixtures, or screenshots where they help reviewers verify behaviour.
4. Complete the [pull request template](.github/PULL_REQUEST_TEMPLATE.md) with the checks you ran (or why you could not run them).
5. Be willing to iterate after feedback. CI passing is necessary but does not automatically mean a design change will be merged.

AI-assisted contributions are welcome. Please understand and review the code you submit, verify its tests and licences, and do not submit large unreviewed generated changes. The person submitting the PR is responsible for it.

Contributions are made under the repository's [MIT licence](LICENSE); no separate contributor licence agreement is currently required.

Maintainers may decline or defer changes that conflict with Kiln's direction, add disproportionate maintenance cost, lack adequate tests, or arrive before their roadmap phase. Review and merge timing are not guaranteed.

## Community standards

Be respectful of other contributors and their time. The [Code of Conduct](CODE_OF_CONDUCT.md) applies to issues, PRs, and other project spaces. Please keep security disclosures out of public issues.
