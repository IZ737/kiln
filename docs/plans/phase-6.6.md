# Phase 6.6 — Static site generation

**Status:** Done in 0.14.0. This phase comes after Phase 6.5 and Phase 6.5.1 (Ember shared stores), and before Phase 7 (Kiln 1.0 production hardening). It moves static site generation out of Phase 7; deployment automation is explicitly Phase 8. The user guide is [Static sites](../guides/static-sites.md).

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

## Decisions

- **Which routes a full export writes (Jake, 2026-10-27):** every `GET` route without parameters, plus the pages `.static(...)` lists for routes with parameters; `.dynamic()` opts a `GET` route out. Non-`GET` routes are never written. The alternative, exporting only marked routes, was declined as configuration a docs or blog site should not need.
- **Rendering:** the export runs the built program with `KILN_EXPORT` set, and `app.listen` renders each page in-process through the ordinary dispatch path (hooks, handlers, Glazes, `on_error`) instead of binding a socket. No port is opened, so exports cannot clash with other servers.
- **Typed page lists:** `app.get` returns `web.Route<P>`, where the checker computes `P` from the pattern: `Unit` without parameters, the parameter's type for one, and a record named by the parameters for more. `.static(fn() { ... })` returns `List<P>`, so a wrong page value is a type error. E0237 reports a list on a route without parameters, or a missing one.
- **Layout:** `/a` is written as `a/index.html`, which every static host serves; a path whose last part has a `.` keeps its name; redirects become meta-refresh pages, except a redirect to the same file (`/docs` to `/docs/`); `on_error`'s 404 page becomes `404.html`.
- **Checks:** a page that reads the request (query, headers, cookies, body), answers other than 200 or a redirect, or is non-HTML at a path without an extension is E0404; an Ember on a written page whose handlers call a `server fn` is E0405, at the call in the `.ember` file; links, forms, images, scripts and stylesheet `url()`s that point at a page the export lacks, or forms that `POST`, are E0406, reported once per target.
- **Mixed mode:** in a running server, `.static` pages are rendered when the server starts, before it accepts connections, and answered from memory after `app.before` hooks; `refresh:` renders a page again on the first request after it expires.
- **Output folder:** `dist/` in the program's folder by default. kiln replaces a folder it wrote (recorded in `.kiln/fire-static.json`) and refuses any other non-empty folder.
- `kiln fire` without `--static` builds the executable as `kiln build` does, until Phase 7 makes it the optimized release build.

## Evidence

- `examples/static_blog` exports with `kiln fire --static`; `tests/browser.rs` serves the result with a plain file server in the test (no Kiln program) and checks styles, image, listed pages, the redirect page, `404.html`, the feed and Ember hydration in Chromium.
- `tests/fire.rs` checks the written files, replacing an earlier export, refusing a foreign folder, and every error above with its source location.
- `conformance/web_static` runs mixed-mode prerendering, unlisted pages, multi-parameter pages and `refresh` on both backends (and on Windows through the conformance scripts). CI's Windows job exports the example with `kiln.exe`.
- The docs site marks its documentation pages `.static(...)`, so a running site serves them from memory; its search Ember calls a `server fn`, so `kiln fire --static site` reports E0405 by design.

## Explicit boundaries

- **Phase 7:** Optimized native `kiln fire` release builds, LLVM, precise garbage collection, performance and stability for Kiln 1.0.
- **Phase 8:** `kiln deploy`, `kiln release`, `kiln-deploy.toml`, remote hosts, migration orchestration, health checks and rollback/approval safeguards.
- **Phase 9+:** Installable PWAs and local-first offline synchronization.

Static output can be uploaded to a host with existing tools; no integrated deployment command is necessary to complete this phase.
