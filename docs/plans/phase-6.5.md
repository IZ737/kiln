# Phase 6.5 — Language review and vision alignment

Phase 6.5 reviewed Kiln as a language against the [vision](../vision.md),
using the programs in this repository (the examples, the docs site, the
registry, kiln/ui and the agent benchmark) as evidence. It was a review
of language design, not of the compiler's internals or performance. No
Kiln programs existed outside the repository, so breaking changes needed
no deprecation period.

**Status: complete; released in v0.12.0.** Every decision is recorded,
with its reason and what it means for existing code, in the
[decision log](../decisions.md); the rules as they stand are in the
[reference](../README.md#language-reference), with the evaluation and
sharing rules in [semantics.md](../reference/semantics.md).

## How the review worked

Each proposal named its evidence (code in the repository that was
awkward, inconsistent or unsafe), a change, and its compatibility cost,
and was marked keep, change, remove, add or defer. All recommended
proposals were accepted, then implemented with tests and documentation
in this order: language, web and standard library, binary data, `ui`
names, and interfaces last.

## Language

| Id | Decision |
| --- | --- |
| L1 | `let some(x) = value else { ... }`, whose `else` must leave (E0111) |
| L2 | `.Variant` where the enum is known, in values and `match` arms |
| L3 | Values are shared references, written down in `semantics.md`; a `const` cannot be changed (E0207) |
| L4 | Copies with changes: `Post(..post, title: t)` and `{ ..record, x: 0 }` |
| L5 | Map literals: `["Ada": 36]` |
| L6 | `"""` multi-line strings, with the closing line's indentation removed |
| L7 | Interfaces as bounds on generic functions: `fn label<T: Named>(x: T)`, structural, compiled per type, never a value's type (E0234, E0235, E0112) |
| L8 | `Any` and `Rest` are reserved for the standard library (E0233) |
| L9 | A real `Bytes` type; `Decimal` removed until a decimal type is designed |

## Web and standard library

| Id | Decision |
| --- | --- |
| W1 | `env(name) -> String?`; `os.env` removed |
| W2 | `status:` and `headers:` on every response function, `web.render` included |
| W3 | `res.set_cookie(...)` with safe defaults; `req.cookie` decodes |
| W4 | Binary bodies: `web.bytes(...)`, `req.bytes()`, form file fields as `Bytes` |
| W5 | Cross-site `POST`/`PUT`/`PATCH`/`DELETE` from browsers get 403 unless `web.app(cross_site: [...])` allows the path |
| W6 | A glaze's `request: web.Request` prop is filled in automatically |
| W7 | `app.before(prefix, hook)` replaces `fn rpc_guard` |
| W8 | The standard library's internals cannot be used by programs (E0230) |

## Glazes and embers

| Id | Decision |
| --- | --- |
| G1 | `ui` keys and words are snake_case (`max_w`, `.row_reverse`) |
| G2 | `event.form<T>()` reads an ember's form as a struct; the map is `event.fields` |
| G3 | A glaze never serves a route by itself; routes stay in code |

## Kept, deferred and documented

| Id | Decision |
| --- | --- |
| M1 | `js.Handle<T>` for typed JavaScript objects: after 1.0 |
| M2 | Private fields: before Wares are widely published |
| M3 | Models and guides write `id: Int = 0` |
| S1 | `semantics.md` and the decision log added |
| S2 | Stale documentation fixed |
| D1 | Forms typed once from one struct, for both the page and the handler: Phase 7 |

## Left for later

- `Bytes` in embers (the browser target reports E0401), and a form file's
  name and content type.
- Interface values and dynamic dispatch; generic interfaces.
- A decimal type for money.
