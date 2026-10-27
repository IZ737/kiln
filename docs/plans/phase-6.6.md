# Phase 6.6 — Static site generation

**Status:** Planned. This phase comes after completed Phase 6.5 **and planned Phase 6.5.1 Ember shared stores/reactivity**, and before Phase 7 (Kiln 1.0 production hardening). It moves static site generation out of Phase 7; deployment automation is explicitly Phase 8.

## Goal

Export a server-rendered Kiln site to ordinary static HTML, CSS, JavaScript bootstrap and WasmGC assets with `kiln fire --static`, suitable for any static web host or CDN. Static generation must work without implementing Phase 7's LLVM-backed optimized native release build pipeline or Phase 8 deployment commands.

## Scope

1. **Static export:** Render statically known routes at build time, write predictable output paths and include referenced styles, images, fonts, and other public assets with working URLs.
2. **Routes with parameters:** Allow routes to provide an explicit finite set of build-time parameter values (for example via `.static(...)`) rather than guessing which URLs exist. Reject ambiguous or unbounded routes in full-static export with actionable diagnostics.
3. **Server-side rendering:** Use the existing Glaze rendering path and preserve layout, slots, escaping, theme styling and HTML semantics. Do not require a resident Kiln server after export.
4. **Embers:** Preserve server-rendered markup and bundle the selected WasmGC hydration assets for interactive islands that can run entirely in a static host.
5. **Server-only boundaries:** Detect and reject static pages or Embers that require server functions, authenticated RPC or other unavailable server behaviour. Do not silently emit broken client interactions.
6. **Mixed-mode prerendering:** Define and test an explicit way to prerender selected routes in an otherwise dynamic Kiln app. A static-only export must not promise that dynamic endpoints or server functions work without a server.
7. **Tooling and documentation:** Make the command discoverable; document build output, hosting assumptions, route enumeration, limitations and error messages. Test Linux and Windows build paths where supported.

## Acceptance

- A documentation/blog example exports with `kiln fire --static`, and the resulting directory can be served by a generic static HTTP server with no Kiln runtime.
- Static routes, parameterized routes with supplied values, images, CSS, themes and assets render correctly.
- A client-only counter Ember hydrates and works on an exported page without handwritten application JavaScript.
- A page requiring typed server RPC fails static export with a clear, source-linked diagnostic instead of a broken artifact.
- A mixed site can prerender opted-in pages without incorrectly exporting dynamic routes.
- Conformance, regression and representative browser tests cover path handling, links, output integrity and unsupported cases.

## Explicit boundaries

- **Phase 7:** Optimized native `kiln fire` release builds, LLVM, precise garbage collection, performance and stability for Kiln 1.0.
- **Phase 8:** `kiln deploy`, `kiln release`, `kiln-deploy.toml`, remote hosts, migration orchestration, health checks and rollback/approval safeguards.
- **Phase 9+:** Installable PWAs and local-first offline synchronization.

Static output can be uploaded to a host with existing tools; no integrated deployment command is necessary to complete this phase.
