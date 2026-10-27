# Glaze syntax reference

Glazes (`.glaze` files) are server-only templates and components: HTML with Kiln code in
it. This is the syntax the current parser accepts; every form here is covered
by a test in `crates/kiln_syntax/tests/glaze.rs`. Code inside braces is
ordinary Kiln, as in [syntax.md](syntax.md).

`kiln check` type-checks glazes together with the `.kiln` files in the same
directory: see [types.md](types.md#glazes) for the rules.

Compiled Glazes, typed `ui` styles, Finishes, CSS Modules, design tokens and
themes are implemented. Browser-capable components use explicit `.ember` files
and support reactive state, WasmGC hydration and typed server calls. Local
[folder modules](projects.md) keep their own components, styles and assets.
This page documents the current syntax; [Rendering](#rendering) describes the
compiled output. Implementation history is in the [Phase 3](../plans/phase-3.md),
[Phase 4.1](../plans/phase-4.1.md) and [Phase 4.2](../plans/phase-4.2.md) plans.

## A page

```glaze
{# post.glaze #}
{% props post: Post, user: User? = none %}
{% layout "base.glaze" title: post.title %}

<article>
  <h1>{post.title}</h1>                  {# escaped automatically #}
  {raw post.html}                        {# trusted HTML, not escaped #}

  {% if user is some(u) and u.can_edit(post) %}
    <a href="/posts/{post.id}/edit">Edit</a>
  {% end %}

  {% for tag in post.tags where tag.visible %}
    <Tag name={tag.name} />              {# the component in tag.glaze #}
  {% empty %}
    <p>No tags</p>
  {% end %}
</article>
```

## Output

| Form | Meaning |
| --- | --- |
| `{value}` | Output a value, escaped for where it appears |
| `{raw value}` | Output trusted HTML without escaping |
| `{# comment #}` | Dropped from the output |
| `{"{"}` | A literal brace in text |

`<script>` and `<style>` contents are raw text: braces there are CSS or
JavaScript, not values.

## Rendering

`web.render("post.glaze", { post })` renders a glaze into a `200` response
with `content-type: text/html; charset=utf-8`, or another status with
`status:` and more headers with `headers:`. Glazes are compiled into the
program like any other code, with nothing interpreted at run time; the
glaze must be named in quotes, so the compiler knows which one it is.

A glaze, layout or component that declares `request: web.Request` in its
props gets the request being handled without anyone passing it, so a
layout can show who is signed in or pick a theme from a cookie:

```glaze
{% props title: String, request: web.Request %}
<body class="{theme_of(request)}">
```

A caller may still pass `request:` itself, and must when there is no
request, as in a test that renders a glaze directly. Embers cannot take
the request, since it does not go to the browser.

- **Values are escaped.** `{value}` and values in attributes have `&`, `<`,
  `>`, `"` and `'` escaped, so a value cannot add markup or end an
  attribute. Values that are not strings are shown as `print` shows them.
  Text written in the glaze itself is copied as written.
- **Links cannot run code.** In attributes that browsers follow as URLs
  (`href`, `src`, `action`, `formaction`, `poster`, `cite` and the like),
  a value that decides the URL's scheme must be relative or `http`,
  `https`, `mailto` or `tel`; anything else, such as `javascript:...`,
  becomes `about:invalid#kiln-unsafe-url`. A value after a fixed start,
  as in `href="/posts/{post.id}"`, is only escaped.
  HTML character references in a static prefix cannot hide a scheme:
  `href="java&#x73;{suffix}"` checks the decoded prefix with the value.
  Only authored text is decoded; character references supplied by a value
  remain literal text.
- **Some attributes take no values** (E0309): `on...` attributes such as
  `onclick` run JavaScript, `srcdoc` is a whole page, and `style` is CSS,
  which accepts only `Int` and `Float` values (`style="width: {pct}%"`).
- **`{raw value}` is the trust boundary.** It outputs a `String` as HTML,
  unescaped: use it only for HTML your program made or trusts, never for
  text from users.
- A `Bool` attribute value adds the attribute or leaves it out:
  `disabled={busy}` renders `disabled` or nothing. Void elements render
  without a closing tag, and `<div />` renders as `<div></div>`.
- A line holding only tags and comments, such as `{% if %}` or
  `{% end %}`, renders as nothing rather than a blank line, and the line
  break that ends the file is left out, so a component fits inside a line.
- A layout renders around the page, with the page in its `{% slot %}`; a
  component renders where its tag is, with its children in its slot. A
  named slot renders the matching `{% fill %}`, or nothing. Slot content
  is placed without the whitespace it starts and ends with.
- A glaze that always renders itself again, through its layout or
  components outside `if`, `for` and `match` blocks, is E0310. Inside a
  block that ends, a component can render itself, as for a tree.
- Browser-capable components use `.ember`. State or `on:` handlers in a
  `.glaze` are E0314; rename the interactive component to `.ember`. A Glaze
  can render an Ember without becoming an island itself (see [Embers](#embers)).

## Tags

| Tag | Meaning |
| --- | --- |
| `{% props name: Type, other: Type = default %}` | The glaze's inputs. Once, at the top level |
| `{% layout "base.glaze" title: value %}` | Wrap this glaze in a layout. Once, at the top level |
| `{% slot %}`, `{% slot name %}` | In a layout or component: where the caller's content goes |
| `{% fill name %} ... {% end %}` | Content for `{% slot name %}`: directly inside a component tag, or at the top level of a page with a layout |
| `{% css "site.css" %}` | Use a global stylesheet. At the top level |
| `{% css module name from "card.module.css" %}` | Use a CSS Module: `name.card` is its renamed class. At the top level |
| `{% state name = value %}`, `{% state name: Type = value %}` | Mutable reactive state in `.ember` files only. At the top level |
| `{% let name = value %}` | A local value |
| `{% if c %} ... {% else if d %} ... {% else %} ... {% end %}` | Conditional |
| `{% for x in xs where c %} ... {% empty %} ... {% end %}` | Loop; `where` and `empty` are optional |
| `{% match v %} {% case pattern if guard %} ... {% end %}` | Pattern match, with the patterns of `match` |

Every block ends with `{% end %}`.

## Layouts and slots

A layout marks where pages go with `{% slot %}`, and other places with
named slots, which pages fill with `{% fill %}`:

```glaze
{# base.glaze #}
{% props title: String %}
<head><title>{title}</title>{% slot head %}</head>
<body><aside>{% slot sidebar %}</aside><main>{% slot %}</main></body>
```

```glaze
{# guide.glaze #}
{% layout "base.glaze" title: "Guide" %}
{% fill sidebar %}<a href="/">Home</a>{% end %}
<h1>Guide</h1>      {# everything else fills the default slot #}
```

Components fill their named slots the same way, among their children:
`<Card>{% fill title %}Hi{% end %}<p>Body</p></Card>`. A fill must name a
slot the glaze has, once each (E0311).

## Styles

Glazes bring CSS three ways, and the program serves all of it itself:

```glaze
{% css "styles/site.css" %}                      {# global #}
{% css module card from "styles/card.module.css" %}
<article class={card.card}>
    <h2 class="title {card.card_title}">A pot</h2>
</article>
<style>
.title { color: #C65A34; }                       {# only this glaze's .title #}
:global(.dark) .title { color: white; }
</style>
```

- **`<style>` is scoped.** Its rules apply only to the glaze's own
  elements, which get an attribute such as `data-k-1a2b3c`, so two glazes
  can both style `.title`. `:global(...)` leaves a part of a selector
  unscoped. Content a glaze passes into a component's slot belongs to that
  glaze. The `<style>` element itself is not in the HTML.
- **`{% css "file.css" %}` is global**, as if linked. Its `@import`s and
  `url(...)`s are followed.
- **`{% css module x from "file.module.css" %}`** renames each class so it
  cannot clash, and gives the glaze a record `x` of the new names, one
  field per class with `-` written `_`: `x.card_title`. A class the file
  does not have is a type error.

Every `<head>` gets a `<link>` to one stylesheet built from all of this, in
CSS cascade layers in a fixed order: `kiln.global`, then `kiln.modules`,
then `kiln.scoped` (and `kiln.ui`, for later). CSS outside layers, such as a
framework's, still wins over them as CSS says. The stylesheet and the files
it refers to are served from inside the program at `/_kiln/name-hash.ext`
and cached for good, since a change gives a new name. The files in the
program's `static/` folder are served at `/static/...`.

Paths are relative to the component's folder, and must stay inside the project. CSS
that does not parse, a missing file, or an `@import` in a `<style>` or CSS
Module is E0312. CSS is read with
[Lightning CSS](https://lightningcss.dev), and written minified.

## Typed styles

`ui={{ ... }}` styles an element with a record of keys, checked by the
compiler and turned into CSS classes; no class names to invent and no
Tailwind:

```glaze
{% css "design.css" %}
<article class="card" ui={{
    grid: true, cols: 1, gap: space.md, p: space.lg,
    bg: color.surface, rounded: radius.card,
    border_color: if featured { color.accent } else { .transparent },
    hover: { shadow: shadow.lg },
    at: { tablet: { cols: 2 } },
}}>
```

In Kiln code, the same record is `ui({ ... })`, of type `Ui`, and any
record where a `Ui` is expected is one. A function that returns styles is
a **Finish**: an ordinary function, which may take an enum to choose a
variant and return a struct of several `Ui`s for a glaze's parts:

```kiln
fn button(tone: Tone) -> Ui {
    let base = ui({ px: space.md, py: space.sm, rounded: radius.card })
    return ui.merge(base, match tone {
        Tone.Plain => ui({ bg: color.surface })
        Tone.Accent => ui({ bg: color.accent, hover: { opacity: 0.9 } })
    })
}
```

`ui.merge(a, b, ...)` combines styles; for each property, state and
breakpoint the later style wins, so `px` after `p` changes only the left
and right padding. `ui.classes(style)` is a style's class names as a
`String`. An element's own `class` stays, before the style's classes.

| Keys | Values |
| --- | --- |
| `bg`, `fg` (text colour), `border_color` | `color.*` tokens, `.transparent`, `.current` |
| `p`, `px`, `py`, `pt`, `pr`, `pb`, `pl`, `gap` | `space.*` tokens, `0` |
| `m`, `mx`, `my`, `mt`, `mr`, `mb`, `ml` | `space.*` tokens, `0`, `.auto` |
| `w`, `h`, `min_w`, `max_w`, `min_h`, `max_h` | `size.*` or `space.*` tokens, `0`, `.auto`, `.full`, `.min`, `.max`, `.fit` |
| `rounded` | `radius.*` tokens, `.none`, `.full` |
| `border` | `true` (1px solid), or a width |
| `shadow` | `shadow.*` tokens, `.none` |
| `flex`, `grid` | `true` (not both in one style) |
| `direction` | `.row`, `.column`, `.row_reverse`, `.column_reverse` |
| `cols`, `rows` | `1` to `12` equal tracks, `.none` |
| `items` | `.start`, `.center`, `.end`, `.stretch`, `.baseline` |
| `justify` | `.start`, `.center`, `.end`, `.between`, `.around`, `.evenly`, `.stretch` |
| `wrap`, `underline` | `true` or `false` |
| `font` | `font.*` tokens |
| `text_size` | `text.*` tokens; `.lg` is `text.lg` |
| `weight` | `.thin`, `.light`, `.normal`, `.medium`, `.semibold`, `.bold`, `.black`, `100`-`1000`, `weight.*` |
| `line_height` | a number, `leading.*` tokens |
| `align` | `.left`, `.center`, `.right`, `.justify`, `.start`, `.end` |
| `tracking` | `tracking.*` tokens |
| `opacity` | `0` to `1` |
| `overflow` | `.hidden`, `.auto`, `.scroll`, `.visible`, `.clip` |
| `transition` | `motion.*` tokens, `.none` |
| `cursor` | `.pointer`, `.default`, `.text`, `.not_allowed`, `.grab`, `.move`, `.wait`, `.help`, `.none` |
| `transform` | a CSS string, `.none` |
| `hover`, `focus`, `focus_visible`, `disabled` | a record of the keys above |
| `at` | a record of breakpoints, each a record of keys (and states) |

Every key also takes a CSS string written out, such as `p: "1rem 2px"`,
which is checked as CSS for its property. Values are written in place:
code chooses between them with `if` and `match`, but cannot compute them
(E0220), because each becomes an atomic CSS class when the program is
compiled. Rarer CSS properties go in a `<style>`.

**Tokens** are the CSS custom properties that global stylesheets declare,
named `--<category>-<name>`: `--space-md` is `space.md` and
`--color-on-accent` is `color.on_accent`. The categories are `color`,
`space`, `size`, `radius`, `shadow`, `font`, `text`, `weight`, `leading`,
`tracking`, `motion` and `breakpoint`. A style refers to the property
(`var(--space-md)`), so a stylesheet that changes it changes every style.
Breakpoints are widths: `at: { tablet: ... }` is
`@media (min-width: <--breakpoint-tablet>)`, in the order they are
declared. A token or breakpoint that is not declared is E0221.

The classes are in the `kiln.ui` layer, after the other layers, and a
class name is `k` and twelve letters and digits.

### Themes

A theme is a value of a struct of Finishes, so the type checker checks
that every theme has every part, with the right arguments and results,
and the glazes take the theme as a prop:

```kiln
struct SiteTheme {
    page: fn() -> PageFinish
    card: fn(Bool) -> CardFinish
    button: fn(Tone) -> Ui
}

fn editorial() -> SiteTheme {
    return SiteTheme(page: editorial_page, card: editorial_card, button: editorial_button)
}

// The one place that chooses the theme.
fn active_theme() -> SiteTheme {
    return editorial()
}
```

A theme's token values are a rule for `.theme-<name>` in a global
stylesheet, and `theme: .<name>` in a style (at its top, not inside a
state or breakpoint) adds that class, so the tokens take the theme's
values on the element and inside it:

```css
.theme-editorial { --color-accent: #c65a34; --space-md: 1rem; }
.theme-vibrant   { --color-accent: #7144bf; --space-md: 0.75rem; }
```

```kiln
fn editorial_page() -> PageFinish {
    return PageFinish(root: ui({ theme: .editorial, bg: color.surface }), ...)
}
```

Every theme must set the tokens that any other theme sets, and every
token's value must suit its category (a `color.*` token is a colour, a
`space.*` token a length); breakpoints are the same in every theme, so a
theme cannot set them. These are E0222, reported at the `{% css %}` that
loaded the stylesheet. An unknown theme is E0221. Styles that every theme
could produce are all in the stylesheet, so changing `active_theme()`
and building again changes the site, layout included.

## Elements and components

- HTML elements are written as usual. Void elements (`<br>`, `<img>`,
  `<input>`, `<meta>`, `<link>`, ...) need no closing tag; any element may
  close itself with `/>`.
- A capitalized tag is a component: `<LikeButton post={post} />` renders
  `like_button.ember` with `post` as its prop. Its children fill its
  `{% slot %}`.
- Attributes:

  | Form | Meaning |
  | --- | --- |
  | `class="post {extra}"` | Text, with values inside |
  | `disabled={busy}` | A value |
  | `hidden`, `client:visible` | A flag |
  | `on:click={ count += 1 }` | An event handler: statements run in the browser |
  | `on:submit.prevent.latest={ ... }` | A handler with modifiers |
  | `key={item.id}` | In a loop: which item an element is, kept across updates |

## Embers

An Ember is a component in an **`.ember` file**. It shares the Glaze markup
grammar, but may declare `{% state %}` and `on:` handlers. Ordinary `{% let %}`
values remain immutable. The suffix declares the kind even without state or
events; template contents never promote a `.glaze` into an Ember.

`<LikeButton>` resolves `like_button.ember` or `like_button.glaze` in the same
folder. Having both is an ambiguity (E0315), regardless of directory order.
Hydration hints on a Glaze target are E0314. To migrate an interactive Glaze,
rename its file to `.ember` and update explicit filename references; component
tags keep their names. Layouts and static pages remain `.glaze`.

An Ember is not client-only by default. The server
renders it like any glaze, inside a `<kiln-island>` element that carries
its props and state as JSON, and the browser takes it over with the
program's WebAssembly (WasmGC): after each event its handlers run, it
renders again, and the page's DOM is updated to match, keeping focus,
typed text and keyed items. A page with no Ember loads no script, even when
unused `.ember` files exist in the project. Unreferenced components are excluded
from the browser build. Layout files must be `.glaze` (E0316).

```glaze
{# like_button.ember #}
{% props post: Post %}
{% state likes = post.likes %}
{% state saving = false %}

<button on:click.latest={
    saving = true
    likes = like_post(post.id) catch err { likes }
    saving = false
}>♥ {likes}{% if saving %}…{% end %}</button>
```

- **State.** `{% state name = value %}` declares a value the browser
  can change. The server computes the first value, so it may call a
  `server fn` or use server modules; the browser gets it as JSON. Props
  cannot be reassigned (E0207); state can. Props and state must have a
  JSON form: functions, styles and handles such as `web.Request` or
  `js.Value` are E0224.
- **Handlers.** `on:<event>={ statements }` runs in the browser when the
  element gets the event (`click`, `input`, `change`, `submit`, `keydown`
  and so on). `event` is an `ember.Event`: its `kind`, the element's
  `value` and `checked`, the `key` pressed, and a form's fields as
  `fields`, a `Map<String, String>`. `event.form<T>()` reads the fields as
  a struct the way `req.form<T>()` does on the server (numbers parsed, a
  ticked box `true`, an empty optional `none`), and fails with a message
  naming the field that does not fit. A form's `submit` never reloads the
  page.
- **Modifiers.** `.prevent` stops the browser's own action, `.stop`
  keeps the event from outer handlers, and `.latest` lets a new event
  replace a call that is still waiting. Others are E0313.
- **Server functions.** A handler statement that calls a `server fn` is a
  typed call to the server (`POST /_kiln/rpc/<name>`): the statements
  after it run when the reply comes. The call can fail (the network, or the
  function's own error), so it takes a `catch` (E0210). It must be a
  statement of its own, such as `x = f(...) catch err { ... }` (E0403).
  While a call is waiting, its element has `aria-busy` and
  `data-k-pending`, and the same event on it is ignored unless the handler
  is `.latest`, which cancels the older call. Calls time out after 30
  seconds and end when the page goes.
- **Server checks.** The routes accept only JSON from the page's own origin
  (`Origin` and `Sec-Fetch-Site`), up to 1 MiB, from a build of the same
  functions (the `x-kiln-rpc` header), and decode the arguments as
  untrusted input (400 when they do not fit). Each function's route is
  `/_kiln/rpc/<name>`, so `app.before("/_kiln/rpc", hook)` guards every
  call and `app.before("/_kiln/rpc/reset", hook)` one function: a response
  or an error from the hook is the answer, such as
  `throw web.error(403, "...")` (see [Before hooks](stdlib.md#web)).
- **Shared state** between embers is a [store](#shared-stores).
- **Hydration hints**, on the component's tag: `client:load` (the
  default), `client:idle` when the browser is idle, `client:visible` when
  the ember is on screen, and `client:only`, which the server does not
  render.
- **Lists.** `key={item.id}` on the elements of a `{% for %}` keeps each
  item's DOM, and the state of embers inside it, when items move.
- **Nesting and slots.** An ember inside an ember keeps its own state. A
  component's slot content stays the caller's: an ember can show and hide
  it, but does not render it again.
- **Browser code.** Handlers can use the [`browser`, `js`](stdlib.md#browser)
  and [npm](stdlib.md#npm-packages) modules; they need no import. Anywhere
  else these are E0223, since the server has no browser.
- **Without WebAssembly** (an old browser), the page stays as the server
  rendered it, with working links and forms, and `<html>` gets
  `data-k-embers="unsupported"` so CSS can hide controls that need it.
- An ember cannot have a layout (E0316): use it as a component in a page
  that has one. An ember that the browser adds (inside another ember's
  `{% if %}`, say) starts from its state's first values, so their code runs
  in the browser; one whose state calls a `server fn` can only be started
  by the server.

### Shared stores

A `store` is application state that embers share without passing it
through props or wrapping them in a provider. Declare it once in a `.kiln`
file, with its type, and read and change it from any ember, handler or
server code that sees the module:

```kiln
// shop/cart.kiln
pub struct Cart {
    lines: List<String> = []
    note: String = ""
}

pub store cart: Cart = Cart()
```

```glaze
{# cart_badge.ember #}
<span>{shop.cart.lines.len()} items</span>
```

```glaze
{# cart_button.ember #}
<button on:click={ shop.cart.lines.push("Book") }>Add</button>
```

- **One store, every ember.** Both embers above use the same cart: a
  click in one renders the other again. A store starts from its value the
  first time it is read.
- **Deep changes are seen.** Pushing to a list in the store, setting a
  field of a struct in it or replacing it whole (`shop.cart = Cart()`)
  all count. After each event (and each reply to a call), the bridge
  compares each store with what the islands last saw, and renders again
  only the islands that read a store that changed, once, in page order.
  An island's own `{% state %}`, what the user typed and where the focus
  is are kept.
- **Reactivity is explicit.** Only `store` declarations are shared and
  watched; `{% state %}` stays local to one island, and ordinary `let` and
  `var` are never reactive.
- **One per request on the server.** Each request starts with fresh
  stores, so a handler can set one for its visitor before rendering
  (`shop.cart = load_cart(req)`) without another visitor ever seeing it.
  A page with embers carries the stores its request started as JSON
  (`<script type="application/json" id="kiln-stores">`), and the browser
  starts from them. A store's value is sent to the browser, so it must
  never hold secrets.
- **One per page in the browser.** A full load of a page, or a reload,
  starts the stores again.
- **Client navigation keeps them.** Links inside an element with
  `client:navigate` load the next page into this one: its title, body and
  stylesheets replace the old ones, the old page's islands stop (their
  calls are cancelled) and the stores carry on. Other links, and pages that
  fail or are not HTML, load the ordinary way. Back and forward work.

  ```glaze
  <nav client:navigate><a href="/cart">Cart</a> <a href="/">Shop</a></nav>
  ```

- **Persistence is opt-in.** `store prefs: Prefs = Prefs() persist
  "prefs"` keeps the store in the browser's `localStorage` under
  `kiln:store:prefs`, and a reload starts from it. Only stores with a key
  are kept, and nothing is synced between browsers. When the store's type
  changes so that a kept value no longer decodes, it is dropped; change the
  key to drop old values on purpose.
- **Tests get their own.** `stores.scope(fn() { ... })` runs code with
  fresh stores and puts the old ones back after, so a test can give a
  store a value of its own (see [`stores`](stdlib.md#stores)).

## Habits from other template languages

These are errors with a suggested fix. Run `kiln explain <code>` for details.

| You wrote | Write instead | Code |
| --- | --- | --- |
| `{{ value }}` | `{value}` | E0304 |
| `{value \| safe}` | `{raw value}` | E0304 |
| `<?php ... ?>`, `<%= ... %>` | `{value}` and `{% %}` tags | E0304 |
| `{% endif %}`, `{% endfor %}` | `{% end %}` | E0303 |
| `{% elif %}`, `{% elseif %}` | `{% else if %}` | E0303 |
| `{% extends %}`, `{% include %}`, `{% block %}` | `{% layout %}`, a component tag, `{% slot %}` | E0303 |
| `className`, `htmlFor` | `class`, `for` | E0306 |
| `onClick={...}`, `@click={...}` | `on:click={...}` | E0306 |

## Folder modules

Same-folder components and layouts keep their owning module's context.
`<posts.Editor />` explicitly names `posts/editor.ember` in a reachable module;
`web.render("posts/index.glaze", {})` uses a project-root-qualified path.
Module-local CSS, scoped URLs and static assets retain that context. See
[projects.md](projects.md) for import reachability, path rules and examples.
