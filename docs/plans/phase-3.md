# Phase 3 — Glazes, native styling, Finishes and themes

**Status:** In progress. Step 1 (compiled Glaze rendering) is done; see
[Rendering](../reference/glazes.md#rendering) for what works today. The
[Rust runtime migration](runtime-rust.md) it waited for is complete.

**Goal:** Build Kiln's multi-page documentation website **in Kiln**, using
compiled server-rendered Glazes, scoped and imported CSS, typed `ui` style
records, design tokens and reusable **Finishes**. The site must be
restylable—including layout changes—by selecting another compatible theme
without rewriting its content or Glazes.

This is the **agreed Phase 3 scope and acceptance plan**, not a statement
that these facilities already work. New code examples are **target syntax**.
Existing [Glaze syntax](../reference/glazes.md), [type rules](../reference/types.md) and
[design decisions](../design.md) remain authoritative for implemented features.
See the [roadmap](../../README.md#roadmap) for overall phase sequencing and
[vision](../vision.md) for the wider aims.

## 1. Design decisions

1. **Server rendering first.** Kiln owns routes explicitly in source files.
   Continue using `web.render("page.glaze", { props })`; no automatic
   file-based routing, interpreter or required client-side framework.
2. **Preserve the existing Glaze language.** Render the already-checked
   props, layouts, slots, components, loops, branches, matches and
   interpolation. Do not replace the template grammar with a new one.
3. **Styling is co-located when useful.** A Glaze may contain markup,
   logic and scoped `<style>`, or import ordinary CSS/CSS Modules.
   Both are first-class; developers never have to choose one approach
   globally.
4. **Native `ui` follows structured approach B.** Styles are typed
   Kiln record expressions using short, Tailwind-inspired keys:
   `bg`, `fg`, `p`, `px`, `gap`, `rounded`, etc. No space-separated
   class string is necessary, and no additional mini-language.
5. **Logic may select styling seamlessly.** Use ordinary expressions,
   `if` and `match` to produce `Ui` or choose Finish variants.
6. **Finishes are simply functions.** "Finish" is the name for an ordinary
   Kiln function returning `Ui` or a record of `Ui` values. No
   `finish` keyword, dedicated declaration kind or hidden evaluator.
7. **Themes are typed contracts.** A theme groups token values and
   compatible Finishes; a theme can change layout as well as colour.
   Phase 3 requires **build-time theme replacement**, not reactive
   browser-side theme switching.
8. **CSS is the standard and the output.** Generate conventional CSS
   assets; preserve cascade, selectors and ordinary HTML `class`.
   Native styling should not require Tailwind, Node or another runtime.
9. **Compiler and LSP own the integration.** Type-check properties,
   tokens, CSS Module class exports, Finish arguments/parts and theme
   compatibility. Reuse maintained Rust CSS tooling for CSS parsing and
   transformations.
10. **Keep future phases separate.** Embers, reactive hydration and
    typed RPC are Phase 4; full ORM/auth/forms are Phase 5; comprehensive
    refactoring and hot reload are later milestones.

### Terminology

| Concept | Meaning |
| --- | --- |
| **Glaze** | A template/component in a `.glaze` file |
| **`Ui`** | Built-in type representing a validated style value |
| **`ui({ ... })`** | Typed style constructor used in Kiln functions |
| **`ui={{ ... }}`** | Contextual Glaze attribute shorthand for a style record |
| **Finish** | Ordinary function returning a `Ui` or typed multi-part `Ui` record |
| **Finish variant** | Normal function parameter, often an enum |
| **Theme** | Compatible group of Finishes and design-token values |
| **Token** | Typed CSS-custom-property reference, e.g. `color.surface` |

## 2. End-to-end developer experience

### A page with markup, logic and styling

```glaze
{# post_card.glaze — target Phase 3 syntax #}
{% props post: Post, featured: Bool = false %}
{% let s = active_theme().post_card(featured) %}

<article ui={ui.merge(s.root, ui({ p: space.lg }))}>
    <h2 ui={s.title}>{post.title}</h2>
    <p class="summary" ui={{ fg: color.muted }}>
        {post.summary}
    </p>
    <a href={post.url} ui={s.action}>Read more</a>
</article>

<style>
    .summary {
        text-wrap: pretty;
    }
</style>
```

This should be easy to understand in one editor view. Styling can be
extracted into a Finish without changing the Glaze's structure.

```kiln
enum CardTone { Normal, Featured }

struct PostCardFinish {
    root: Ui
    title: Ui
    action: Ui
}

fn editorial_post_card(featured: Bool) -> PostCardFinish {
    {
        root: ui({
            grid: true,
            cols: 1,
            gap: space.md,
            bg: color.surface,
            rounded: radius.none,
            at: { desktop: { cols: 2 } }
        }),
        title: ui({
            fg: color.heading,
            weight: .bold,
            text_size: if featured { .xl } else { .lg }
        }),
        action: ui({ fg: color.accent, underline: true })
    }
}
```

**Syntax contract:** In a Glaze, `ui={{ ... }}` means an attribute
expression containing an ordinary record literal, contextually converted
to `Ui`. In a Kiln function, use `ui({ ... })`. Existing style values
pass as `ui={s.root}`. These are two entry points to the *same* type
and CSS generator, not parallel styling systems.

## 3. Glaze compilation and render semantics

### Rendering pipeline

- Extend the existing AST/type-checker and lower Glazes into typed render
  functions in Kiln IR. Compile with the native Cranelift backend and
  retain parity with the C backend as supported.
- Render HTML with proper output escaping. Handle static text,
  interpolated expressions, typed props and defaults, HTML attributes,
  void elements, locals, conditional blocks, loops/filters/empty
  branches and exhaustive matches.
- Compile component calls with typed props, and layout nesting with
  named/default slots. Reuse current component resolution rules from
  `docs/reference/glazes.md` instead of inventing a new layout language.
- Implement `web.render` to execute the compiled template and return an
  HTML response with `text/html; charset=utf-8`. Route failures through
  existing web error handling; a JSON or text route should still work
  without any Glaze.
- Keep routes explicit, e.g. `app.get("/guide", fn(req) {
  web.render("guide.glaze", { ... }) })`. One folder remains a program;
  general directory modules belong to the later Wares/module work.
- Make generated output inspectable and deterministic, with compiler
  diagnostics tied to the original Glaze source spans.

### Security and correctness

- Escape interpolation according to **HTML text vs quoted attribute**
  context. Validate dynamic URL-bearing attributes to prevent
  `javascript:` and other dangerous protocol injection.
- `{raw ...}` stays an explicit trust boundary, with tests and clear docs
  about untrusted HTML.
- Never concatenate untrusted inputs into CSS selectors, generated class
  identifiers, stylesheet text or raw style attributes.
- Detect unknown component/layout references, invalid props, malformed
  nesting and recursive template cycles with useful errors rather than
  crashing the compiler.
- Handle special characters, Unicode, empty branches and render failures
  consistently on Linux/Windows. Do not emit partial responses on
  recoverable template errors where headers have not been sent.
- Avoid uncontrolled recursion/output growth through reasonable
  runtime/compiler safeguards and targeted tests.

## 4. Typed `Ui` styling

### Initial shorthand surface

| Area | Keys | Intended semantics |
| --- | --- | --- |
| Colour | `bg`, `fg`, `border_color` | Background, foreground/text and border colour |
| Spacing | `p`, `px`, `py`, `pt`, `pr`, `pb`, `pl`, `m`, `mx`, `my`, `gap` | Typed spacing and margins |
| Sizing | `w`, `h`, `min_w`, `max_w`, `min_h`, `max_h` | Typed dimensions |
| Surface | `rounded`, `border`, `shadow` | Radius, border and elevation |
| Layout | `flex`, `grid`, `direction`, `cols`, `rows`, `items`, `justify`, `wrap` | Common flex/grid patterns |
| Typography | `font`, `text_size`, `weight`, `line_height`, `align`, `tracking` | Type and readability |
| Effects | `opacity`, `overflow`, `transition`, `cursor`, `transform`, `underline` | Common visual interaction |
| States | `hover`, `focus`, `focus_visible`, `disabled` | Nested typed style records |
| Responsive | `at: { tablet: { ... } }` | Named breakpoints from token declarations |

Names use Tailwind-inspired brevity for common operations, but prefer
unambiguous words such as `text_size` over introducing dozens of obscure
one-letter aliases. `fg` always means text colour; `bg` means the
background paint. Rare CSS properties belong in ordinary CSS, not in an
ever-growing duplicated CSS vocabulary.

```glaze
<article ui={{
    grid: true,
    cols: 1,
    gap: space.md,
    p: space.lg,
    bg: color.surface,
    fg: color.text,
    rounded: radius.card,
    hover: { shadow: shadow.lg },
    at: { tablet: { cols: 2 } },
    border_color: if featured {
        color.accent
    } else {
        color.transparent
    }
}}>
    {post.title}
</article>
```

### Type rules

- `Ui` is a compiler-recognized style type. Ordinary records stay
  ordinary records, except where a `Ui` is expected or `ui({ ... })`
  explicitly constructs one.
- `bg` accepts appropriate paint/colour values, `p` expects spacing,
  `rounded` expects radius/length, `cols` expects a suitable grid
  column count, etc. Wrong types are compiler errors with useful
  diagnostics and suggested alternatives.
- Static variants and scoped responsive/pseudo-state declarations are
  represented in a typed style IR, not flattened into strings early.
- Mutually incompatible keys (e.g. `flex: true` and `grid: true` in
  one effective state) get a diagnostic. Unknown breakpoints/tokens
  also get diagnostics.
- Use existing Kiln expressions, enums, conditionals and matches for
  style selection, without a separate conditional styling syntax.
- Keep CSS units explicit and typed; don't quietly accept arbitrary
  unvalidated interpolated strings as styles.

### Generation and state

- Compile statically enumerable styles into deterministic CSS classes.
  Deduplicate identical rules and ship them as standard CSS assets.
- Request-time conditions choose among **finite precompiled variants**,
  switching class IDs rather than generating a stylesheet per request.
- `hover`, `focus_visible` etc. become standard CSS selectors;
  responsive `at` blocks become media queries. No JavaScript required.
- Truly arbitrary dynamically computed CSS values may be deferred or
  supported only through safe typed CSS custom-property bindings;
  arbitrary runtime CSS generation is **not required** in Phase 3.
- Generated styles must preserve inherited CSS custom properties so
  theme values can affect components across Glaze boundaries.

### Composition: `ui.merge`

```kiln
ui.merge(
    ui({ p: space.lg, bg: color.surface }),
    ui({ px: space.sm }),
    if selected {
        ui({ border_color: color.accent })
    } else {
        ui({})
    }
)
```

- Merge **left to right**; later arguments override earlier values for
  the same normalized CSS property and state/breakpoint.
- Normalize `p` into individual edges so `px` overrides only left/right,
  not top/bottom. Do the same for other overlapping aliases.
- Nested states and breakpoint values merge property-by-property.
- Merge semantics apply to `Ui` values, not to arbitrary imported CSS:
  ordinary stylesheet cascade rules still determine external overrides.
- Generated CSS order, identifiers and output remain reproducible.

## 5. Standards-based CSS and token system

### Scoped local CSS

A Glaze can include ordinary `<style> ... </style>`. CSS is parsed as
CSS, **not** Kiln code; Glaze interpolation does not activate inside
it. By default, styles are scoped to the owning Glaze by generated
selectors/attributes, **without Shadow DOM**.

- Inheritance and custom-property cascade must continue across
  component boundaries.
- Support pseudo-classes/elements, nested selectors, keyframes,
  media/container queries and modern CSS syntax through a maintained
  CSS parser/transformer.
- Provide a documented `:global(...)` escape for selectors that must
  intentionally cross component boundaries.
- Do not double-apply styles, leak selectors across same-named classes,
  rewrite CSS custom-property names incorrectly or change global reset
  semantics unintentionally.

### Plain CSS and CSS Modules

Use explicit, narrowly scoped Glaze import directives:

```glaze
{% css "./site.css" %}
{% css module styles from "./card.module.css" %}

<article class={styles.card} ui={{ p: space.md }}>
    <h2 class={styles.title}>Hello</h2>
</article>
```

- Plain `.css` import stays **global** and respects the CSS cascade.
  Standard `@import` in CSS must also be handled or passed through
  safely with resolvable asset paths.
- `.module.css` import generates hashed class identifiers with
  *typed* keys. `styles.card` works; `styles.unknown` fails in
  `kiln check` and should offer close suggestions.
- CSS files belong to the dependency graph, with watched changes,
  source spans, path completion, go-to-definition and helpful
  diagnostics. Guard against reading files outside allowed roots.
- Preserve plain HTML `class`, `class={...}` and `style` attributes,
  so CSS frameworks and existing markup remain interoperable.

### Design tokens

The **source of values is ordinary CSS custom properties**; Kiln
indexes them into typed token namespaces using documented categories
rather than requiring duplicate declarations in a bespoke token DSL.

```css
/* design.css: baseline theme tokens */
:root {
    --color-surface: #faf9f6;
    --color-text: #282828;
    --color-accent: #7144bf;
    --color-muted: #696969;
    --space-md: 1rem;
    --space-lg: 1.5rem;
    --radius-card: 0.75rem;
}
```

```kiln
ui({
    bg: color.surface,     // var(--color-surface)
    fg: color.text,        // var(--color-text)
    p: space.lg,           // var(--space-lg)
    rounded: radius.card  // var(--radius-card)
})
```

- Index and type token categories such as colours, spacing, radii,
  typography, shadows, motion and breakpoints. Prefer a small clear
  naming scheme (`--color-*`, `--space-*` etc.) over a new styling
  language. Type-check token references and CSS values where possible.
- Tokens resolve to CSS variables, not compile-time fixed values:
  changing the active CSS theme value should propagate everywhere.
- Provide an LSP-visible token catalog; hover can show its definition,
  expected kind and currently selected default value.
- Different compatible themes must supply the required tokens with the
  same categories. Missing or mistyped token definitions are errors.
- Tokens alter appearances and some layouts, but cannot change DOM
  structure. Finishes handle more substantial style/layout changes.

### Cascade and conflicts

Define deterministic CSS output layers and order for Kiln-controlled
styles, conceptually **theme/global → component CSS → scoped local CSS
→ Finish/inline `ui`**. Specify actual layer names and specificity in
tests; standard CSS cascade always applies (including unlayered rules
and `!important`). Do not promise that `ui` overrides arbitrary
external CSS regardless of browser semantics.

## 6. Finishes and themes

### A Finish is ordinary Kiln

No special declaration, annotation, interpreter or separate recipe
configuration. A Finish can return one style or a record of styles:

```kiln
enum ButtonTone { Primary, Danger }

fn button(tone: ButtonTone) -> Ui {
    let base = ui({
        p: space.md,
        rounded: radius.button,
        weight: .medium
    })
    let variant = match tone {
        .Primary => ui({ bg: color.primary, fg: color.onPrimary })
        .Danger => ui({ bg: color.danger, fg: color.onDanger })
    }
    ui.merge(base, variant)
}

struct CardFinish {
    root: Ui
    title: Ui
    summary: Ui
    action: Ui
}

fn card(featured: Bool) -> CardFinish {
    {
        root: ui({ flex: true, direction: .column, gap: space.md }),
        title: ui({ fg: color.heading, weight: .bold }),
        summary: ui({ fg: color.muted }),
        action: ui({ fg: color.accent })
    }
}
```

The multi-part names `root`, `title`, `summary` and `action` are just
**ordinary typed record fields**. They are not Glaze content slots,
and they cannot create markup. A Glaze owns its structure and may
apply several fields from the same Finish.

### Typed theme contracts

The smallest composable theme contract is a Kiln type describing the
Finish functions and their parameter/return types:

```kiln
struct SiteTheme {
    card: fn(Bool) -> CardFinish
    button: fn(ButtonTone) -> Ui
}

fn active_theme() -> SiteTheme {
    editorial_theme()
}
```

The concrete functions `editorial_theme()` and `vibrant_theme()` each
produce a compatible `SiteTheme` value using regular Kiln functions.
No extra `theme` keyword or compiler-only Finish subclass is needed.

- Theme contract checks include function signature compatibility,
  enum parameters, return style types, multi-part record fields and
  required token categories.
- For Phase 3, allow implementations within the existing same-folder
  Kiln program model; do not require the later subdirectory module
  system or a theme package registry.
- **Theme switch acceptance:** change one `active_theme()` selection
  and rebuild. Both themes must render the **same Glazes and content**
  with appreciably different visual identities, including a layout
  change such as grid versus flex.
- Local overrides can merge over themed Finishes explicitly. Theme
  switching must not require changing the content model or component
  props.
- Static analysis gathers the finite styles reachable from candidate
  theme functions. Including both themes' CSS in the initial bundle
  is acceptable; tree-shaking unused themes and live browser toggling
  are later optimizations.
- Don't execute arbitrary Kiln application code at build time to
  discover styles: compile literal style branches and enumerate
  statically representable variants; report limitations explicitly.

## 7. Build pipeline, linking and assets

1. Parse/type-check Kiln files and Glazes together, including
   component/layout and Finish references.
2. Lower Glaze rendering to IR and compile for both native targets,
   keeping the reference C backend tested where supported.
3. Create style IR from contextual `Ui` values, normalize aliases,
   collect token dependencies, extract finite styles and emit
   deterministic CSS with hashed identifiers.
4. Process imported/local styles using maintained CSS tooling;
   scope local rules, resolve modules and produce CSS assets.
5. Bundle HTML render code and required CSS/fonts/images into the
   supported standalone executable, preserving one-command
   `kiln run`/`kiln build` semantics.
6. Serve resources with correct MIME types, stable URLs/cache
   behaviour and no source/config/secrets exposure. Guard against
   path traversal and accidental inclusion of private files.

**Do not add a Node/Tailwind runtime or require a separate CSS builder
on users' machines.** Build-from-source may still use Rust tools;
downloaded Kiln and generated executables remain self-contained
on their supported targets.

A single CSS style appearing in multiple components should not
generate duplicate runtime copies; generated output must be
reproducible given identical inputs.

## 8. Editor and diagnostics

Extend the existing **Rust Kiln LSP** and the VS Code extension,
not a second independent checker.

- Highlight mixed Glaze HTML, Kiln expressions, `ui` keys/values and
  embedded CSS correctly.
- Complete `Ui` keys based on expected style type, and tokens by
  category; provide errors for unknown/incompatible properties,
  invalid breakpoints and conflicting aliases.
- Complete, hover and navigate Finish functions, parameters,
  multi-part fields and token declarations.
- Use mature CSS language tooling where possible for embedded CSS
  syntax, property completion, diagnostics and import navigation.
- Resolve CSS Modules, give typed class completion and diagnostic
  for missing classes, and navigate to declarations.
- Handle multiple Glazes, live/unsaved buffers, UTF-16 LSP offsets
  and recovering from malformed work-in-progress code.
- Full generated-CSS hover previews, refactoring and cross-file
  rename remain later enhancements, not completion blockers.

## 9. Implementation steps and review gates

Make these separate, reviewable PR-sized steps, with tests in every
step and no expansion into Embers or the general module system.

**Step 1 — Compiled Glaze rendering**

- AST → typed IR → native HTML render functions.
- `web.render`, props, interpolation, attributes, escaping and HTML
  content type.
- **Gate:** a minimal page renders from a compiled native server,
  on Linux and Windows, with no template interpreter.

**Step 2 — Composable components and layouts**

- Typed components, props/defaults, layouts, nested/default/named
  slots, locals, loops/empty branches, conditions and matches.
- Preserve current checker rules and clean diagnostics.
- **Gate:** multi-page SSR app with shared layout and reusable
  components; HTML/XSS regression tests pass.

**Step 3 — Standards CSS pipeline**

- Rust CSS parser/transformer, scoped local `<style>`,
  global CSS directives, CSS Modules, assets and cascade tests.
- **Gate:** duplicate local class names do not collide; CSS Module
  exports are typed; CSS/fonts/images load from standalone binary.

**Step 4 — Typed `Ui` and tokens**

- Style value/type and contextual record conversion; shorthand
  properties, states, responsive `at`, static/finite CSS extraction,
  design token indexing and deterministic `ui.merge`.
- **Gate:** nontrivial responsive component and conditional styling
  work without Tailwind; wrong style/token types fail at compile time.

**Step 5 — Finishes and theme contracts**

- Finish functions, typed multi-part records, enum variants,
  theme-shape function fields, compile-time theme choice and
  compatible token value sets.
- **Gate:** switching one theme-selection function changes
  typography/colours/spacing **and layout** without editing any
  application Glaze or content.

**Step 6 — Editor, packaging and examples**

- LSP token/CSS/Finish intelligence, VSIX smoke tests,
  CSS output integrity and cross-target build tests.
- Document style alias names/types, escaping, cascading, scoping,
  token naming, CSS imports and theme contracts.
- **Gate:** editing Glazes with live diagnostics and style
  completion in VS Code feels like editing an established language.

**Step 7 — Kiln docs site (dogfood acceptance)**

- Build Kiln's **actual documentation website** with Kiln routing
  and Glazes: landing, installation, getting started, language
  reference and navigation/shared layouts. Content can initially
  reuse authored docs through a controlled content-loading path.
- Use native `ui`, local CSS, an external CSS file, CSS Modules,
  design tokens, Finishes and two compatible themes.
- Run e2e tests for rendered HTML, static assets, responsiveness,
  style output, accessibility basics and security.
- **Gate:** all checks below are met. Only then mark Phase 3 done
  in the README.

## 10. Definition of done

- [ ] The compiler **renders**, not merely checks, existing
      Glaze templates through `web.render`, including typed props,
      components, layouts, slots, loops, conditionals and matches.
- [ ] HTML and attributes are safely escaped, raw HTML is explicit,
      security/injection and error-path tests pass.
- [ ] `Ui`, `ui({ ... })`, `ui={{ ... }}`, conditional variants,
      `ui.merge` and native aliases generate valid deterministic CSS.
- [ ] Design tokens are indexed from CSS into **typed** Kiln names;
      invalid/missing tokens or style property types give useful
      compiler diagnostics.
- [ ] Embedded `<style>` has predictable scoping; global CSS and
      CSS Modules import and bundle correctly, with type-checked
      module class names and CSS-standard cascade behaviour.
- [ ] Finishes are regular functions with typed variants and
      optional multi-part records; theme contracts reject
      missing/incompatible Finish functions and parts.
- [ ] Two compatible themes substantially reskin the same
      Glazes (including layout), selected in one place and rebuilt.
- [ ] LSP/VS Code support inline CSS, token names, Finish calls,
      imports and styling errors in real multi-file workspaces.
- [ ] Kiln's genuine **multi-page docs website runs in a self-contained
      native executable** on supported Linux and Windows targets,
      serving CSS, images and fonts without Node/Tailwind/extra
      build dependencies for end users.
- [ ] Existing CLI, HTTP API and both compilation backends remain
      covered by CI as applicable; cross-target HTML/CSS smoke and
      conformance tests pass.
- [ ] A developer can start from a documented Glaze+Finish example,
      style inline, extract a Finish, use CSS imports and switch
      themes without learning a second style language.

## 11. Explicitly deferred

- **Phase 4:** Embers/WasmGC, reactive UI and style updates, selective
  hydration, generated server RPC and JS/browser interop; see the
  [Phase 4 plan](phase-4.md).
- **Phase 5:** SQL/ORM, database migrations, auth/sessions, generated
  admin UI and rich typed forms, integrating with existing Embers/RPC;
  see the [Phase 5 plan](phase-5.md). Basic HTML forms are fine in Phase 3.
- **Phase 6:** Wares, hot reload/formatter, advanced editor refactoring
  and the integrated [`kiln test` runner](testing.md), covering **unit,
  integration and automated browser/Glaze/Ember tests**. Phase 3 still
  needs compiler/HTML/CSS smoke and regression tests but does not depend
  on the finished public test command.
- **Phase 6.6:** Static site export, including `kiln fire --static`.
- **Phase 7:** Release-build optimization, major runtime/compiler hardening
  and stability guarantees for 1.0.
- **Phase 8:** Deployment/release automation on existing infrastructure.
- **Later:** automatic runtime theme switching, user theme
  preferences, theme registry/distribution, style previews,
  arbitrary dynamic computed CSS and new CSS-replacement DSLs.

Do **not** use Phase 3 to reimplement every CSS feature in native
`Ui`, introduce a mandatory styling framework, rewrite routing,
or invent a dedicated language for Finishes.

## 12. Final review principles

- A Glaze should make its **structure, logic and common styling**
  understandable together.
- A Finish should feel like **ordinary Kiln code**, not configuration.
- A theme should change a site's **visual identity** without
  owning its content or business logic.
- CSS should remain **native, familiar, accessible and inspectable**.
- Styles should be **type-checked, secure and deterministic**.
- New abstractions are justified only when they actually eliminate
  glue; avoid novelty for its own sake.

**In one sentence:** **Glazes define structure and behaviour, typed
`Ui` records express local appearance, Finishes compose reusable
presentation through ordinary functions, and themes change the
visual identity—while CSS remains the underlying standard.**
