# Language decisions

Kiln's design decisions, newest first, with why each was made and what it
means for existing code. A decision here is settled until new evidence
reopens it; the [design overview](design.md) gives the goals and the
[reference](README.md#language-reference) the rules as they stand.

Phase 6.5 reviewed the whole language against the [vision](vision.md);
[its record](plans/phase-6.5.md) lists every proposal by id.

| Date | Decision | Why | Compatibility |
| --- | --- | --- | --- |
| 2026-10-27 | **Keep `{% state %}` as the canonical Ember-local reactive declaration; do not replace it with `{% var %}`.** Shared stores will have explicit reactive roots, not automatically reactive `var` bindings ([Phase 6.5.1](plans/phase-6.5.1.md)). | Someone scanning a file should immediately recognize what is component state and what is merely an ordinary mutable variable. | No break: preserves the implemented syntax; the earlier `{% var %}` idea is rejected. |
| 2026-10-27 | Deferred: forms typed once from one struct for both the page and its handler (Phase 7); `js.Handle<T>` for typed JavaScript objects (after 1.0); private fields (before Wares are widely published) | Each is worth doing, but none blocks 1.0 programs, and each needs design beyond this review | None |
| 2026-10-27 | Models and the guides write `id: Int = 0`, so a new record is made without inventing a key | Examples passed `id: 0` by hand to every insert | None: a style change |
| 2026-10-27 | Structural interfaces, used only as bounds on generic functions (`fn label<T: Named>(x: T)`), compiled per type with no dynamic dispatch; a type fits by having the methods | Generic code could only show, compare and hash, so Wares could not offer reusable code over the program's types; dynamic dispatch would need a runtime representation 1.0 does not need | Additive; `interface` is a keyword only at the start of a declaration |
| 2026-10-27 | `Bytes` is real: `text.bytes()`, `data.text()` (which fails on bad UTF-8), `len`, `slice`, `fs.read_bytes`/`write_bytes`, `web.bytes`, `req.bytes()`, file fields in forms, Base64 in JSON and blobs in SQL | A server could not send an image or read an upload, and binary data hid in `String`s that are supposed to be text | `base64.encode` takes `Bytes` and `base64.decode` returns them; a form's `String` field rejects non-UTF-8 data |
| 2026-10-27 | `Decimal` is removed; money is whole cents in an `Int` until a decimal type is designed | It was a name with no literals, methods or runtime, promising something 1.0 would not have | None: nothing could use it |
| 2026-10-27 | `ui` keys and words are snake_case (`max_w`, `text_size`, `focus_visible`, `.row_reverse`) | Every other Kiln name is snake_case; camelCase keys were the one exception, a CSS habit | A camelCase key or word is an error whose fix is the snake_case name |
| 2026-10-27 | The server refuses a `POST`, `PUT`, `PATCH` or `DELETE` a browser sent from another site, with 403; `web.app(cross_site: [...])` names prefixes that take them | Every example carried the same hand-written `same_origin` check; forgetting it was a CSRF hole | A cross-site form post that worked gets 403 unless its path is in `cross_site` |
| 2026-10-27 | A glaze's `request: web.Request` prop is filled in with the request being handled | Layouts needed values from the request, so every route threaded them through every render | Additive |
| 2026-10-27 | `env(name)` returns `String?`, and `os.env` is removed | An unset variable and an empty one were the same `""`, so `if env("X") == "" { default } else { env("X") }` was everywhere; `env("X") ?? default` says it | Code that used `env(...)` as a `String` adds `?? default`; `os.env` becomes `env` |
| 2026-10-27 | `status:` and `headers:` on every response function, `web.render` included | A page with another status, or a JSON reply with a cache header, had to be built by hand | Additive |
| 2026-10-27 | `res.set_cookie(name, value, ...)` with safe defaults (`HttpOnly`, `Secure`, `SameSite=Lax`), values percent-encoded and `req.cookie` decoding them | Cookies were written as raw `set-cookie` text, and a second cookie replaced the first | `req.cookie` now decodes `%XX` |
| 2026-10-27 | `app.before(prefix, hook)` runs ahead of the routes under a prefix; `fn rpc_guard` is removed | One general hook covers sign-in checks for pages and guards for `server fn`s, which are routes under `/_kiln/rpc` | A program with `rpc_guard` moves it to `app.before("/_kiln/rpc", ...)` |
| 2026-10-27 | The standard library's internals (`__` names, `ember.call`, `ember.npm`, `ember.Island`, `ember.Reply`) cannot be used by programs and are not offered by the editor | They are the compiler's protocol, and changing them would have broken programs | Code that used them no longer compiles |
| 2026-10-27 | `event.form<T>()` reads an ember's form as a struct; the map is `event.fields` | Handlers decoded each field by hand, unlike `req.form<T>()` on the server | `event.form["x"]` becomes `event.fields["x"]` |
| 2026-10-27 | `let pattern = value else { ... }` binds or leaves; the `else` block must leave (E0111) | Unwrapping an optional and returning early was written as `if not (x is some(_))` followed by `x ?? throw "unreachable"` | Additive |
| 2026-10-27 | `.Variant` and `.Variant(args)` stand for a variant of the enum the code expects, in expressions and patterns | Repeating the enum's name in every arm and argument added nothing the checker did not know | Additive; `Tone.Accent` still works |
| 2026-10-27 | Structs, records, lists, maps and sets are shared references, and this is documented as the model ([semantics.md](reference/semantics.md)); `let` only stops rebinding; a `const` is read-only all the way down (E0207) | Copy-on-assign would have changed every program and the runtime for little gain; the risk was that it was undocumented and that constants could be changed | Code that changed a `const` no longer compiles |
| 2026-10-27 | `Post(..post, title: "x")` and `{ ..record, x: 1 }` make shallow copies | With shared references, a copy with changes needs a short form | Additive |
| 2026-10-27 | `[key: value]` map literals | Building a map entry by entry was the only way | Additive |
| 2026-10-27 | `"""` strings for multi-line text, with the closing line's indentation removed | Emails, SQL and markdown in code needed `\n` joins | Additive |
| 2026-10-27 | `Any` and `Rest` are reserved for the standard library (E0233) | They let user code escape the type system; generics say the same thing safely | Code that named them no longer compiles |
| 2026-10-27 | A Glaze page never serves a route by itself; every route is registered in code, such as `app.get("/", fn(req) { web.render("home.glaze") })` | The vision keeps routing explicit in application code: Glaze files are views and components, not URLs | None: this was never implemented |
| 2026-10-26 | WasmGC is the only browser target through 1.0; pages keep their server-rendered HTML where it is missing | Phase 4 confirmed the browser support the plan assumed; a JavaScript backend stays a later option | None |
