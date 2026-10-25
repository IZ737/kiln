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
reference. This page documents the currently recognized forms.

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

## Tags

| Tag | Meaning |
| --- | --- |
| `{% props name: Type, other: Type = default %}` | The glaze's inputs. Once, at the top level |
| `{% layout "base.glaze" title: value %}` | Wrap this glaze in a layout. Once, at the top level |
| `{% slot %}`, `{% slot name %}` | In a layout or component: where the caller's content goes |
| `{% state name = value %}` | Browser-side state; makes the glaze an ember. At the top level |
| `{% let name = value %}` | A local value |
| `{% if c %} ... {% else if d %} ... {% else %} ... {% end %}` | Conditional |
| `{% for x in xs where c %} ... {% empty %} ... {% end %}` | Loop; `where` and `empty` are optional |
| `{% match v %} {% case pattern if guard %} ... {% end %}` | Pattern match, with the patterns of `match` |

Every block ends with `{% end %}`.

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
