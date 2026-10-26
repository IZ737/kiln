# Kiln syntax reference

This is the syntax the phase 0 parser accepts. It is the source of truth
for the grammar until a formal specification exists; every example here is
also covered by a test in `crates/kiln_syntax/tests/`.

## Files and lines

- Source files end in `.kiln`. Templates and components (glazes) end in
  `.glaze`; see [glazes.md](glazes.md).
- A newline ends a statement. There are no semicolons.
- Newlines are ignored inside `( )`, `[ ]` and record literals `{ name: value }`,
  so long calls can span lines.
- An expression continues onto the next line when the line ends with a binary
  operator, or when the next line starts with `.` (method chains).
- Comments: `// line` and `/* block */`.
- A folder is one module. Its `.kiln` files share declarations; imports stay
  per source file. `import posts` and `import posts.search` load explicit local
  folder modules from the project root. `pub` exports declarations. See
  [projects.md](projects.md) for roots, visibility and optional `kiln.toml`.

## Declarations

Only declarations may appear at the top level of a file.

```kiln
import web, sql, web.router

const PAGE_SIZE: Int = 20

pub struct Post {
    id: Int
    title: String
    published: Bool = false
}

enum Role {
    Admin
    Editor
    Viewer(since: Date)
}

impl Post {
    fn slug(self) -> String { self.title.lower() }
}

fn add(a: Int, b: Int) -> Int { a + b }

fn load(id: Int) -> Post ! DbError { ... }   // can fail with DbError
fn main() ! { ... }                            // can fail with the default error

server fn like_post(id: Int) -> Int ! web.Error { ... }   // callable from the browser
```

- Function parameters always have types. Closure parameters may omit them.
- Members of a `struct`, `enum` or `impl` are separated by newlines or commas.

## Types

| Form | Meaning |
| --- | --- |
| `Int`, `String`, `web.Error` | A named type, optionally qualified by module |
| `List<Post>`, `Map<String, Int>` | A generic type |
| `User?` | An optional: a `User` or `none` |
| `fn(Request, Int) -> Response !` | A function type; `!` means it can fail |
| `{ name: String, age: Int }` | An anonymous record type, with its fields in order |

There is no `null`.

## Statements

```kiln
let name = "Kiln"          // immutable
var count: Int = 0         // mutable
count += 1                 // also -=, *=, /=, =
return value
for post in posts where post.published { ... }
for i in 0..10 { ... }                        // 0 to 9: the end is excluded
while count < 10 { ... }
break
continue
```

## Expressions

Precedence, from loosest to tightest:

| Level | Operators |
| --- | --- |
| 1 | `expr catch err { ... }` |
| 2 | `or` |
| 3 | `and` |
| 4 | `not` (prefix) |
| 5 | `==` `!=` `<` `<=` `>` `>=` `is` |
| 6 | `??` |
| 7 | `..` (range) |
| 8 | `+` `-` |
| 9 | `*` `/` `%` |
| 10 | prefix `-`, `try`, `throw` |
| 11 | calls `f(x)`, generic calls `f<T>(x)`, fields `a.b`, indexing `a[i]` |

Other forms:

```kiln
"Hi ${user.name}"                        // string interpolation
250ms  2s  5m  1h                        // durations
[1, 2, 3]                                // list
{ posts, title: "Home" }                 // record; `posts` means `posts: posts`
web.json(post, status: 201)              // named arguments
fn(p) { p.published }                    // closure
.title                                   // field shorthand, same as fn(x) { x.title }
if a { 1 } else if b { 2 } else { 3 }
match role {
    Admin => "admin"
    Viewer(since) if since.year < 2020 => "veteran"
    some(x) => x
    _ => "member"
}
user is some(u) and u.active             // pattern test
try db.one<Post>(sql, id)                // propagate an error
row ?? throw DbError.NotFound            // default, or fail
```

A `{` in expression position always starts a record. Blocks appear only as
the bodies of functions, closures, `if`, loops, `catch` and `match` arms.

## Patterns

`_`, a literal, a lowercase name (binds the value), `none`, and variants:
`Admin`, `Role.Admin`, `some(x)`, `Viewer(since)`.

## Habits from other languages

These are errors with a suggested fix, because coding agents and people
coming from other languages write them by reflex. Run `kiln explain <code>`
for details.

| You wrote | Write instead | Code |
| --- | --- | --- |
| `null`, `nil`, `undefined` | `none` | E0106 |
| `;` at line end | nothing | E0107 |
| `function`, `func`, `def`, `let mut`, `elif` | `fn`, `fn`, `fn`, `var`, `else if` | E0108 |
| `===`, `!==` | `==`, `!=` | E0109 |
| `&&`, `\|\|`, `!x` | `and`, `or`, `not x` | E0110 |

## Component source files

`.glaze` is server-only; `.ember` is browser-capable and may also render on the
server. Both use the [template grammar](glazes.md). Reactive `{% state %}` and
`on:` handlers require `.ember`; ordinary `{% let %}` stays immutable. Components
resolve by their same-folder filename, with an error if both suffixes define the
same name. Page layouts remain `.glaze`.

Qualified tags such as `<posts.search.Result />` explicitly reference components
in reachable folder modules. Folder segments are exact module paths; the final
capitalized name maps to the component's snake-case filename.
