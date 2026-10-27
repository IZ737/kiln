# Standard library

The standard library's signatures are Kiln files in
[`crates/kiln_check/std/`](../../crates/kiln_check/std). The prelude is visible
everywhere; every other module is imported by name (`import fs`). This page
lists the modules the compiler can build into native programs today. The
parts of `web` not listed here type-check, and building a program that uses
them reports E0401 until their phase arrives. `sql`, `orm` and `admin` have a
page of their own: [databases, models and migrations](data.md). Tests
import `test`, which `kiln test` provides: see [Testing](testing.md).

## Prelude

`print`, `env` and `panic`, and the methods of `Int`, `Float`, `String`,
`Bytes`, `Duration`, `List`, `Map` and `Set`. `env(name) -> String?` reads an
environment variable, which is `none` when it is not set:
`env("PORT") ?? "8080"`. The full list is in
[`prelude.kiln`](../../crates/kiln_check/std/prelude.kiln); the ones most
programs reach for:

| Type | Methods |
| --- | --- |
| `Int` | `to_string`, `to_float`, `abs`, `min`, `max`, `pow`, `clamp(lo, hi)` |
| `Float` | `to_string`, `round`, `floor`, `ceil` (to `Int`), `abs`, `sqrt`, `min`, `max`, `fixed(digits)` (`3.14159.fixed(2)` is `"3.14"`) |
| `String` | `len`, `upper`, `lower`, `trim`, `trim_start`, `trim_end`, `contains`, `starts_with`, `ends_with`, `index_of -> Int?`, `slice(start, end)`, `split`, `lines`, `chars`, `replace`, `repeat`, `pad_start(width, fill)`, `pad_end`, `to_int -> Int?`, `to_float -> Float?`, `bytes -> Bytes` |
| `Bytes` | `len`, `is_empty`, `slice(start, end) -> Bytes`, `text -> String !` |
| `List<T>` | `len`, `first`, `last`, `get -> T?`, `push`, `pop -> T?`, `insert(index, item)`, `remove_at(index)`, `contains`, `index_of`, `slice`, `take`, `skip`, `concat`, `reverse`, `sort`, `sort_by(key)`, `min`, `max`, `filter`, `map`, `flat_map`, `find`, `any`, `all`, `count`, `fold(initial, combine)`, `join` |
| `Map<K, V>` | `len`, `get`, `set`, `remove`, `contains`, `keys`, `values`, `entries -> List<{ key: K, value: V }>` |
| `Set<T>` | `len`, `add`, `remove`, `contains`, `to_list` |
| `Duration` | `to_millis -> Int`, `to_seconds -> Float` |

String positions count characters, not bytes. `slice` on a string or a list
clamps its bounds to the value, while `list[i]` and `remove_at` panic when
`i` is out of range. `sort`, `min` and `max` order numbers by value,
strings by their bytes, `false` before `true`, `none` before any value,
lists item by item, structs and records field by field in the order they are
written, and enums by variant and then fields. Maps, sets and functions
have no order; sort those with `sort_by`.

`Bytes` is binary data, such as an image, a download or an uploaded file.
A `String` is always UTF-8 text, so the two do not mix: `text.bytes()` gives
a string's bytes, and `data.text()` reads bytes as text, failing when they
are not UTF-8. Bytes positions count bytes, and `slice` clamps its bounds.
`print` shows bytes by their size (`<15 bytes>`), JSON carries them as
Base64 text, and SQL as a blob. `fs.read_bytes`, `req.bytes()`, a form's
file field and `base64.decode` make them, and `fs.write_bytes`,
`web.bytes` and `base64.encode` take them. Embers do not have `Bytes` yet
(E0401).

## `time`

Times are UTC. A `time.Instant` is a point in time, stored as `unix_nanos`,
nanoseconds since 1970-01-01.

| Function | |
| --- | --- |
| `time.now() -> Instant` | The current time |
| `time.unix(seconds: Int) -> Instant` | The instant `seconds` after 1970 |
| `time.since(start: Instant) -> Duration` | Time passed since `start` |
| `time.sleep(duration: Duration)` | Pauses the program, or in a server only the request's task |
| `time.today() -> Date` | Today's date |
| `time.parse(text: String) -> Instant !` | Reads RFC 3339 (`2026-10-24T21:30:00Z`, `...+02:00`) or a date alone |

`Instant` methods: `unix()` (whole seconds), `add(duration)`,
`since(earlier)`, `date()`, `iso()` and `format(pattern)`.

```kiln
import time

fn main() {
    let start = time.now()
    print(start.format("EEEE, MMMM d, yyyy 'at' HH:mm"))  // Saturday, October 24, 2026 at 21:30
    print(time.since(start).to_millis())
}
```

Format patterns: `yyyy` and `yy` years; `MMMM` (October), `MMM` (Oct),
`MM` and `M` months; `dd` and `d` days; `EEEE` (Saturday) and `EEE` (Sat);
`HH`, `H`, `hh` and `h` hours with `a` for AM or PM; `mm` minutes; `ss`
seconds; `SSS` fractions of a second. Text in single quotes is copied, and
`''` is a quote.

## `fs`

Paths are relative to the working directory. Functions that touch the disk
can fail with an `Error` whose message names the path and the reason, such
as `cannot read notes.txt: No such file or directory`.

| Function | |
| --- | --- |
| `fs.read(path) -> String !` | The whole file |
| `fs.read_bytes(path) -> Bytes !` | The whole file as bytes |
| `fs.read_lines(path) -> List<String> !` | Lines without `\n` or `\r\n` |
| `fs.write(path, text) !` | Replaces the file, creating it if needed |
| `fs.write_bytes(path, data) !` | Replaces the file with `data`, creating it if needed |
| `fs.append(path, text) !` | Adds to the end, creating the file if needed |
| `fs.exists(path) -> Bool` | |
| `fs.is_dir(path) -> Bool` | |
| `fs.list(path) -> List<String> !` | Names in a directory, sorted |
| `fs.mkdir(path) !` | Creates the directory and its parents |
| `fs.remove(path) !` | Removes a file or an empty directory |
| `fs.rename(from, to) !` | |

## `os`

| Function | |
| --- | --- |
| `os.args() -> List<String>` | Command-line arguments, without the program's name |
| `os.exit(code: Int) -> Never` | Ends the program |
| `os.cwd() -> String` | The working directory |
| `os.temp_dir() -> String` | `$TMPDIR`, or `/tmp` |
| `os.pid() -> Int` | The process id |
| `os.platform() -> String` | `"linux"`, `"macos"`, `"windows"` or `"unknown"` |

## `io`

| Function | |
| --- | --- |
| `io.read_line() -> String?` | The next line of standard input, or none at the end |
| `io.read_lines() -> List<String>` | The remaining lines |
| `io.read_all() -> String` | The rest of standard input |
| `io.write(text)` | Writes to buffered standard output without a newline |
| `io.flush()` | Flushes pending standard output without writing text or a newline |
| `io.print_err(value)` | Prints on standard error, as `print` does |

```kiln
import io

fn main() {
    var total = 0
    for line in io.read_lines() {
        total += line.to_int() ?? 0
    }
    print(total)
}
```

`fs`, `os` and `io` need the server, so embers cannot use them; the
browser's modules are [`browser`](#browser), [`js`](#js) and
[npm packages](#npm-packages).

`io.write()` may leave text buffered. Call `io.flush()` to make a prompt
visible before waiting for input; `io.read_line()` does not automatically
flush standard output.

```kiln
import io

fn main() {
    io.write("> ")
    io.flush()
    let input = io.read_line()
    print(input)
}
```

## `web`

An HTTP/1.1 server. `web.app()` makes an app, each route adds a handler,
and `listen` serves them:

```kiln
import web

fn main() ! {
    let app = web.app()
    app.get("/posts/{id: Int}", fn(req, id) {
        if id > 100 {
            return web.not_found()
        }
        return web.text("post ${id}, sorted by ${req.query("sort") ?? "date"}")
    })
    app.post("/posts", fn(req) {
        print("new post: ${req.body}")
        return web.text("created", status: 201)
    })
    try app.listen(":8080")
}
```

**Routes.** `app.get`, `post`, `put`, `patch` and `delete` take a pattern
and a handler. Each `{name}` segment of the pattern is a handler parameter,
after the request: a `String` by default, or `Int`, `Float` or `Bool` with a
type (`{id: Int}`). A segment that does not convert, such as `/posts/abc`
for `{id: Int}`, does not match. Segments are compared after `%XX` escapes
are decoded. The first route that matches the method and the path handles
the request; `HEAD` requests go to `GET` routes and get no body. A path
that no route matches gets 404, and a path that matches only with other
methods gets 405 with an `allow` header. A handler can also be a function:
`app.get("/", home)` with `fn home(req: web.Request) -> web.Response !`.

**Before hooks.** `app.before(prefix, hook)` runs `hook` ahead of the
routes for every request whose path is `prefix` or below it at a `/`, so
`/admin` covers `/admin/users` but not `/administrators`. The hook takes
the request and returns a `web.Response?`: a response answers the request
then and there, `none` lets it go on to the next hook and then its route,
and an error answers as a handler's would. Hooks run in the order they
were added.

```kiln
app.before("/admin", fn(req) {
    let some(who) = try sessions.current(req) else {
        return web.redirect("/sign-in")
    }
    if not who.roles.contains("admin") {
        throw web.error(403, "admins only")
    }
    return none
})
```

The routes of `server fn`s are under `/_kiln/rpc/<name>`, so a hook on
`/_kiln/rpc` guards every call from an ember, and one on
`/_kiln/rpc/reset` guards one function.

**Requests.** A `web.Request` has `method`, `path` (without the query
string), `query_string`, `headers` (a `Map<String, String>` with names in
lower case) and `body`.

| Method | |
| --- | --- |
| `req.query(name) -> String?` | A query parameter, decoded (`+` is a space) |
| `req.header(name) -> String?` | A header; the name is not case-sensitive |
| `req.cookie(name) -> String?` | A cookie from the `cookie` header, with `%XX` escapes decoded |
| `req.json<T>() -> T !` | The body read as JSON into a `T` (see [`json`](#json)) |
| `req.form<T>() -> T !` | A submitted form read into a `T` (see [`validate`](#validate)) |
| `req.bytes() -> Bytes` | The body as it came, for a body that need not be text |

When `req.json` or `req.form` fails and the handler passes the error on
with `try`, the server answers 400 with `{"error": "..."}` naming what was
wrong, instead of 500.

**Forms.** `req.form<T>()` reads a body sent as
`application/x-www-form-urlencoded` or `multipart/form-data` into a
struct. A form carries text, and the fields' types say what to make of it:

| Field type | |
| --- | --- |
| `String` | The value as it was sent; 400 when it is not UTF-8 text |
| `Bytes` | The value's bytes: a file field's contents |
| `Int`, `Float` | The value as a number, or 400 when it is not one |
| `Bool` | `on`, `true`, `1` or `yes` is true; a box nobody ticked is not sent at all, so a missing `Bool` is `false` |
| `T?` | `none` when the field is missing or empty, or a file field sent without a file |
| `List<T>` | Every value sent with that name, in order |
| an enum | The name of a variant without fields |

A field of another type, or a field that is missing and is neither
optional, a `Bool` nor a `List`, is the client's mistake and answers 400.
A file arrives as a `multipart` part: read it into a `Bytes` field, and
save it with `fs.write_bytes`. Its file name is not a field yet.

**Responses.** A `web.Response` has `status`, `headers` and `body`, and can
be changed before it is returned (`res.headers["cache-control"] = "no-store"`)
or made directly with `web.Response(status: 204, headers: Map(), body: "")`.

| Function | |
| --- | --- |
| `web.json(value, status: 200, headers: Map())` | `value` as JSON, `application/json` |
| `web.text(body, status: 200, headers: Map())` | `text/plain; charset=utf-8` |
| `web.html(body, status: 200, headers: Map())` | `text/html; charset=utf-8` |
| `web.render("page.glaze", { props }, status: 200, headers: Map())` | The glaze rendered as HTML (see [Rendering](glazes.md#rendering)) |
| `web.redirect(to, status: 302, headers: Map())` | Sends the client to `to` |
| `web.bytes(data, content_type: "application/octet-stream", status: 200, headers: Map())` | Binary data, such as an image or a download |
| `web.not_found()` | 404, made like the server's own (see Errors) |

A response's `body` is a `String`; one made by `web.bytes` holds its
bytes as they are, which need not be text.

`headers` are added to the ones the function sets itself and replace
those with the same name: `web.json(page, headers: ["cache-control":
"no-store"])`. Write header names in lower case.

**Cookies.** `res.set_cookie(name, value, max_age: none, path: "/",
same_site: .Lax, http_only: true, secure: true)` adds a `set-cookie`
header and returns the response, so calls chain:

```kiln
return web.redirect("/", status: 303)
    .set_cookie("theme", "dark", max_age: 720h)
    .set_cookie("seen_intro", "1", same_site: .Strict)
```

The defaults are the safe ones: scripts cannot read the cookie, it is
sent only over HTTPS, and not with requests from other sites except
top-level links (`web.SameSite` is `.Lax`, `.Strict` or `.None`, and
`.None` always adds `Secure`). Without `max_age` the cookie lasts until
the browser closes, and `max_age: 0s` deletes it. A value is
percent-encoded where a cookie cannot hold it as it is, and
`req.cookie` decodes it; a name that is not an HTTP token panics.

The server adds `content-length` and `date`, and `connection: close` when it
will close the connection.

**Errors.** When a handler fails, the server logs the error on standard
error (`error GET /posts/7 failed: ...`) and answers 500 with `Internal
Server Error`, since an error's message can hold details the client should
not see; a panic is logged the same way (`... panicked: ...`). The server
goes on with the next request either way.

To answer with another status, fail with `web.error(status, message)`, an
`Error` whose status (400 to 599) and message are for the client. It can
come from any function the handler calls with `try`:

```kiln
fn find(id: Int) -> Post ! {
    if id != 1 {
        throw web.error(404, "there is no post ${id}")
    }
    return Post(id: 1, title: "Firing schedules")
}

app.get("/posts/{id: Int}", fn(req, id) {
    let post = try find(id)
    return web.json(post)
})
```

Errors of 500 and over are also logged. Error responses (from `web.error`,
`web.not_found()`, 404 and 405 when no route matches, failed handlers, and
requests whose body is too large or malformed) are `{"error": "message"}`
when the request's `accept` or `content-type` mentions JSON, and the
message as text otherwise. A request whose head the server cannot read
gets a bare 400, 414 or 431 with no body, and one whose head does not
arrive in time gets 408 as text. `app.on_error` makes them yourself, such as an HTML error page:

```kiln
app.on_error(fn(req, status, message) {
    return web.html("<h1>${status}</h1><p>${message}</p>", status: status)
})
```

Its handler gets the request, the status, and the message: a `web.error`'s
message, or else the status's reason, such as `Not Found`. If it panics,
the server makes the response itself.

**Listening.** `app.listen(address)` serves until the program ends. The
address is `host:port`: `:8080` is every IPv4 address, `localhost:8080`
only this machine, `[::1]:8080` an IPv6 address, and port 0 any free port.
It logs `info listening on http://localhost:8080` when it starts, and fails
if it cannot listen, such as when the port is in use.

**Access log.** The server logs a line on standard error for each
request it answers, with the method, the path without the query string,
the status and the time it took: `info GET /posts/7 200 1.2ms`.

**Connections.** Each connection is served on a task of its own (see
[tasks](#tasks)), so a handler that sleeps or a client that sends slowly
holds up only its own request. Connections stay open for more requests
(HTTP/1.1 keep-alive) until the client sends `connection: close`, and a
handler can close one by setting that header on its response. The HTTP
itself (reading requests, chunked bodies, `expect: 100-continue`,
keep-alive and writing responses) is [hyper](https://hyper.rs)'s. Header
names in responses are sent in lower case, a header whose name or value
HTTP does not allow is left out (with an error logged), and the
`content-length` of a response is its body's length, except in answer to
`HEAD`.

Query strings are decoded as forms are, path segments by their `%XX`
escapes, and cookies as the `cookie` header lays them out; decoded bytes
that are not UTF-8 become U+FFFD.

**Settings.** `web.app()` works as it is in production, and takes
settings for when the defaults do not fit:

| Setting | Default | |
| --- | --- | --- |
| `max_body` | `16_777_216` | Bytes of body; a larger request gets 413 |
| `max_header` | `65_536` | Bytes of request line and headers; more, or more than 100 headers, gets 431 |
| `timeout` | `10s` | How long a client has to send a request's head (408 after that), and how long it may stall while sending the body or reading the response |
| `keep_alive` | `5s` | How long an idle connection stays open; `0s` closes each after one response |
| `access_log` | `true` | Whether to log each request |
| `cross_site` | `[]` | Path prefixes that take `POST`, `PUT`, `PATCH` and `DELETE` from other sites, such as `["/webhooks"]` |

**Requests from other sites.** A browser says where a request comes from
(`sec-fetch-site`, or else `origin`). The server answers 403 to a
`POST`, `PUT`, `PATCH` or `DELETE` that a page on another site sent,
before any hook or route sees it, so a form on someone else's page cannot
act as the person signed in here. Requests without those headers, such as
those from curl, servers and `kiln test`, pass. Paths under a prefix in
`cross_site` take requests from anywhere, for webhooks and public APIs.
Session tokens (`sessions.check`) still protect against a page on a
sibling subdomain.

```kiln
let app = web.app(max_body: 1_048_576, timeout: 30s)
```

### Tasks

The server runs on lightweight tasks, which share one core: a task runs
until it waits for the network or for `time.sleep`, and then another
runs. Waiting never stops the others, but a handler that computes for a
long time without waiting holds up every request until it is done.

Tasks are not yet something programs create themselves. Handlers may share
values, but Kiln does not promise that changing a shared value from
several handlers is safe: today's single-core scheduler only switches
tasks where they wait, and a later one will run tasks on several cores.
Keep shared values immutable; `Mutex`, `Atomic` and channels will come for
the rest.

## `auth`

Passwords, sessions kept in the database, and the identity of a request.
A password is stored as an Argon2id hash, never in a form it can be read
back from. A session is a long random value the browser keeps in a
cookie, and the database holds only its digest, so a copy of the table is
not a set of usable sessions.

```kiln
import web, sql, orm, auth

struct Account {
    id: Int
    email: String
    password: String
    role: String
}

fn main() ! {
    let db = try sql.open(env("DATABASE_URL") ?? "sqlite:app.db")
    let sessions = try auth.sessions(db)
    let app = web.app()

    app.post("/sign-in", fn(req) {
        let form = try req.form<{ email: String, password: String }>()
        let found = try orm.find<Account>(db).where(fn(a) { a.email == form.email }).first()
        if found is some(account) and auth.verify_password(form.password, account.password) {
            let home = web.redirect("/", status: 303)
            return try sessions.sign_in(home, account.id.to_string(), roles: [account.role])
        }
        // The same answer either way, so the form cannot be used to find
        // out which addresses have accounts.
        return web.render("sign-in.glaze", { wrong: true }, status: 401)
    })

    app.get("/notes", fn(req) {
        let who = try sessions.require(req)
        let notes = try orm.find<Note>(db).where(fn(n) { n.author == who.subject }).all()
        return web.render("notes.glaze", { notes, csrf: try sessions.csrf(req) })
    })

    try app.listen(":8080")
}
```

| Function | |
| --- | --- |
| `auth.hash_password(password) -> String !` | The hash to store; a different one every time |
| `auth.verify_password(password, hash) -> Bool` | Whether the password is the one the hash was made from |
| `auth.token(bytes: Int = 32) -> String` | A value no one can guess, for a link in an email or an API key |
| `auth.sha256(text: String) -> String` | The SHA-256 of the text's bytes, as 64 lower-case hex digits: for checksums, and API keys stored hashed (never passwords) |
| `auth.sessions(db, cookie: "kiln_session", idle: 12h, lifetime: 720h, secure: true) -> Sessions !` | Sessions in `db` |

`auth.sessions` makes its table, `kiln_sessions`, when it is not there
yet, and never changes it when it is; the migrations of
[`kiln db`](data.md#migrations) do not describe it, because it is the runtime's and
not one of the program's models. `idle` is how long a session may sit
unused and `lifetime` how long it may last at all; `0s` is no limit.
Keep `secure` on, so the cookie is only ever sent over HTTPS.

| Method | |
| --- | --- |
| `sessions.sign_in(res, subject, roles: []) -> web.Response !` | A new session, and its cookie on `res` |
| `sessions.sign_out(req, res) -> web.Response !` | Ends this request's session and clears its cookie |
| `sessions.sign_out_all(subject) -> Int !` | Ends every session of a subject, after a password change |
| `sessions.current(req) -> Identity?` ! | Who the request is from, or `none` |
| `sessions.require(req) -> Identity !` | The same, or an error the server answers 401 with |
| `sessions.csrf(req) -> String !` | The token a form on this page must send back |
| `sessions.check(req, token) !` | Lets the request through only with this session's token, and otherwise fails with 403 |
| `sessions.sweep() -> Int !` | Deletes the sessions that are over |

An `Identity` has `subject` (what `sign_in` was given, usually a user's
key as text), `roles` and `started`. Identity is a request's, never the
program's: a server answers many requests at the same time, so there is no
ambient signed-in user.

**Forms that only this site can post.** Put `sessions.csrf(req)` in a
hidden field of every form that changes something, and call
`sessions.check(req, form.csrf)` in the handler before the change. Signing
in starts a session of its own and leaves any old one behind, so a session
someone else put in the browser cannot become a signed-in one, and
`sign_out_all` ends the sessions a changed password should not keep.

**Who an ember's call is from.** A `server fn` whose first parameter is a
`web.Request` is given the request it was called by, and the browser sends
the rest:

```kiln
server fn publish(req: web.Request, id: Int) -> Bool ! {
    let who = try sessions.require(req)
    ...
}
```

```glaze
<button on:click={ done = publish(post.id) catch err { false } }>Publish</button>
```

## `validate`

Checking submitted input. A rule is a function that returns the problem it
found, or `none` when the value is fine, so a program's own functions are
rules too: `validate.check` keeps the problems that are there.

```kiln
import web, validate

struct Signup {
    name: String
    email: String
    age: Int
    plan: String
    newsletter: Bool
}

fn main() ! {
    let app = web.app()
    app.post("/signup", fn(req) {
        let input = try req.form<Signup>()
        let checked = validate.check([
            validate.required("name", input.name),
            validate.length("name", input.name, min: 2, max: 60),
            validate.email("email", input.email),
            validate.range("age", input.age, 13, 120),
            validate.one_of("plan", input.plan, ["free", "paid"]),
            validate.when(input.plan == "paid" and not input.newsletter,
                "newsletter", "a paid plan comes with the newsletter"),
        ])
        if not checked.valid {
            // The same form again, with the problems in it.
            return web.render("signup.glaze", { input, checked }, status: 422)
        }
        return web.redirect("/welcome", status: 303)
    })
    try app.listen(":8080")
}
```

A form the server cannot read at all is the client's mistake, and answers
400; input that reads but breaks a rule is the person's, and the handler
shows the form again, usually with 422.

| Rule | The problem it finds |
| --- | --- |
| `validate.required(field, value)` | Empty, or only spaces |
| `validate.length(field, value, min: 0, max: 0)` | Too short or too long in characters; a `max` of 0 is no limit |
| `validate.range(field, value, min, max)` | A number outside the range |
| `validate.email(field, value)` | Not an email address. An empty value is for `required` |
| `validate.one_of(field, value, choices)` | Not one of the choices |
| `validate.same(field, value, other)` | Two values that must match, such as a password and its confirmation |
| `validate.when(wrong, field, message)` | `message`, when `wrong`. For a rule of the program's own |
| `validate.problem(field, message)` | A `Problem` directly, for a check that is not a rule |

`validate.check(problems) -> Checked` keeps the problems in the order they
were written. A `Checked` has `valid` and `problems` (a
`List<Problem>`, each with a `field` and a `message`), `checked.error(field)`
for the first message about one field, next to the field in a Glaze, and
`checked.messages()` for every message.

Rules run on the server, where every write is checked. An ember gets the
same answers by calling a `server fn` that runs these rules, rather than
checking in the browser, which a client can skip.

## `html`

Rendered glazes escape every value themselves; these are for HTML a
program builds by hand.

| Function | |
| --- | --- |
| `html.escape(text) -> String` | `&`, `<`, `>`, `"` and `'` as entities, safe in HTML text and quoted attributes |
| `html.safe_url(url) -> String` | The URL when it is relative or `http`, `https`, `mailto` or `tel`, otherwise `about:invalid#kiln-unsafe-url` |

```kiln
import html, web

fn greeting(name: String) -> web.Response {
    return web.html("<p>Hello, ${html.escape(name)}</p>")
}
```

Escaping is the [html-escape](https://github.com/magiclen/html-escape)
crate's, in the runtime's Rust part. `safe_url` finds the scheme as
browsers do, ignoring leading spaces and control characters and tabs or
line breaks inside it, so `java\tscript:` is not safe either.

## `base64`

| Function | |
| --- | --- |
| `base64.encode(data: Bytes) -> String` | `data` in standard Base64, with `=` padding |
| `base64.decode(text) -> Bytes !` | The bytes `text` encodes; fails when it is not Base64 |

Text is encoded as its bytes: `base64.encode("hi".bytes())`. A decoded
image can be sent with `web.bytes`. Programs carry their binary files (see
[Styles](glazes.md#styles)) this way. The runtime uses the
[base64](https://github.com/marshallpierce/rust-base64) crate.

## `json`

JSON is derived from types when the program is compiled; there is no
reflection at run time.

| Function | |
| --- | --- |
| `json.encode(value) -> String` | `value` as compact JSON |
| `json.decode<T>(text) -> T !` | Reads `text` as a `T` |

```kiln
import json

struct Post {
    id: Int
    title: String
    tags: List<String>
    summary: String?
}

fn main() ! {
    let post = try json.decode<Post>("{\"id\": 1, \"title\": \"Hi\", \"tags\": []}")
    print(post.summary ?? "no summary")        // no summary
    print(json.encode(post))  // {"id":1,"title":"Hi","tags":[],"summary":null}
}
```

| Kiln | JSON |
| --- | --- |
| `String`, `Bool` | string, `true` or `false` |
| `Int` | an integer; decoding rejects fractions and numbers past 64 bits |
| `Float` | a number; infinities and NaN encode as `null` |
| `Duration` | an integer number of nanoseconds |
| `T?` | `null` when none |
| `List<T>`, `Set<T>` | an array (sets of strings or integers decode) |
| `Map<K, V>` | an object; `Int` keys are written as strings |
| structs, records | objects with a member per field |
| enum variants | the name as a string (`"Dot"`) when it has no fields, otherwise `{"Circle": {"radius": 1.5}}`, `{"Some": 3}` for one unnamed field, or `{"Rect": [2, 3]}` for several |

When decoding, members a struct does not have are ignored, and a missing
field is an error unless its type is optional (it becomes `none`); a
field's default value is not used yet. Errors say what was expected and
where: `cannot read JSON: expected a string at $.tags[1], found a number`,
or `invalid JSON at line 1, column 10: ...` for text that is not JSON.

The encoder and decoder are [serde_json](https://github.com/serde-rs/json),
in the runtime's Rust part. Text to decode must be UTF-8, and string
escapes must be well formed (a `\u` surrogate needs its other half);
encoding writes bytes of a string that are not UTF-8 as U+FFFD. Values
nested past 128 levels in the type being read are rejected; nesting in
members that are skipped has no limit and cannot overflow the stack.

## `markdown`

| Function | |
| --- | --- |
| `markdown.to_html(text) -> String` | The HTML for CommonMark text, with GitHub's tables, strikethrough and task lists |

HTML written in the Markdown is shown as text, links and images keep only
the URLs `html.safe_url` keeps, and headings get ids from their words
(`## Getting started` is `<h2 id="getting-started">`), so `#getting-started`
links to it. The result is safe to place in a glaze with `{raw ...}`:

```kiln
import markdown, web

fn about(text: String) -> web.Response {
    return web.render("page.glaze", { body: markdown.to_html(text) })
}
```

```glaze
{% props body: String %}
<article>{raw body}</article>
```

The runtime uses the [pulldown-cmark](https://github.com/pulldown-cmark/pulldown-cmark)
crate.

## `browser`

The browser, for embers. Only an ember's `on:` handlers can use it (E0223
elsewhere), and they need no `import`.

| Function | |
| --- | --- |
| `browser.log(message)`, `browser.warn(message)` | Writes to the browser's console |
| `browser.title() -> String`, `browser.set_title(title)` | The page's title |
| `browser.location() -> String`, `browser.navigate(url)` | The page's address, and going to another (an address that would run script is refused) |
| `browser.now() -> Float` | Milliseconds since the page started loading |
| `browser.confirm(message) -> Bool` | Asks the user with the browser's dialog |
| `browser.get_item(key) -> String?`, `browser.set_item(key, value) !`, `browser.remove_item(key)` | The site's local storage |
| `browser.element(selector) -> js.Value?` | The first element that matches a CSS selector |
| `browser.focus(selector)` | Moves the focus to the first element that matches |
| `browser.fetch(url) -> String !` | GETs a URL and gives its text; an answer other than 2xx is an error. Waits |
| `browser.copy(text) !` | Puts text on the clipboard. Waits |

A function that waits is called like a `server fn`: as a statement of its
own in a handler, whose later statements run when it is done.

```glaze
{% props text: String %}
{% state copied = false %}
<button on:click={
    browser.copy(text) catch err { return }
    copied = true
}>{% if copied %}Copied{% else %}Copy{% end %}</button>
```

## `js`

JavaScript values, for what `browser` and npm bindings cannot reach. A
`js.Value` is a handle on a value that stays in the browser; it cannot be
state, a prop or a `server fn` argument (E0224). Turning one into a Kiln
value is checked, and fails with an error rather than guessing.

| Function | |
| --- | --- |
| `js.global(name) -> js.Value` | `globalThis[name]`, such as `js.global("document")` |
| `js.string(text)`, `js.float(n)`, `js.bool(b)`, `js.null()` | JavaScript values of Kiln ones |
| `value.get(name) -> js.Value`, `value.set(name, v)` | A property (`undefined` when there is none) |
| `value.call(method, args = []) -> js.Value !` | Calls a method; an exception is an error |
| `value.as_string() !`, `as_float() !`, `as_int() !`, `as_bool() !` | Checked conversions; `as_int` needs a whole number in `Int`'s range |
| `value.is_nullish() -> Bool`, `value.type_of() -> String` | `null` or `undefined`, and `typeof` |

```kiln
let doc = js.global("document")
let title = doc.get("title").as_string() catch err { "?" }
let n = doc.as_float() catch err { 0.0 }   // err.message: "expected a number, found an object"
```

## npm packages

`kiln add npm:<package>@<version>` adds a browser library for embers:

```sh
kiln add npm:nanoid@5.0.9
```

It downloads the package from the registry (`KILN_NPM_REGISTRY`, or
npm's), checks the tarball against the registry's sha512 integrity and runs
none of its scripts. It keeps, in `npm/<package>/`, only the package's
browser ES module and the files it imports, its TypeScript declarations,
`package.json` and its licence, and writes `npm.lock` with the SHA-256 of
each. A build refuses a file that changed. A package that imports other
packages, uses Node.js modules (`node:fs`) or is CommonJS only is refused,
with why.

The declarations become a module named after the package
(`canvas-confetti` is `canvas_confetti`, `@scope/name` is `scope_name`),
which ember handlers use without an import. `string` is `String`,
`number` is `Float`, `boolean` is `Bool`, `any` and `unknown` are
`js.Value`, `T | null`, `T | undefined` and optional parameters are
optional, and arrays of strings, numbers and booleans are lists; names
become snake case. Every
call can fail, as JavaScript can throw. A function that returns a
`Promise` waits, like a `server fn`. A constant is a function with no
arguments. Functions and constants Kiln cannot map yet (callbacks,
overloads, other types) are left out, and `kiln add` and the checker say
why; classes and interfaces are not bound yet, and `js.Value` reaches
them.

```glaze
{% state id = "" %}
<button on:click={ id = nanoid.nanoid(10.0) catch err { err.message } }>New id</button>
<code>{id}</code>
```

The build serves the files at `/_kiln/npm/<package>@<version>/`, and the
page loads them with the embers.

## Adding to the standard library

1. Declare the signature in `crates/kiln_check/std/<module>.kiln`, and list a
   new module in `STD_MODULES` in `crates/kiln_check/src/checker.rs`.
2. For the compiler, add the module to `LIB_MODULES` in
   `crates/kiln_ir/src/lower.rs`; its calls become `ExprKind::Lib`. Build
   each function in the runtime (`crates/kiln_codegen/runtime/kiln.c`, as a
   `K_API` function) and in both backends: `lib_call` in
   `crates/kiln_codegen/src/emit.rs` for C, and `lib_call` in
   `crates/kiln_native/src/gen.rs` for native code, with the function's C
   signature in `crates/kiln_native/src/runtime.rs`. A runtime function that
   fails sets `k_lib_error` to its message. The runtime is also built for
   Windows with mingw-w64: put code that differs there under
   `#ifdef _WIN32`. Generated code that calls a C library function directly
   (such as `round`) needs it listed in `kiln_keep_*` in
   `crates/kiln_native/runtime/native.c`, so the Windows runtime has it.
3. Add a conformance program.
