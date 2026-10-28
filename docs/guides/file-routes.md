# Pages as files

Routes in Kiln are written in code: `app.get("/about", ...)`. When a site
has many pages, `app.file_routes` can find them in a folder instead:
each `.glaze` file is a page, and its path is its URL. It is a choice,
not a project layout: a program without it works as before, and one
with it can still write any route by hand.

```kiln
import web

fn main() ! {
    let app = web.app()
    app.file_routes("pages")
    try app.listen(":8080")
}
```

```text
pages/
  index.glaze              GET /
  about.glaze              GET /about
  plants/
    index.glaze            GET /plants
    [slug]/
      index.glaze          GET /plants/{slug}
      route.kiln           its data, and the pages a static site has
  api/water/route.kiln     POST /api/water
  _components/layout.glaze not a page
```

[`examples/garden`](https://github.com/IZ737/kiln/tree/main/examples/garden)
is a complete site built this way, and `kiln new --pages` makes a small
one to start from.

The routes are found **when the program is compiled**, never by reading
folders while it runs. `app.file_routes(...)` adds ordinary routes, the
same ones `app.get` would, so hooks, `on_error`, `kiln test`, embers,
pre-rendering and `kiln fire --static` treat them like any other.

## Mounting a folder

```kiln
app.file_routes("pages")                         // pages/ answers under /
app.file_routes("admin", at: "/admin")           // admin/index.glaze is /admin
app.file_routes("pages", ignore: ["_components", "partials"])
```

- The folder is relative to the project root, written with `/`.
- `at:` is the URL the folder's pages go under (default `/`).
- `ignore:` lists folder names, at any depth, that hold components
  rather than pages. It replaces the default, `["_components"]`, so
  name `_components` again to keep it. Folders whose names start with
  `.` are always left out.

The three are written out as string literals, since the compiler reads
them. A program can mount several folders.

A mounted folder is part of the project, like an imported one: its
components are named by their path, as in
`<pages._components.PlantLink />`, and work in a folder named with
brackets too (`<pages.plants.[slug].Card />`).

## Pages and URLs

- `about.glaze` is `/about`, and `index.glaze` is its folder's URL.
- `[slug]`, as a folder or a file name, is a URL parameter: one whole
  segment, so `pages/blog/[slug].glaze` answers `/blog/{slug}`. A page's
  props of the same name are given the parameter.

Catch-alls (`[...rest]`), optional segments (`[[page]]`), route groups
(`(marketing)`) and parameters inside a segment (`post-[id]`) are not
part of `file_routes`; each is an error that says what to do instead.
Register such a route by hand with `app.get`.

A parameter is a `String` unless something gives it a type: a page
prop, a `load` parameter or a handler parameter. `{% props id: Int %}`
makes `/posts/[id]` a `/posts/{id: Int}` route, which does not match
`/posts/new`. A parameter can be `String`, `Int`, `Float` or `Bool`; two
places that give it different types are an error.

## route.kiln

A folder's `route.kiln` is a module of the folder that gives its
`index.glaze` page data, or answers other methods. Only `pub`
declarations with these names are part of the route:

```kiln
// pages/plants/[slug]/route.kiln
import plants

// The page's props, from the URL's parameters by name.
pub fn load(slug: String) -> { plant: plants.Plant } ! {
    return { plant: try plants.find(slug) }
}

// The pages a static site has, and that the server renders when it starts.
pub fn static_params() -> List<String> {
    return plants.all().map(fn(p) { p.slug })
}
```

- **`load`** returns the page's props, as a record or a struct. It takes
  URL parameters by name, after an optional `req: web.Request`, and may
  fail: `throw web.error(404, "...")` answers like any handler would.
  Every field it returns is a page prop, and every prop the URL and
  `load` do not give needs a default.
- **`get`, `post`, `put`, `patch`, `delete`** make a whole response for
  their method, with the same parameters as `load`. A folder with only a
  `route.kiln` is an endpoint, such as a JSON API. `load` and `get` both
  answer `GET`, so a folder has one or the other.
- **`static_params`** lists the pages of a route with parameters: the
  parameter's value for each, or a record of them when there are
  several. It is `.static(...)` for the route.
- **`pub const static_refresh = 10m`** renders a page again on the first
  request after that long, as `.static(refresh: 10m)` does. For a page
  without parameters it alone renders the page when the server starts.
- **`pub const dynamic = true`** leaves the route out of static pages,
  as `.dynamic()` does. It cannot go with `static_params` or
  `static_refresh`.

A private declaration with one of these names gets a warning, since the
route does not read it.

## Order and conflicts

Explicit routes are tried in the order they were added, and a mount adds
its routes where it is called, in this order: literal segments before
parameters, and `Int` and `Bool` parameters before `Float`, and those
before `String`. So `pages/posts/new.glaze` answers `/posts/new` before
`pages/posts/[slug]` sees it.

The compiler reports what would surprise you instead of choosing:

- two files with the same URL and method (`about.glaze` and
  `about/index.glaze`), or two routes in one mount that match some of
  the same URLs while neither covers the other: error **E0239**;
- a page under `/_kiln` or on a path Kiln itself serves: E0239;
- a filesystem route an earlier explicit route shadows, so it never
  answers some URL: warning **E0241**.

Errors about a page or a `route.kiln` point at that file. In an editor,
hovering `app.file_routes` lists the routes it adds, and go to
definition opens their files.

## Static sites

`kiln fire --static` writes a mount's pages like any routes (see
[Static sites](static-sites.md)): every page without parameters, and
the pages `static_params` lists. An API-only `route.kiln` that a static
host cannot serve is an error until it says `pub const dynamic = true`.

A page that is rendered ahead of time, by `static_params`,
`static_refresh` or `.static(...)`, must not depend on who asks. When
such a page reads the request (a cookie, a header, the query) the server
logs an error and renders it on every request instead of keeping it,
and `kiln fire --static` fails. Hooks added with `app.before` run on
every request and may read it.
