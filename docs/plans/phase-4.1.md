# Phase 4.1 — Explicit Ember components (`.ember`)

**Status:** Complete; included in v0.9.0. The compiler, LSP and real-browser regression suites pass. This is the implementation contract for the corrective milestone after Phase 4 and before Phase 5. Phase 4 originally implemented Embers as `.glaze` files that implicitly became interactive when they contained `{% state %}` or `on:` handlers; Phase 4.1 replaces that source contract with explicit `.ember` files. The exit gate below is met.

**Goal:** Make the server/client component boundary visible and enforceable at the file level: `.glaze` is a server-rendered template/component; `.ember` is a browser-capable component that may also render its initial HTML on the server. Preserve Phase 4's working WasmGC, hydration, state, typed RPC and JS/npm interoperability. **Do not rewrite or replace the runtime just to change the extension.**

**Done when:** A project uses `.glaze` for SSR pages/layouts and `.ember` for reactive islands; the compiler, CLI, LSP, VS Code extension, conformance suite and browser tests all enforce and demonstrate the distinction. Static-only routes still ship no Ember runtime, and existing interactive examples work after an intentional migration.

**Authority:** This document supersedes the *implicit Glaze-to-Ember classification* in [Phase 4](phase-4.md), notably its sections 2 and 4. All other Phase 4 contracts (WasmGC target, selective hydration, RPC, browser/npm interop, security and lifecycle) remain in force. [Phase 5](phase-5.md) may depend on this source model without taking ownership of it; [Phase 6](phase-6.md) owns general multi-folder modules and Wares.

## 1. Decisions (not open design questions)

1. **Extension declares component kind.** Only a `.ember` file defines an Ember. State or events **never implicitly promote** a `.glaze` file. No `client` keyword, special wrapper, or separate template language is required.
2. **Shared declarative markup.** `.ember` uses the existing Glaze template grammar: typed `{% props %}`, `{% state %}`, `{% let %}`, control flow, HTML, components, slots, scoped styles, `ui`, and `on:` handlers where valid. `{% state %}` remains the explicit notation for mutable reactive values. The new suffix does **not** make ordinary `let` bindings mutable or reactive.
3. **No state/handlers in `.glaze`.** A `.glaze` containing `{% state %}` or `on:` is a compile-time error with a source span and a suggestion to move/rename it to `.ember`. A `.glaze` remains a server-side component even if it uses a nested `.ember`.
4. **`.ember` does not mean client-only.** By default an Ember produces SSR HTML and hydrates using the existing Phase 4 island mechanism. `client:only` keeps its existing explicit meaning. File kind does not imply loss of SSR, separate SPA routing, or access to server secrets in the browser.
5. **Kind is stable even without state.** A `.ember` with no state/events is still an Ember source file, not silently downgraded to a Glaze. Do not choose component kind from template contents. Eliminate unnecessary assets only when this preserves all observable Ember semantics.
6. **No permanent dual mode.** Kiln is pre-1.0. Migrate in-repository examples and tests rather than indefinitely accepting reactive `.glaze` files; keep diagnostics actionable instead of silently altering behavior.
7. **Preserve isolation.** Browser-reachable code cannot use filesystem, server-only `web`/`sql`/`auth` capabilities or secrets. SSR evaluation of an Ember must not turn browser-only handler code into server-executed code. Maintain Phase 4 serialization and security restrictions.

## 2. Developer-facing examples

A server page and its layout remain ordinary Glazes:

~~~glaze
{# page.glaze — server-rendered only #}
{% layout "layout.glaze" title: "Counter" %}
<h1>Count</h1>
<Counter label="Clicks" client:visible />
~~~

An interactive component is a different source file:

~~~glaze
{# counter.ember — SSR initially, then WasmGC hydration #}
{% props label: String %}
{% state count = 0 %}
<button on:click={ count += 1 }>{label}: {count}</button>
~~~

A plain `.glaze` containing that `state` directive or `on:click` must fail with an error pointing at the directive. Likewise, adding `client:visible` to a *plain Glaze component* must not quietly turn it into an Ember.

These examples reuse current Phase 4 handler/state syntax; this milestone does **not** introduce a new reactivity model.

## 3. Component lookup, compilation and build contract

- **Discovery:** Include `.ember` alongside `.glaze` in the existing same-folder component discovery for `kiln check`, `run` and `build`. The current case-to-filename convention for tags (for example `<Counter>` to `counter.ember`) remains unchanged.
- **Name conflicts:** If both `counter.glaze` and `counter.ember` would resolve to `<Counter>` in the same scope, give a deterministic ambiguity/error naming both files; never let directory iteration order choose one. Do not invent cross-folder namespace syntax in this milestone.
- **Static vs interactive resolution:** A `.glaze` can contain a named `.ember`; a `.ember` may contain nested Embers and ordinary Glaze-provided content subject to the existing slot/rendering rules. Preserve state isolation, ownership, nesting and key identity in Phase 4.
- **SSR and islands:** A referenced `.ember` uses the same SSR HTML, serialized public props/state, stable island identity, `client:load` (default), `client:idle`, `client:visible`, `client:only` and WasmGC bootstrap as the Phase 4 implementation. SSR-only output from a `.glaze` page is not itself an island.
- **Asset reachability:** Emit browser Wasm/JS only for reachable/used Ember islands, not merely because `.ember` files exist in a project. Glaze-only pages must continue serving without client runtime assets.
- **Guards:** Restrict hydration hints on component invocations to Ember targets; reject `client:*` on ordinary Glaze targets with a helpful error. Never infer client capabilities from `.glaze` source markup or from a filename that is not `.ember`.
- **Layouts:** Keep layouts server-side (`.glaze`). Retain the existing error for assigning a page layout directly inside an Ember; Ember components can be placed inside Glaze pages/layouts.
- **Shared types and RPC:** Keep current `.kiln` types and `server fn` RPC declarations. No duplicated DTOs, extra client imports, hand-written JS bridges, or separate browser frameworks are introduced.
- **Source metadata:** Preserve the actual `.ember` source filename/line spans in parsing, checking, error traces, module discovery and emitted asset diagnostics. Avoid internally fabricating a `.glaze` path that confuses users.
- **Cross-target support:** Compiler checking and native SSR must work on supported Linux/Windows backends; client compilation remains WasmGC. Do not regress the existing C/native/wasm conformance paths.

## 4. Diagnostics and developer tools

- Emit precise, actionable diagnostics for: `{% state %}` or `on:` inside `.glaze`; `client:*` targeting a Glaze; duplicate Glaze/Ember component names; invalid browser-only access across the server boundary; missing Ember component files; and unsupported Ember layouts.
- Prefer a fix-it suggestion for renaming an interactive file to `.ember`; do **not** silently rename files or pick an extension based on syntax. Allocate final diagnostic codes consistently with the current catalog; do not reuse codes with different meanings.
- Update the VS Code extension's file associations, icons, syntax highlighting, and editor commands so `.ember` is first-class. Ensure Kiln LSP check, completion, hover, go-to-definition, symbols, diagnostics and relevant source spans work in `.ember` and across `.glaze` -> `.ember` references.
- Preserve existing `.glaze` highlighting, component navigation and formatter-compatible parsing where currently supported. Phase 6 can later extend these capabilities across multi-folder modules.

## 5. Migration plan

1. Inventory repository files using `{% state %}`, `on:` or implicit interactive component detection (including conformance fixtures, examples and the Kiln docs site's interactive components).
2. Rename *only actually interactive components* from `.glaze` to `.ember`. Keep static pages, layouts and reusable SSR components as `.glaze`. Update any path-based includes/references and tests.
3. Change parser/discovery/checker/compiler classification from "state/handler makes a Glaze an Ember" to "`.ember` is an Ember source kind"; remove the implicit promotion path and add errors for disallowed directives in Glazes.
4. Reuse existing Phase 4 rendering and browser runtime code. Modify extension assumptions at the boundary rather than cloning Glaze and Ember compilers.
5. Update docs reference, Getting Started examples, docs site's generated samples, vocabulary and CHANGELOG to reflect the new *implemented* syntax only when the code lands. Label existing Phase 4 behavior as historical if still referenced.
6. Run baseline/regression tests, verify browser functionality manually, then release/tag. No database, ORM or broader project-system work is required to complete this change.

## 6. Acceptance test matrix

| Area | Positive case | Negative / regression case |
| --- | --- | --- |
| Parsing and checking | `counter.ember` parses, checks and retains source spans | `{% state %}` and `on:click` in `counter.glaze` fail with helpful fixes |
| Resolution | `<Counter>` in `page.glaze` resolves `counter.ember` | Same-name `counter.glaze` + `counter.ember` fails deterministically |
| Hydration | SSR Ember hydrates for load/idle/visible; only skips SSR | `client:*` on a Glaze-only component is rejected |
| Reactivity | Counter, form, list, nested islands, focus and keyed items work | No state leakage or duplicate listeners after dispose/navigation |
| RPC and interop | Existing typed `server fn`, `browser` and npm paths still work | Browser cannot directly access server capabilities or secrets |
| Static routes | Glaze-only docs/pages render with no Ember bootstrap | An unused `.ember` source does not force asset inclusion |
| Tooling | Extension and LSP recognize/complete/navigate `.ember` | Missing file / wrong kind errors point at correct file and line |
| Cross-target | Native server + browser tests; C/native/wasm conformance stays green | Generated assets and SSR serialization/security do not regress |

**Required end-to-end fixture:** One app has a Glaze layout/page, a static child Glaze, at least two independent `.ember` components with different hydration hints, a typed RPC call, and a Glaze-only route. Run real-browser E2E checks for state, focus and RPC while asserting **zero Ember assets** on the Glaze-only route. The docs site's copy control is a useful real-world migration fixture; it is not a substitute for the full test matrix.

## 7. Boundaries and handoff

**In scope:** source classification, discovery/resolution, diagnostics, tests, first-class `.ember` editor/LSP support, repository migration and documentation updates.

**Out of scope:** replacing WasmGC/reactivity/RPC, introducing a new client state syntax, changing option B browser/npm interop, Phase 5 SQL/ORM/auth/admin, Phase 6 multi-folder projects/Wares/test-runner, JS fallback targets, or broad performance optimization.

**Implementation order:** (1) fail-first language + resolver tests; (2) source-kind/checker/compiler changes; (3) migration; (4) LSP/extension; (5) browser/security regression suite; (6) docs/release notes.

**Exit gate:** A fresh developer can tell server-only and browser-capable source files apart by their suffix; compilation strictly enforces that intent; pre-existing Phase 4 capabilities still work; and the full CI/real-browser suite passes. **Only then mark Phase 4.1 complete and tag the corrected release.**
