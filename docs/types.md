# Kiln type rules

These are the rules `kiln check` enforces in phase 0. Every rule here has a
test in `crates/kiln_check/tests/check.rs`. Run `kiln explain <code>` for
the details of any error.

## Inference

- Function parameters and return types are always written. A function without
  `->` returns nothing.
- `let` and `var` types are inferred from the value; an annotation such as
  `let n: Int = ...` checks it.
- Closure parameters are inferred from where the closure is passed:
  `posts.filter(fn(p) { p.published })` knows `p` is a `Post`.
- Generic functions infer their type arguments from the arguments or from the
  expected type. When neither says, write them: `json.decode<Post>(text)`
  (E0212).

## Optionals

- A `T` can be used where a `T?` is expected, but not the reverse (E0202).
- Unwrap with `??`, `match`, or `if x is some(v) { ... }`. Fields and methods
  of a `T?` are not available until it is unwrapped (E0208).

## Lists, maps and sets

- `[1, 2]` is a `List<Int>`; `list[i]` panics when `i` is out of range, while
  `list.get(i)` returns a `T?`.
- Create an empty map or set with its type: `let m: Map<String, Int> = Map()`
  or `Set<String>()`. `map[key]` is a `V?` (`none` when the key is missing),
  and `map[key] = value` adds or replaces an entry.
- Maps and sets keep their keys in the order they were first added, so
  printing and looping are predictable. Two maps are equal when they hold the
  same keys with equal values, in any order.
- `for x in set` loops over a snapshot of the set's items.

## Errors

- `fn f() -> T ! E` can fail with an `E`. A bare `!` means the standard
  `Error` type.
- A call that can fail needs `try` (pass the error to the caller) or
  `catch err { ... }` (handle it) (E0203). `try` on a call that cannot fail is
  an error (E0204).
- `try` and `throw` need the enclosing function to be able to fail (E0206).
- Under `! E`, only errors of type `E` can be passed on (E0205). Convert others
  with `catch`:

  ```kiln
  let row = db.one<User>(query, id) catch err {
      throw DbError.Unavailable(err.message)
  }
  ```

  Under a bare `!`, any error can be passed on. An error that is not an
  `Error` becomes one on the way, with its shown value as the `message`, so
  `catch err { err.message }` always works.
- A closure can fail if its body uses `try` or `throw`; this is inferred.

## Calls

- Arguments are positional, then named: `web.json(post, status: 201)`.
  Parameters with defaults may be left out; others may not (E0209).
- A struct is built by calling its name with its fields as named arguments:
  `Post(title: "Hi")`. Fields with defaults may be left out.
- Methods with `self` are called on a value (`post.slug()`); methods without
  `self` are called on the type (`Post.untitled()`).

## Match

- `match` must cover every case of an enum, of `Bool`, and both `some` and
  `none` of an optional (E0210). Other types need a `_` arm.
- All arms must have the same type (E0202).

## Routes

- A route path's `{name}` segments become handler parameters after the
  request, in order. `{id: Int}` is an `Int`, `{name}` is a `String`:

  ```kiln
  app.get("/posts/{id: Int}", fn(req, id) { ... })   // id: Int
  ```

  A handler with the wrong number of parameters is E0211.

## Glazes

A directory is one app: its glazes see the types and functions of its `.kiln`
files, and its `.kiln` files see the props of its glazes.

- `{value}` must have a text form: `String`, `Int`, `Float`, `Bool`,
  `Decimal`, `Duration` or `Date` (E0217). Show a field of a struct, loop
  over a list, unwrap an optional (`{note ?? ""}`), and `{% match %}` an enum.
  `{raw value}` takes a `String`.
- `{% if %}` conditions are `Bool`; `{% if x is some(v) %}` binds `v` inside.
  `{% for %}` loops over a `List` or `Set`. `{% match %}` must cover every
  case (E0210).
- `web.render("post.glaze", { post })` is checked against the page's
  `{% props %}`: every prop without a default must be given, with the right
  type (E0209, E0202). The file must exist (E0219).
- A component tag `<PostCard post={post} />` is checked the same way against
  `post_card.glaze`; a text attribute gives a `String`, a bare attribute a
  `Bool`. A component takes children only if it has a `{% slot %}`.
  `client:load`, `client:visible`, `client:idle` and `client:only` are
  hydration hints, not props.
- `{% layout "base.glaze" title: value %}` passes props to the layout, which
  must contain `{% slot %}`.
- An ember (a glaze with `{% state %}` or an `on:` handler) runs in the
  browser: `sql` and `auth` are server-only (E0218). Call a `server fn`
  instead. Handlers cannot fail, so errors from a `server fn` are handled with
  `catch`. Props cannot be reassigned; state can.
