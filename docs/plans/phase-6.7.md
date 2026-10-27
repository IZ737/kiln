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

The exact method/options spelling, file naming conventions and handler contracts below are **provisional** until decided.

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
- `[slug].glaze` maps to a parameter segment. The final spelling and parameter type inference are not decided.
- For complex routes, a directory may instead contain `index.glaze` plus an optional `route.kiln` (see §5). For instance `blog/[slug]/index.glaze` would map to the same path as `blog/[slug].glaze`; including both is an error.
- Plain components outside mounted directories are not route candidates. Only **mounted directories** are scanned.
- The initial automatic-page convention is for **server-rendered `.glaze` pages**; `.ember` components can be embedded normally. Whether standalone `.ember` files may directly claim routes is an **open question**.

**Routing collisions and path correctness**

- Reject duplicate method/URL mappings discovered in the same mount, including file-versus-directory equivalents.
- Within one mount, a literal path (e.g. `/blog/new`) must take precedence over the corresponding parameterised path (e.g. `/blog/{slug}`). Specify the ordering during route-table generation.
- The existing router dispatches **the first matching route**. Do not silently change global precedence for applications using explicit `app.get` etc. A mount occupies its position in explicit registration order; interactions between multiple mounts need precise tests and documented rules.
- Preserve URL percent-decoding, typed route parameter matching, HEAD→GET behaviour, 404/405 responses, `app.before` hooks, and server-side security rules.
- Reject path traversal, unexpected symlinks, case-folded collisions and invalid parameter names. Treat case sensitivity consistently across supported operating systems.
- Consider overlap/conflicts with reserved Kiln endpoints such as `/_kiln/rpc` during implementation review.

## 5. Handler escape hatch: provisional

A page that needs data fetching, authentication, non-GET actions or custom responses may use a route directory:

```text
pages/
  blog/
    [slug]/
      index.glaze
      route.kiln
```

Possible **illustrative** handler syntax:

```kiln
import web, posts

pub fn get(req: web.Request, slug: String) -> web.Response ! {
    let post = try posts.find(slug)
    return web.render("index.glaze", { post })
}
```

This is *not* agreed syntax or behaviour; it is a design candidate.

The handler should ultimately be an ordinary typed Kiln function registered with the existing router. A route directory might also declare `post`, `put`, `patch` and `delete` handlers. Do not add a separate request/response type.

**To decide:** exact handler filename (possibly `route.kiln`), whether handlers can be colocated with a flat `about.glaze` or only in route directories, how their exports are discovered, where their module boundary lies, whether a missing `get` permits automatic Glaze rendering, and the contract for request, parameter and props types.

## 6. Dynamic URL parameters and data loading: not decided

Three options remain under consideration; previous discussion did **not** select one:

- **A — Automatic parameter-to-Glaze prop injection.** For a handler-free `blog/[slug].glaze`, the route parameter `slug` becomes an ordinary Glaze prop with a type-checked contract. This is the most concise option for simple dynamic pages, but typing, default props and precedence require design.
- **B — Require a handler for dynamic pages.** Avoids implicit data flow, but adds boilerplate to simple cases.
- **C — An optional typed `load` function.** Returns a record of props automatically rendered by the paired Glaze, preserving full programmatic routing for advanced cases. Potentially elegant, but another convention to learn and maintain.

Open details: typed `[id]` parameters (`Int`, `Bool`, etc.), optional versus required props, multi-segment parameter records, missing objects/404 and redirects, async/fallible handlers, and type-checker diagnostics. Prefer existing route-parameter and `web.render` rules whenever possible.

## 7. Layouts, components, aliases

- For the first version, Glazes continue to use explicit `{% layout "..." %}`, slots and ordinary component tags. **Do not silently introduce automatic nested layout inheritance**.
- A routed Glaze may embed an Ember using existing rendering/hydration behaviour. A page without Embers must not gain an unnecessary client runtime.
- Cross-folder Glaze/Ember resolution and imports must be correct before this becomes reliable. Track those separately in [#75](https://github.com/IZ737/kiln/issues/75) and [#76](https://github.com/IZ737/kiln/issues/76).
- The planned built-in `@/` project-root import alias and configurable aliases in `kiln.toml` belong to **general import design** ([#76](https://github.com/IZ737/kiln/issues/76)), not to the route scanner. They can help routed pages access components anywhere in the project.
- `_components` exclusion and import aliases solve different problems: one determines which files become URLs, the other determines where source imports resolve.

## 8. Static-site generation must reuse Phase 6.6

Discovered routes must enter the existing route/export machinery, not implement a second exporter:

- A discovered `GET /about` with no route parameters is exported automatically by `kiln fire --static`, just like an explicit nonparameterised GET route.
- A discovered `GET /blog/{slug}` must have a **finite, typed enumeration** of export values; how a filesystem route supplies this through existing `.static(...)()` semantics is to be designed. Do **not** guess all database records or URLs at build time.
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

**Needs an explicit decision before locking the specification:**
1. **Dynamic page model:** direct URL-parameter props, required handlers, or a typed optional loader? (§6)
2. **Complex-route convention:** which handler filename, signatures, module boundaries and how flat Glaze files graduate to route directories? (§5)
3. **Dynamic static export:** what supplies `.static(...)` values and allows `.dynamic()` for filesystem routes? (§8)
4. **Route file naming:** `[slug]` spelling, typed parameters, extensions and optional `.ember` page files? (§4)
5. **Collision policy:** explicit route versus filesystem mount interactions, duplicate mount URL scope, and reserved paths? (§4)
6. **Ignore API details:** confirm `ignore` name and replacement rather than additive semantics; currently recorded as proposed in §3.
7. **Roadmap:** candidate Phase 6.7 before 1.0, or a post-1.0 compatible addition? (§10)

Keep this document updated as each decision is made, with changes in this draft PR until the spec is ready to merge.
