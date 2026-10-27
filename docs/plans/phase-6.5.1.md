# Phase 6.5.1 — Ember shared stores and reactivity alignment

**Status:** Done (release 0.13.0). The contract below was implemented as `store` declarations; the [reference](../reference/glazes.md#shared-stores) is the current documentation, and the implementation notes at the end record how each checkpoint was met.

## Why this phase exists

The implemented [Ember reference](../reference/glazes.md#embers) describes instance-local `{% state %}`, SSR/WasmGC hydration, handlers and typed RPC. It does **not** specify or expose importable shared reactive stores, optional scoped overrides, per-request store isolation, or client-side-navigation lifetime rules. These were agreed while designing Embers; completing Phase 4 and Phase 6 did not imply they shipped.

**Design authority:** Preserve the confirmed decisions below. **`{% state %}` is the confirmed and already implemented declaration for reactive component-local Ember state.** Its spelling is settled: do not rename it to `{% var %}`. Shared-store API examples remain illustrative until the parser, checker, backends, runtime and docs agree.

## Previously agreed behaviour (non-negotiable)

1. **Importable, Zustand-style shared stores.** Declare a store once in an ordinary Kiln module and import/use it directly from unrelated Embers. No mandatory React-style provider/context hierarchy, special root component, or prop threading. The same store identity is shared across an application instance.
2. **Typed, explicit reactive roots and automatic deep reactivity.** A value becomes a tracked reactive root by an explicit declaration/creation step, not because all ordinary Kiln values are implicitly reactive. Within that root, nested structs/records/lists/maps react automatically to meaningful writes; no manual `notify()`, replacement of every ancestor, or one-signal-per-field boilerplate. Track reads so only affected subscribers rerender.
3. **Batched DOM updates.** Multiple synchronous writes in a handler are batched into a consistent update, preserving user input, focus, keyed identity and unrelated component state; render/effect ordering must be deterministic and tested.
4. **Application lifetime across client navigation.** Stores survive **opt-in client-side navigation** between pages in the same application. Client navigation is itself opt-in; ordinary links/full navigation continue to work without a SPA runtime. A full document reload starts with fresh store state unless persistence has been opted into.
5. **Persistence is opt-in.** No automatic disk/local-storage persistence or cross-session sync. Any opt-in persistence must specify data selection, serialization, storage ownership, versioning/migration and sensitive-data safeguards. Keeping state during client navigation is not the same as persisting state between reloads.
6. **SSR isolation per request/user.** Store state created for SSR must not be a single mutable process-global shared by requests or users. Initialize scoped state per request/render, serialize only safe typed hydration snapshots, and connect that snapshot to the correct browser application instance without leaking another visitor's data.
7. **Optional scoped context/overrides.** Normal usage needs no context provider, but explicit scoping/overrides should be available for tests, nested app instances and deliberate subtree isolation. This must not make ordinary stores dependent on provider boilerplate.
8. **Local Ember state remains local.** Different instances don't accidentally share instance state; prop changes do not reset it, while removal/disposal and changed `key=` identity have defined cleanup/reset behaviour. Shared store changes should not force wholesale island recreation.
9. **Tasks and subscriptions have owners.** Component-owned reactions/work cancel/dispose with the component. Any store-owned work has an explicit lifetime and must not be accidentally cancelled on client navigation or leak after application teardown. Preserve explicit cancellation and observable error behaviour.

## Settled syntax — keep `{% state %}`

**Decision (2026-10-27): Keep `{% state %}` for reactive component-local Ember state.** The earlier proposal to use top-level `{% var %}` was reconsidered and rejected. The explicit `state` marker makes it immediately obvious to someone scanning a component which declarations participate in reactivity and are retained as component state.

- `{% state count = 0 %}` is the canonical, already implemented Ember-local reactive declaration. **Do not replace it with `{% var %}`.**
- Ordinary `var` continues to mean a mutable local binding in Kiln; mutability alone does not imply reactive component state. Ordinary `{% let %}` remains nonreactive.
- Shared stores must likewise opt into reactivity explicitly through their store/reactive-root definition. A `let` binding that refers to such a store can still refer to reactive data; the binding keyword itself is not what makes it reactive.
- Preserve existing source compatibility: no migration, deprecation, alternative synonym, or formatter rewrite from `state` to `var` is required. Keep compiler, documentation, highlighter and LSP support for `state`.

This phase implements the missing shared-store semantics **without reopening the local-state keyword choice**. See [the language decision log](../decisions.md).

## Candidate developer experience (API spelling not finalized)

~~~kiln
// app/cart.kiln — illustrative
pub struct Cart {
    items: List<String> = []
}

pub let cart = reactive.store(Cart())
~~~

~~~glaze
{# cart_badge.ember — illustrative, not yet supported #}
{# importing app/cart follows the chosen module/component visibility rules #}
<span>{cart.items.len()}</span>
~~~

~~~glaze
{# cart_button.ember — illustrative, not yet supported #}
<button on:click={ cart.items.push("Book") }>Add</button>
~~~

The intent is: both Embers reference the **same store**, updates in one automatically rerender the other, and neither needs an intervening provider. Decide exact import/scope syntax, write policies, reactive collection support and SSR initialization in the implementation contract.

## Implementation checkpoints

1. **Semantics first:** Define store initialization, identity, read dependency tracking, nested mutations, batching, derived reads, disposal, error paths, snapshots, module boundaries and invalid/unsupported usages. Specify interaction with Kiln's ordinary shared-reference values.
2. **Compiler and runtime:** Add type-checked store declarations/access in local modules and Wares; implement SSR and WasmGC store runtime with consistent semantics. Ensure server-only APIs and sensitive values never become browser globals.
3. **Cross-island updates:** Demonstrate two independent Embers on one page reading/writing one store, deep list/record mutations, dynamic subscriptions and precise/batched invalidation.
4. **Scope and SSR:** Prove parallel users/requests get independent state, hydration uses the corresponding typed snapshot, and explicit overrides can isolate tests or nested applications.
5. **Navigation lifetime:** Provide an **opt-in** client-side navigation path, keep application store identity across it, and specify cleanup/cancellation and fallback to normal navigation. Verify that a full reload resets state by default.
6. **Optional persistence:** Supply or formally define an explicit persistence mechanism, with tests for reload opt-in and no accidental persistence of server secrets. Do not turn persistence or syncing on by default.
7. **Syntax consistency:** Preserve the settled `{% state %}` declaration for component-local reactivity; test that ordinary `let`/`var` retain their existing semantics and that shared-store reactive roots are explicit. Do not add `{% var %}` as a competing reactive spelling.
8. **Developer experience:** Update reference, design/decision log, introductory examples, docs site, LSP/VS Code highlighting/completion and `kiln test` browser/conformance coverage; keep static Glazes free of unnecessary Ember runtime assets.

## Completion gate

- A realistic multi-folder app declares one typed store and **two unrelated Embers** both read and mutate it without context providers or prop threading; a nested list/record mutation automatically updates the other Ember.
- Multiple writes batch predictably, do not reset unrelated local state, and preserve focus and keyed children.
- Request A's server render/hydration never sees request B's state; explicit test scopes can override one store independently.
- Opt-in client navigation preserves the store; a normal full reload resets it, and explicit persistence alone enables reload retention.
- Removed components unsubscribe and cancel their own work, while correctly owned app/store work survives internal navigation and is cleaned up at teardown.
- `{% state %}` remains the canonical and documented local reactive-state syntax, with no `state` → `var` migration; ordinary `let`/`var` do not silently become reactive component declarations.
- Browser tests, project/module tests, server isolation tests, and native/Wasm conformance pass; current example apps and Glazes keep working or get documented migrations.

## Explicit exclusions

This is **not** an always-on SPA router, automatic offline storage/sync, arbitrary browser-wide mutable globals, an unreviewed new frontend framework, or a new client compilation target. Phase 6.6 remains static site generation, Phase 7 remains 1.0 hardening, and Phase 8 remains deployment automation.

## Implementation notes

**Syntax (decided 2026-10-27):** `[pub] store name: Type = value [persist "key"]`, a top-level declaration beside `const`. Embers, handlers and server code read and change it by name (`shop.cart.lines.push(x)`, `shop.cart = shop.Cart()`); `{% state %}` is unchanged.

- **Semantics.** A store's value lives in the running scope's table: one per request on the server (started in `k_web_handle`, kept per task across `k_task_switch`), one per page in the browser. It starts from its declared value on first read. The type must be written (E0236) and have a JSON form (E0224). A store is browser-visible state: never put secrets in one.
- **Compiler and runtime.** The IR has `StoreSlot`/`SetStore`; each store gets a `<store name>` getter. The C and native backends use `k_store_slot` in `kiln.c`; WasmGC keeps one global per store and reports reads to the bridge (`store_read`). The browser module exports `__kiln_store_names`, `__kiln_store_json` and `__kiln_store_load`.
- **Cross-island updates.** After each event or reply, the bridge compares each store's JSON with what the islands last saw and rerenders, once and in page order, the islands that read a changed store (reads are tracked per render, so subscriptions follow what an island actually read). Comparing values catches changes through any alias without instrumenting writes.
- **Scope and SSR.** Each request has fresh stores; a page with embers carries the stores its request started in `<script type="application/json" id="kiln-stores">` (with `<` escaped), and hydration starts from it. `stores.scope(fn() { ... })` runs code with fresh stores and restores the outer ones after, in every backend; tests use it to give a store their own value.
- **Navigation.** `client:navigate` on a link or an element around links loads same-origin HTML pages into the document (title, body, new stylesheets; history and back/forward), disposes the old islands and their calls, keeps started stores and starts new ones from the new page's snapshot. Anything else falls back to an ordinary load. A page with no embers loads no script, so its links stay ordinary.
- **Persistence.** `persist "key"` keeps a store in `localStorage` as `kiln:store:<key>`, written when it changes and read before the page's snapshot on a full load; a value that no longer decodes is dropped. Changing the key is the migration for an incompatible type.
- **Tests.** `conformance/stores` (both native backends), `tests/browser/stores` with `stores.mjs` (two unrelated embers on a multi-folder app, batching, focus, navigation, reload, persistence, overlapping requests) and its `kiln test` unit tests, parser and grammar tests.
