# Phase 4 — Embers, WasmGC and typed full-stack interactivity

**Status:** Planned implementation and acceptance specification. Syntax snippets here describe **targets**, not functions that currently compile. The existing Glaze parser recognizes `{% state %}`, `on:...` and hydration hints, but compiling interactive Embers remains future work.

**Post-implementation correction (2026-10-26):** Phase 4 landed with *implicit* Embers (a `.glaze` containing state/handlers). That behavior is currently implemented, but it does not match the later decision for explicit `.ember` files. [Phase 4.1](phase-4.1.md) is the authoritative follow-up for component file kinds; it supersedes conflicting implicit classification language below, not the WasmGC, RPC, hydration or interop design. Do not infer that `.ember` currently compiles from this plan.

**Goal:** Complete Kiln's browser-interactivity and typed client/server language capabilities **before** the database and authentication framework. Phase 3's compiled Glazes and styles and the existing HTTP/JSON runtime suffice for browser WasmGC, selective hydration, reactive state, and checked `server fn` RPC. Phase 5 will add persistent data, sessions and authorization on top of that boundary.

**Done when:** A native Kiln server runs a genuine **database-free, interactive website** with ordinary SSR content, selective WasmGC islands, typed state/events, and a browser calling a **pure or ephemeral-state server function** through generated RPC. Invalid/oversized/cross-origin calls are rejected. No ORM, database, user accounts, authentication framework or handwritten application JavaScript/RPC is required.

This plan refines the [vision for Embers](../vision.md#embers-interactivity-when-and-where-needed), the [Glaze reference](../reference/glazes.md#embers) and [Phase 3](phase-3.md). **Phase 3 is still in active development**; this roadmap adjustment does not mark it complete. [Phase 5's SQL/auth plan](phase-5.md) and [Phase 6's ecosystem plan](phase-6.md) follow, rather than gate, browser interactivity.

## 1. Architectural decisions

1. **SSR is the baseline.** Ordinary Glazes render using Kiln's compiled server-side pipeline, with no Wasm or JS bootstrap on pages without interactive components.
2. **Embers are compiled, not interpreted.** Client-reachable Kiln source is checked, lowered through shared typed IR where feasible, then compiled for the browser WasmGC target; no runtime Kiln interpreter or unrelated reactive DSL.
3. **Server/client are distinct trust zones.** Browser code cannot directly import server-only functionality, native filesystem/OS/FFI or secret configuration. Phase 5's future `sql` and `auth` modules also stay server-only.
4. **Hydration is selective.** Existing `client:load`, `client:visible`, `client:idle` and `client:only` syntax should have defined loading/fallback semantics; choose a sensible default for an Ember used without a hint.
5. **Typed reactivity.** `{% state %}` is local typed state; `on:event` triggers ordinary Kiln handler logic. Changes update only affected DOM while preserving focus, selection, list identity and nested state where possible.
6. **`server fn` is a checked network boundary.** Derive client proxy, request/response types, codecs and server handler from one declaration. Validate every request, explicitly expose allowed functions and provide policy hooks. Phase 4 can use public pure functions; Phase 5 supplies identity and per-record authorization.
7. **Explicit async state.** Network failures, cancellation, stale responses, optimistic updates and error recovery must be observable. Never silently repeat unsafe mutations.
8. **Use browser standards and existing tools.** Reuse maintained Wasm and JS interop libraries/bundlers, and permit ordinary HTML/CSS/JS integrations. The implementation is replaceable; Kiln developer-facing semantics are the durable contract.
9. **No premature ecosystem scope.** Local multi-folder modules and an optional minimal `kiln.toml` follow in [Phase 4.2](phase-4.2.md); Wares, official UI Ware, lockfiles, public `kiln test`, formatter and compiler MCP remain Phase 6.
10. **Testing now, polished runner later.** Extend compiler conformance and real-browser CI during Phase 4. Phase 6 supplies the unified public test runner, **including Ember hydration/RPC from its first release**.
11. **WasmGC-only browser target for Phase 4.** Do not build or require a JavaScript code-generation fallback in this phase. When browser feature detection shows missing WasmGC support, preserve usable SSR HTML and native link/form navigation wherever possible; Ember-only functionality may be unavailable. A JavaScript fallback backend is a [long-term vision item](../vision.md#future-javascript-compiler-fallback), not a 1.0 gate. The minimal JavaScript bootstrap/interop bridge for WasmGC is not such a backend.

## 2. Target developer experience

A plain SSR Glaze stays unchanged:

~~~glaze
{% props title: String %}
<article>
    <h2>{title}</h2>
    <p>A plain Glaze needs no browser runtime.</p>
</article>
~~~

An Ember uses familiar Glaze syntax:

~~~glaze
{# Illustrative Phase 4 target; async syntax is not finalized #}
{% props label: String %}
{% state count = 0 %}

<button on:click={ count += 1 }>{label}: {count}</button>
~~~

A server function can be **pure** Kiln:

~~~kiln
// Proposed browser-callable contract, not currently executable.
server fn add(a: Int, b: Int) -> Int {
    a + b
}
~~~

An Ember could call its generated proxy without needing a database:

~~~glaze
{# Proposed async handler syntax — to be finalized #}
{% state value = 0 %}
<button on:click={ value = add(value, 1) catch err { value } }>
    Server result: {value}
</button>
~~~

An SSR page selectively hydrates a component:

~~~glaze
{% props title: String %}
<article>
    <h1>{title}</h1>
    <Counter label="Clicks" client:visible />
</article>
~~~

**Important:** Phase 4 must **not** implement ORM, durable sessions or authentication merely to demonstrate RPC. It must, however, define a safe request-bound transport and authorization extension points so Phase 5 can layer session identity and per-record permissions without replacing generated routes or codecs. Async syntax is not finalized.

## 3. WasmGC compiler and runtime contract

- Add a browser code-generation target from checked Kiln IR with semantics for structs/records/enums, optionals, generic specializations, closures, strings, collections and error paths. Use browser WasmGC allocation/collection rather than bundling native Boehm GC.
- Build a **client-reachable** call graph separately from server-reachable code. Only needed browser types/functions, island logic and necessary minimal runtime helpers are shipped. This is early reachability/dead-code elimination, not the complete Phase 7 optimizer.
- Track forbidden browser imports/effects during checking and emit source-linked diagnostics. The browser must never gain direct access to server secrets, operating-system resources or native handles; Phase 5's SQL and session APIs must also be server-only.
- Define compiler-level interop and type conversion for JS/DOM strings, numbers, arrays, records, handles, callbacks and promises, including lifetime/GC and exception propagation.
- Generate content-addressed Wasm and minimal JS bridge/assets through Kiln's existing asset pipeline. Release output must not require a separate JavaScript project or Node runtime on the deployed server.
- Document a tested WasmGC browser support matrix and feature-detect unsupported platforms before initializing an Ember. Preserve usable SSR content and native navigation/form behaviour when interactivity cannot run. Do not implement a separate JavaScript code-generation fallback in Phase 4; that is a longer-term compatibility goal.
- Source maps/error traces and useful debug symbols should point back to Kiln/Glaze where feasible; don't leak secret-only server code into browser assets.

**Gate:** a real browser runs generated WasmGC from a Kiln program with representative data structures, errors, generic code and closures; C/native and Wasm outputs have shared semantic regression cases.

## 4. Ember islands and hydration

- A Glaze using `{% state %}` or `on:...` becomes an Ember and remains SSR-capable unless `client:only` explicitly opts out.
- Define when `client:load`, `client:visible`, `client:idle` and `client:only` fetch/init an island, including fallback for browsers without the relevant scheduling API.
- Server HTML contains stable island IDs and escaped, explicitly serializable public initial props. Protect against HTML/script context injection (including breaking out of `</script>`), unsafe URL/attribute and malicious user-provided content.
- Browser reconciles SSR DOM with typed state. A mismatch is diagnosed, not silently duplicated; stable keys preserve per-item state in lists and avoid accidentally resetting unrelated islands.
- Nested Embers share runtime assets where practical without unintentionally sharing local state. Listener/task cleanup runs when an island unmounts.
- No Ember code must be loaded on an ordinary static-only Glaze route, and ordinary SSR navigation/form submissions should work without JavaScript.

**Gate:** a server-rendered page has two independent Embers with different hydration hints alongside static content; real browsers exhibit correct timing, state isolation and fallback.

## 5. Reactive state, handlers and forms

- `{% state name = expr %}` creates local, typed state initialized consistently with SSR. Writes trigger minimal DOM updates and batched invalidation, with deterministic ordering and no hidden full-page rerender.
- Typed event handlers initially cover click/input/change/submit/keydown and standard event data; provide explicit prevent-default/propagation semantics.
- Respect native controls, labels, IME input, focus, cursor, selection, keyboard accessibility and HTML validation behaviour. Support conditional children, lists and keyed identity.
- Lifecycle/effects/subscriptions need explicit cleanup so removed components do not leak listeners, timers or state updates. No global cross-user component state.
- Phase 4 covers reactive input/event plumbing and basic typed value constraints **without** requiring a form/ORM framework. Phase 5 adds shared server-side validation and typed field-error feedback for Ember forms; ordinary POST fallback remains viable.
- If reactive components need asynchronous work, represent pending/error/success and cancellation as typed, inspectable state rather than magical invisible promises.

**Gate:** interactive form, counter, changing list and nested Ember respond properly; focus remains stable and disposed components don't continue updating.

## 6. Typed server functions and generated RPC

- `server fn` declarations are server-only implementations, with an explicit set of browser-callable exports. Do not accidentally expose every internal helper by generating guessable endpoints.
- Compile serializers/codecs only for safe, serializable public types. Reject DB handles, file descriptors, closures/function pointers, secrets and unsupported native-only types in RPC arguments/results.
- Generate typed client proxies and server routes with consistent message framing/versioning, clear errors, timeouts and cancellation. Runtime still validates untrusted client values, sizes and schema versions.
- **Phase 4 baseline:** explicitly exposed functions, validated/bounded payloads, same-origin/origin checks, safe CORS/content-types, CSRF protection for credentialed mutations where applicable, rate limits/abuse controls and safe error messages. Public functions are deliberately marked as such, never implicitly granted privileges.
- **Phase 5 extension point:** provide a secure request context and per-call policy hooks to add authenticated user/session/record authorization without changing client signatures or wire conventions.
- Reject malformed, oversized, unauthorized, stale-version and untrusted inputs safely. Prevent accidental retry of non-idempotent writes.
- Shared type definitions should eliminate duplicate DTOs, HTTP wrappers and JSON encoding glue; no invented client-only model semantics.

**Gate:** a browser Ember receives a typed result from an explicitly exposed, pure `server fn`; an unexposed function, bad origin, malformed/oversized request or invalid codec is safely rejected. Phase 5 tests authenticated record mutations.

## 7. Asynchrony, optimistic updates and consistency

- Typed pending/success/failure states and explicit errors for `server fn` calls. Support cancellation on navigation/disposal and documented handling of concurrent responses arriving out of order.
- Allow deliberate optimistic UI with rollback/reconciliation on server failure. Do not silently overwrite fresh state with stale network responses.
- Provide observability for resource caching/invalidation if introduced; avoid a hidden browser ORM cache that users cannot inspect.
- SSE/WebSocket subscription integration may follow as extension if required by dogfood apps, but isn't necessary to mark core typed RPC and state complete.

**Gate:** double clicks, a delayed response, a failed optimistic mutation and navigation away leave client/server consistent and error feedback usable.

## 8. JavaScript and browser interoperability

**Finalized architecture (2026-10-25): Option B — strongly typed by default, with an explicit dynamic escape hatch.** Kiln authors must be able to use browser standards and compatible existing npm packages *without writing application JavaScript*. The compiler/toolchain generates the minimal JavaScript bootstrap, browser/WasmGC bridge and package bundling. This is **not** a JavaScript code-generation backend for Kiln; WasmGC remains the Phase 4 browser target.

### 8.1 Typed browser APIs

- Provide browser-targeted typed modules (initially `browser`) for representative DOM events, clipboard, storage, fetch and element references; expand to history/location, files, canvas, observers and other standards as needed. Wrap host APIs rather than reimplementing browser behavior.
- Browser-only modules and functions are rejected when called from server-only code or during SSR evaluation. Browser effects belong in client lifecycle/tasks/event handlers. Browser permissions, secure-context requirements, same-origin and CSP rules continue to apply.
- Current Phase 4 implementation recognizes Embers implicitly within `.glaze` files; the agreed explicit `.ember` file model and the removal of implicit promotion are specified and scheduled in [Phase 4.1](phase-4.1.md). Until that lands, the `.ember` suffix is not a supported source kind. Ordinary static Glazes continue to render without a browser bridge.

### 8.2 npm packages with automatically generated bindings

- Support direct, clearly namespaced npm imports in browser-targeted Kiln code, with a proposed convenience command such as `kiln add npm:canvas-confetti`; exact grammar/command naming remains a syntax/tooling decision, not a prerequisite to the architectural contract.
- Resolve compatible browser exports and TypeScript declaration files (bundled types or `@types`) and translate **supported** types/signatures to Kiln bindings; compile those calls into the JS/WasmGC bridge and bundle only the required browser-side assets where possible.
- Do not silently convert an unmappable typed API to an unchecked dynamic type. Explain unsupported overloads, complex TypeScript conditional types, Node-only built-ins, framework-specific runtime assumptions and missing declarations with source-linked diagnostics and suggested alternatives.
- Use framework-independent browser libraries first. React-specific components, Node-only modules and arbitrary npm packages are **not** guaranteed to interoperate. Wares are Kiln-native packages; optional curated Wares may wrap popular npm libraries but are not required for ordinary supported npm imports.
- Phase 4 needs a pinned, reproducible path for at least one small browser library, **without** requiring Phase 6's general Ware registry, multi-folder package system or public package-manager UX. Phase 6 can unify resolution, manifest/lockfile and dependency-management commands. Released builds must identify and integrity-check the resolved JS artifacts; installation must not execute arbitrary npm lifecycle scripts by default.

### 8.3 Typed foreign values and explicit dynamic escape hatch

- Provide opaque typed `js.Handle<T>`-style references for external JS/DOM object identities and an explicitly marked `js.Value`-style dynamic representation for APIs that generated bindings cannot model. These names describe the decided model; exact parser syntax and core type definitions remain to be finalized.
- Dynamic calls/property access are opt-in and visibly foreign. **Never** make an unsupported operation implicitly dynamic. Converting `js.Value` into a Kiln value requires a checked, recoverable conversion; no unchecked casts by default.
- Allow explicit Kiln-authored foreign binding declarations for awkward libraries. An escape hatch does not require developers to write JavaScript, although third-party JS implementations still execute in the browser.
- Start with explicit conversion rules: JS `string`/boolean to Kiln `String`/`Bool`; JS `number` to `Float` unless a verified narrower conversion is requested; JS `bigint` to `Int` only after bounds checks; supported arrays/records to typed collections/records; `null` or `undefined` to optionals where their distinction is unimportant. Preserve their distinction explicitly if an API depends on it.
- Specify copied values vs retained handles, equality/identity, handle table/rooting and cross-GC lifetime. No raw JavaScript object or host capability is treated as a serializable Kiln/server-RPC value by default.

### 8.4 Async, callbacks, errors and DOM ownership

- JS promises integrate with Kiln's agreed implicit async/task model and cancellation/error policies; no JavaScript `await` needs to appear in user code. Promise rejection and synchronous JS exceptions become typed, inspectable failures at the foreign boundary.
- Callback closures can be passed to browser APIs with explicit rooting, lifetime and disposal semantics. Subscriptions, event listeners, observers, timers and retained JS handles must be released on Ember disposal/navigation where applicable.
- JS-driven DOM libraries must integrate through an explicit owned/borrowed element boundary so they do not mutate Ember-owned reconciled DOM unpredictably. A JS library's mutations do **not** automatically become Kiln reactive state: foreign callbacks update Kiln state through explicit typed writes.
- Respect browser security and permissions. Typed bindings improve developer confidence but do **not** sandbox third-party JS or eliminate supply-chain risk.

### 8.5 Build and diagnostics

- Generate deterministic, content-addressed Wasm and JS bridge/assets through Kiln's build pipeline, without a developer-maintained Vite/Webpack/npm project or a Node runtime on the deployed Kiln server. Existing build tools may be reused internally.
- Produce diagnostics for server/browser misuse, failed type mapping, unsafe dynamic conversions, unsupported packages, stale/disposed handles, incompatible platform features, missing browser permissions and JS exceptions. Source locations should point back to Ember/Kiln where practical.
- Static-only pages must not load an Ember bootstrap or third-party JS. Unsupported WasmGC browsers retain SSR content and native links/forms as far as possible; a separate fallback JS compiler remains a longer-term vision item.

**Gate:** a real-browser Ember invokes a typed native browser API and a small pinned npm library without handwritten JavaScript; an explicitly dynamic `js.Value` call works and fails safely on a bad conversion; exception/promise/callback cleanup, Node-only import diagnostics, SSR/browser guardrails, CSP and production asset delivery pass conformance tests.

## 9. Security, performance and cross-target CI

| Domain | Positive cases | Negative and adversarial cases |
| --- | --- | --- |
| Codegen/GC | Structs, optionals, closures, mutation, errors | Invalid captures, GC lifetime bugs, unsupported browser feature |
| Hydration | SSR match, lazy island, nested list | Duplicate nodes, ID collisions, malformed state, script breakout |
| Events/DOM | Focus, keyboard, conditional updates | Leaked listener, stale DOM, clobbered input |
| RPC | Exposed typed pure function, result/error codec | Unexposed function, malformed/oversized input, invalid origin/CSRF, error leakage |
| Async | Pending, cancellation, optimistic success | Stale response, duplicate mutation, failed rollback |
| Interop | Typed browser API, pinned npm package, explicit dynamic call, JS callback/promise | Disposed handles, Node-only imports, failed checked conversions, SSR-only misuse, exception/cancellation |
| Delivery | No Wasm on SSR-only page, Linux/Windows server | CSP breakage, wrong MIME, excessive unnecessary Wasm |

- Real-browser smoke/regression testing belongs in Phase 4 despite the public `kiln test` runner arriving later.
- Keep C/native server conformance green and test WasmGC semantics as a third target. Browser support is tested explicitly; standalone native application requirements remain.
- Measure emitted Wasm size, initialization/hydration time, memory and DOM updates as a baseline, without requiring premature full compiler optimization.

## 10. Implementation steps and gates

| Step | Scope | Gate |
| --- | --- | --- |
| **1 — WasmGC backend** | Typed IR/codegen, browser runtime subset and compatibility tests | Real-browser Kiln logic executes, comparing well with native |
| **2 — SSR hydration** | Island identities, client asset emission, `client:*` policy and serialization | Two selective islands hydrate safely alongside static content |
| **3 — State and events** | Reactive dependencies, typed handlers, DOM updates and disposal | Counter, list and form update without losing focus |
| **4 — Server functions** | RPC contracts/codecs/server endpoints, validation/origin/CSRF hooks | Public pure function succeeds, unexposed/hostile calls fail without auth or a database |
| **5 — Async UX** | Pending/cancel, stale results, optimistic rollback | Repeated actions/failure/reorder leave consistent state |
| **6 — JS interoperability** | Typed browser API, TypeScript declaration bindings, pinned npm bundling, explicit dynamic escape hatch and JS bridge | Browser API + external library run without handwritten JS; invalid dynamic conversions and unsupported packages fail safely |
| **7 — Polish** | Diagnostics/LSP, CSP/assets, cross-target CI, performance basics | Static pages ship no Ember runtime, platform suites pass |
| **8 — Dogfood** | Real database-free SSR site with independent islands, reactive inputs and pure/ephemeral RPC | Full browser-server interaction works in Kiln without SQL/auth/JavaScript glue |

Each step is a separately reviewable milestone with diagnostics, conformance/browser cases, documentation and acceptance. Avoid bundling the entire browser compiler/runtime into a single untestable PR.

## 11. Definition of done

- [ ] WasmGC executes checked Kiln browser code with coherent semantics for supported types, generics, closures, errors and GC.
- [ ] Server-rendered Glazes remain default; Embers selectively hydrate and don't force full-page JavaScript.
- [ ] Unsupported WasmGC browsers retain usable SSR markup and native navigation/form behaviour where possible, without a second JavaScript compiler target.
- [ ] State/handlers update DOM predictably with safe lifecycle, list identity and keyboard/focus handling.
- [ ] `server fn` calls compile into typed serialized RPC with failure, cancellation and version checks, without handwritten frontend route wrappers.
- [ ] The RPC transport implements explicit export control, input validation, origin/CORS/CSRF protection as appropriate, bounded resources and request context/policy hooks ready for Phase 5 authentication.
- [ ] Browser JS interoperability uses typed browser APIs and compatible npm bindings by default, provides an explicit checked dynamic escape hatch, bundles reproducibly without handwritten JavaScript and enforces browser-only/lifecycle constraints.
- [ ] Deterministic Wasm/assets integrate with Phase 3 CSS/Finishes; static-only pages don't emit Ember code.
- [ ] Real-browser/CI regression cases and Linux/Windows native server checks pass.
- [ ] A database-free interactive Kiln app with SSR, islands, reactive state, and typed pure/ephemeral-state RPC works end-to-end without writing client JavaScript or transport glue.

## 12. Explicit boundaries

- **Phase 5:** SQL/ORM, schema import/migrations, typed forms/validation, sessions/auth, generated admin and authenticated database-backed Ember/RPC applications.
- **Phase 4.2:** Local multi-folder modules, optional minimal `kiln.toml`, and compiler/editor module support.
- **Phase 6:** Extended manifest dependency fields, `kiln.lock`, Wares/registry/global cache, official accessible UI Ware, public `kiln test` (with Ember tests **at launch**), formatter/live reload/refactoring and compiler MCP.
- **Phase 7:** Deployment, static-site export, extensive optimization/hardening and long-term compatibility guarantees.
- **Later:** Optional JavaScript fallback code generation for browsers without WasmGC (from the same checked Kiln semantics, subject to feasibility and cross-backend conformance), PWA offline/sync, advanced desktop reuse, arbitrary npm/React/Node compatibility, freestanding targets and complete debugger.
- **Non-goals:** Forced SPA routing, client-side access to server data stores/secrets, runtime template interpretation or a mandatory second JavaScript framework.

**In one sentence:** **Phase 4 completes Kiln's browser-interactivity and typed server/client language capabilities; Phase 5 then connects that boundary to persistence, accounts and authorization.**
