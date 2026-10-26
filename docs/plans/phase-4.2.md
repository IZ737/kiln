# Phase 4.2 — Multi-folder projects and local modules

**Status:** Planned, after [Phase 4.1](phase-4.1.md) and before [Phase 5](phase-5.md). This is an **implementation contract**, not a claim that cross-folder imports or a project manifest are already supported.

**Goal:** Make Kiln practical for ordinary multi-folder applications *before* adding a database/ORM/admin framework. A developer can put routes, domain logic, types, server Glazes and browser Embers in different folders, import local modules, check and run the application from one root, and get useful cross-module diagnostics/editor navigation—with no Wares, registry, lockfile or Node project.

**Done when:** A manifest-free multi-folder Kiln web app builds and runs on Linux and Windows; an optional minimal `kiln.toml` can select its entrypoint; visibility and import cycles are enforced; Glazes/Embers and static assets resolve correctly across module boundaries; native/C builds, LSP and conformance regression cases pass. Phase 5 uses this foundation for its database-backed dogfood app.

**Ownership:** This milestone moves the **Phase 6.1 project foundation** out of [the Phase 6 plan](phase-6.md). It supersedes earlier documents assigning multi-folder local modules to Phase 6 (including historical Phase 4/4.1 boundary notes). Phase 6 retains Wares/external dependencies, `kiln.lock`, registry, integrated public `kiln test`, formatter/dev watcher, advanced refactoring and MCP. Phase 4.1's explicit `.glaze`/`.ember` source distinction is a prerequisite; do **not** modify the Ember reactivity/runtime contract while implementing folder modules.

## 1. Required design decisions

1. **Keep simple programs simple.** `kiln run hello.kiln` and `kiln run folder` continue to work with no project file, dependency installation or module declarations.
2. **One folder, one module namespace.** Existing multiple `.kiln` files in a folder share declarations and types, while imports remain lexical/per-source-file. A child directory is a separate local module, **not** automatically merged into its parent.
3. **Folders are imported explicitly.** `import posts` names a local folder module `posts/`; `import posts.search` names a nested module `posts/search/`. Use the same existing `import` form as standard modules. Names are resolved from the *project root* and stable in any importing source file, not dependent on the file's working directory.
4. **Public means exported.** Declarations with `pub` are visible to importers; module-private declarations remain usable within that folder only. Support `pub fn`, `pub struct`, `pub enum` and `pub const`, and compatible existing public forms. Reject public signatures leaking inaccessible types.
5. **Stable type identity.** Nominal types from separate module paths are distinct, even if names/fields happen to match. Qualified lookup cannot accidentally select a homonymous type or local declaration.
6. **No hidden imports or initialization.** Importing a module makes its public API available but does not implicitly register routes, run database migrations, start jobs or execute top-level side effects. Caller explicitly invokes setup functions.
7. **Optional project manifest only.** A tiny `kiln.toml` may identify a root and entrypoint, but is never required for local imports. Existing manifest-free programs are not forced to add one. Phase 6 extends this *same* file for external Wares/dependency configuration.
8. **No silent ambiguity.** Duplicate public names, colliding module paths, unresolved imports, cyclic imports, bad root/entry and inconsistent path capitalization are compile-time diagnostics. The compiler never chooses based on filesystem enumeration order.
9. **One dependency graph.** Module reachability, checking, codegen, Glaze/Ember assets and editor navigation share deterministic import and source-location information. This need not introduce a new runtime module loader.

## 2. Target developer experience

~~~text
fieldnotes/
  main.kiln
  layout.glaze
  posts/
    models.kiln
    routes.kiln
    index.glaze
    editor.ember
    post.css
  accounts/
    users.kiln
    routes.kiln
    login.glaze
~~~

`main.kiln`:

~~~kiln
import web, posts, accounts

fn main() ! {
    let app = web.app()
    posts.routes(app)
    accounts.routes(app)
    try app.listen(":8080")
}
~~~

`posts/routes.kiln` can import `web`, declare `pub fn routes` and render a local `index.glaze`. All `.kiln` files inside `posts/` share the `posts` namespace; no `import posts.models` is needed inside that same folder merely to use its locally declared types. A different module importing `posts` sees only its public exports.

Nested module imports use `import posts.search` for `posts/search/`; importing `posts` does not automatically expose its nested modules.

An **optional** root `kiln.toml` may be used when a project has multiple candidate entrypoints:

~~~toml
[package]
name = "fieldnotes"
kind = "app"
entry = "main.kiln"
~~~

These examples are **target Phase 4.2 syntax** until conformance tests prove support. A `[wares]` section, `kiln.lock`, registry source, `kiln add` and multi-package workspaces are expressly *not* required or implemented by this milestone.

## 3. Project-root and module resolution

- Explicit CLI file/folder target takes priority. For commands issued within a directory tree, the nearest **applicable** ancestor `kiln.toml` may establish project root and entrypoint; otherwise the explicitly selected folder is the root. Never walk past an explicitly selected root or unexpectedly capture an unrelated parent manifest.
- `kiln run .` uses the manifest entry if present; otherwise a documented unambiguous conventional entry such as root `main.kiln`. If multiple plausible `main` entrypoints exist, diagnose and request explicit selection; do not choose arbitrarily.
- Resolve local modules under the chosen root by folder segments, not arbitrary dynamic filesystem paths. A module exists when its target folder contains suitable Kiln source. A directory without source is not an implicitly executable module.
- Resolve `import web` and other standard-library imports to their established modules, not a colliding local folder. A colliding reserved name must produce an actionable diagnostic or require a deliberate explicit disambiguation scheme; never silently shadow the standard library.
- Normalize `.`/`..`/separators for canonical comparisons while prohibiting import escape outside the selected root. Reject symlink loops, traversal and duplicate case-folded paths; behavior must not diverge unpredictably on Linux vs Windows.
- Record the complete import chain in cycle diagnostics; reject cycles for the initial deliverable. Do not introduce a half-specified type-only cycle mechanism or hidden module initialization ordering.
- Import environments are per source file even when a folder shares a declaration namespace; exports are resolved from their defining module regardless of which sibling source file imported another dependency.
- Determine graph ordering deterministically and share source file hashes/IDs across check, native/C lowering, browser reachability and LSP. Incremental caching is desirable, but correct one-graph compilation is the gate.

## 4. Visibility, names and diagnostics

- Map `pub` on exported declarations to explicit module-qualified symbols; unmarked declarations are private to their module. Do not permit unqualified access to non-exported declarations from other folders.
- Make exported struct fields/constructors and enum variants follow existing access rules consistently; an exported function whose argument or result uses a private nominal type is an error.
- Preserve nominal origin, type inference, generics/specializations, error types, serialization and `server fn` boundaries when types cross modules.
- Actionable errors for unknown module, unknown exported member, private access, ambiguous name, duplicate export, import cycle, invalid nested path, invalid manifest/root, and case/path collision must include source spans and suggestions.
- Diagnostic output and editor source navigation must retain the actual paths of `.kiln`, `.glaze` and `.ember` files; avoid generated aliases masking origin files.

## 5. Glazes, Embers and assets in folder modules

- Inside a module, same-folder component lookup continues to work as before. A route defined inside `posts/` can render its own `index.glaze` and its local `editor.ember` without moving components into the root.
- Resolve template/component references with a stable **owning module context** (and project-root-qualified file paths where already supported), not the caller's current working directory. A bare same-name component in a different module must not hijack a lookup.
- If another module needs to reference a component directly, require an explicit, checked qualified path/module reference; prefer reusing established render/component syntax and do not add ambiguous global component-name search. If the parser needs a new qualified component spelling, document it and cover it by conformance before claiming support.
- `.glaze` remains SSR-only; `.ember` is browser-capable with optional SSR and hydration, per Phase 4.1. Do not reintroduce implicit state-triggered Ember classification.
- CSS Modules, native `ui`, Finishes, layouts, slots, generated styles, static assets and WasmGC island assets must use module-aware paths without collisions. Ensure only reachable islands load browser assets; a Glaze-only route stays script-free.
- `server fn` calls, generated typed RPC, serialization, browser/JS/npm interop and forbidden browser imports keep existing trust boundaries across modules; no client bundle may include private server logic or secrets.

## 6. Minimal `kiln.toml` (Phase 4.2)

- Support the minimal optional `[package]` fields above and a clear documented schema/version policy. Parse proper TOML with location-aware invalid-key, invalid-type and missing-entry diagnostics.
- Explicit CLI options override manifest defaults. Use a valid canonical path *within the root* for the entry; no relative traversal or entry pointing to an unrelated project.
- No plaintext secrets, custom executable setup/install scripts, compulsory per-folder manifests or dependency-specific lockfile behavior.
- A single `.kiln` script or simple folder works with *zero* manifest; a multi-folder app also works manifest-free where entry/root is unambiguous.
- **Phase 6** may later add `[wares]`, package versions, publishing metadata, build/test configuration and `kiln.lock` without changing basic root/module semantics. If currently unsupported future sections are present, report them clearly rather than pretending they were resolved.

## 7. CLI, LSP and implementation steps

1. **Fail-first conformance:** Build fixtures for multi-folder positive cases, private/export errors, imports, cycles, case collisions, nested folders and unchanged one-file/one-folder behavior.
2. **Compilation graph:** Introduce folder module identities and deterministic traversal/resolution in syntax/checker/IR/compiler driver. Verify native + C semantics and unchanged standard-library imports.
3. **Project entry:** Implement CLI root/entry resolution and optional minimal TOML support, with well-defined precedences and diagnostics. Existing `kiln check`/`run`/`build` should accept local module graphs.
4. **Web integration:** Exercise module-local Glazes, Embers, styles, static assets, nested components, RPC and browser reachability in a real multi-folder HTTP application. Retain security and SSR-only behavior.
5. **Editor integration:** LSP/VS Code identify the project root; provide cross-module completion, hover, definition, symbols and actionable diagnostics for local modules. Broader rename/refactor/test UI remains Phase 6.
6. **Docs/verification:** Update syntax and Glaze/Ember reference, design notes, VS Code guides, conformance, native Linux/Windows cross-build/smoke and browser E2E. Update current-feature claims only as code lands.

## 8. Acceptance matrix

| Dimension | Must work | Must fail safely |
| --- | --- | --- |
| Compatibility | `kiln run hello.kiln`; old same-folder projects without TOML | Existing lookup/order silently changes |
| Module discovery | `import posts` and nested `import posts.search` from multiple folders | Missing folder, path escape, cycles, case/alias conflicts |
| Visibility/type identity | Public functions/types used by other modules | Private member access, leaked private types, accidental nominal equivalence |
| Entrypoint | Root app with or without optional `kiln.toml` | Ambiguous `main`, malformed TOML, entry outside root |
| Web/source assets | Module-local Glaze, Ember, CSS, static assets and RPC | Collision, broken paths, accidental private/server code in Wasm |
| Builds | Linux/Windows native builds, C-backend regression and Wasm browser checks | Differences in import/case semantics by OS |
| Editor | Import completion, cross-folder definition/symbols, source-linked diagnostics | Stale or wrong source paths |

**Required dogfood:** Implement a small `fieldnotes/` app with separate `posts/` and `accounts/` folder modules; two route registrations; an SSR Glaze page; an Ember form that calls a typed `server fn`; module-local styling; and at least one route loading **no Ember JS/Wasm**. Include a manifest-free run and optional-manifest entry test. Data may remain in memory—Phase 5 adds durable persistence.

## 9. Definition of done and exclusions

- [ ] One-file/one-folder programs regressions pass unchanged.
- [ ] Multi-folder modules compile, check, run and import nested modules with precise, deterministic errors and `pub` visibility.
- [ ] Optional `kiln.toml` can select an entrypoint but is unnecessary for local imports or simple scripts.
- [ ] Glazes, Embers, scoped CSS/assets, browser target, server RPC and component source maps work across module boundaries.
- [ ] Project-aware LSP, cross-folder definitions/completion/diagnostics and CLI paths work.
- [ ] Multi-folder HTTP + SSR/Ember E2E dogfood and Linux/Windows native/C/wasm conformance pass.
- [ ] Docs and roadmap correctly describe Phase 4.2 vs Phase 5/6 ownership.

**Explicit exclusions:** SQL/ORM/auth/admin (Phase 5); Wares/registry/lockfile/cache, fully developed manifest dependency schema, official component Ware, public `kiln test`, formatter/live rebuild, advanced refactoring and compiler MCP (Phase 6); deploy/SSG (Phase 7).

**Exit gate:** A developer can organize and run a small full-stack Kiln app into sensible folders *before* adding a database, without configuring a dependency manager. Phase 5 must build on this rather than inventing a competing module/project system.
