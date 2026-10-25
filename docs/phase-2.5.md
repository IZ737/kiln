# Phase 2.5 — VS Code developer experience

**Status:** Done (0.5.0). Phase 2 (HTTP) is complete; this milestone sits before
phase 3 (glazes and server-rendered frontend development).

**Goal:** A developer can install the Kiln VS Code extension and comfortably
write, navigate, check, build and run Kiln programs without manually wiring
up an LSP client. Language intelligence must come from the existing Rust
compiler/LSP, not from a second parser or type checker in the extension.

## Scope

### VS Code extension

- Register `.kiln` and `.glaze` languages, with syntax highlighting for
  Kiln constructs and embedded Kiln expressions in glazes.
- Use Kiln's existing SVG brand assets for appropriate file icons; add
  recognizable icons/associations for source and glaze files, and use
  VS Code file-icon theme support rather than hard-coded editor UI icons.
- Launch and manage `kiln lsp` using the installed Kiln binary. Let users
  configure its path, and give actionable guidance if it cannot be found.
  The extension must not reimplement the checker in TypeScript.
- Expose convenient Command Palette actions for `kiln check`, `kiln run`
  and `kiln build` for the current program, with errors and output visible
  through VS Code. Reuse existing CLI behavior.
- Document local development, packaging and installing the extension.

### Enhanced LSP (usable by all LSP editors)

- Preserve existing diagnostics and suggested quick fixes, including
  unsaved-buffer checking.
- Add context-aware completion for visible declarations, variables,
  functions, types and members, using resolved symbol/type information.
- Add hover information for inferred types, declarations and signatures.
- Add go-to-definition and document/workspace symbol navigation for Kiln
  declarations and local references where resolution is available.
- Handle multiple source files in a shared program directory and edits
  without needing a full compiler rebuild. Prefer meaningful empty
  responses over crashes for incomplete source and unresolved symbols.
- Maintain LSP protocol correctness for positions/ranges, especially
  UTF-16 coordinates, unsaved documents and incremental editor use.

## Implementation plan

1. **Editor foundation (done).** Add a VS Code extension project, Kiln/Glaze
   language registrations, basic syntax grammars, brand-based file icons
   and a local extension launch configuration. Smoke-test associations
   on both light and dark VS Code themes.
2. **Compiler-backed language intelligence (done).** Extend `kiln lsp` to
   surface completions, hovers, definitions and symbols from shared
   compiler name/type resolution. Add focused protocol tests for
   same-file, cross-file and incomplete/unsaved Kiln programs. Keep
   diagnostics and quick fixes working unchanged.
3. **Extension integration (done).** Connect the extension to `kiln lsp`, add
   binary path configuration and Run / Build / Check commands. Surface
   missing compiler and command failures clearly; don't run arbitrary
   project-defined scripts.
4. **Polish and validation (done).** Document installation, settings, supported
   commands and limitations. Add extension packaging checks and a VS Code
   integration smoke test; run relevant LSP tests on Linux and Windows.
   Ensure other LSP editors continue to work.

## Done when

A developer can install the Kiln VS Code extension, open a project containing
`.kiln` and `.glaze` files, see appropriate icons and highlighting, get
live compiler diagnostics and quick fixes, use completion, hover, go to
definition and symbol navigation in Kiln code, and check/build/run the
project from VS Code. The Rust LSP continues to provide its intelligence
to other editors, with automated regression coverage.

## Deliberately left for later

- **No debugger in phase 2.5:** no DAP adapter, breakpoints, stepping,
  variable inspection or runtime debugging UI. Debugger design belongs in
  a separate later milestone.
- Full find-references and rename refactoring, richer Glaze component/prop
  semantics and cross-language navigation, formatter, hot reload and
  testing workflow integration remain in phase 5.
- Wares/package tooling, MCP tooling and the coding-agent guide also
  remain in phase 5.
- Phase 3 still owns implementing and serving Glazes as frontend
  components; phase 2.5 only improves editing support for their syntax.

## Design rule

The VS Code extension is a thin client over `kiln lsp` and the Kiln CLI.
Any feature that benefits other editors should live in the language server,
not be implemented only for VS Code.
