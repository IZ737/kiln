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
| Override | **Option A: mount-specific `ignore` replaces default exclusions** rather than extending them; `ignore: []` disables all directory exclusions and `_components` is no longer special unless listed explicitly |
| Ignore matching syntax | **Directory basenames only** in the initial filesystem router; recurse at any depth and do not interpret glob, wildcard, slash-delimited path or pattern syntax |
| Reuse | Excluded files are still importable/compilable; exclusion only changes route discovery |
| Scope | The `_components` convention is specific to filesystem routing; it is **not** a reserved Kiln folder elsewhere |
| Explicit routes | **Registration order determines precedence** between code-defined and filesystem routes; the first matching route wins regardless of origin. Shadowed-route diagnostics are advisory, not a new override rule |
| Framework-reserved endpoints | **Reject filesystem routes that could claim a Kiln-owned endpoint**, such as `/_kiln/rpc`, at compile time regardless of registration order. Ordinary application-route overlaps retain the established first-match precedence |
| Multiple filesystem mounts | **Registration order determines precedence across separate mounts** too. Cross-mount overlaps are allowed with non-blocking shadowing warnings, while duplicates *within* one mount remain compile-time errors |
| Shared infrastructure | Reuse current typed handlers, Glazes, Embers, hooks, SSG and error reporting |
| Page data model | **Three levels:** automatic Glaze with typed URL props → optional typed `load` returning page props → ordinary full `get` handler |
| GET ownership | **`load` and `get` are mutually exclusive within one route**; `post`, `put`, `patch` and `delete` may coexist with `load` |
| Conventional logic file | **`route.kiln`** in the complex route's directory; exported `load` or HTTP-method functions live there |
| Entrypoint visibility | **Only `pub fn` exports named `load` or supported HTTP methods are discovered**; private `fn` declarations are ordinary module helpers and do not claim routes |
| Loader arguments | **Optional `req: web.Request` first**, followed by **typed path parameters matched by name**; request-free loaders need not mention `req` |
| Loader URL parameter selection | **Declare only URL parameters the loader needs**; unmentioned route parameters are still parsed/typed in the route graph and available directly to Glaze props, without positional placeholders |
| HTTP handler URL parameter selection | **Like loaders, full method handlers may declare only the path parameters they need**, matched by name with consistent typed route validation; no unused positional arguments |
| HTTP handler request argument | **`req: web.Request` is optional as the first argument**, just as in loaders; when omitted, route handlers receive only their declared named path parameters (or no arguments) |
| Glaze prop assembly | **Automatically combine typed URL path parameters with the `load` return record**; duplicate field names are a compile-time error |
| Loader return-field matching | **Strict by name and type:** every field returned by `load` must match a declared Glaze prop; unexpected fields are compile-time errors. Glaze defaults need not be returned, and URL parameters are supplied separately |
| Loader return types | **Allow both anonymous records and named structs** with statically known fields matching Glaze props. No forced manual reconstruction or relaxation of strict field checking |
| Parameter typing | **Infer `[id]` from the Glaze, `load`, or route handler declarations**, defaulting to `String` when no explicit type is supplied; conflicting declarations are compile-time errors |
| Dynamic filename syntax | **Square-bracket `[name]` segments** in both `.glaze` filenames and route directories, mapping to ordinary Kiln `{name}` URL parameters; no curly-brace filenames or encoded types |
| Loader failures | **Reuse ordinary Kiln error propagation**: `web.error(status, message)` for intentional HTTP failures, existing 500 handling for unexpected errors, and `app.on_error` for custom responses; **no implicit `none` → 404** |
| Static parameter enumeration | **Route-local** `static_params()` in `route.kiln` supplies finite typed values; a route-local `dynamic` marker opts out of static export; use existing Phase 6.6 implementation |
| Enumeration without loader/handler | **`static_params()` may be the sole exported function in `route.kiln` alongside `index.glaze`**; its values pre-render the ordinary Glaze using typed URL props, with no dummy `load` or `get` required |
| Server-mode static pages | **`static_params()` also pre-renders its enumerated pages when the server starts**, matching explicit `.static(...)`; reuse existing in-memory response serving, hook ordering and fallback behaviour |
| Pre-rendering safety | **Reject routes whose cached response depends on request-specific data** (auth/session, cookies, arbitrary headers/query/body); apply consistently to filesystem `static_params()` and explicit `.static(...)` routes |
| Server-mode cache refresh | **Opt-in route-local `static_refresh` interval** in `route.kiln`, equivalent to explicit `.static(refresh: ...)`; never relax safe-pre-rendering checks |
| Zero-parameter refresh | **`static_refresh` alone opts a parameter-free route into server startup pre-rendering and interval refresh**; no `static = true` or `static_params()` required |
| Dynamic-only route scope | **`pub const dynamic = true` is valid on parameter-free and parameterised routes alike**, excluding the route from `kiln fire --static` while keeping server-mode routing; cannot combine with `static_params()` or `static_refresh` |
| API-only routes | **`route.kiln` may define HTTP methods without any `index.glaze`**; `load` requires a Glaze to render and is invalid without one |
| Routable page source | **Only `.glaze` files are automatically discovered as pages; `.ember` files do not independently claim URLs**. Embed Embers in a Glaze for interactive pages. Full HTTP handlers can still provide routes without a Glaze |

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

**Mount semantics** (the `ignore` replacement behaviour is agreed; other mount API details remain provisional)

- The directory operand is resolved against the **Kiln project root**, not process working directory. It must remain within the allowed project/workspace boundary.
- A mount specifies where routes come from and (optionally) the URL path prefix; a mount does **not** change Kiln's module/import rules.
- Both source directory and route prefix should be statically knowable to the compiler. Precise syntax/validation of constant expressions is TBD.
- **Default:** exclude every directory named `_components`, recursively. No need to write an option for normal usage.
- **Override — DECIDED (Option A, 2026-10-27):** specifying `ignore` **replaces** the default `_components` exclusion list; it never implicitly adds to it. `app.file_routes("pages", ignore: ["partials"])` excludes `partials` directories recursively but does **not** exclude `_components`, which can now contain discoverable pages. To keep both exclusions, list both explicitly: `ignore: ["_components", "partials"]`. `ignore: []` means **no directory-basename exclusions at all**. With no `ignore` argument, `_components` remains excluded by default. The exact spelling is `ignore` as shown here.
- **Matching syntax — DECIDED (Option A, 2026-10-27):** entries in `ignore` are **literal directory basenames**, matched recursively at any directory depth below the mount. For example, `ignore: ["_components", "partials", "internal"]` excludes every directory named exactly `_components`, `partials`, or `internal`, including their descendants. The list does **not** accept glob patterns, wildcard operators (`*`, `?`, `**`), slash-delimited relative paths such as `admin/internal`, or arbitrary path expressions in the initial implementation. There is no relative-path-specific matching rule: an entry excludes every matching directory name throughout that mount. A later glob enhancement would require an explicit, separately specified extension rather than silently changing this API.
- The compiler should validate ignore entries as well-formed **single directory basenames** (not empty strings, `.`, `..`, absolute paths, path separators or glob meta-characters) and report source-linked errors for invalid entries. Exact case handling and cross-platform filesystem normalisation should remain consistent with route discovery's operating-system safety policy; do not invent OS-dependent glob semantics.
- Ignore rules apply **per mount**, not globally. Ignoring a directory skips route discovery recursively under it, but does not suppress normal compilation/import of its files if referenced. A second mount may apply a different `ignore` list to the same source folder. The compiler/LSP must still be able to resolve and import those files when referenced. Do not treat `_components` as a reserved keyword or force its exclusion if a mount's explicit list omits it.
- **Across separate mounts — agreed:** overlapping registrations use normal **first matching route wins** behaviour in mount-call registration order. They are allowed with non-blocking source-linked warnings when detectable; they are **not** compilation errors simply because two mounts map to the same URL. True duplicate routes *within one mount* remain errors. Distinct URL-prefix mounts and methods can coexist normally. Still reject malformed mount paths, unsafe source traversal and invalid configuration; reserved endpoint interactions require an explicit policy.

## 4. File-first layout: agreed dynamic syntax; other naming rules provisional

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
- **Dynamic filename syntax — DECIDED (Option A, 2026-10-27): use square brackets.** A dynamic segment is written **`[name]`** in a directory or the stem of a `.glaze` filename; `[slug].glaze` and `[slug]/index.glaze` both map to the same URL parameter segment, `/blog/{slug}`. Use the existing Kiln `{slug}` syntax only in generated/explicit URL patterns, **not** `{slug}` filesystem names.
- The bracketed filename supplies **only the parameter name**, not its type. The inferred type comes from matching Glaze, loader or handler declarations, defaulting to `String` when nothing declares it; do not introduce `[slug-int]`, `{slug}` or inline filename type annotations.
- `[name]` represents a **whole path segment**, not embedded wildcard fragments such as `post-[slug].glaze`, for the initial convention. Validate that the name is a supported Kiln parameter identifier; malformed or empty bracket groups and duplicate names in one URL must receive source-linked diagnostics. Catch-all or optional segment syntax is a **separate future design decision**, not implied by `[name]`.
- For complex **page routes**, a directory may contain `index.glaze` plus **`route.kiln`** when route-specific code is needed (see §5). For **API-only routes**, `route.kiln` may exist **without** an `index.glaze` and register its exported HTTP methods directly (see §5). For instance `blog/[slug]/index.glaze` would map to the same path as `blog/[slug].glaze`; including both is an error.
- Plain components outside mounted directories are not route candidates. Only **mounted directories** are scanned.
- **Agreed (2026-10-27): automatic filesystem page discovery is restricted to `.glaze` files.** A standalone `.ember` file does **not** define a page or claim a URL. For an interactive page, create a `.glaze` page that embeds the Ember normally. This keeps `.glaze` responsible for page composition/SSR and `.ember` responsible for browser-side interactivity. API-only `route.kiln` HTTP methods remain valid without a Glaze.

**Routing collisions and path correctness**

- Reject duplicate method/URL mappings discovered in the same mount, including file-versus-directory equivalents.
- Within one mount, a literal path (e.g. `/blog/new`) must take precedence over the corresponding parameterised path (e.g. `/blog/{slug}`). Specify the ordering during route-table generation.
- **Agreed (2026-10-27): explicit versus filesystem routes follow registration order.** The existing router dispatches **the first matching route**, regardless of which registration mechanism produced it. An `app.file_routes(...)` mount expands to its discovered route registrations **at that call's position** in the application's route-registration sequence. A preceding `app.get("/about", ...)` wins over a later `pages/about.glaze`; putting the mount first reverses the precedence. Do **not** give code-defined routes implicit priority or globally reorder mounts.
- **Method-specific matching:** a preceding `app.post("/about", ...)` must not suppress a later filesystem GET for `/about`; the first *matching method and path* wins according to the existing router semantics. Likewise, route parameters and methods retain existing type/matching rules.
- **Within one filesystem mount**, keep the separate already documented rules: true duplicate method/URL claims (e.g. `about.glaze` vs `about/index.glaze`) are **compile-time errors**, and generated literal routes precede matching parameter routes. This internal deterministic ordering does not override the relative placement of the mount against explicit route registrations.
- If an explicit route shadows a discovered route, or vice versa, offer a **useful non-blocking diagnostic** (warning/editor hint) with both source locations when statically detectable. Shadowing may be intentional; do not reject it as a duplicate error across the explicit/file boundary. Explain which route wins based on registration order. Whether universally unreachable paths are an error remains out of scope pending a general route-analysis policy.
- **Separate filesystem mounts — DECIDED (Option A, 2026-10-27):** overlapping method/URL patterns in **different** `app.file_routes` registrations use the same registration-order priority as explicit-versus-filesystem overlaps. The first matching registration handles the request; reversing mount-call order reverses the winner. Emit an advisory shadowing warning with both source files/mount calls where a conflict is statically detectable; do not fail compilation for a cross-mount overlap. Repeated mounts of the same source URL scope follow the same rule, though warn that the later registration may be unreachable. Internal literal-before-parameter ordering remains **local to each mount** and must not reorder routes across calls.
- For example, if `app.file_routes("pages")` precedes `app.file_routes("legacy/pages")` and both contain `about.glaze`, `/about` resolves to `pages/about.glaze`. If the order is reversed, `legacy/pages/about.glaze` wins. Unlike two `about.glaze` equivalents discovered inside **one** mount, this is not a hard duplicate-route error.
- Overlap means a potential match for the same **HTTP method and URL**: GET and POST endpoints at the same URL remain independent, and different non-overlapping mount prefixes cause no conflict. A warning should not claim certain shadowing when parameter type constraints make the patterns disjoint. Exact warning severity/code and analysis of partially overlapping dynamic patterns remain implementation decisions.
- Preserve URL percent-decoding, typed route parameter matching, HEAD→GET behaviour, 404/405 responses, `app.before` hooks, and server-side security rules.
- Reject path traversal, unexpected symlinks, case-folded collisions and invalid parameter names. Treat case sensitivity consistently across supported operating systems.
- **Framework-owned endpoint protection — DECIDED (Option A, 2026-10-27):** a discovered filesystem route **must not claim or potentially intercept a framework-reserved method/URL**, such as Kiln's internal `/_kiln/rpc` endpoint. This is a **compile-time error regardless of mount registration order or what an ordinary application route would otherwise do**. The existing registration-order rule applies to *application routes*, not to overriding Kiln's reserved infrastructure.
  - Check the **resolved URL after applying each mount's `at:` prefix**, not just the raw filesystem path. A mount `at: "/_kiln"` can therefore collide even when its source directory has an innocuous name.
  - Check applicable HTTP methods and matching URL shapes, including a parameterised file route that can match a reserved endpoint. For example, a dynamic `[_segment]/rpc` route at the root can potentially claim `/_kiln/rpc` when its parameter type permits `_kiln`. Use the router's actual typed-pattern match semantics rather than simplistic string equality; a provably disjoint pattern is not automatically a conflict.
  - The compiler should rely on a **single framework-maintained registry of reserved endpoint patterns (and any explicitly reserved namespaces)**, shared with the route generator, rather than duplicating an incomplete hard-coded list in the filesystem router. A specific internal path like `/_kiln/rpc` is an illustrative known example; any additional reserved prefixes/endpoints must reflect Kiln's actual runtime declarations, not be invented ad hoc.
  - Report the conflicting discovered file/mount, reserved endpoint and overlapping method/path with a source-linked diagnostic; suggest moving the page, changing the mount prefix or using a non-reserved route. Treat as a hard error in `kiln check`, builds, dev/LSP route analysis and static export, not a warning or an order-dependent behaviour.
  - **Scope:** this RFC decision protects Kiln infrastructure against **filesystem-discovered routes**. Do not silently change already-shipped explicit `app.get`/router precedence or public behaviour beyond the filesystem router without a separate compatibility decision.


## 5. Three-level page model — agreed direction; syntax provisional

**Decision (2026-10-27):** Support **all three complementary levels**, so complexity is opt-in:

1. **Just a Glaze:** pages with no custom data requirements are discovered and rendered automatically. Where a page's path contains parameters, Kiln automatically provides matching, **type-checked** URL parameter props.
2. **Glaze + typed loader:** when data must be retrieved or transformed before rendering, an optional `load` function supplies a typed record of Glaze props. Kiln handles the ordinary rendering step; no repetitive `web.render` call is required.
3. **Full handler:** when the application needs direct request/response control (authentication, redirect/status/headers, unusual content types, etc.), an ordinary typed `get` handler constructs the response explicitly using existing `web` functions.

**The convention is agreed:** route-specific logic lives in a file named **`route.kiln`** inside the relevant route directory. That file may export **`load`** for typed page data or ordinary HTTP-method functions such as **`get`**, **`post`**, **`put`**, **`patch`** and **`delete`** for direct request/response handling. 

The **filename, general roles, `load`/`get` exclusivity, optional request/typed path-parameter argument model, `pub` entrypoint requirement, and standard Kiln error propagation** are decided; detailed return contracts and other edge cases remain open. Examples below are illustrative Kiln-like pseudocode and **are not currently implemented**.

**Decision (2026-10-27) — route function visibility, Option A:** filesystem discovery only treats **publicly exported** functions with recognised entrypoint names as route logic: `pub fn load(...)` for the paired Glaze or `pub fn get(...)`, `pub fn post(...)`, `pub fn put(...)`, `pub fn patch(...)` and `pub fn delete(...)` for matching HTTP methods. The public marker means ordinary Kiln module visibility, not a routing-specific implicit export. A private function named `get` or `load` is **not discovered** as an endpoint/loader; private helpers remain fully usable from within `route.kiln`.

```kiln
// Conceptual route.kiln: public entrypoint plus private helper.
pub fn get() -> web.Response {
    return web.text(greeting())
}

fn greeting() -> String {
    return "Hello!"
}
```

The compiler/LSP should flag likely missing-`pub` mistakes with a useful diagnostic when a private recognised name appears in a route context but cannot be discovered; do not silently promote private declarations into public exports. **`pub` applies to route entrypoints**, not to all helper functions or all source declarations in a route directory. Other route-local metadata (`static_params()`, `dynamic`, `static_refresh`) has its own declaration form; confirm any remaining visibility/constant requirements in §8 rather than inferring extra syntax from this decision.


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
- Glaze defaults continue to work for props not supplied by either source. **Decision (2026-10-27) — strict loader field matching, Option A:** every returned loader field must correspond to a **declared Glaze prop** with a compatible type. An unexpected/unused field is a compile-time error, even if it would otherwise be harmless; require developers to return only the data the page contract declares. This applies to named fields of the loader's statically known return type, not only to literals, and includes fields supplied through aliases or shared records.
- This rule does **not** require the loader to return every Glaze prop: typed URL path params are already supplied automatically, defaulted Glaze props may be omitted, and the existing Glaze mechanism may provide a declared `request: web.Request` prop independently. Every **required** prop must still be satisfiable after combining all legitimate sources. A loader must never duplicate a URL parameter field.
- Diagnose extra fields, missing required fields, duplicate URL/loader fields and incompatible types with source-linked errors. Suggest removing/renaming the returned field or declaring it in the Glaze when appropriate. **Decision (2026-10-27) — named-struct loader returns, Option A:** a loader may return **either an anonymous typed record or a named struct** whose statically known fields exactly match its contributions to the paired Glaze's declared props. The compiler checks fields individually by name and compatible type; it does **not** require reconstructing a duplicate anonymous record. A struct with an additional field is still rejected, even when that field was intended for other callers. URL path parameters remain separately supplied and cannot be repeated as struct fields.
- Parameter name uniqueness and type compatibility should be enforced in the shared route/compiler checker, not by runtime maps with silent shadowing.

Kiln type-checks that the returned record or named struct satisfies the paired Glaze's required props and renders it automatically.

**Illustrative named-struct return (Option A):**

```kiln
struct BlogPageData {
    post: Post,
    author: User,
}

pub fn load(slug: String) -> BlogPageData ! {
    return try posts.page_data(slug)
}
```

```glaze
{% props post: Post, author: User, slug: String %}
<h1>{post.title}</h1>
```

The `post` and `author` struct fields are automatically passed to Glaze, while `slug` is sourced from the URL. The syntax is illustrative; no automatic arbitrary struct-to-record conversion beyond statically checked page-prop delivery is implied. Field visibility/access rules must honour normal Kiln module privacy and be confirmed during implementation.

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

A valid path parameter must match a named segment in the discovered URL pattern and have a compatible type with Kiln's existing typed route rules. Unsupported types, unknown names, duplicate parameter declarations, and a misplaced request parameter must fail with clear source-linked diagnostics. The **special request argument is explicitly opt-in**; it is not made available implicitly through magic globals. **The type is inferred from declarations in the Glaze, loader or route handler; otherwise it defaults to `String`.**

**Decision (2026-10-27) — loader parameter selection, Option A:** a `load` function may declare **any subset** of the route's URL parameters, including none, and they are bound **by name**, not by their position in the URL. Omitted parameters do not require dummy arguments and are **not forwarded into the loader**. They remain part of the typed route pattern and can still supply the paired Glaze's matching declared props automatically. For example:

```text
pages/shops/[shop]/products/[id]/
  index.glaze
  route.kiln
```

```kiln
// Conceptual route.kiln: the loader needs only id.
pub fn load(id: Int) -> { product: Product } ! {
    return { product: try products.find(id) }
}
```

```glaze
{% props shop: String, id: Int, product: Product %}
<h1>{product.name}</h1>
```

For `/shops/acme/products/42`, the URL contributes `shop: "acme"` and `id: 42` to the Glaze, while `load` receives only `id: 42`. This does **not** weaken parameter type agreement: if the loader and Glaze give incompatible types to the same named parameter, compilation fails. Nor does it permit returning duplicate URL parameter names in the loader result. **Full HTTP method handlers may also declare only the needed URL parameters** (see Level 3 below); use the same named binding and route typing.

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

**Decision (2026-10-27) — HTTP handler parameter selection, Option A:** like `load`, each `route.kiln` HTTP method handler may declare **only the named URL parameters it actually consumes**; it need not declare every dynamic segment in the route path. The path pattern still matches and converts *all* segments according to the compiler's unified inferred types; missing declarations do not become wildcard coercions or bypass matching. For example:

```text
pages/shops/[shop]/products/[id]/
  route.kiln
```

```kiln
// Conceptual HTTP handler for GET /shops/{shop}/products/{id}.
pub fn get(req: web.Request, id: Int) -> web.Response ! {
    return web.json(try products.find(id))
}
```

The `get` handler receives `req` and typed `id` but does not require an unused `shop: String` argument. This subset convention applies independently to `get`, `post`, `put`, `patch`, `delete` and any future supported method export: each method can consume a different subset. Declared parameter names must match the route's bracket names, types must agree with other declarations for the same route, and unknown/duplicate/incompatible declarations are compile-time errors. Do not infer types from argument position; if an adapter is required by the existing explicit router, produce it through the existing compiled route machinery instead of changing ordinary explicit handler signatures or adding a second dispatch engine.

**Decision (2026-10-27) — optional HTTP request parameter, Option A:** every filesystem `route.kiln` HTTP method handler may **omit `req: web.Request` entirely** when it does not need the incoming request. When needed, `req: web.Request` is declared **first**, followed by any subset of typed URL parameters bound by name. This applies independently to GET, POST, PUT, PATCH and DELETE. All are valid conceptual examples:

```kiln
pub fn get() -> web.Response {
    return web.text("Hello!")
}

// On a parameterised route such as /products/[id], without request context:
pub fn get(id: Int) -> web.Response ! {
    return web.json(try products.find(id))
}

// When request-specific context is needed:
pub fn post(req: web.Request, id: Int) -> web.Response ! {
    return web.json(try products.update_from_request(id, req))
}
```

These declarations illustrate **alternative route signatures** rather than functions to define together in a single `route.kiln`. In particular, the two `get` examples belong to different example routes.

There is **no implicit request global**, no argument inserted behind the developer's back, and no positional path-parameter binding. Reject misplaced, repeated, wrong-type or otherwise unsupported `web.Request` parameters with clear source-linked diagnostics. When the request is omitted, the compiled adapter still receives the real request internally, preserves all normal hooks, HTTP method/status behaviour, error handling and response conversion, and invokes the user handler with only the declared arguments. This feature is restricted to **filesystem-discovered handler conventions** unless the explicit `app.get(...)` API separately adopts the same convention; do not silently alter existing explicit handler contracts.

This API convenience does **not** relax shared static-cache safety: a handler that really reads request-dependent data remains ineligible for pre-rendering, whether or not it explicitly declares the request parameter.

### API-only filesystem routes — Option A agreed (2026-10-27)

A route directory containing **`route.kiln` alone is valid**, with **no `index.glaze` required**. Its ordinary exported HTTP-method functions define the endpoint(s). This is not a fourth rendering mechanism: it is the existing full-handler route model without a page view.

```text
pages/
  api/
    users/
      route.kiln             → /api/users
```

```kiln
// Conceptual: ordinary typed HTTP handlers in pages/api/users/route.kiln.
import web, users

pub fn get(req: web.Request) -> web.Response ! {
    return web.json(users.all())
}

pub fn post(req: web.Request) -> web.Response ! {
    return web.json(try users.create(req))
}
```

**Rules:**

- `get`, `post`, `put`, `patch` and `delete` exports register their corresponding methods at the directory's path and preserve existing request/response types, errors, hooks and typed path parameters. A `post`-only route need not define `get`; an incoming GET then follows ordinary 405/no-matching-method behaviour.
- **`load` is invalid without an `index.glaze`** in the same route directory, because there is no Glaze to receive and render the returned props. Report a source-linked error explaining how to add a view or replace `load` with a full `get` handler.
- If there **is** an `index.glaze` and `route.kiln` exports only non-GET HTTP methods, Kiln still provides the normal implicit GET page render. A full `get` export overrides implicit GET rendering; a `load` export provides typed props for implicit rendering. The existing `load`/`get` exclusivity rule remains unchanged.
- A `route.kiln` with neither a routable HTTP method nor a usable Glaze/`load` page is an error or an unused file, not an invisible successful endpoint; choose the exact diagnostic policy when implementing discovery.
- A GET returning JSON or other non-HTML content is **not automatically static-exportable** merely because it comes from a filesystem route. Use the same Phase 6.6 export checks as explicit GET routes (including valid extensions for non-HTML output), and allow a route-local `dynamic` marker for endpoints intentionally excluded from `kiln fire --static`. POST and other non-GET endpoints are omitted from full-static export as usual.
- No mandatory Glaze, empty page stub, new API directory type, separate router, or different auth/middleware machinery is introduced.

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

- **Route parameters — DECIDED for loaders and full HTTP handlers:** the filename supplies **only a parameter name**, e.g. `[id]`; the compiler **infers its type** from matching declarations in the Glaze, `load` or full route handler. If no declaration supplies an explicit type, the parameter is a **`String`**. If two or more declarations disagree (e.g. `id: Int` in a loader and `id: String` in the Glaze), this is a **compile-time error**, not implicit coercion or precedence. Keep existing route matching/conversion rules for supported types (`Int`, `Float`, `Bool`, `String`). **Loaders and full HTTP handlers each declare only the subset of URL parameters they use**, bound by name; omitted params still participate in full URL pattern matching and Glaze may receive them as declared props. Optional first `req: web.Request` is now settled for **both loaders and filesystem HTTP handlers**: omit it entirely when unused, otherwise place it first. All explicit path-type declarations must agree.
- **Prop combination and exact loader fields — DECIDED:** the Glaze receives the **union of URL parameter props and the loader-returned typed record or named struct fields**. Duplicate names across those sources are **compile-time errors**, even for equal values/types. **Every loader return field must match a declared Glaze prop by name with a compatible type**; extra/unused loader fields are compile-time errors. Missing required props are errors, but Glaze defaults, URL param props and an existing implicit request prop can satisfy their corresponding declared inputs without loader forwarding. **Named structs and anonymous typed records are both allowed** (Option A); strict matching applies to both, including fields defined in a reusable struct. Normal module access/privacy rules still apply.
- **Handler discovery — DECIDED:** **`route.kiln` in the route directory is settled**, and only `pub fn load` or `pub fn` with a supported HTTP-method name participates in automatic route discovery. A same-named private `fn` remains a normal helper, never an implicitly exported handler. Reuse ordinary module visibility and show actionable diagnostics for likely missing `pub` mistakes. Precise module boundaries, return/error contracts and the visibility of route-local static metadata remain for implementation/design review.
- **`load` vs `get` — DECIDED:** they are **mutually exclusive per route** and must trigger a **compile-time error** when both are defined. `load` may coexist with non-GET method exports (`post`, `put`, `patch`, `delete`). Do not invent fallback or invocation ordering between `load` and `get`.
- **Failure handling — DECIDED:** reuse `web.error(status, message)`, ordinary `try`/`throw`, existing unexpected-error (500) handling, and `app.on_error`; **do not** convert `none` to 404 automatically. Authentication and authorization failures must be propagated through this same path. Exact examples and language type signatures still require implementation-time validation.
- **HTML and non-HTML — DECIDED:** API-only `route.kiln` directories may omit `index.glaze` and expose ordinary HTTP-method handlers; `load` without an `index.glaze` is an error. Full handlers can return arbitrary `web.Response`, subject to existing static-export constraints.
- **Upgrade path:** moving `blog/[slug].glaze` to `blog/[slug]/index.glaze` plus `route.kiln` should preserve `/blog/{slug}`; defining both forms simultaneously is a deterministic collision error.
- **Testing:** include simple direct parameter props, typed loader props, full handlers, invalid prop/type contracts, access control, errors, cross-folder imports and consistent static export behaviour.

## 7. Layouts, components, aliases

### Glazes define pages; Embers are embedded — Option A agreed (2026-10-27)

A `.ember` is an interactive browser-capable component, **not a direct filesystem-route entrypoint**. An Ember placed under a mounted directory does not become a URL based on its path; no autogenerated page wrapper, implied layout, or new route registration is created solely from `.ember` files.

```text
pages/
  dashboard/
    index.glaze       → GET /dashboard
    dashboard.ember    → component used by the Glaze; NOT a route
```

The Glaze is responsible for SSR/page structure and may embed the Ember using Kiln's existing component reference and selective WasmGC hydration semantics. This works for completely interactive page bodies as well as small interactive islands; it does **not** require the server-rendered page to contain substantial noninteractive markup. Components living inside an ignored `_components` folder are importable but also never become routes.

If a directory contains only `.ember` files and no Glaze or routable `route.kiln` HTTP method exports, it does not produce a filesystem route. Whether to warn on such directories is an editor/diagnostics decision, not an implied request to add Ember routing later. Continue supporting existing Embers in ordinary code-defined routes and outside mounted directories without changing their meaning.

**First implementation scope:** do not implement a separate standalone-Ember page convention. A future proposal could revisit it without altering the `.glaze` and route-handler model accepted here.

### Existing composition and imports

- For the first version, Glazes continue to use explicit `{% layout "..." %}`, slots and ordinary component tags. **Do not silently introduce automatic nested layout inheritance**.
- A routed Glaze may embed an Ember using existing rendering/hydration behaviour. A page without Embers must not gain an unnecessary client runtime.
- Cross-folder Glaze/Ember resolution and imports must be correct before this becomes reliable. Track those separately in [#75](https://github.com/IZ737/kiln/issues/75) and [#76](https://github.com/IZ737/kiln/issues/76).
- The planned built-in `@/` project-root import alias and configurable aliases in `kiln.toml` belong to **general import design** ([#76](https://github.com/IZ737/kiln/issues/76)), not to the route scanner. They can help routed pages access components anywhere in the project.
- `_components` exclusion and import aliases solve different problems: one determines which files become URLs, the other determines where source imports resolve.

## 8. Static-site generation — route-local metadata (Option A agreed, 2026-10-27)

Discovered routes must enter the **existing Phase 6.6 route/export machinery**, not implement a second exporter. The developer chooses static export behaviour **alongside the corresponding route** in `route.kiln`, rather than returning to centralized route configuration in `main.kiln`.

### Automatic static pages

- A discovered `GET /about` with no parameters is exported automatically by `kiln fire --static`, like an explicitly registered nonparameterized GET route. A simple `about.glaze` requires no special file or annotations.
- A parameterised route must have a **finite, typed enumeration** of parameter values to be exported, or an explicit **dynamic-only opt-out**. Do not guess database records or dynamically crawl URLs at build time.

### Route-local parameter enumeration: `static_params()`

For example, a complex blog route could provide its own list of slugs:

```text
pages/
  blog/
    [slug]/
      index.glaze
      route.kiln
```

```kiln
// Conceptual function convention; not yet implemented.
pub fn static_params() -> List<String> {
    return posts.all_slugs()
}
```

For a route with multiple named URL parameters, the return type should reuse the **same typed record shape** required by the existing `.static(...)` API:

```kiln
// Conceptual only, for a route like /docs/[section]/[page].
pub fn static_params() -> List<{ section: String, page: String }> {
    return [{ section: "guides", page: "install" }]
}
```

The compiler checks that each returned element matches the route's inferred parameter type(s). No parameter conversion or fallback is invented specifically for filesystem routing. For a zero-parameter route, no enumeration is necessary. The precise export visibility requirements and permitted function effects are still to be defined.

**Decision (2026-10-27) — independent `static_params()`, Option A:** a parameterised route may provide **only `index.glaze` and `route.kiln` exporting `static_params()`**, with **no `load()` or `get()`**. Enumeration controls which parameter combinations are pre-rendered; it does **not** require application-specific data loading or full request/response control. Kiln continues to generate the implicit GET page renderer, supplies typed URL parameters to declared Glaze props, and uses that same renderer to pre-render the enumerated URLs in server mode and export them under `kiln fire --static`. No dummy loader, duplicate record construction, or explicit GET handler is necessary.

```text
pages/
  blog/
    [slug]/
      index.glaze
      route.kiln
```

```glaze
{# pages/blog/[slug]/index.glaze — conceptual #}
{% props slug: String %}
<h1>{slug}</h1>
```

```kiln
// pages/blog/[slug]/route.kiln — conceptual
pub fn static_params() -> List<String> {
    return ["welcome", "about", "updates"]
}
```

This declares `/blog/welcome`, `/blog/about` and `/blog/updates` as pre-rendered outputs. In normal server mode, requests to other slugs retain the previously agreed `.static(...)` dynamic fallback; in static export only the enumerated paths are generated. **The existing Glaze prop contract still applies:** if an `index.glaze` needs required fields other than typed URL props, declared defaults or existing built-in props, omitting a needed loader is a compile-time error. If there is no GET-capable Glaze and no full GET handler at all (for example, a POST-only API directory), `static_params()` cannot by itself invent a page/GET endpoint; the precise diagnostic should explain the missing renderable route. As before, `static_params()` conflicts with `dynamic = true` and its cached output must pass the shared request-independence checks.

**Decision (2026-10-27) — server-mode behaviour, Option A:** `static_params()` is the filesystem-route counterpart of **`.static(...)` for *both* static export and a running server**. When the application starts in server mode, Kiln evaluates the declared parameter list and pre-renders those GET pages into the same in-memory pre-rendered response mechanism used by explicit `.static(...)` routes. When serving subsequent requests for enumerated paths, preserve the existing `app.before` hook ordering and use the pre-rendered response. Requests for parameter values *not* in the list must retain the existing `.static(...)` behaviour (ordinary request-time rendering rather than a fabricated 404), unless some other routing rule prevents them. If pre-rendering fails at startup, follow the current `.static(...)` fallback: log the failure and render on demand instead of silently treating it as a successfully cached page. No independent cache, renderer or error pathway is introduced.

This is an explicit design commitment: `static_params()` is **not export-only**. **Decision (2026-10-27):** route-local refresh intervals are supported, matching existing `.static(refresh: ...)` rather than requiring developers to switch to explicit code-defined routing. See the following section.

### Route-local refresh interval — Option A agreed (2026-10-27)

When a route is pre-rendered in server mode, developers may optionally specify a refresh interval in that route's `route.kiln`. **Decision (2026-10-27): for a route *without* URL parameters, declaring `static_refresh` by itself also enables server-mode pre-rendering**; do not require either an additional `static = true` marker or a meaningless `static_params()` enumeration. The working API spelling is **`static_refresh`**; the final declaration form and visibility/export rules are pending language-syntax review:

```kiln
// Illustrative route.kiln — prospective syntax.
pub fn static_params() -> List<String> {
    return products.all_ids()
}

pub const static_refresh = 10m
```

For a zero-parameter page, this alone is sufficient:

```text
pages/
  dashboard/
    index.glaze
    route.kiln
```

```kiln
// pages/dashboard/route.kiln — conceptual syntax
pub const static_refresh = 10m
```

The single `/dashboard` page is pre-rendered at server startup and refreshed when requested after the interval expires. **A parameter-free `.glaze` file without `static_refresh` remains a normal request-rendered page in server mode**, although it is included automatically in `kiln fire --static` as required by Phase 6.6. Static export and server-mode pre-rendering are distinct operations.

**Agreed semantics:**

- `static_refresh` maps onto the already implemented **`.static(refresh: 10m)`** mechanism. Initial enumerated pages are pre-rendered on server startup. Once a cached page's interval has expired, the **first subsequent request** re-renders it through the same ordinary route machinery. This is request-triggered refresh, **not** periodic background fetching or an independent TTL cache.
- Refresh applies **per generated URL/cache entry**, not by mutating the shared response for unrelated parameter values. Non-enumerated parameters continue to render on request under ordinary `.static(...)` rules.
- The interval must be a valid, positive, compile-time-known **Kiln duration**; an invalid or negative/zero interval is a source-linked validation error. Exact declaration visibility and duration-type syntax are to be confirmed against the compiler's supported grammar.
- With **no `static_refresh`**, preserve the normal non-refreshing `.static(...)` semantics; do not introduce a new default expiration policy.
- `kiln fire --static` still produces a **one-time snapshot**, without running refresh or promising background updates on the static host. The refresh setting matters only when a Kiln server is running.
- **Security is unchanged:** request-dependent/personalised output remains ineligible for shared pre-rendering, with or without refresh. A short interval does **not** make caching user-specific responses safe.
- A route opting out using `dynamic = true` must not simultaneously declare `static_refresh`. On a **parameterised** route, a duration with **no `static_params()` enumeration** is invalid; do not silently ignore it. On a **zero-parameter** route, the route itself is the single valid pre-render target, so `static_refresh` requires no separate marker.
- Preserve the current `.static(refresh: ...)` approach to re-render errors, hooks and response handling, rather than inventing a filesystem-router-specific refresh strategy.

**Settled:** for a non-parameterised GET route, `static_refresh` is sufficient to opt into server-side pre-rendering, without `static_params()` or `static = true`. The resulting implementation should correspond to ordinary `.static(refresh: ...)` for one route. It remains an error to supply a parameterised route's `static_refresh` without `static_params()`.

### Safety rule — no request-dependent shared pre-rendering (Option A agreed)

**Decision (2026-10-27):** A route's pre-rendered **body/response must not depend on per-request data** that can differ between users or requests. When a compiler can determine that rendering (directly or transitively through a loader, GET handler or Glaze/helper) depends on authentication/session/user identity, cookies, request headers, query strings, request body or other variable request context, **reject pre-rendering with a clear, source-linked diagnostic**. Do not cache personalised output even when an explicit `static_params()` list exists. If static safety cannot be established for a request-dependent response, treat it conservatively; no unsafe "trust me" opt-in is proposed for the initial feature.

The **typed URL path parameters enumerated by `static_params()`** are part of the static page's identity and are **not prohibited** by this rule: `/blog/hello` and `/blog/world` can each be pre-rendered from their respective `slug` values. The unsafe boundary is *additional* request-specific inputs that would change the same page's output for different requests.

A `load(req: web.Request, ...)` signature is allowed for ordinary runtime rendering. **Merely declaring `req` is not, by itself, evidence of an unsafe read**; check the actual reachable operations, not just the parameter's presence, wherever analysis supports this. A loader that really needs cookies/auth/headers should run per request and must not declare `static_params()`; for a parameterised route, use the route-local `dynamic` opt-out if producing a full static export. The same prohibition should apply to a full `get` handler attempting to pre-render request-dependent output.

**This safeguard must also cover ordinary code-defined routes with `.static(...)`**, not just newly discovered filesystem routes, so both routing models share the same safety boundary. Phase 6.6 already checks request-specific reads in **static export** (E0404); extending/using those checks for **server-mode pre-rendering** is an explicit implementation requirement of this RFC, not a claim the current server-mode implementation already enforces it. Prefer reusing the same static-analysis infrastructure and diagnostics while distinguishing server-mode versus export context in explanations.

**Hooks are different from cached page generation:** Kiln's existing `app.before` hooks run on each request *before* returning a pre-rendered response. They may still enforce per-request authorization or reject access, provided the response cached for an allowed request is independent of user identity. A per-user page cannot become safe simply because a hook runs first; nor should the compiler reject a public static page solely because a separate request-time hook reads a cookie.

Illustrative invalid combination:

```kiln
// Conceptual, for pages/account/[id]/route.kiln.
pub fn load(req: web.Request, id: Int) -> { profile: Profile } ! {
    let user = try auth.require(req)
    return { profile: try profiles.for_user(user, id) }
}

pub fn static_params() -> List<Int> {
    return profiles.ids()
}
// Invalid: the cached profile response depends on the requesting user.
```

A compiler diagnostic should point at the conflicting `static_params()` declaration **and** the request-dependent read where feasible, with advice to remove pre-rendering or make the page genuinely request-independent. Do not silently downgrade a declared pre-render into request-time rendering without signalling the error.

**A route originally authored as a flat `[slug].glaze` can be moved to `[slug]/index.glaze` plus `route.kiln` without changing the URL**, when it needs `static_params()` or other route-local logic.

### Route-local opt-out: `dynamic` — Option A agreed (2026-10-27)

**Decision:** A dynamic route that should **not** be included in `kiln fire --static` may opt out in `route.kiln` **regardless of whether its URL has parameters**. Personalised/authenticated pages commonly live at fixed paths such as `/dashboard` or `/account`; no artificial path parameter or separate exclusion mechanism is required.

```text
pages/
  dashboard/
    index.glaze
    route.kiln
```

For example, a parameter-free authenticated `/dashboard` remains request-rendered in server mode but is excluded from static export by this declaration:

```kiln
// Conceptual marker; final spelling and validation pending.
pub const dynamic = true
```

The semantics are the filesystem-route equivalent of `web.Route.dynamic()`: a static-only build skips this route rather than generating an invalid page; the route still runs normally when the server is running. **`dynamic = true` is an explicit opt-out from static export, not a request to force additional caching or a different server dispatch mechanism.** It applies equally to a parameter-free Glaze/loader page, a parameterised page, or a full-handler GET endpoint intended to remain server-only. No HTML representation is generated during static export for a dynamic-only route; deployment of that endpoint requires a running server.

**Contradictions:** a route declaring `dynamic = true` may **not** also declare `static_params()` or `static_refresh`, since the former is an export/pre-render enumeration and the latter enables server-mode shared pre-rendering (even for zero-parameter pages). Report a clear source-linked compiler error rather than silently prioritising one setting. Request-specific/personalised content is never made eligible for shared pre-rendering by an opt-out marker. `dynamic` may appear in a `route.kiln` containing no exported loader/GET handler when its sibling `index.glaze` defines the implicit GET.

**Decision:** `static_params()` and a `dynamic` marker live in **`route.kiln`**, not on a mount-specific `.route("/path")` chain in `main.kiln`. The example names are the working convention, subject to a final syntax check. Avoid requiring dynamic metadata for ordinary static pages.

### Compile-time validation and Phase 6.6 behaviour

- If a parameterised filesystem GET route has **neither** a finite `static_params()` enumeration nor a dynamic-only marker, `kiln fire --static` should report the existing missing-static-enumeration error (E0237 or its compatible extension). Ordinary dynamic `kiln run` and `kiln dev` do **not** require a static enumeration.
- A route that declares **both** `static_params()` and `dynamic = true` has contradictory export intent: reject the combination with a source-linked diagnostic, rather than silently picking one. Also reject **`dynamic = true` together with `static_refresh`**, including for parameter-free routes.
- For `static_params()` with invalid return type, missing/duplicate record keys, incompatible parameter types, or values that fail enumeration, report useful diagnostics consistent with ordinary `.static(...)` routes.
- Use the normal route graph, handler/Glaze rendering, HTML and asset generation, Ember hydration and errors: a page that requires unavailable server RPC, reads forbidden request data during static export, or produces broken links **must fail** using existing Phase 6.6 validation.
- **Pre-rendering safety — DECIDED:** both filesystem `static_params()` and explicit `.static(...)` routes must reject request-dependent cached responses in server mode as well as in static export. Analyze transitive request reads, excluding legitimate route-parameter identity and separately executed `app.before` hooks. Do not merely assume existing E0404 static-export enforcement covers server-mode caching until the implementation is verified/extended.
- **Server-mode pre-rendering — DECIDED:** `static_params()` also enumerates and pre-renders pages when the server starts, just as explicit `.static(...)` does. Reuse its in-memory serving, request hooks, unlisted-parameter fallback, and startup-render failure handling. Do **not** make `static_params()` an export-only convention.
- **Cache refresh — DECIDED:** optional route-local `static_refresh` uses the existing `.static(refresh: ...)` request-triggered re-render policy in server mode only. For zero-parameter GET routes, `static_refresh` **alone enables server startup pre-rendering** without `static_params()` or `static = true`; parameterised routes still need `static_params()`. Validate duration and contradictory/unused declarations. A refresh interval does not permit caching request-dependent responses.
- Continue preserving hooks, 404/error behaviour, redirects and output path conventions. A representative app must demonstrate `kiln run` and `kiln fire --static` from **the same source**.

**Still open:** exact `static_params()` visibility/return signatures, how to handle failing enumeration in static export, and a route whose HTTP handlers return non-HTML content. **An implicit Glaze GET with `static_params()` alone is now explicitly valid**, as is `dynamic` on parameter-free routes; contradicting `dynamic`/`static_params`/`static_refresh` declarations are diagnosed. All details must reuse the existing static export contract.

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
- [ ] Nested `_components` directories are excluded by default; **`ignore:` replaces** the exclusion list for its mount rather than extending it. `ignore: ["partials"]` makes `_components` discoverable, `ignore: ["_components", "partials"]` excludes both, and `ignore: []` excludes no directories by name; ignored components remain importable.
- [ ] Ignore lists apply recursively and independently to each filesystem mount, without changing ordinary source imports or silently suppressing diagnostics for imported files.
- [ ] Each `ignore` entry matches a literal directory basename at any depth; neither `admin/internal` nor `**/_components` nor other glob/path patterns are accepted. Invalid empty, dot, traversal, absolute or wildcard entries yield source-linked diagnostics; a same-named directory anywhere under the mount is consistently excluded.
- [ ] Collision and malformed path cases produce precise, source-linked diagnostics on Linux and Windows.
- [ ] For overlapping explicit `app.get` and discovered GET routes, registration order chooses the first matching route; swapping call order swaps the winner, without inventing explicit-route priority.
- [ ] Shadowing between explicit and discovered **application** routes can be diagnosed non-fatally with source information, while duplicate claims within a single mount remain compile-time errors.
- [ ] A filesystem route claiming an actual framework-reserved HTTP method/path such as `/_kiln/rpc` is a compile-time error **regardless of registration order**; diagnostics identify the generated route and reserved endpoint and recommend a safe path.
- [ ] Reserved endpoint checks use the generated mount prefix, relevant HTTP methods, and matching dynamic parameter types; source forms that could match a reserved endpoint fail while provably non-overlapping methods/patterns do not produce spurious conflicts.
- [ ] Framework-owned endpoint data comes from a shared runtime/compiler registry rather than an incomplete special-case list; existing explicit code-defined route behaviour remains unchanged unless separately specified.
- [ ] Two separate mounts registering overlapping method/URL patterns resolve in the order of their `app.file_routes` calls, with non-blocking source-linked warnings where detectable. Swapping the mounts swaps the winner; distinct methods and URL prefixes coexist without spurious conflicts.
- [ ] A repeated mount of the same source is not an automatic hard error; predictable first-match routing and advisory diagnostics apply, while duplicate URL claims *inside a single mount* still fail compilation.
- [ ] Different HTTP methods at the same path coexist, and internal literal-before-parameter route sorting does not change a mount's relative position among explicit registrations.
- [ ] Parameter routes and complex handlers use the same type-checked request/response behaviour as the explicit router.
- [ ] A handler-free dynamic Glaze receives declared URL parameters with type checking; unexpected/missing required props produce clear diagnostics.
- [ ] A typed optional loader supplies a record of Glaze props, automatically rendered with consistent error handling.
- [ ] An unexpected `load` return field not declared by the paired Glaze is a compile-time error, whether from an inline record or another statically known record type; compatible declared fields succeed.
- [ ] A `load` function returning a named struct passes its compatible named fields directly to Glaze without reconstructing an anonymous record; extra, duplicate, inaccessible or mismatched fields remain diagnostic errors, and ordinary Kiln field/module visibility is respected.
- [ ] Defaulted or automatically supplied Glaze props need not appear in `load` results, and every required prop must be satisfied by a valid source; no unused return data is silently forwarded.
- [ ] Loader `throw web.error(404, ...)` and other intentional statuses use existing HTTP error handling and `app.on_error`; unexpected errors use existing safe 500 handling.
- [ ] An optional/absent loader result is not silently converted to a 404 or an empty page; no routing-specific error protocol is introduced.
- [ ] The generated Glaze receives the type-checked combination of URL path props and loader result fields; the loader need not return an already-supplied URL parameter.
- [ ] A duplicate prop name between a URL segment and loader result causes a source-linked compile error, never silent overriding; missing required Glaze props also fail clearly.
- [ ] Loader functions work without `req` when only typed path data is needed, and with an explicitly declared first `req: web.Request` when request context is required.
- [ ] A loader for `/shops/{shop}/products/{id}` may declare only `id: Int`; the Glaze still receives the compatible declared `shop` and `id` URL props, with no placeholder loader argument or ordering dependency. Missing, unknown or incompatible *declared* parameter names/types receive source-linked diagnostics.
- [ ] A `get`/`post`/`put`/`patch`/`delete` handler for `/shops/{shop}/products/{id}` can declare only `id: Int`, without `shop`; other methods in the same route may use different subsets, and the full typed pattern still matches. Invalid names or conflicting types remain compile-time diagnostics.
- [ ] A filesystem HTTP handler can omit `req: web.Request` (`pub fn get() -> web.Response` or `pub fn get(id: Int) -> web.Response`), or declare it first when needed; a misplaced/wrongly typed/duplicated request parameter is a compile-time diagnostic.
- [ ] Omission of request context in a filesystem handler still preserves existing request-time middleware/hooks and full HTTP dispatch; code-defined explicit `app.get(...)` signatures are not silently changed.
- [ ] Generated subset handler binding reuses existing router and handler type checking; explicit code-defined handler semantics do not silently change.
- [ ] URL parameter binding uses names and compatible types; invalid names, types, duplicate/misplaced request parameters and mismatches report precise errors.
- [ ] `[id]` derives its type from matching Glaze, `load` or handler declarations, or defaults to `String`; conflicting annotations produce compile-time errors with both relevant source locations.
- [ ] Bracketed dynamic `.glaze` filenames and route directories (e.g. `[slug].glaze`, `[slug]/index.glaze`) map consistently to Kiln `{slug}` route patterns. Curly-brace filename syntax, embedded partial segments and filename type annotations are not recognized as dynamic routes; malformed brackets/names report useful diagnostics.
- [ ] No filename-encoded type syntax is required; generated route matching uses the same typed conversion rules as explicit Kiln routes.
- [ ] A full GET handler may directly return `web.Response` without bypassing existing hooks, validation or SSG integration.
- [ ] Route directories discover their `route.kiln` consistently; its `load` and HTTP-method functions retain the ordinary language's type checking.
- [ ] Only `pub fn load` and supported `pub fn` HTTP-method exports are registered as route logic. Private helper functions (including private `fn get` or `fn load`) are not implicitly discovered; likely missing-`pub` mistakes receive useful diagnostics without changing Kiln module visibility.
- [ ] Defining both `load` and `get` for the same filesystem route is a compile-time diagnostic with a useful fix; `load` plus a `post`/`put`/`patch`/`delete` handler compiles and dispatches by method correctly.
- [ ] `pages/api/users/route.kiln` without a Glaze registers exported `get`/`post` (and other supported methods), with ordinary typed responses and 404/405 handling; `load` without a sibling `index.glaze` fails compilation.
- [ ] A sibling `index.glaze` remains an implicit GET when `route.kiln` defines only non-GET handlers; API responses retain current Phase 6.6 static-export rules, including non-HTML and dynamic-only behaviour.
- [ ] Only `.glaze` files are discovered as implicit filesystem pages; `.ember` files alone never claim a URL or trigger creation of a generated page wrapper.
- [ ] A routable `index.glaze` may embed an Ember, preserving normal SSR, selective hydration, asset bundling and static-export validation; a standalone Ember in the same folder stays a component.
- [ ] Static, dynamic, mixed, and Ember-using examples preserve Phase 6.6 semantics under `kiln fire --static`.
- [ ] Parameterised routes use typed, finite `static_params()` from `route.kiln` for static export, including multi-parameter records; routes without parameters export without additional boilerplate.
- [ ] A parameterised `index.glaze` plus a `route.kiln` containing only `pub fn static_params()` compiles, pre-renders enumerated Glaze pages at server startup, and exports the enumerated pages under `kiln fire --static`, without `load` or `get`.
- [ ] Enumerated Glaze-only routes still enforce required-prop checks and safe-cache restrictions; an enumeration function cannot invent a GET endpoint for a POST-only directory.
- [ ] In server mode, `static_params()` pre-renders its enumerated pages at startup through the ordinary `.static(...)` code path, honours `app.before` on requests, and retains existing behaviour for unlisted parameter values and failed startup pre-renders.
- [ ] Route-local `static_refresh` uses `.static(refresh: ...)` semantics, including per-page expiration and first-request-after-expiry regeneration; static export remains a one-time output.
- [ ] A zero-parameter `index.glaze` + `route.kiln` declaring only `static_refresh` pre-renders at startup and refreshes on request; no dummy `static_params()` or `static` flag is required. Without `static_refresh`, the page remains request-rendered in server mode while still being included in static export.
- [ ] Parameterised routes with `static_refresh` but no `static_params()` are rejected; the existing shared-cache/request-dependency security rule still applies for zero-parameter pages.
- [ ] Missing, contradictory, invalid or otherwise inapplicable pre-render/refresh declarations yield useful diagnostics; no option can bypass request-dependent shared-cache restrictions.
- [ ] Both filesystem `static_params()` and ordinary explicit `.static(...)` reject request-dependent cached page output, including transitive auth/session/cookie/header/query/body reads; diagnostics identify the unsafe read and the pre-render declaration where possible.
- [ ] Typed route parameters remain valid inputs to pre-rendered pages; request-time `app.before` authorization remains usable for genuinely shared page output and is not mistaken for the cached render phase.
- [ ] Regression tests demonstrate that two users cannot receive one another's personalised cached page after declared pre-rendering; no silent downgrade or unsafe override bypasses the restriction.
- [ ] A route-local `dynamic` marker opts **parameter-free and parameterised** routes out of static export while preserving normal server routing; combining `dynamic` with either `static_params()` or `static_refresh` is a source-linked compile error.
- [ ] A personalised parameter-free `/dashboard` with an implicit `index.glaze` and only `pub const dynamic = true` in `route.kiln` is omitted from `kiln fire --static` but handles normal per-request server GETs; no dummy path parameter or full handler is required.
- [ ] Missing enumeration/dynamic intent on a parameterised GET route fails static export with the existing compatible diagnostic; existing Phase 6.6 validations still apply.
- [ ] `kiln check`, `run`, `dev`, `test`, LSP and VS Code agree on the project route graph.
- [ ] Real-browser tests cover SSR, selective hydration, typed RPC restrictions in static export and common error paths.
- [ ] Documentation names all conventions and limitations; examples distinguish proposed syntax from shipped functionality until delivered.

## 12. Decision log and next questions

**Settled in conversation, 2026-10-27:**
- Opt-in routing from a directory with conventional file-based pages, with complex routes able to use directories.
- Coexistence with explicit `app.get` routing rather than replacing it.
- Default recursive `_components` exclusion; **custom mount-specific `ignore` lists replace the default exclusions** (Option A), and `ignore: []` disables directory-name exclusions entirely. Ignored files remain importable, and `_components` is not intrinsically reserved.
- **Directory-basename-only ignore entries (Option A):** no glob or slash-delimited path patterns in the initial API. Each listed literal basename is excluded recursively at any depth; validate malformed names rather than accepting ambiguous patterns.
- Project-root aliases are a separate general import feature (see #76), and `_components` remains the routing convention by default.
- **Three-level progressive routing**: simple Glaze page with automatic typed URL props; Glaze plus optional typed `load` for data; or a full ordinary route handler for control. Exact signatures remain under design.
- **Complex route convention:** route-specific functions belong in a file named **`route.kiln`** inside the route directory; it may export `load` or an ordinary `get` handler, plus other HTTP-method functions including `post`, `put`, `patch` and `delete`. Exact return/error contracts are still under design.
- **Explicit `pub` entrypoint requirement (Option A):** only publicly exported `load` and supported HTTP-method functions in `route.kiln` are automatically discovered. Private functions are ordinary helpers, including when named `get` or `load`; no implicit visibility promotion.
- **GET exclusivity:** `load` and `get` cannot coexist for the same route; diagnose the conflict at compile time. `load` can coexist with handlers for other HTTP methods.
- **Loader inputs (Option A):** loaders may optionally declare `req: web.Request` **first**, followed by typed URL path parameters bound by name. Request-free loaders remain concise.
- **Glaze props (Option A):** combine typed URL route parameters with the typed record returned by `load`; duplicate names cause compile-time errors and required Glaze props are type-checked against that combination.
- **Strict loader return fields (Option A):** extra/unused fields from `load` that are not declared by the Glaze are compile-time errors, even if they are part of a shared record type. Matching remains by name and type; URL params and defaulted props need not be forwarded by the loader.
- **Named-struct loader returns (Option A):** loaders may return either anonymous typed records or named structs directly. Both use the same strict field name/type matching, duplicate detection and normal access/privacy rules; no redundant anonymous-record reconstruction required.
- **Subset URL parameters in loaders (Option A):** `load` declares only the named path parameters it uses. Parameters omitted from its signature remain part of the compiled route and are provided to matching Glaze props independently. No unused dummy arguments or positional binding; type mismatches and duplicate returns remain errors.
- **Subset URL parameters in HTTP handlers (Option A):** ordinary `get`, `post`, `put`, `patch` and `delete` exports in `route.kiln` may declare only their needed URL parameters. Bind by name, check the full route against the shared inferred type graph, and generate ordinary route adapters as necessary. This does not change explicit registration contracts.
- **Optional HTTP request parameter (Option A):** all filesystem route HTTP method functions can omit `req: web.Request` when unused, just like loaders; if present it must be the first argument. Request-free handlers can have no arguments or only selected named path parameters, without changing ordinary hooks, dispatch or cache safety.
- **Parameter typing (Option A):** `[id]` is only a name; infer the route parameter type from matching Glaze, `load` or handler declarations, defaulting to `String` without an annotation. Conflicting declarations are compiler errors. Do not encode types in route filenames.
- **Loader error handling (Option A):** use existing Kiln `try`/`throw`, `web.error`, unexpected-error 500 and `app.on_error` behaviour. No special `none` → 404 conversion.
- **Filesystem SSG (Option A):** parameterised routes enumerate typed pages with route-local `static_params()` in `route.kiln`, or opt out through a route-local `dynamic` marker. The implementation must reuse the existing `.static(...)`/`.dynamic()` and static-export pipeline, not add a second exporter.
- **Standalone `static_params()` with Glaze (Option A):** `route.kiln` may export only `static_params()` alongside `index.glaze`; a loader or GET handler is not required. The implicit Glaze GET receives typed URL props, and its enumerated instances are pre-rendered for both server mode and static export. No GET-capable page means enumeration alone cannot register one.
- **Server-mode `static_params()` (Option A):** the same enumeration also pre-renders pages at server startup via existing `.static(...)` behaviour, including hooks and fallback for unlisted or unsuccessfully pre-rendered pages. It is not export-only.
- **Safe pre-rendering (Option A):** reject request-dependent *cached output* (personalisation, cookies, auth/session, arbitrary headers/query/body) for filesystem and explicit `.static(...)` routes; typed enumerated URL parameters are safe inputs, and separately executed `app.before` hooks may still inspect request context. Extend static-export safeguards to server-mode pre-rendering where necessary.
- **Route-local refresh (Option A):** support an optional `static_refresh` duration in `route.kiln` with the same server-mode first-request-after-expiry re-render semantics as `.static(refresh: ...)`. For a route with **no URL parameters**, `static_refresh` alone triggers startup pre-rendering: no `static_params()` or extra `static` declaration needed. A static export is still a snapshot; refresh does not override safe-cache restrictions.
- **Dynamic-only routes without URL parameters (Option A):** `pub const dynamic = true` is valid on fixed paths (e.g. an authenticated `/dashboard`) as well as parameterised ones. It excludes the route from `kiln fire --static` while retaining normal server-mode request rendering, and conflicts with either `static_params()` or `static_refresh`.
- **API-only routes (Option A):** a directory with `route.kiln` alone can register typed HTTP methods without `index.glaze`. A `load` export requires a sibling `index.glaze` and is an error without one. A view plus only non-GET exports retains normal implicit GET rendering; JSON APIs are subject to existing static-export restrictions.
- **Standalone Ember pages (Option A):** `.ember` files never automatically register filesystem routes. A `.glaze` page embeds an Ember for interaction; API-only `route.kiln` methods remain independently routable.
- **Explicit vs filesystem route precedence (Option A):** preserve the existing **first matching route wins** rule using registration order. `app.file_routes` occupies its call position; earlier explicit routes shadow matching filesystem routes and earlier filesystem routes can shadow later explicit routes. Provide nonfatal shadowing diagnostics where detectable. Do not alter the separate duplicate-error rule inside a single mount.
- **Dynamic route filenames (Option A):** use square-bracket `[name]` path segments in `.glaze` filenames or route directories (both flat and nested forms), translating to ordinary `{name}` URL parameters. No curly-brace filenames or encoded types; validate identifiers and duplicate parameter names.
- **Multiple filesystem mounts (Option A):** cross-mount overlaps resolve by mount registration order, with nonfatal shadowing warnings. Do not turn cross-mount overlaps into compile errors; retain hard duplicate checks only for claims *within* a single mount.
- **Framework-reserved endpoint collisions (Option A):** compiler error when any discovered filesystem route can intercept a Kiln-reserved method/path (e.g. `/_kiln/rpc`), irrespective of registration order. Check mount URL prefixes and compatible dynamic patterns against framework-declared reservations; normal application-route precedence is unchanged.

**Needs an explicit decision before locking the specification:**
1. **Remaining loader contract:** final precise return-type restrictions and minor syntax/visibility questions (§§5–6). Named parameter binding, **loader subset URL parameters**, optional first `req`, automatic props merging, **strict no-extra-fields matching**, named-struct/anonymous-record success returns, duplicate-name errors, type inference/default `String` and ordinary error/404 propagation are settled.
2. **`route.kiln` contracts:** remaining exact function return/error typing, module boundaries, and how flat Glaze files graduate to route directories (§5). The filename, **`pub` requirement for loader/HTTP handler discovery**, optional request context for both, subset URL parameter binding for both, and permission for handler-only API routes are settled.
3. **SSG refinements:** exact export/marker signatures, enumeration failures and unusual handler results (§8). Route-local enumeration **without requiring `load` or `get` when an implicit Glaze GET exists**, server-startup pre-render, `static_refresh` alone on zero-parameter routes, contradictory declaration errors and rejection of request-dependent cached output (including explicit `.static(...)`) are settled.
4. **Remaining route file naming:** filename/path character restrictions, handling reserved names, static-file extensions, catch-all or optional patterns and other edge cases (§4). Square-bracket `[name]` dynamic segments are now settled; standalone `.ember` pages are excluded.
5. **Remaining collision policy:** partial overlaps of dynamic parameter patterns within one mount (§4). **Framework-owned endpoint collisions are now always compile-time errors** for filesystem routes (Option A), including conflicting mount prefixes or dynamic route patterns. Registration-order precedence remains settled for explicit/file and cross-mount filesystem application routes; true duplicates within one mount remain errors. The exact framework reservation registry requires implementation review.
6. **Remaining ignore API details:** exact case/Unicode normalisation across supported filesystems and diagnostics for invalid entries (§3). The `ignore` option name, per-mount scope, recursive exclusion, **directory-basename-only matching (no globs)** and replacement (not additive) behaviour, including `ignore: []`, are settled.
7. **Roadmap:** candidate Phase 6.7 before 1.0, or a post-1.0 compatible addition? (§10)

Keep this document updated as each decision is made, with changes in this draft PR until the spec is ready to merge.
