# Glaze syntax reference

Glazes (`.glaze` files) are templates and components: HTML with Kiln code in
it. This is the syntax the phase 0 parser accepts; every form here is covered
by a test in `crates/kiln_syntax/tests/glaze.rs`. Code inside braces is
ordinary Kiln, as in [syntax.md](syntax.md).

`kiln check` type-checks glazes together with the `.kiln` files in the same
directory: see [types.md](types.md#glazes) for the rules.

**Phase 3 plan:** [compiled Glazes, native typed `ui`, Finishes, local CSS,
CSS Modules, design tokens and themes](../plans/phase-3.md). That plan contains
*target syntax* and implementation steps not yet part of this syntax
reference. This page documents the currently recognized forms, and
[Rendering](#rendering) what the compiler makes of them.

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
with `content-type: text/html; charset=utf-8`. Glazes are compiled into the
program like any other code, with nothing interpreted at run time; the
glaze must be named in quotes, so the compiler knows which one it is.

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
- Embers are not compiled yet (E0401): building a glaze with
  `{% state %}`, an `on:` handler or a `client:` hint needs phase 5.
  See the [Phase 5 Embers plan](../plans/phase-5.md).

## Tags

| Tag | Meaning |
| --- | --- |
| `{% props name: Type, other: Type = default %}` | The glaze's inputs. Once, at the top level |
| `{% layout "base.glaze" title: value %}` | Wrap this glaze in a layout. Once, at the top level |
| `{% slot %}`, `{% slot name %}` | In a layout or component: where the caller's content goes |
| `{% fill name %} ... {% end %}` | Content for `{% slot name %}`: directly inside a component tag, or at the top level of a page with a layout |
| `{% css "site.css" %}` | Use a global stylesheet. At the top level |
| `{% css module name from "card.module.css" %}` | Use a CSS Module: `name.card` is its renamed class. At the top level |
| `{% state name = value %}` | Browser-side state; makes the glaze an ember. At the top level |
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

Paths are relative to the program's folder, and must stay inside it. CSS
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
    borderColor: if featured { color.accent } else { .transparent },
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
| `bg`, `fg` (text colour), `borderColor` | `color.*` tokens, `.transparent`, `.current` |
| `p`, `px`, `py`, `pt`, `pr`, `pb`, `pl`, `gap` | `space.*` tokens, `0` |
| `m`, `mx`, `my`, `mt`, `mr`, `mb`, `ml` | `space.*` tokens, `0`, `.auto` |
| `w`, `h`, `minW`, `maxW`, `minH`, `maxH` | `size.*` or `space.*` tokens, `0`, `.auto`, `.full`, `.min`, `.max`, `.fit` |
| `rounded` | `radius.*` tokens, `.none`, `.full` |
| `border` | `true` (1px solid), or a width |
| `shadow` | `shadow.*` tokens, `.none` |
| `flex`, `grid` | `true` (not both in one style) |
| `direction` | `.row`, `.column`, `.rowReverse`, `.columnReverse` |
| `cols`, `rows` | `1` to `12` equal tracks, `.none` |
| `items` | `.start`, `.center`, `.end`, `.stretch`, `.baseline` |
| `justify` | `.start`, `.center`, `.end`, `.between`, `.around`, `.evenly`, `.stretch` |
| `wrap`, `underline` | `true` or `false` |
| `font` | `font.*` tokens |
| `textSize` | `text.*` tokens; `.lg` is `text.lg` |
| `weight` | `.thin`, `.light`, `.normal`, `.medium`, `.semibold`, `.bold`, `.black`, `100`-`1000`, `weight.*` |
| `lineHeight` | a number, `leading.*` tokens |
| `align` | `.left`, `.center`, `.right`, `.justify`, `.start`, `.end` |
| `tracking` | `tracking.*` tokens |
| `opacity` | `0` to `1` |
| `overflow` | `.hidden`, `.auto`, `.scroll`, `.visible`, `.clip` |
| `transition` | `motion.*` tokens, `.none` |
| `cursor` | `.pointer`, `.default`, `.text`, `.notAllowed`, `.grab`, `.move`, `.wait`, `.help`, `.none` |
| `transform` | a CSS string, `.none` |
| `hover`, `focus`, `focusVisible`, `disabled` | a record of the keys above |
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
  `like_button.glaze` with `post` as its prop. Its children fill its
  `{% slot %}`.
- Attributes:

  | Form | Meaning |
  | --- | --- |
  | `class="post {extra}"` | Text, with values inside |
  | `disabled={busy}` | A value |
  | `hidden`, `client:visible` | A flag |
  | `on:click={ count += 1 }` | An event handler: statements run in the browser |

## Embers

A glaze with `{% state %}` or an `on:` handler is an ember: it renders on the
server, then hydrates in the browser as WebAssembly. Hints such as
`client:visible` on the component tag choose when it hydrates.

```glaze
{% props post: Post %}
{% state likes = post.likes %}

<button on:click={
    likes = like_post(post.id) catch { likes }
}>♥ {likes}</button>
```

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
