# Phase 5 — Embers, WasmGC and typed full-stack interactivity

**Status:** Planned implementation and acceptance specification. Syntax snippets here describe **targets**, not functions that currently compile. The existing Glaze parser recognizes `{% state %}`, `on:...` and hydration hints, but compiling interactive Embers remains future work.

**Goal:** Complete Kiln's full-stack programming **capability**, building on Phase 3's compiled SSR Glazes and styles and Phase 4's data/forms/authentication. Interactive components (**Embers**) compile to **WasmGC**, hydrate only where needed, update the DOM reactively and call checked Kiln `server fn` functions through generated typed RPC. Static pages should not pay for a client runtime.

**Done when:** A native Kiln server runs a real database-backed application with ordinary SSR content, selective client WebAssembly interactivity, typed client/server data, secure authenticated mutations, browser interop and tested error handling. No application-authored JavaScript or handwritten RPC endpoints are necessary for the core journey.

This plan refines the [vision for Embers](../vision.md#embers-interactivity-when-and-where-needed), the [Glaze reference](../reference/glazes.md#embers), [Phase 3](phase-3.md) and [Phase 4](phase-4.md). The [Phase 6 ecosystem specification](phase-6.md) follows it; **do not make Wares, public `kiln test`, project manifests or the registry a prerequisite**.

## 1. Architectural decisions

1. **SSR is the baseline.** Ordinary Glazes render using Kiln's compiled server-side pipeline, with no Wasm or JS bootstrap on pages without interactive components.
2. **Embers are compiled, not interpreted.** Client-reachable Kiln source is checked, lowered through shared typed IR where feasible, then compiled for the browser WasmGC target; no runtime Kiln interpreter or unrelated reactive DSL.
3. **Server/client are distinct trust zones.** Browser code cannot import server-only `sql`, auth internals, native filesystem, secret configuration, OS or arbitrary FFI.
4. **Hydration is selective.** Existing `client:load`, `client:visible`, `client:idle` and `client:only` syntax should have defined loading/fallback semantics; choose a sensible default for an Ember used without a hint.
5. **Typed reactivity.** `{% state %}` is local typed state; `on:event` triggers ordinary Kiln handler logic. Changes update only affected DOM while preserving focus, selection, list identity and nested state where possible.
6. **`server fn` is a checked network boundary.** Derive client proxy, request/response types, codecs and server handler from a single declaration, but still validate all client data and authorize every request.
7. **Explicit async state.** Network failures, cancellation, stale responses, optimistic updates and error recovery must be observable. Never silently repeat unsafe mutations.
8. **Use browser standards and existing tools.** Reuse maintained Wasm and JS interop libraries/bundlers, and permit ordinary HTML/CSS/JS integrations. The implementation is replaceable; Kiln developer-facing semantics are the durable contract.
9. **No premature ecosystem scope.** General multi-folder modules, Wares, official UI Ware, `kiln.toml`, public `kiln test`, formatter and compiler MCP follow in Phase 6.
10. **Testing now, polished runner later.** Extend compiler conformance and real-browser CI during Phase 5. Phase 6 supplies the unified public test runner, **including Ember hydration/RPC from its first release**.

## 2. Target developer experience

A plain SSR Glaze stays unchanged:

~~~glaze
{% props post: Post %}
<article>
    <h2>{post.title}</h2>
    <p>{post.body}</p>
</article>
~~~

An Ember uses familiar Glaze syntax:

~~~glaze
{# Illustrative Phase 5 target; async syntax is not finalized #}
{% props post: Post %}
{% state likes = post.likes %}
{% state pending = false %}

<button disabled={pending} on:click={
    pending = true
    likes = like_post(post.id) catch err { likes }
    pending = false
}>♥ {likes}</button>
~~~

And a server function is typed Kiln:

~~~kiln
// Target contract only; request-bound auth API is not finalized.
server fn like_post(id: Int) -> Int ! {
    let user = try auth.require()
    try auth.authorize(user, "post:like", id)
    try increment_likes(id, user.id)
}
~~~

An SSR page selectively hydrates the button:

~~~glaze
{% props post: Post %}
<article>
    <h1>{post.title}</h1>
    <LikeButton post={post} client:visible />
</article>
~~~

**Important:** The present no-argument `auth.require()` stub is not a design mandate to use global ambient request state. Generated RPC handlers need authenticated **request-bound context**. The correct async/await spelling, task semantics and access to request context must be settled during implementation.

## 3. WasmGC compiler and runtime contract

- Add a browser code-generation target from checked Kiln IR with semantics for structs/records/enums, optionals, generic specializations, closures, strings, collections and error paths. Use browser WasmGC allocation/collection rather than bundling native Boehm GC.
- Build a **client-reachable** call graph separately from server-reachable code. Only needed browser types/functions, island logic and necessary minimal runtime helpers are shipped. This is early reachability/dead-code elimination, not the complete Phase 7 optimizer.
- Track forbidden browser imports/effects during checking and emit source-linked diagnostics. The browser must never gain direct access to server SQL, sessions, secrets or native handles.
- Define compiler-level interop and type conversion for JS/DOM strings, numbers, arrays, records, handles, callbacks and promises, including lifetime/GC and exception propagation.
- Generate content-addressed Wasm and minimal JS bridge/assets through Kiln's existing asset pipeline. Release output must not require a separate JavaScript project or Node runtime on the deployed server.
- Document supported browsers/WasmGC capabilities and feature-detect unsupported platforms gracefully without destroying usable SSR content.
- Source maps/error traces and useful debug symbols should point back to Kiln/Glaze where feasible; don't leak secret-only server code into browser assets.

**Gate:** a real browser runs generated WasmGC from a Kiln program with representative data structures, errors, generic code and closures; C/native and Wasm outputs have shared semantic regression cases.

## 4. Ember islands and hydration

- A Glaze using `{% state %}` or `on:...` becomes an Ember and remains SSR-capable unless `client:only` explicitly opts out.
- Define when `client:load`, `client:visible`, `client:idle` and `client:only` fetch/init an island, including fallback for browsers without the relevant scheduling API.
- Server HTML contains stable island IDs and escaped, explicitly serializable public initial props. Protect against HTML/script context injection (including breaking out of `</script>`), unsafe URL/attribute and malicious database/user-provided content.
- Browser reconciles SSR DOM with typed state. A mismatch is diagnosed, not silently duplicated; stable keys preserve per-item state in lists and avoid accidentally resetting unrelated islands.
- Nested Embers share runtime assets where practical without unintentionally sharing local state. Listener/task cleanup runs when an island unmounts.
- No Ember code must be loaded on an ordinary static-only Glaze route, and ordinary SSR navigation/form submissions should work without JavaScript.

**Gate:** a server-rendered page has two independent Embers with different hydration hints alongside static content; real browsers exhibit correct timing, state isolation and fallback.

## 5. Reactive state, handlers and forms

- `{% state name = expr %}` creates local, typed state initialized consistently with SSR. Writes trigger minimal DOM updates and batched invalidation, with deterministic ordering and no hidden full-page rerender.
- Typed event handlers initially cover click/input/change/submit/keydown and standard event data; provide explicit prevent-default/propagation semantics.
- Respect native controls, labels, IME input, focus, cursor, selection, keyboard accessibility and HTML validation behaviour. Support conditional children, lists and keyed identity.
- Lifecycle/effects/subscriptions need explicit cleanup so removed components do not leak listeners, timers or state updates. No global cross-user component state.
- Phase 4 form validation stays authoritative on the server; client validation and display helpers reuse the same types/rules where feasible, preserving field errors and form data. A normal POST fallback remains viable for progressive enhancement.
- If reactive components need asynchronous work, represent pending/error/success and cancellation as typed, inspectable state rather than magical invisible promises.

**Gate:** interactive form, counter, changing list and nested Ember respond properly; focus remains stable and disposed components don't continue updating.

## 6. Typed server functions and generated RPC

- `server fn` declarations are server-only implementations, with an explicit set of browser-callable exports. Do not accidentally expose every internal helper by generating guessable endpoints.
- Compile serializers/codecs only for safe, serializable public types. Reject DB handles, file descriptors, closures/function pointers, secrets and unsupported native-only types in RPC arguments/results.
- Generate typed client proxies and server routes with consistent message framing/versioning, clear errors, timeouts and cancellation. Runtime still validates untrusted client values, sizes and schema versions.
- Integrate Phase 4 session identity, per-route/per-record authorization, CSRF/origin policy, CORS, rate limits and safe errors. Browser-provided user IDs or claims must not replace authenticated server identity.
- Reject malformed, oversized, unauthorized, stale-version and untrusted inputs safely. Prevent accidental retry of non-idempotent writes.
- Shared type definitions should eliminate duplicate DTOs, HTTP wrappers and JSON encoding glue; no invented client-only model semantics.

**Gate:** a signed-in Ember mutates a record and sees the result; forbidden record access, absent CSRF, forged type, missing session and oversized requests fail.

## 7. Asynchrony, optimistic updates and consistency

- Typed pending/success/failure states and explicit errors for `server fn` calls. Support cancellation on navigation/disposal and documented handling of concurrent responses arriving out of order.
- Allow deliberate optimistic UI with rollback/reconciliation on server failure. Do not silently overwrite fresh state with stale network responses.
- Provide observability for resource caching/invalidation if introduced; avoid a hidden browser ORM cache that users cannot inspect.
- SSE/WebSocket subscription integration may follow as extension if required by dogfood apps, but isn't necessary to mark core typed RPC and state complete.

**Gate:** double clicks, a delayed response, a failed optimistic mutation and navigation away leave client/server consistent and error feedback usable.

## 8. JavaScript and browser interoperability

- Provide typed bindings for practical DOM/browser APIs and safe reuse of small framework-independent JavaScript libraries.
- Use TypeScript declarations for binding generation when mappings are sound, with explicit annotations for unusual APIs; unsupported overloads and Node-only packages should get diagnostics.
- Define string/array/record/object-handle conversion, callback lifetime, errors and promises across JS↔WasmGC boundaries.
- Let existing HTML/CSS/JS work alongside Kiln components; do not claim every npm package (or React-specific component) can run as Kiln/WasmGC.
- Build assets through existing tooling without a developer-maintained Vite/Webpack/npm project; follow normal CSP/origin/browser permission boundaries.

**Gate:** one Ember integrates a browser API/external library with typed calls and handles errors predictably.

## 9. Security, performance and cross-target CI

| Domain | Positive cases | Negative and adversarial cases |
| --- | --- | --- |
| Codegen/GC | Structs, optionals, closures, mutation, errors | Invalid captures, GC lifetime bugs, unsupported browser feature |
| Hydration | SSR match, lazy island, nested list | Duplicate nodes, ID collisions, malformed state, script breakout |
| Events/DOM | Focus, keyboard, conditional updates | Leaked listener, stale DOM, clobbered input |
| RPC | Typed response, database mutation, correct user | IDOR, forged data, missing CSRF/auth, unsafe error leakage |
| Async | Pending, cancellation, optimistic success | Stale response, duplicate mutation, failed rollback |
| Interop | DOM handles, JS callback, promise/error | Disposed handles, Node-only imports, invalid conversion |
| Delivery | No Wasm on SSR-only page, Linux/Windows server | CSP breakage, wrong MIME, excessive unnecessary Wasm |

- Real-browser smoke/regression testing belongs in Phase 5 despite the public `kiln test` runner arriving later.
- Keep C/native server conformance green and test WasmGC semantics as a third target. Browser support is tested explicitly; standalone native application requirements remain.
- Measure emitted Wasm size, initialization/hydration time, memory and DOM updates as a baseline, without requiring premature full compiler optimization.

## 10. Implementation steps and gates

| Step | Scope | Gate |
| --- | --- | --- |
| **1 — WasmGC backend** | Typed IR/codegen, browser runtime subset and compatibility tests | Real-browser Kiln logic executes, comparing well with native |
| **2 — SSR hydration** | Island identities, client asset emission, `client:*` policy and serialization | Two selective islands hydrate safely alongside static content |
| **3 — State and events** | Reactive dependencies, typed handlers, DOM updates and disposal | Counter, list and form update without losing focus |
| **4 — Server functions** | RPC contracts/codecs/server endpoints, auth/CSRF/security | Authenticated record mutation succeeds, hostile calls fail |
| **5 — Async UX** | Pending/cancel, stale results, optimistic rollback | Repeated actions/failure/reorder leave consistent state |
| **6 — JS interoperability** | Browser API/TypeScript bindings and JS bridge | Small external browser library works via typed Kiln |
| **7 — Polish** | Diagnostics/LSP, CSP/assets, cross-target CI, performance basics | Static pages ship no Ember runtime, platform suites pass |
| **8 — Dogfood** | Real Phase 4 DB/auth/forms site with interactive Embers | Full-stack app works entirely through Kiln abstractions |

Each step is a separately reviewable milestone with diagnostics, conformance/browser cases, documentation and acceptance. Avoid bundling the entire browser compiler/runtime into a single untestable PR.

## 11. Definition of done

- [ ] WasmGC executes checked Kiln browser code with coherent semantics for supported types, generics, closures, errors and GC.
- [ ] Server-rendered Glazes remain default; Embers selectively hydrate and don't force full-page JavaScript.
- [ ] State/handlers update DOM predictably with safe lifecycle, list identity and keyboard/focus handling.
- [ ] `server fn` calls compile into typed serialized RPC with failure, cancellation and version checks, without handwritten frontend route wrappers.
- [ ] Phase 4 database/auth/form policies apply to each server function; CSRF/authorization/unsafe-input tests pass.
- [ ] Browser JS interoperability supports practical existing APIs with honest compatibility boundaries.
- [ ] Deterministic Wasm/assets integrate with Phase 3 CSS/Finishes; static-only pages don't emit Ember code.
- [ ] Real-browser/CI regression cases and Linux/Windows native server checks pass.
- [ ] A real database-backed authenticated interactive Kiln app works end-to-end without manually writing client JavaScript or network glue.

## 12. Explicit boundaries

- **Phase 6:** Multi-folder modules, `kiln.toml`/`kiln.lock`, Wares/registry/global cache, official accessible UI Ware, public `kiln test` (with Ember tests **at launch**), formatter/live reload/refactoring and compiler MCP.
- **Phase 7:** Deployment, static-site export, extensive optimization/hardening and long-term compatibility guarantees.
- **Later:** PWA offline/sync, advanced desktop reuse, arbitrary npm/React/Node compatibility, freestanding targets and complete debugger.
- **Non-goals:** Forced SPA routing, client-side access to server data stores/secrets, runtime template interpretation or a mandatory second JavaScript framework.

**In one sentence:** **Phase 5 completes Kiln's full-stack programming capability—compiled SSR by default, browser-native interactivity only when needed, and a typed and secure bridge to the server.**
