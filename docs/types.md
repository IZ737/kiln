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

  Under a bare `!`, any error can be passed on.
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
