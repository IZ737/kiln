# Phase 6 — Projects, Wares, testing, tooling and agent integration

**Status:** Planned specification. This is an implementation contract and reviewable proposal for API details; **none of the Phase 6 commands or syntax below is claimed to be implemented** unless later marked in the reference documentation.

**Goal:** Take Kiln from a capable full-stack language to a coherent **day-to-day development and package ecosystem**: multi-folder application structure, optional project metadata, reproducible third-party Wares in a safe shared cache, a first useful official UI Ware, one integrated unit/integration/real-browser test command, polished editor/formatter/live-reload workflows, and an agent-friendly tooling interface.

**End-to-end gate:** A developer can create a multi-folder Kiln app, import local modules, add a versioned UI Ware from a **locally hosted registry**, commit a reproducible lockfile, build without a network connection after cache priming, run Kiln-authored unit/database/real-browser tests, edit safely with formatter/LSP/live reload, and use a structured local MCP tool interface—**without writing Node scripts, per-project dependency installation directories or custom glue**. Its native release output remains self-contained.

This implements previously agreed direction in the [design overview](../design.md#programs-and-folders), [vision](../vision.md#7-wares-templates-and-a-real-ecosystem) and [integrated testing plan](testing.md). Phase 4's [data/auth/admin plan](phase-4.md) makes the full-stack acceptance tests possible. Phase 5 already delivers Embers, WasmGC, hydration and typed browser/server RPC; Phase 6 builds the ecosystem around them.

> **Design status:** The broad capabilities, separation from other phases, per-user global Ware storage, manifest-free scripts, and built-in real-browser testing are established. The exact file layouts, names, command flags, registry endpoints, security policy format and test API in this document are **proposals to validate during Phase 6**, not promises of current Kiln syntax.

## 1. Scope and governing decisions

1. **One-folder programs stay simple.** All `.kiln` files in one folder share a namespace. A single file or folder runs with no `kiln.toml`, lockfile or registry.
2. **Subfolders are local modules.** `pub` defines their external interface. Local modules must compile, type-check and work offline independently of Wares.
3. **Manifest is declarative and optional.** `kiln.toml` describes project identity, optional entrypoint, build/test defaults and dependencies. `kiln-deploy.toml` stays separate (Phase 7).
4. **One lockfile, deterministic dependency graph.** `kiln.lock` pins exact immutable versions, source identities, hashes and transitive edges. Build/release can require locked resolution.
5. **Global, per-user Ware cache, not `node_modules`.** Immutable source is stored once by content hash and shared across projects; versions and dependency choices belong to each project.
6. **Wares have no install-time execution hooks.** Fetching, resolving and checking a dependency does not run arbitrary package-provided shell commands.
7. **Registry trust is layered, not magical.** Publisher verification, review status and hashes communicate different claims. A trusted-looking label is never a promise of sandboxed or vulnerability-free code.
8. **Wares should feel like local modules.** One namespace/import model, no separate runtime package loader, no compulsory publish step for ordinary project structuring.
9. **First official Ware is useful and inspectable.** A Kiln-maintained accessible UI component collection uses Glazes, CSS, typed Ui, Finishes, themes and Phase 4 forms; source-first editing/copy is supported.
10. **One test workflow from function to browser.** `kiln test` owns Kiln-written unit, integration and real-browser E2E tests and uses maintained browser tooling internally—not a separate developer-visible JavaScript project.
11. **Tooling is incremental and ordinary.** A deterministic formatter, cross-module LSP/refactoring, watcher/rebuilder and editor tasks share the existing compiler. Do not invent a new language service.
12. **MCP reuses CLI/compiler semantics.** Provide useful local agent tools with explicit side-effect boundaries; do not expose unguarded shell/credential/network access to arbitrary clients.
13. **No premature compatibility freeze.** Kiln is pre-1.0; capture the correct developer contracts now, then evolve internals behind conformance and end-to-end tests.
14. **No forced cloud service.** Local modules, path/Git Wares, and a local or static registry must remain useful even if the eventual public registry is unavailable.

## 2. Organize Phase 6 as five deliverable subphases

| Subphase | Main deliverable | Independent gate |
| --- | --- | --- |
| **6.1 — Project foundation** | Local folder modules, visibility, optional `kiln.toml` and cross-module compiler/LSP | A multi-folder program builds/runs; one-file programs remain manifest-free |
| **6.2 — Ware ecosystem** | Resolver, `kiln.lock`, global cache, local/Git/registry sources, publish/verify tooling, first official accessible UI Ware | Two apps resolve reproducibly, offline, from a trusted local registry with shared cached contents |
| **6.3 — Integrated testing** | `kiln test` unit/integration/real-browser, fixtures, reporting and CI | Real browser submits a form to an isolated Kiln app and validates results and focus |
| **6.4 — Developer workflow** | Formatter, live rebuild/restart, deeper LSP and editor integration | Safe format/watch/refactor cycle across Kiln, Glaze, CSS and imported Wares |
| **6.5 — Agent experience and dogfooding** | Compiler MCP, machine-readable tool contracts, docs/agent guide, full project example | A human or agent can inspect/check/test a full multi-folder app without hidden deployment or shell behaviour |

Treat subphase numbers as **planning subdivisions**, not new language versions; each should break into reviewable PRs. The public registry's large-scale hosting, social/discovery features and organizational billing are not Phase 6 blockers.

## 3. Developer walkthrough (future API)

~~~text
kiln run hello.kiln                  # no manifest
kiln new fieldnotes --template web   # optional starter; no scripts executed
cd fieldnotes
kiln add kiln/ui                     # update kiln.toml and kiln.lock
kiln check .
kiln test --unit
kiln test --integration
kiln test --browser
kiln fmt --check
kiln build --locked --offline
~~~

`kiln new` and `kiln add` are proposals; the exact starter scaffolding and package name are not decided. Browser provisioning may require a one-time external binary download, but never a hand-maintained Node test project.

Example larger app:

~~~text
fieldnotes/
  kiln.toml
  kiln.lock
  main.kiln
  posts/
    queries.kiln
    routes.kiln
    post.glaze
  accounts/
    auth.kiln
    login.glaze
  assets/
    styles.css
  tests/
    unit/
      test_posts.kiln
    integration/
      test_routes.kiln
    browser/
      test_login.kiln
~~~

Sample target import experience:

~~~kiln
// main.kiln
import web, posts, accounts

fn main() ! {
    let app = web.app()
    posts.routes(app)
    accounts.routes(app)
    try app.listen("127.0.0.1:8080")
}
~~~

Files inside `posts/` share their folder's namespace, and `import posts` references public exports. Nested modules may be addressed with `import posts.search` if the design needs deeper packages. **Final import spelling is a design choice to confirm in the first 6.1 PR**, but avoid duplicate syntaxes for the same operation.

## 4. Phase 6.1 — Local module semantics

### Directory model and root discovery

- Preserve the existing rule: all `.kiln` files directly in a folder form **one compilation module/namespace**, and existing Glaze lookup from that folder remains valid.
- For `kiln run .`, root resolution uses an explicit argument first, then the nearest applicable `kiln.toml` when present, otherwise the specified/source folder. Avoid accidentally choosing an unrelated parent manifest.
- A nested folder becomes a local module by being imported, not by requiring its own manifest or package registration.
- Resolve import paths relative to the project root, with a deterministic rule for same-folder vs root paths. Define how a folder's optional module entry and Glazes are discovered. No implicit filesystem traversal of unrelated folders.
- Normalize paths and reject ambiguous capitalization/collisions consistently across Windows and case-sensitive filesystems. Canonicalize only inside allowed roots; guard symlink loops and traversal.
- Standard-library module names are reserved or have an explicit unambiguous shadow/alias rule. Avoid a local `web/` accidentally changing `import web` semantics.
- Local module dependency cycles are diagnosed with the entire path; no infinite compiler recursion. Decide whether type-only cycles are representable, but do not make value initialization order implicit.
- The compiler builds a graph once, shares type information and supports incremental invalidation; do not recompile the entire world for every changed leaf without a reason.

### Visibility and type identity

- `pub fn`, `pub struct`, `pub enum`, `pub const` expose declarations; unmarked declarations remain private to their module. Field/method visibility must be specified consistently with Kiln's existing `pub` syntax and ordinary struct initialization.
- Nested public API types cannot leak a private type accidentally; diagnose inaccessible types in exported signatures.
- Cross-module type identity includes the defining module/package, not just the type's text name. Prevent unrelated `models.User` from silently matching `accounts.User`.
- Respect separate imports per source file (even when declarations in the same folder share a namespace). Determine the name-resolution environment for public declarations independently of whichever file imported a dependency.
- Distinguish inaccessible-private, missing-module and missing-export diagnostics, with suggestions and precise source locations.
- Make module initialization/order explicit if eventually introduced. For Phase 6, prefer top-level constants and ordinary functions over general package startup hooks.

### Components and assets across modules

- Specify how an app references a `.glaze` component exposed by another local module without breaking current same-folder component lookup.
- Generated CSS/Finishes/assets use the same dependency graph; namespace hashed/scoped assets so identical filenames don't collide.
- Components can use the public types they declare; ordinary Glaze scope/type checking continues to report errors mapped to their original files.
- A module import should not silently mount HTTP routes, database migrations or admin pages; caller explicitly registers them.

**6.1 module gate:** two local modules with their own Glazes/CSS compile on native and C backends; private access, cyclic imports, ambiguous names and missing imports produce precise errors; LSP finds cross-module definitions; the unchanged simple folder example still works.

## 5. Phase 6.1 — `kiln.toml` project contract

Indicative manifest (**proposed**):

~~~toml
[package]
name = "fieldnotes"
version = "0.1.0"
kind = "app"
entry = "main.kiln"

[wares]
"kiln/ui" = "0.1"
"example/markdown" = { git = "https://example.com/repo.git", rev = "0123456789abcdef" }
"local/widgets" = { path = "./vendor/widgets" }

[build]
target = "linux"

[test]
browser = true
~~~

- Parse real TOML with location-aware diagnostics; reject unknown critical keys, conflicting sources and invalid versions. Schema versioning or explicit format revisions must permit future evolution.
- Explicit CLI options win over manifest defaults; documented environment variables may provide credentials or opt-in settings but do not silently rewrite pinned dependencies. No plaintext secret fields in project manifests/lockfiles.
- A manifest is optional for scripts, single-folder projects **and basic multi-folder local projects**. It becomes necessary when an app wants declarative **external** Ware dependency management or publishing identity.
- `entry` resolves to a known project file; no ambiguous fallback when several `main` functions exist. Source conventions should not accidentally treat test entrypoints as release entrypoints.
- `[package] kind = "app"` vs `"ware"` distinguishes executable entry and distributable reusable source. Packages can export multiple public modules without shipping an executable.
- Keep ordinary project files free of auto-generated per-module manifests. Future `[workspace]` for multi-package monorepos is optional, not a Phase 6 completion blocker.
- `kiln-deploy.toml` stays outside the package contract. Do not implicitly migrate or deploy from `kiln build`.

**6.1 manifest gate:** documented precedence, errors and root discovery; manifest-free programs regressions pass; a multi-folder manifest can choose an entry; malformed TOML and invalid dependency specs identify the offending line.

## 6. Phase 6.2 — Ware identity, imports and packaging

### Package identity and namespaces

- Adopt a unique, normalized **publisher/package** identity, illustratively `kiln/ui` and `example/markdown`; require explicitly resolved names to avoid typo-confused package lookalikes.
- A Ware is **source code plus declared static resources and a manifest**. It is not a dynamically loaded shared library or an opaque binary copied into deployed applications.
- Wares re-use local module exports and `pub` visibility. Importing from another Ware must specify the package namespace or an explicit alias, so two versions/owners never silently collide.
- Specify case rules, Unicode normalization, reserved names, max lengths and names that cannot masquerade as the Kiln standard library.
- Package archives/indexes must have reproducible canonical content hashing, stable file ordering, normalized path handling and forbidden symlink/traversal rules. A hash covers **all** source/metadata that affects compilation, not just one manifest.
- For a source package, protect against path references outside its package root unless the dependency is an explicitly authorized local-path source; no arbitrary filesystem escape via Glaze/CSS imports.
- A Ware may contain Kiln, Glazes, CSS, assets, tests, examples and documentation. Document which files are published and which are excluded (secrets, build artifacts, lockfiles, private keys).

### SemVer and resolution

- Prefer SemVer constraints, pre-release semantics and explicit 0.x compatibility expectations. Lockfile pins **exact** releases; manifest ranges are not a source of nondeterministic fresh resolution during locked builds.
- Resolve transitive dependencies with deterministic, documented conflict behaviour. Support distinct versions of the same Ware where possible by package/instance identity; if that requires disambiguated imports, make diagnostics explicit rather than collapsing versions incorrectly.
- No silent preference for cache content over the lockfile. Resolver identity should include source URL/registry + package + release/revision + integrity hash.
- Git dependencies pin immutable **commit SHAs** in the lockfile, even when the manifest mentions a branch or tag. Clearly warn that a moving Git ref is not deterministic until locked.
- Path dependencies remain live source; the lockfile must reflect their presence without pretending they have an immutable registry digest. Document whether `--locked` permits changes in a local path package (source changes are intentional, unlike remote dependency drift).
- Resolve upgrades with a dedicated command such as `kiln update [name]`; `kiln upgrade` remains exclusively for upgrading the **installed compiler**.
- Unknown/unavailable versions, source conflicts, duplicate identities, cycles and policy-denied dependencies receive actionable resolution traces.

### CLI shape

~~~text
kiln add kiln/ui@0.1
kiln add example/widgets --git https://example.com/widgets.git --rev COMMIT
kiln add local/widgets --path ./vendor/widgets
kiln remove kiln/ui
kiln update                 # update lockfile within allowed manifest constraints
kiln update kiln/ui         # specific Ware
kiln tree                   # show resolved dependency graph
kiln tree --why kiln/ui     # explain why a dependency is present
kiln build --locked
kiln build --offline --locked
~~~

Exact flags and import naming remain proposed. `kiln add/remove/update` must modify manifest and lockfile together atomically; a failed resolve cannot leave the two inconsistent.

## 7. Phase 6.2 — Lockfile, cache and offline builds

### Lockfile guarantees

- Machine-readable, documented `kiln.lock` with a schema version, root identity, exact graph edges, source identity, exact version/commit and cryptographic content hash.
- Deterministically serialized: equivalent resolution produces byte-identical lockfiles across Linux/Windows (modulo documented local path normalization).
- A locked build rejects manifest/lock mismatch; an offline build never contacts any network endpoint for Wares, advisory databases or tool updates.
- Verify package integrity **before** placing an immutable source tree into the shared cache, and re-verify/correctly invalidate corrupted cache contents when used.
- Source and compiled objects are cached separately; compiled artifacts are keyed by source hash + compiler/runtime version + target triple + backend + relevant features/build options.
- Do not reuse compiled artifacts across incompatible compiler versions or target ABIs.

### Per-user global cache

Default path per platform: Linux `$XDG_CACHE_HOME/kiln` or `~/.cache/kiln`; Windows appropriate LocalAppData path; macOS the platform's user cache directory. Allow documented override suitable for CI and air-gapped systems.

- No `node_modules`-style or `ware_modules`-style copies in each project. Projects store their manifests, lockfiles and normal source only.
- Atomic downloads/extraction with temp staging + integrity validation + atomic publish. Prevent Zip Slip/path traversal, crafted symlinks, files masquerading as directories, oversized decompression and stale temporary files.
- Concurrent compilers/processes must safely coordinate cache writes and reads without deadlock or partially visible packages.
- Read-only use of cache is supported for CI; downloading into a writable cache remains a separate explicit action.
- Source package contents are immutable after verification. Local-path sources are **not** copied into the global store.
- Cache pruning must never damage active in-progress builds; expose a dry run. It is not required to know every external project that might someday need an offline version.

Proposed commands:

~~~text
kiln cache dir
kiln cache stats
kiln cache verify
kiln cache prune --dry-run
kiln cache prune
~~~

**Cache gate:** two unrelated apps using one hash share one cached copy; a second pinned version coexists; an offline build succeeds after priming and fails clearly when required content is missing; corrupted archives and parallel fetches cannot yield a silently compromised build.

## 8. Phase 6.2 — Ware registry and publication

### Bootstrap strategy

Start with a **locally hosted registry implementing the same protocol as the future public service**. Because Phase 4 supplies HTTP, SQL, auth and an admin, the initial registry itself can be built **in Kiln** and serve as a substantial dogfooding app. A static index/Git source can be a simpler transitional read-only transport. Do **not** require a globally hosted public service or community accounts to finish Phase 6.

Separate three roles:

- **Compiler/client:** resolves index metadata and downloads immutable archives by hash.
- **Registry server:** authenticates publishers, records immutable releases, serves metadata/blobs and yank/advisory status.
- **Trust/review metadata:** describes publisher identity and independent review status. Registry claims must be verifiable, sourced and presented accurately.

### Minimum registry protocol (target, versioned)

- Read: package identity/version listing, exact release metadata, dependency manifest, checksums, archive download, yank status, optional trust/advisory metadata.
- Write: authenticated upload/publish with immutable version and verified content hash; publisher ownership checks; idempotent retry semantics that reject content mismatch.
- Use a versioned `/v1/` API (exact endpoint naming to settle during implementation) and stable schema; treat index metadata and blob payloads as **untrusted network input**.
- Server pagination, cache headers/ETags, size limits, request limits, integrity verification, event/audit logs, auth token scopes and secure error handling. No credentials in lockfiles or logs.
- A **yank** hides a version from new resolutions but does not break an existing valid lockfile without explicit policy. An issue/security warning can be surfaced separately. Do not silently delete an immutable release.
- Publisher names cannot be taken over through package deletion or an expired account without a deliberate, auditable policy.
- Local dev registry may use plaintext HTTP **only on explicit loopback endpoints**; remote credentials/publishing require TLS unless the user explicitly opts into an unsafe development mode with a warning.

Proposed CLI:

~~~text
kiln publish --registry http://127.0.0.1:8081
kiln publish --dry-run
kiln login --registry https://registry.example
kiln add kiln/ui --registry http://127.0.0.1:8081
kiln ware verify kiln/ui
~~~

`kiln login` should use secure platform credential storage or a scoped environment token; never embed tokens in a manifest. The actual registry URL/command spellings are provisional.

### Package permissions, security and trust tiers

**Policy model to design and test:**

1. `Unverified` — provenance or ownership has not received elevated verification.
2. `Verified publisher` — an organization/publisher identity has been checked; **not** a full security review of code.
3. `Peer-reviewed` — a specific immutable release passed a documented independent process; does not guarantee zero vulnerabilities. Review badges must be bound to exact release hashes.

- Require explicit, machine-readable capability declarations for relevant effects (filesystem, network, process execution, environment/secrets, native/FFI, future plugins). Reuse or introduce compiler effect analysis to the extent sound; otherwise call out incomplete inference instead of presenting static detection as complete.
- A project can choose an allowlist or policy file that restricts dependency publishers, sources and declared capabilities; evaluate **transitive** dependencies against it.
- Running arbitrary scripts at fetch/install time is prohibited. Compilation/build may still process untrusted text, and using a Ware can execute its code at runtime: hashes and policy **do not amount to sandboxing**.
- Surface permissions, trust tier, resolved source, changed capabilities and transitive dependency changes before install/update; get explicit approval for escalations where interactive, fail safely in CI without an approved policy.
- Protect cache/manifest paths and lockfile integrity. Package source must not gain privileged compiler hooks or unrestricted custom build code as an accidental loophole.
- Any sandboxed execution or enforced OS-level capability restrictions are separate future security engineering, not implied by labels or manifest permission metadata.

**Registry gate:** a Kiln-authored local server can publish a Ware and serve a second project; publish conflicts, mismatched hashes, unauthorized accounts, revoked/yanked versions, policy violations and offline/locked consumers have conformance/integration tests.

## 9. Phase 6.2 — First official Ware: accessible Kiln UI

The flagship first Ware is a **source-first collection** of reusable Glaze/CSS/Ui/Finish components. `kiln/ui` is a **working package identity** pending final namespace ownership and version selection.

~~~glaze
{# Illustrative target syntax, subject to module/import resolution #}
<form method="post">
    <Input
        name="email"
        type="email"
        label="Email address"
        required
    />
    <Button type="submit" tone={.Primary}>Save</Button>
</form>
~~~

- Initial SSR components: `Button`, `Input`, `Textarea`, `Select`, `Checkbox`, `Radio`, `FormField`, `Label`, `Card`, `Alert`, `Badge`, `Table`, `EmptyState`, `Tabs` where CSS/native HTML can provide the intended behavior. Start with a small reliable subset; defer difficult interactive widgets.
- Correct HTML semantics, label/input association, consistent field descriptions/errors, keyboard focus, contrast, disabled states, reduced-motion preference and responsive default styles.
- Typed props for common variants (e.g. tone, size, error, disabled) and passthrough of safe ordinary attributes; integrate Phase 4 form field errors without making plain Input dependent on a database.
- Use standard Phase 3 Glazes, CSS, Ui, Finishes and typed tokens. Users can override a theme, component style, selected field widget or authored Glaze. Avoid independent theme or runtime styling machinery.
- Offer **dependency use** and a **source-copy** path, e.g. proposed `kiln ui add button`, so developers can own and modify individual component source without fork-heavy workflows. Copying must show provenance/license and should not overwrite local edits.
- Publish docs/examples and compile conformance; tests need keyboard/focus/ARIA smoke plus manual accessibility review. **Do not claim automated checks prove accessible UX**.
- Phase 5 supplies native browser interactivity through Embers, so the initial UI Ware may include tested interactive widgets rather than requiring an unrelated JavaScript framework.

**UI gate:** a real Phase 4 form/admin page can use this Ware (or copied source), compile for native Linux/Windows, render a consistent accessible page with two different Phase 3 themes, and remain editable without dependency lock-in.

## 10. Phase 6.3 — One `kiln test` runner

Detailed requirements remain in [testing.md](testing.md); this section fixes its dependencies, delivery order and full Phase 6 gate.

### Discovery and authoring

- Tests are Kiln code, compiled by the ordinary parser/type checker/backends. No separate testing language, JavaScript config project or hidden interpretation of Glazes.
- Proposed convention: `tests/unit/`, `tests/integration/` and `tests/browser/`, using `test_*.kiln` files and ordinary functions whose names begin `test_`. The final scheme must coexist with per-folder namespaces and package access rules.
- A manifest may choose test roots/filters/fixtures, but **manifest-free projects can run basic unit tests** using the same discovery defaults.
- Assertions are normal Kiln standard-library calls with typed failure messages and source spans. Return values/errors integrate with Kiln's `!` propagation; no invented `test` keyword needed.
- Fixture setup/teardown is explicit and deterministic. Tests can access public app APIs; a mechanism for testing module-internal functions must not accidentally make those functions public to ordinary dependencies.

Illustrative unit API:

~~~kiln
import test

fn test_addition() {
    test.equal(2 + 2, 4)
    test.not_equal("x", "y")
}
~~~

### CLI and process contract

~~~text
kiln test
kiln test --unit
kiln test --integration
kiln test --browser
kiln test --filter login
kiln test --watch
kiln test --json
kiln test --list
~~~

- Consistent counts, timings, typed assertion failures, expected/actual diffs, exit statuses (including a distinct infrastructure-failure category), filters and deterministic ordering.
- Reuse normal project root/dependency resolution and a locked/offline mode for tests.
- Parallelize isolated cases with explicit resource locks and test-specific temporary directories, ports and databases. Use a fresh DB fixture/migration state per integration suite where feasible.
- Make retries **opt-in and visible**, never silently greenwash intermittent failures; timeouts and cancellation always reap child processes and release ports.
- JSON event/summary format has a version, source URIs/spans and stable IDs suitable for CI/IDE/MCP consumption. Offer familiar JUnit XML format as an integration format when practical.
- Code coverage measurement can begin with precise unit/function and file summaries; do not block the rest of testing on perfect coverage from every native codegen optimization level.

### Browser mode: no Node project

- Use maintained browser automation transport internally (WebDriver, CDP or Playwright-compatible engine); this is **not** a mandate to reimplement a browser or embed Chromium in app binaries.
- Provision/cache pinned compatible browser executables separately. Explain first-download behaviour, platform support, offline setup and explicit version pinning.
- Auto-launch a compiled Kiln HTTP server on a safe free port; wait for readiness, pass a typed base URL fixture and clean up on failures. Prevent accidental testing against production external URLs by default.
- Kiln-authored browser APIs: navigate, find by accessible role/label/text, fill, click, submit, keyboard input, inspect URL/text/attributes, check focus, viewport, computed CSS and console/network errors.
- Capture screenshot on failure and meaningful logs/traces when available; use source-linked failure diagnostics and reproducible artifact locations.
- Test forms, cookie sessions, CSRF, permission checks and responsive Glaze/Finish themes with **real** browser semantics.
- Interactive Ember state, hydration and typed server RPC must be supported by the Phase 6 public runner from its first release.

**6.3 gate:** one command runs (a) a pure function test, (b) an isolated SQL/HTTP/auth integration test and (c) a headless browser scenario creating a record via form and an Ember server function, checking reactive updates, validation, keyboard focus, navigation, content, CSS and login/logout. Failures produce screenshots/logs, meaningful exit codes and machine-readable summaries on Linux and Windows.

## 11. Phase 6.4 — Formatter, live reload and editor workflows

### `kiln fmt`

- Parse and format existing Kiln source through a deterministic, stable, idempotent layout engine. Establish canonical whitespace, imports, line wrapping, multiline structs/calls, closures and match formatting. Preserve comments, diagnostics and literal content.
- Proposed modes: `kiln fmt`, `kiln fmt --check`, and optional `kiln fmt PATH`. Provide editor format-on-save through LSP using **the same** formatter.
- **Glaze whitespace is semantic.** Never casually normalize newlines/indentation inside HTML text, CSS, `<pre>`, `<script>` or `<style>`. Start by formatting embedded Kiln code regions safely and add conservative markup formatting only with byte/render-equivalence tests.
- CSS formatting should reuse recognized CSS tooling; no need to implement an entirely new CSS pretty printer. Do not overwrite malformed text when the formatter cannot preserve semantics reliably.
- Golden fixtures + property/idempotence testing + compile/render equivalence checks; never turn a valid Kiln program into an invalid one.

### `kiln dev` / watch

- Provide a development watcher that rebuilds changed Kiln/Glaze/CSS/assets and **restarts** the running native server when necessary; keep existing successful server running if the new build fails.
- No requirement for in-process native code hot patching, special runtime interpreter or Node build system.
- Detect dependency/inclusion graph changes and invalidate only affected units, debounce editor save bursts, avoid file watcher loops on generated outputs; survive rename/delete.
- Preserve clear logs, show diagnostics from source spans and expose what process/port is active. Graceful restart and session persistence follow the app's actual server/session semantics; do not promise live local-variable retention.
- Optional browser live-reload refresh may use a dev-only channel injected into served pages; never bundle that mechanism into release applications. Document security of binding dev servers to localhost unless explicitly configured.
- Watcher must be interruptible and clean child processes, sockets and temp outputs on Linux/Windows. Handle occupied ports with useful suggestions.

### LSP and VS Code

- Multi-root/project graph diagnostics, import completion, go-to-definition and references across local modules, Wares, Glazes, Finish functions, CSS modules, model/form fields.
- Find references and rename respect public/private boundaries and avoid rewriting vendored immutable cached Ware source. Warn when an external/public rename would break dependants.
- Formatter integration, test discovery/run status, error code actions, quick import fixes, manifest/lockfile validation, dependency tree view as feasible. VS Code should orchestrate CLI/LSP, **not fork language semantics in TypeScript**.
- Refactor/format changes must be previewable where risky. Editor does not secretly add a Ware to resolve an ambiguous import.

**6.4 gate:** a multi-folder app with a Ware survives format/check/watch cycles and cross-module rename; a broken source edit reports diagnostics while the last-known-good server remains available; Linux/Windows smoke tests pass.

## 12. Phase 6.5 — Agents, MCP and documentation

### Compiler/tooling MCP

Proposed `kiln mcp` runs a local Model Context Protocol server giving an agent structured access to the **same** compiler, package and test APIs:

| Tool type | Example | Safety level |
| --- | --- | --- |
| Introspection | Project graph, exports, compiler version, dependency tree, source docs | Read-only |
| Check | Type-check changed files, structured diagnostics, explain errors | Read-only with bounded compilation resources |
| Test | Run named test suites and retrieve artifacts | **Executes app/test code**: requires an explicitly trusted project context |
| Build | Compile target with explicit settings | Writes artifacts, may consume resources; request explicit workspace scope |
| Format | Preview formatting diff or apply requested formatting | Diff/preview first, explicit write permission |
| Add/update Ware | Propose manifest/lock changes, present capability diff | Network/filesystem mutation; require explicit approval |
| Publish/deploy/migrate | Reserved for later, separate permissions and confirmations | **Never implicitly enabled** |

- MCP is transport and structured tooling **not** an alternative compiler and not an application-runtime MCP standard library. Application-side MCP APIs can arrive separately.
- Expose only allowed workspace roots; protect secrets, environment vars, tokens and arbitrary filesystem paths. Do not turn a “check” tool into shell execution or silently run project code. Compilers still parse untrusted input: resource limits matter.
- Distinguish **safe to read/parse** from **may execute code**; running tests can contact a database or external system. Scope execution to a trusted local project and display side effects.
- Use versioned tool schemas, stable diagnostic/error types, bounded outputs with pagination/links to artifacts, cancellation and clear capability boundaries.
- No hidden background automation: tools respond to explicit requests and report completion/errors accurately.

### Agent guide and benchmark

- Document an idiomatic minimal Kiln app, language pitfalls, Glaze styling, SQL/ORM/auth, local module conventions, add/update Wares and how to use `kiln test` to validate agent output.
- Provide a concise machine-friendly guide (e.g. `llms.txt` and optional proposed `kiln docs --llm`). A generated starter `AGENTS.md` is convenience, not a required part of app compilation.
- Maintain realistic benchmark tasks: simple CLI tool; HTTP API; SSR page; imported DB query; auth-protected form; use of a Ware; broken compilation and test repair.
- Measure compile/run success, number of corrective iterations, failure quality and changes to existing code; do not make unsupported claims about token reduction. Gate regressions after baseline stabilizes.

### Dogfooding criteria

- Complete a genuine multi-folder Kiln site, ideally the docs site plus a **Kiln-authored local Ware registry** built on Phase 4 storage/auth and a separate small application consuming it.
- Publish the initial UI Ware to the local registry, consume it from a separate project, switch themes, replace one component with local source and demonstrate locked offline builds.
- Drive a full `kiln test` suite including real browser tests and deliberately failing tests; exercise LSP/formatter/dev watch while changing imports and Glazes.
- Have an agent use `kiln mcp` to get project graph, check and run authorized tests. Inspect outputs, error reports and permissions.

**6.5 gate:** A newcomer (human or agent) can follow one documented path from an empty Kiln install to a multi-folder website with a package, authenticated form, deterministic build and browser-tested behaviour.

## 13. Testing, CI and security matrix

| Area | Must succeed | Must fail safely |
| --- | --- | --- |
| Module graph | Cross-folder import, public exports, Glaze/CSS references | Private access, unresolved modules, cycles, case/path collisions |
| Manifest | Optional scripts, correct root/entry selection, config precedence | Broken TOML, contradictory keys, secret exposure, invalid entry |
| Resolver | Local/Git/registry Wares, transitive version resolution | Conflict, mismatched lock, moving refs in locked build |
| Cache | Identical hash shared, offline build, concurrent writers | Corruption, archive escape, size bombs, stale partial download |
| Registry | Local authenticated publish, immutable versions, yanks | Unauthorized publish, hash mismatch, replacement of existing release |
| Permissions | Declared capabilities, transitive policy checks, tier metadata | Unapproved escalation, inaccurate verified/review labels, install hooks |
| UI Ware | SSR rendering, safe overrides, two themes, accessible form | Invalid props, unsafe attribute injection, broken label/focus |
| Unit tests | Discover, assert, filter, run fast, readable failures | Hidden panics, unreaped processes, flaky retries marked green |
| Integration | Isolated database/auth/API tests and cleanup | Accidental production URL, leaked credentials, leftover test DB |
| Browser | Actual navigation/forms/focus/CSS, screenshot failures | Flaky unbounded wait, browser missing without guidance, unsafe targets |
| Formatter/dev | Idempotent formatting, last-good server, cross-file rename | Whitespace/Glaze mutation, watcher loop, corrupt auto-rename |
| MCP/agents | Read/check/test structured tools within scope | Test execution treated as read-only, silent package mutation, credential leaks |

CI must test native Linux/Windows and the C backend where supported, plus cross-module project conformance, locked/offline builds, registry/local protocol fixtures, malformed/untrusted metadata, and pinned real-browser tests. Use disposable servers and databases, including negative authentication tests. No test should require arbitrary external public service availability.

## 14. Definition of done

- [ ] Existing single-file/single-folder examples still compile and run with **no manifest**, while a multi-folder Kiln project has deterministic module resolution, `pub` visibility, typed cross-folder imports and source-linked errors.
- [ ] Optional `kiln.toml`, a well-defined project root and `kiln.lock` support exact repeatable dependency graphs with local/Git/registry Wares.
- [ ] Package name/source identities, SemVer constraints, transitive resolution, namespace collisions and locked/offline behaviour are documented and have regressions.
- [ ] Immutable **per-user global Ware source cache** works with integrity verification, concurrency safety, optional compiled-object caching, inspect/verify/prune CLI and no project-local dependency tree.
- [ ] A **locally hosted registry**, preferably built in Kiln, can accept authenticated immutable Ware releases and serve clients using a documented protocol with hash, yank and trust/permission metadata.
- [ ] Package installs do not execute arbitrary scripts; capability reports and transitive policy checks give honest security boundaries, with no claim of perfect sandboxing.
- [ ] The first official accessible UI Ware is publishable, usable in real SSR/forms/admin flows, customizable through normal Glaze/CSS/Ui/Finish patterns and optionally source-copied.
- [ ] `kiln test` discovers/runs Kiln unit, integration and **real-browser** tests in one workflow, supports isolated resources and reports failures/screenshots/logs/CI results on Linux and Windows.
- [ ] `kiln fmt` is deterministic/idempotent and preserves Glaze output; the LSP/VS Code support modules, Wares, references/rename and tests; a live rebuild/restart loop is safe.
- [ ] `kiln mcp` exposes bounded, versioned tooling without silently authorizing filesystem writes, arbitrary project-code execution, credential reads, publish or deployment.
- [ ] A full dogfood project demonstrates the local registry, UI Ware, locked offline rebuild, auth/forms/Glazes, editor workflow and browser suite.
- [ ] Every implemented feature has reference docs, conformance/CI checks, clear unsupported-case diagnostics and no regression to compiler backends or standalone target deployment.

## 15. Explicit deferrals and non-goals

- **Phase 5 prerequisite:** Embers/WasmGC, reactive hydration, generated RPC and state are already available; Phase 6 exposes them through its integrated testing and ecosystem workflows.
- **Phase 7:** One-command deploy/release, static site export, broad stability/performance hardening, arbitrary code optimization or a mandatory new native backend.
- **Later registry scaling:** Commercial hosting, public social discovery, web search/rankings, organization billing, cross-user global cache, complete security auditing/review infrastructure at industrial scale.
- **Later tooling:** Full debugger/DAP and variable inspection, deep time-travel debugging, arbitrary live-patching of native code, self-hosting of all compiler internals.
- **Optional later work:** Full multi-package workspace publishing, global dependency feature unification, a universal WASI/container sandbox, all-language FFI bindings, automatic use of arbitrary npm modules.
- **Non-goals:** Mandatory manifests for scripts; checked-in downloaded Wares; install-time shell scripts; a Node/JS testing repository; a custom browser implementation; automatic publishing/deploying from `build`; pretending metadata labels enforce OS-level permissions.

## 16. Review principles

- Adding a local module must be as unsurprising as adding a folder.
- One app must never be able to corrupt another app's locked dependency selection.
- A verified download is not automatically a trustworthy application capability.
- Tests should encourage quick experimentation and leave permanent regression coverage as Kiln's internals evolve.
- A formatting or editor convenience must not silently change Glaze output or authorization semantics.
- AI tooling should expose the same intelligible operations and errors a human developer sees, with **more explicit**, not fewer, safety boundaries.
- Phase 6 should **reduce tooling glue without replacing sound existing tools purely for novelty**.

**In one sentence:** **Phase 6 makes Kiln a practical ecosystem: organize an application, reuse trusted source, build reproducibly, test from function to browser, and develop confidently in one coherent toolchain.**
