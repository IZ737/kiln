# Static sites

A Kiln web app can also be a static site: `kiln fire --static` renders
its pages once and writes them as files, with their styles, images and
embers, for any static host or CDN (GitHub Pages, Netlify, Cloudflare
Pages, an S3 bucket, or `python3 -m http.server`). No Kiln program runs
on the host. The same program still runs as a server with `kiln run`
or `kiln dev` while you write it.

```sh
kiln fire --static                        # the program in this folder, into dist/
kiln fire --static examples/static_blog -o public
```

[`examples/static_blog`](https://github.com/IZ737/kiln/tree/main/examples/static_blog) is a complete
site: a layout, a stylesheet, an image, posts listed by slug, an Atom
feed, a redirect, a 404 page and an ember.

## Which pages a site has

`kiln fire --static` builds the program and runs it. When it reaches
`app.listen`, it renders pages instead of serving them:

- every `GET` route without parameters, such as `/` or `/about`, and
  the files the app serves (its CSS, `static/` files and ember code);
- for a route with parameters, the pages its `.static(...)` lists.
  The function returns each page's parameter, or a record of them when
  the route has more than one:

```kiln
app.get("/posts/{slug}", fn(req, slug) { show_post(slug) })
    .static(fn() { posts().map(fn(p) { p.slug }) })

app.get("/docs/{dir}/{page}", fn(req, dir, page) { show_doc(dir, page) })
    .static(fn() { [{ dir: "guides", page: "tooling" }] })
```

- `.dynamic()` leaves a `GET` route out, and `POST`, `PUT`, `PATCH` and
  `DELETE` routes are never written: a static host cannot answer them.

Each page is rendered with an empty request: no query string, headers,
cookies or body.

## Where pages go

| Path | File | |
| --- | --- | --- |
| `/` | `index.html` | |
| `/about` | `about/index.html` | Every host serves it at `/about` and `/about/` |
| `/docs/` | `docs/index.html` | |
| `/feed.xml` | `feed.xml` | A path whose last part has a `.` is written as it is |
| a redirect | a small page that sends the browser on | `/docs` to `/docs/` is left to the host |
| `app.on_error` | `404.html` | The page most hosts show for a missing path |

So that a host serves each file with the right type, a page that is not
HTML needs a file extension in its path: write `/feed.xml`, not `/feed`.

The output folder is `dist/` in the program's folder unless you give
`-o`. kiln replaces a folder it wrote before, and refuses to touch a
folder of yours that is not empty.

## What is checked

A site that would be broken on its host is an error, with the source it
comes from, and the command fails:

| Error | |
| --- | --- |
| `E0237` | A route with parameters and no `.static(...)` list, or one whose list fails |
| `E0404` | A page that reads the query string, a header, a cookie or the body; that answers other than 200 or a redirect; or that is not HTML at a path without an extension |
| `E0405` | An ember on a written page calls a `server fn`, which needs a server |
| `E0406` | A link, form, image, script or stylesheet `url()` points to a page the site does not have, or a form sends a `POST` |

`kiln explain` gives each one's fix. Links to other sites are not
checked, and a link to a route already reported is not reported again.

## Embers

Embers work on a static site as they do on a server: the page has their
HTML, and the browser hydrates them with the same WebAssembly. Their
state, events, stores, `browser` and npm calls need nothing more. A
`server fn` does, so an ember that calls one cannot be on a static page.

## Mixing static and dynamic pages

In a program that runs as a server, `.static(...)` marks pages to render
ahead of time: the server renders them once when it starts, before it
takes requests, and answers them from memory after `app.before` hooks
run. A page the list leaves out is rendered on each request, as usual.

```kiln
app.get("/", home).static()                        // rendered once
app.get("/prices", prices).static(refresh: 10m)    // again every ten minutes at most
app.get("/search", search)                         // on each request
```

With `refresh`, the first request after that long renders the page again.
A page that fails when the server starts is logged and rendered on each
request instead.
