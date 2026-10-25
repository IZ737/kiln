# Standard library

The standard library's signatures are Kiln files in
[`crates/kiln_check/std/`](../../crates/kiln_check/std). The prelude is visible
everywhere; every other module is imported by name (`import fs`). This page
lists the modules the compiler can build into native programs today. The
others (`sql`, `auth`), and the parts of `web` not listed here,
type-check, and building a program that uses them reports E0401 until their
phase arrives.

## Prelude

`print`, `env` and `panic`, and the methods of `Int`, `Float`, `String`,
`Duration`, `List`, `Map` and `Set`. The full list is in
[`prelude.kiln`](../../crates/kiln_check/std/prelude.kiln); the ones most
programs reach for:

| Type | Methods |
| --- | --- |
| `Int` | `to_string`, `to_float`, `abs`, `min`, `max`, `pow`, `clamp(lo, hi)` |
| `Float` | `to_string`, `round`, `floor`, `ceil` (to `Int`), `abs`, `sqrt`, `min`, `max`, `fixed(digits)` (`3.14159.fixed(2)` is `"3.14"`) |
| `String` | `len`, `upper`, `lower`, `trim`, `trim_start`, `trim_end`, `contains`, `starts_with`, `ends_with`, `index_of -> Int?`, `slice(start, end)`, `split`, `lines`, `chars`, `replace`, `repeat`, `pad_start(width, fill)`, `pad_end`, `to_int -> Int?`, `to_float -> Float?` |
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
| `fs.read_lines(path) -> List<String> !` | Lines without `\n` or `\r\n` |
| `fs.write(path, text) !` | Replaces the file, creating it if needed |
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
| `os.env(name) -> String?` | An environment variable, or none when unset |
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
| `io.write(text)` | Writes to standard output without a newline |
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

`fs`, `os` and `io` need the server, so embers cannot use them.

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

**Requests.** A `web.Request` has `method`, `path` (without the query
string), `query_string`, `headers` (a `Map<String, String>` with names in
lower case) and `body`.

| Method | |
| --- | --- |
| `req.query(name) -> String?` | A query parameter, decoded (`+` is a space) |
| `req.header(name) -> String?` | A header; the name is not case-sensitive |
| `req.cookie(name) -> String?` | A cookie from the `cookie` header |
| `req.json<T>() -> T !` | The body read as JSON into a `T` (see [`json`](#json)) |

When `req.json` fails and the handler passes the error on with `try`, the
server answers 400 with `{"error": "..."}` naming what was wrong, instead
of 500.

**Responses.** A `web.Response` has `status`, `headers` and `body`, and can
be changed before it is returned (`res.headers["cache-control"] = "no-store"`)
or made directly with `web.Response(status: 204, headers: Map(), body: "")`.

| Function | |
| --- | --- |
| `web.json(value, status: 200)` | `value` as JSON, `application/json` |
| `web.text(body, status: 200)` | `text/plain; charset=utf-8` |
| `web.html(body, status: 200)` | `text/html; charset=utf-8` |
| `web.render("page.glaze", { props })` | The glaze rendered as HTML (see [Rendering](glazes.md#rendering)) |
| `web.redirect(to, status: 302)` | Sends the client to `to` |
| `web.not_found()` | 404, made like the server's own (see Errors) |

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
`req.form` comes later.

**Settings.** `web.app()` works as it is in production, and takes
settings for when the defaults do not fit:

| Setting | Default | |
| --- | --- | --- |
| `max_body` | `16_777_216` | Bytes of body; a larger request gets 413 |
| `max_header` | `65_536` | Bytes of request line and headers; more, or more than 100 headers, gets 431 |
| `timeout` | `10s` | How long a client has to send a request's head (408 after that), and how long it may stall while sending the body or reading the response |
| `keep_alive` | `5s` | How long an idle connection stays open; `0s` closes each after one response |
| `access_log` | `true` | Whether to log each request |

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
| `base64.encode(data) -> String` | The bytes of `data` in standard Base64, with `=` padding |
| `base64.decode(text) -> String !` | The bytes `text` encodes; fails when it is not Base64 |

The decoded bytes need not be UTF-8, so a decoded image can be a
response's body. Programs carry their binary files (see
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
