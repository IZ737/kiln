# Phase 6.5.1 — Ember shared stores and reactivity alignment

**Status:** Planned; the **immediate next phase** after completed Phase 6.5 and before [Phase 6.6 static site generation](phase-6.6.md). This phase restores previously agreed Ember design that did not land in the current implementation. It is implementation work, not a new brainstorming/review phase.

## Why this phase exists

The implemented [Ember reference](../reference/glazes.md#embers) describes instance-local `{% state %}`, SSR/WasmGC hydration, handlers and typed RPC. It does **not** specify or expose importable shared reactive stores, optional scoped overrides, per-request store isolation, or client-side-navigation lifetime rules. These were agreed while designing Embers; completing Phase 4 and Phase 6 did not imply they shipped.

**Design authority:** Preserve the confirmed decisions below. Examples of syntax are **illustrative** until the parser, checker, native/Wasm backends, runtime and docs agree. Do not present a proposed API spelling as an already ratified interface.

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

## Existing syntax discrepancy to reconcile before 1.0

The earlier Ember design also **accepted top-level `{% var %}` for reactive component-local state** instead of `{% state %}`, and distinguished mutable reactive roots from ordinary nonreactive values. However, Phase 4/4.1 *implemented* `{% state %}` and documented it as the public syntax.

This phase must explicitly reconcile that mismatch: evaluate migration to the previously agreed `{% var %}` syntax against actual compiler/Glaze compatibility, settle the canonical pre-1.0 form in a recorded decision, update examples/reference/LSP/formatter/diagnostics, and add a transition diagnostic if a spelling is removed. **Do not silently claim `{% var %}` already works or change the existing syntax without documenting the break.** The shared-store behaviour above is not conditional on which spelling wins.

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
7. **Syntax alignment:** Resolve the `{% var %}` versus implemented `{% state %}` discrepancy above with an explicit decision and migration/tests if changed.
8. **Developer experience:** Update reference, design/decision log, introductory examples, docs site, LSP/VS Code highlighting/completion and `kiln test` browser/conformance coverage; keep static Glazes free of unnecessary Ember runtime assets.

## Completion gate

- A realistic multi-folder app declares one typed store and **two unrelated Embers** both read and mutate it without context providers or prop threading; a nested list/record mutation automatically updates the other Ember.
- Multiple writes batch predictably, do not reset unrelated local state, and preserve focus and keyed children.
- Request A's server render/hydration never sees request B's state; explicit test scopes can override one store independently.
- Opt-in client navigation preserves the store; a normal full reload resets it, and explicit persistence alone enables reload retention.
- Removed components unsubscribe and cancel their own work, while correctly owned app/store work survives internal navigation and is cleaned up at teardown.
- All accepted pre-1.0 syntax decisions are reconciled with implemented `{% state %}`, and unsupported forms produce intentional diagnostics rather than silent mismatches.
- Browser tests, project/module tests, server isolation tests, and native/Wasm conformance pass; current example apps and Glazes keep working or get documented migrations.

## Explicit exclusions

This is **not** an always-on SPA router, automatic offline storage/sync, arbitrary browser-wide mutable globals, an unreviewed new frontend framework, or a new client compilation target. Phase 6.6 remains static site generation, Phase 7 remains 1.0 hardening, and Phase 8 remains deployment automation.
