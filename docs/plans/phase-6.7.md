# Draft RFC — Optional filesystem routing (candidate Phase 6.7)

**Status: Design in progress; not approved for implementation.**  
**Last discussed: 2026-10-27.**  
**Roadmap placement: candidate Phase 6.7, not yet a committed prerequisite for 1.0.**  
**Implementation: none.** All API examples below are proposed, not valid Kiln code today.

This is a **living design document**. Update this PR as decisions are made. Do not infer acceptance of an item merely because it appears in an illustrative example. The [current HTTP API](../reference/stdlib.md#web), [project/module model](../reference/projects.md), [Glazes/Embers](../reference/glazes.md), and [static-site behaviour](../guides/static-sites.md) remain authoritative until implementation and documentation are updated.

## 1. Why this exists

Kiln is web-first and aims to eliminate unnecessary glue and boilerplate. Many sites should be expressible by placing a page in a folder, without manually registering a handler for every static page. At the same time, Kiln must preserve explicit, typed code-defined routing, custom application structure and the ability to mix both approaches.

**Goals**

1. Opt-in route discovery from a specifically mounted project directory.
2. **File-first** pages: a single `.glaze` file should suffice for an ordinary page, while more complex routes may use a directory and a `.kiln` handler.
3. One routing implementation: filesystem discovery produces registrations with the **same semantics** as `app.get`, `app.post`, etc.; no parallel request pipeline.
4. Predictable compilation, type checking, diagnostics, security, assets, Ember hydration and SSG.
5. Source location and editor navigation remain understandable.
6. Import aliases and reusable components work independently of whether a directory is routed.

**Non-goals (initial version)**

- Mandatory or globally enabled filesystem routing.
- A new standalone frontend/router framework or new Glaze/Ember syntax for the sake of routing.
- Runtime filesystem scanning on each HTTP request.
- Implicit routing of every Glaze or Ember in a project.
- Automatic nested layouts, group syntax, special error files, catch-all segments, or filesystem-based middleware before their designs have been considered.
- A separate static-export engine.

## 2. Agreed directions (2026-10-27)

These are decisions reached in the design discussion, not claims of implemented APIs.

| Topic | Direction |
| --- | --- |
| Activation | **Opt-in**, per route directory; keep existing code-defined routes |
| Routing model | **Option A: file-based pages**, with optional route directories for complex routes |
| Route discovery | **Build-time/project-aware**, not per-request directory scanning |
| Mounting | Explicit mount such as `app.file_routes("pages")`; optional URL prefix per mount |
| Default excluded directory | Directories named **`_components`** are excluded from route discovery at **any depth** under a mounted directory |
| Override | The mount may specify an **`ignore` list**, replacing the default exclusions with developer-selected directory names |
| Reuse | Excluded files are still importable/compilable; exclusion only changes route discovery |
| Scope | The `_components` convention is specific to filesystem routing; it is **not** a reserved Kiln folder elsewhere |
| Explicit routes | Coexist with discovered routes; preserve the current router's registration-order behaviour |
| Shared infrastructure | Reuse current typed handlers, Glazes, Embers, hooks, SSG and error reporting |
| Page data model | **Three levels:** automatic Glaze with typed URL props → optional typed `load` returning page props → ordinary full `get` handler |
| GET ownership | **`load` and `get` are mutually exclusive within one route**; `post`, `put`, `patch` and `delete` may coexist with `load` |
| Conventional logic file | **`route.kiln`** in the complex route's directory; exported `load` or HTTP-method functions live there |
| Loader arguments | **Optional `req: web.Request` first**, followed by **typed path parameters matched by name**; request-free loaders need not mention `req` |
| Glaze prop assembly | **Automatically combine typed URL path parameters with the `load` return record**; duplicate field names are a compile-time error |
| Parameter typing | **Infer `[id]` from the Glaze, `load`, or route handler declarations**, defaulting to `String` when no explicit type is supplied; conflicting declarations are compile-time errors |
| Loader failures | **Reuse ordinary Kiln error propagation**: `web.error(status, message)` for intentional HTTP failures, existing 500 handling for unexpected errors, and `app.on_error` for custom responses; **no implicit `none` → 404** |

The mount API, URL filename conventions and exact handler signatures below are **provisional** until decided. The **`route.kiln`** file name and its use for the `load`/HTTP-method exports are agreed.

## 3. Proposed opt-in API

```kiln
import web

fn main() ! {
    let app = web.app()

    // Enable discovered pages under "/" for this app only.
    app.file_routes("pages")

    // Mix in ordinary explicit routes.
    app.get("/health", fn(req) { return web.text("ok") })

    try app.listen(":8080")
}
```

Mounting another directory under a URL prefix, and choosing another exclusion convention:

```kiln
app.file_routes("admin/pages", at: "/admin", ignore: ["components", "partials"])
```

**Proposed semantics**

- The directory operand is resolved against the **Kiln project root**, not process working directory. It must remain within the allowed project/workspace boundary.
- A mount specifies where routes come from and (optionally) the URL path prefix; a mount does **not** change Kiln's module/import rules.
- Both source directory and route prefix should be statically knowable to the compiler. Precise syntax/validation of constant expressions is TBD.
- **Default:** exclude every directory named `_components`, recursively. No need to write an option for normal usage.
- **Override:** `ignore: ["components", "partials"]` replaces the default list, so `_components` is no longer excluded unless included explicitly. `ignore: []` opts out of this exclusion rule. Names are **directory basenames**, not globs or arbitrary path expressions, for the first version.
- Ignore rules apply per mount, not globally. Nested files in an ignored directory do not claim URLs. The compiler/LSP must still be able to resolve and import those files when referenced.
- Define and reject overlapping/ambiguous mounts deterministically; do not silently hijack routes.

## 4. File-first layout: provisional naming convention

```text
my-app/
├── kiln.toml
├── main.kiln
├── pages/
│   ├── index.glaze                  → GET /
│   ├── about.glaze                  → GET /about
│   ├── contact.glaze                → GET /contact
│   ├── blog/
│   │   ├── index.glaze              → GET /blog
│   │   ├── [slug].glaze             → GET /blog/{slug}
│   │   └── _components/
│   │       └── post_card.glaze      → NOT a route
│   └── _components/
│       └── navbar.glaze             → NOT a route
├── components/
│   └── button.glaze                 → NOT a route (outside mount)
└── layouts/
    └── main.glaze                   → NOT a route (outside mount)
```

**Candidate mapping rules**

- `index.glaze` maps to the directory's URL; a top-level index maps to `/`.
- `about.glaze` maps to `/about`.
- `blog/index.glaze` maps to `/blog`.
- `[slug].glaze` maps to a parameter segment named `slug`. **Agreed: the segment's type comes from matching Glaze, loader or handler declarations**, not filename type markers; where none specifies a type, it defaults to `String`. The exact `[name]` delimiter convention remains subject to final route-naming review.
- For complex routes, a directory may instead contain `index.glaze` plus **`route.kiln`** when route-specific code is needed (see §5). For instance `blog/[slug]/index.glaze` would map to the same path as `blog/[slug].glaze`; including both is an error.
- Plain components outside mounted directories are not route candidates. Only **mounted directories** are scanned.
- The initial automatic-page convention is for **server-rendered `.glaze` pages**; `.ember` components can be embedded normally. Whether standalone `.ember` files may directly claim routes is an **open question**.

**Routing collisions and path correctness**

- Reject duplicate method/URL mappings discovered in the same mount, including file-versus-directory equivalents.
- Within one mount, a literal path (e.g. `/blog/new`) must take precedence over the corresponding parameterised path (e.g. `/blog/{slug}`). Specify the ordering during route-table generation.
- The existing router dispatches **the first matching route**. Do not silently change global precedence for applications using explicit `app.get` etc. A mount occupies its position in explicit registration order; interactions between multiple mounts need precise tests and documented rules.
- Preserve URL percent-decoding, typed route parameter matching, HEAD→GET behaviour, 404/405 responses, `app.before` hooks, and server-side security rules.
- Reject path traversal, unexpected symlinks, case-folded collisions and invalid parameter names. Treat case sensitivity consistently across supported operating systems.
- Consider overlap/conflicts with reserved Kiln endpoints such as `/_kiln/rpc` during implementation review.

## 5. Three-level page model — agreed direction; syntax provisional

**Decision (2026-10-27):** Support **all three complementary levels**, so complexity is opt-in:

1. **Just a Glaze:** pages with no custom data requirements are discovered and rendered automatically. Where a page's path contains parameters, Kiln automatically provides matching, **type-checked** URL parameter props.
2. **Glaze + typed loader:** when data must be retrieved or transformed before rendering, an optional `load` function supplies a typed record of Glaze props. Kiln handles the ordinary rendering step; no repetitive `web.render` call is required.
3. **Full handler:** when the application needs direct request/response control (authentication, redirect/status/headers, unusual content types, etc.), an ordinary typed `get` handler constructs the response explicitly using existing `web` functions.

**The convention is agreed:** route-specific logic lives in a file named **`route.kiln`** inside the relevant route directory. That file may export **`load`** for typed page data or ordinary HTTP-method functions such as **`get`**, **`post`**, **`put`**, **`patch`** and **`delete`** for direct request/response handling. 

The **filename, general roles, `load`/`get` exclusivity, optional request/typed path-parameter argument model, and standard Kiln error propagation** are decided; detailed return contracts, exports/visibility and other edge cases remain open. Examples below are illustrative Kiln-like pseudocode and **are not currently implemented**.

### Level 1 — just a Glaze

```text
pages/
  about.glaze        → GET /about
  blog/
    [slug].glaze     → GET /blog/{slug}
```

```glaze
{# pages/blog/[slug].glaze — illustrative #}
{% props slug: String %}
<h1>Article: {slug}</h1>
```

For a request to `/blog/hello`, `slug` is supplied from the typed route parameter. A Glaze with other **required** props and no loader/handler must yield a helpful compile-time diagnostic; Kiln must not invent values. Pages without parameters and without required external props need no route code.

### Level 2 — optional typed loader

A page with dynamic data may graduate to a route directory, retaining the same public URL:

```text
pages/
  blog/
    [slug]/
      index.glaze
      route.kiln          # agreed conventional logic filename
```

```kiln
// Illustrative only: return/error contract remains under discussion.
import posts

pub fn load(slug: String) -> { post: posts.Post } ! {
    return { post: try posts.find(slug) }
}
```

```glaze
{% props post: posts.Post, slug: String %}
<article><h1>{post.title}</h1><p>Slug: {slug}</p></article>
```

**Decision (2026-10-27) — Glaze props, Option A:** Kiln automatically combines the **typed URL path parameters** with the **typed record returned by `load`**, then supplies that combined record to the Glaze. In the example, `slug` comes from the path and `post` comes from the loader; the loader is not required to repackage the slug. This applies regardless of whether `load` itself declares/uses the corresponding parameter.

- **Duplicate names are a compile-time error**, even if the values or types happen to agree. E.g. `load` returning `{ slug: "other", post: ... }` for a route with `[slug]` must be rejected, rather than choosing an override order.
- The combined prop names/types are checked against the Glaze's declared props; required props that cannot be supplied remain build errors.
- Glaze defaults continue to work for props not supplied by either source. The precise policy for unexpected extra loader fields and loader return-shape constraints is still to be specified.
- Parameter name uniqueness and type compatibility should be enforced in the shared route/compiler checker, not by runtime maps with silent shadowing.

Kiln type-checks that the returned record satisfies the paired Glaze's required props and renders it automatically.

**Decision (2026-10-27) — loader input contract, Option A:** the loader may declare an **optional first argument** of type `web.Request`; after that, it declares whichever typed URL path parameters it needs, bound **by parameter name**, rather than by positional URL order. A loader that only needs URL parameters may omit the request entirely:

```kiln
// Conceptual; assumes a route with [slug].
pub fn load(slug: String) -> { post: Post } ! {
    return { post: try posts.find(slug) }
}
```

A loader that needs authentication, cookies, headers or other request context may explicitly request it:

```kiln
// Conceptual; exact error type and imports intentionally omitted.
pub fn load(req: web.Request, slug: String) -> { post: Post } ! {
    let user = try sessions.require(req)
    return { post: try posts.find_for(user, slug) }
}
```

A valid path parameter must match a named segment in the discovered URL pattern and have a compatible type with Kiln's existing typed route rules. Unsupported types, unknown names, duplicate parameter declarations, and a misplaced request parameter must fail with clear source-linked diagnostics. The **special request argument is explicitly opt-in**; it is not made available implicitly through magic globals. **The type is inferred from declarations in the Glaze, loader or route handler; otherwise it defaults to `String`.** Whether every URL parameter must be consumed by the loader/handler remains under design.

The presence of `req` does **not** imply a page is exportable as a static file: reuse existing Phase 6.6 analysis/checks of request-dependent behaviour. Conversely, merely accepting an optional `req` does not itself define a new static-export policy. Loader error handling is defined in §6 and uses existing Kiln semantics.

A loader is **optional**, not required for a parameterised route whose Glaze can render from URL props alone. Prefer no implicit database calls, request mutation or hidden runtime conventions.

**Agreed GET ownership rule:** a `load` export and a `get` export **must not coexist in the same `route.kiln` for the same route**. The compiler must reject this with an actionable, source-linked diagnostic: `load` already defines the automatic GET page response; use `get` instead for full response control, or remove `get` to keep automatic rendering. Other HTTP-method exports (such as `post`, `put`, `patch` and `delete`) **may coexist** with `load` and handle their own methods using the normal router. This avoids a hidden call order or two competing GET implementations.

### Level 3 — ordinary full handler

```kiln
// Illustrative only: precise handler signature/module boundaries TBD.
import web, posts

pub fn get(req: web.Request, slug: String) -> web.Response ! {
    let post = try posts.find(slug)
    return web.render("index.glaze", { post })
}
```

This handler uses the existing request, typed path parameters, response types, hooks and error behaviour. Additional `post`, `put`, `patch` and `delete` handlers should eventually use the same ordinary routing semantics, subject to the handler convention being settled. A full handler may return something other than HTML.

**Design constraint:** these levels describe progressively greater *control*, not separate frameworks. Generate ordinary route registrations for all three. Do not create a parallel rendering, middleware, authentication or static-export pipeline.

## 6. Parameters, props, precedence and errors — remaining design work

### Loader errors — decided (Option A, 2026-10-27)

**A loader uses the same error mechanism as any ordinary Kiln function called by a route handler.** No filesystem-routing-specific result/error protocol is introduced.

- A loader can use `try`, `throw` and the usual fallible function semantics. An intentional HTTP failure uses **`web.error(status, message)`**. For example, `throw web.error(404, "Post not found")` produces the existing 404 handling, and `web.error(403, "Forbidden")` uses the existing forbidden response.
- **Unexpected errors** follow the ordinary server error path (normally 500, with internal error details not exposed). `app.on_error` applies to loader-generated responses just as it does for explicitly registered routes; other existing error middleware and hooks remain in effect.
- **Returning `none` does not implicitly produce HTTP 404.** Kiln must not silently treat optional loader results as missing pages. The loader should deliberately fail with `web.error(404, ...)` or use a full `get` handler for custom response construction.
- Loader success returns a typed record to be merged with route parameters for Glaze rendering. Whether the compiler should accept or reject an optional *successful* record return type is a separate typing decision; it must never silently implement a 404 conversion.
- Preserve current response formatting and content negotiation for errors; do not create parallel Glaze/file-route errors. Static export continues to obey existing Phase 6.6 error/output checks, so exporting an error page is not silently considered a successful route.

**Conceptual example (error type/signature spelling to be verified against current language rules):**

```kiln
import web, posts

pub fn load(slug: String) -> { post: posts.Post } ! {
    let post = try posts.find(slug)
    if not post.is_some() {
        throw web.error(404, "Post not found")
    }
    return { post: post.unwrap() }
}
```

The main contract is the error semantics, not the particular lookup API or example's optional-value syntax.

### Remaining contracts

The **three levels above are agreed**; their other exact contracts need decisions:

- **Route parameters — DECIDED:** the filename supplies **only a parameter name**, e.g. `[id]`; the compiler **infers its type** from matching declarations in the Glaze, `load` or full route handler. If no declaration supplies an explicit type, the parameter is a **`String`**. If two or more declarations disagree (e.g. `id: Int` in a loader and `id: String` in the Glaze), this is a **compile-time error**, not implicit coercion or precedence. Keep existing route matching/conversion rules for supported types (`Int`, `Float`, `Bool`, `String`). `load` still binds by parameter name and accepts optional first `req: web.Request`. Whether unused path parameters must appear in a handler signature remains open; all explicit declarations must agree.
- **Prop combination — DECIDED:** the Glaze receives the **union of URL parameter props and the loader-returned typed record**. Duplicate names across those sources are **compile-time errors**, even for equal values/types. Missing required props are errors and existing Glaze defaults apply when a prop is not supplied. Still decide whether unused/extra loader fields are allowed and what loader return types qualify.
- **Handler discovery:** **`route.kiln` in the route directory is settled.** Determine its visibility/export rules, signature typing, how it participates in ordinary folder-module imports/privacy and which declarations may be used as route handlers.
- **`load` vs `get` — DECIDED:** they are **mutually exclusive per route** and must trigger a **compile-time error** when both are defined. `load` may coexist with non-GET method exports (`post`, `put`, `patch`, `delete`). Do not invent fallback or invocation ordering between `load` and `get`.
- **Failure handling — DECIDED:** reuse `web.error(status, message)`, ordinary `try`/`throw`, existing unexpected-error (500) handling, and `app.on_error`; **do not** convert `none` to 404 automatically. Authentication and authorization failures must be propagated through this same path. Exact examples and language type signatures still require implementation-time validation.
- **HTML and non-HTML:** only a route designed for page rendering should receive automatic Glaze behaviour. A full handler can return arbitrary `web.Response`.
- **Upgrade path:** moving `blog/[slug].glaze` to `blog/[slug]/index.glaze` plus `route.kiln` should preserve `/blog/{slug}`; defining both forms simultaneously is a deterministic collision error.
- **Testing:** include simple direct parameter props, typed loader props, full handlers, invalid prop/type contracts, access control, errors, cross-folder imports and consistent static export behaviour.

## 7. Layouts, components, aliases

- For the first version, Glazes continue to use explicit `{% layout "..." %}`, slots and ordinary component tags. **Do not silently introduce automatic nested layout inheritance**.
- A routed Glaze may embed an Ember using existing rendering/hydration behaviour. A page without Embers must not gain an unnecessary client runtime.
- Cross-folder Glaze/Ember resolution and imports must be correct before this becomes reliable. Track those separately in [#75](https://github.com/IZ737/kiln/issues/75) and [#76](https://github.com/IZ737/kiln/issues/76).
- The planned built-in `@/` project-root import alias and configurable aliases in `kiln.toml` belong to **general import design** ([#76](https://github.com/IZ737/kiln/issues/76)), not to the route scanner. They can help routed pages access components anywhere in the project.
- `_components` exclusion and import aliases solve different problems: one determines which files become URLs, the other determines where source imports resolve.

## 8. Static-site generation must reuse Phase 6.6

Discovered routes must enter the existing route/export machinery, not implement a second exporter:

- A discovered `GET /about` with no route parameters is exported automatically by `kiln fire --static`, just like an explicit nonparameterised GET route.
- A discovered `GET /blog/{slug}` must have a **finite, typed enumeration** of export values; how a filesystem route supplies this through existing `.static(...)` semantics is to be designed. Do **not** guess all database records or URLs at build time.
- Need a file-routing equivalent of opt-out `.dynamic()` where appropriate, rather than forcing dynamic endpoints into a static export.
- Use existing HTML/asset/Ember generation and validation, including the established error for client code needing server RPC, broken links, non-static request-dependent behaviour and invalid output.
- Mixed-mode pre-rendering, route hooks, 404/error behaviour, redirects and output path conventions must remain consistent.
- An example site should demonstrate both `kiln run` and `kiln fire --static` from the **same** app source.

## 9. LSP / IDE / scaffolding

The LSP and VS Code should eventually provide:

- Completion for route filenames, parameters, layout/component imports, and `app.file_routes` mount paths.
- Go-to-definition from generated routes and handler references to the source files that provide them.
- Diagnostics for duplicate routes, invalid folder patterns and missing handlers/props, attached to useful file locations.
- Correct refresh when route files are created, moved, renamed or edited while unsaved.
- Optional filesystem-routing web template in `kiln new`, after the conventions are stable ([#80](https://github.com/IZ737/kiln/issues/80)).
- Reuse compiler/project resolution and existing LSP symbol infrastructure ([#79](https://github.com/IZ737/kiln/issues/79)); do not create VS Code-only semantics.

## 10. Work breakdown (provisional)

If adopted as **Phase 6.7**, a reviewable breakdown might be:

1. **6.7A — Foundation.** Source discovery/mounts, ignored directories, static `.glaze` pages, conflict handling and preservation of ordinary route semantics.
2. **6.7B — Dynamic pages.** URL parameters, settled handler/loading convention, validated props, methods and SSG enumeration.
3. **6.7C — Developer experience.** LSP navigation and diagnostics, `kiln new` template, full E2E/browser/static-site tests and reference docs.

**Timing is uncommitted.** This may be a pre-1.0 phase or a compatible feature added after 1.0; the RFC itself should not block the currently planned dogfooding and hardening work.

## 11. Acceptance criteria (candidate)

- [ ] Ordinary applications without `file_routes` retain their current behaviour.
- [ ] Adding `app.file_routes("pages")` makes `pages/index.glaze` available at `/` and `pages/about.glaze` at `/about`.
- [ ] A mount's `at:` prefix works without unexpectedly altering explicit routes or module paths.
- [ ] Nested `_components` directories are excluded by default; `ignore:` changes the exclusion convention per mount; excluded components are still importable.
- [ ] Collision and malformed path cases produce precise, source-linked diagnostics on Linux and Windows.
- [ ] Parameter routes and complex handlers use the same type-checked request/response behaviour as the explicit router.
- [ ] A handler-free dynamic Glaze receives declared URL parameters with type checking; unexpected/missing required props produce clear diagnostics.
- [ ] A typed optional loader supplies a record of Glaze props, automatically rendered with consistent error handling.
- [ ] Loader `throw web.error(404, ...)` and other intentional statuses use existing HTTP error handling and `app.on_error`; unexpected errors use existing safe 500 handling.
- [ ] An optional/absent loader result is not silently converted to a 404 or an empty page; no routing-specific error protocol is introduced.
- [ ] The generated Glaze receives the type-checked combination of URL path props and loader result fields; the loader need not return an already-supplied URL parameter.
- [ ] A duplicate prop name between a URL segment and loader result causes a source-linked compile error, never silent overriding; missing required Glaze props also fail clearly.
- [ ] Loader functions work without `req` when only typed path data is needed, and with an explicitly declared first `req: web.Request` when request context is required.
- [ ] URL parameter binding uses names and compatible types; invalid names, types, duplicate/misplaced request parameters and mismatches report precise errors.
- [ ] `[id]` derives its type from matching Glaze, `load` or handler declarations, or defaults to `String`; conflicting annotations produce compile-time errors with both relevant source locations.
- [ ] No filename-encoded type syntax is required; generated route matching uses the same typed conversion rules as explicit Kiln routes.
- [ ] A full GET handler may directly return `web.Response` without bypassing existing hooks, validation or SSG integration.
- [ ] Route directories discover their `route.kiln` consistently; its `load` and HTTP-method functions retain the ordinary language's type checking.
- [ ] Defining both `load` and `get` for the same filesystem route is a compile-time diagnostic with a useful fix; `load` plus a `post`/`put`/`patch`/`delete` handler compiles and dispatches by method correctly.
- [ ] Static, dynamic, mixed, and Ember-using examples preserve Phase 6.6 semantics under `kiln fire --static`.
- [ ] `kiln check`, `run`, `dev`, `test`, LSP and VS Code agree on the project route graph.
- [ ] Real-browser tests cover SSR, selective hydration, typed RPC restrictions in static export and common error paths.
- [ ] Documentation names all conventions and limitations; examples distinguish proposed syntax from shipped functionality until delivered.

## 12. Decision log and next questions

**Settled in conversation, 2026-10-27:**
- Opt-in routing from a directory with conventional file-based pages, with complex routes able to use directories.
- Coexistence with explicit `app.get` routing rather than replacing it.
- Default `_components` exclusion, with mount-specific configurable exclusions.
- Project-root aliases are a separate general import feature (see #76), and `_components` remains the routing convention by default.
- **Three-level progressive routing**: simple Glaze page with automatic typed URL props; Glaze plus optional typed `load` for data; or a full ordinary route handler for control. Exact signatures remain under design.
- **Complex route convention:** route-specific functions belong in a file named **`route.kiln`** inside the route directory; it may export `load` or an ordinary `get` handler, plus other HTTP-method functions including `post`, `put`, `patch` and `delete`. Exact signatures are still under design.
- **GET exclusivity:** `load` and `get` cannot coexist for the same route; diagnose the conflict at compile time. `load` can coexist with handlers for other HTTP methods.
- **Loader inputs (Option A):** loaders may optionally declare `req: web.Request` **first**, followed by typed URL path parameters bound by name. Request-free loaders remain concise.
- **Glaze props (Option A):** combine typed URL route parameters with the typed record returned by `load`; duplicate names cause compile-time errors and required Glaze props are type-checked against that combination.
- **Parameter typing (Option A):** `[id]` is only a name; infer the route parameter type from matching Glaze, `load` or handler declarations, defaulting to `String` without an annotation. Conflicting declarations are compiler errors. Do not encode types in route filenames.
- **Loader error handling (Option A):** use existing Kiln `try`/`throw`, `web.error`, unexpected-error 500 and `app.on_error` behaviour. No special `none` → 404 conversion.

**Needs an explicit decision before locking the specification:**
1. **Remaining loader contract:** admissible successful return record shapes, unused/extra field policy and unused parameter rules (§§5–6). Named parameter binding, optional first `req`, automatic props merging, duplicate-name errors, type inference/default `String` and ordinary error/404 propagation are settled.
2. **`route.kiln` contracts:** exact function signatures, module boundaries, exports and how flat Glaze files graduate to route directories (§5). The filename is settled.
3. **Dynamic static export:** what supplies `.static(...)` values and allows `.dynamic()` for filesystem routes? (§8)
4. **Route file naming:** `[slug]` spelling, typed parameters, extensions and optional `.ember` page files? (§4)
5. **Collision policy:** explicit route versus filesystem mount interactions, duplicate mount URL scope, and reserved paths? (§4)
6. **Ignore API details:** confirm `ignore` name and replacement rather than additive semantics; currently recorded as proposed in §3.
7. **Roadmap:** candidate Phase 6.7 before 1.0, or a post-1.0 compatible addition? (§10)

Keep this document updated as each decision is made, with changes in this draft PR until the spec is ready to merge.
