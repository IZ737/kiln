//! The registry of error codes.
//!
//! Codes are stable once released: a code is never reused for a different
//! error. `kiln explain <code>` prints the explanation and examples, so every
//! entry carries a wrong and a right example.
//!
//! Ranges: `E00xx` lexing, `E01xx` parsing, `E02xx` names and types,
//! `E03xx` glazes (templates and components), `E04xx` the compiler.

pub struct ErrorCode {
    pub code: &'static str,
    pub title: &'static str,
    pub explanation: &'static str,
    pub wrong: &'static str,
    pub right: &'static str,
}

pub const UNEXPECTED_CHAR: &str = "E0001";
pub const UNTERMINATED_STRING: &str = "E0002";
pub const UNTERMINATED_COMMENT: &str = "E0003";
pub const INVALID_ESCAPE: &str = "E0004";
pub const INVALID_NUMBER: &str = "E0005";
pub const UNCLOSED_INTERPOLATION: &str = "E0006";

pub const EXPECTED_TOKEN: &str = "E0101";
pub const EXPECTED_EXPRESSION: &str = "E0102";
pub const EXPECTED_TYPE: &str = "E0103";
pub const EXPECTED_ITEM: &str = "E0104";
pub const EXPECTED_PATTERN: &str = "E0105";
pub const NO_NULL: &str = "E0106";
pub const NO_SEMICOLONS: &str = "E0107";
pub const WRONG_KEYWORD: &str = "E0108";
pub const NO_STRICT_EQUALITY: &str = "E0109";
pub const NO_SYMBOL_LOGIC: &str = "E0110";
pub const LET_ELSE_MUST_LEAVE: &str = "E0111";
pub const MISPLACED_BOUND: &str = "E0112";

pub const LOCAL_MODULE: &str = "E0230";
pub const UNDEFINED_NAME: &str = "E0201";
pub const TYPE_MISMATCH: &str = "E0202";
pub const MISSING_TRY: &str = "E0203";
pub const NOTHING_TO_TRY: &str = "E0204";
pub const ERROR_TYPE_MISMATCH: &str = "E0205";
pub const CANNOT_FAIL: &str = "E0206";
pub const ASSIGN_TO_IMMUTABLE: &str = "E0207";
pub const NO_SUCH_MEMBER: &str = "E0208";
pub const BAD_ARGUMENTS: &str = "E0209";
pub const NON_EXHAUSTIVE_MATCH: &str = "E0210";
pub const ROUTE_PARAMS: &str = "E0211";
pub const CANNOT_INFER: &str = "E0212";
pub const DUPLICATE_DEFINITION: &str = "E0213";
pub const NOT_CALLABLE: &str = "E0214";
pub const MISSING_VALUE: &str = "E0215";
pub const UNKNOWN_TYPE: &str = "E0216";
pub const NOT_DISPLAYABLE: &str = "E0217";
pub const SERVER_ONLY: &str = "E0218";
pub const UNKNOWN_GLAZE: &str = "E0219";
pub const BAD_STYLE: &str = "E0220";
pub const UNKNOWN_TOKEN: &str = "E0221";
pub const BAD_TOKEN: &str = "E0222";
pub const BROWSER_ONLY: &str = "E0223";
pub const NOT_SERIALIZABLE: &str = "E0224";
pub const BAD_MODEL: &str = "E0231";
pub const BAD_QUERY: &str = "E0232";
pub const RESERVED_TYPE: &str = "E0233";
pub const UNMET_BOUND: &str = "E0234";
pub const INTERFACE_MISUSE: &str = "E0235";
pub const STORE_TYPE: &str = "E0236";
pub const STATIC_PAGES: &str = "E0237";
pub const FILE_ROUTES: &str = "E0238";
pub const ROUTE_CONFLICT: &str = "E0239";
pub const ROUTE_FILE: &str = "E0240";
pub const ROUTE_SHADOWED: &str = "E0241";

pub const UNCLOSED_GLAZE_TAG: &str = "E0301";
pub const MISMATCHED_CLOSING_TAG: &str = "E0302";
pub const UNKNOWN_GLAZE_TAG: &str = "E0303";
pub const FOREIGN_TEMPLATE_SYNTAX: &str = "E0304";
pub const MISPLACED_DECLARATION: &str = "E0305";
pub const FOREIGN_ATTRIBUTE: &str = "E0306";
pub const UNCLOSED_GLAZE_CODE: &str = "E0307";
pub const STRAY_CLOSING_TAG: &str = "E0308";
pub const UNSAFE_ATTRIBUTE: &str = "E0309";
pub const ENDLESS_GLAZE: &str = "E0310";
pub const MISPLACED_FILL: &str = "E0311";
pub const BAD_CSS: &str = "E0312";
pub const UNKNOWN_MODIFIER: &str = "E0313";
pub const EMBER_REQUIRED: &str = "E0314";
pub const AMBIGUOUS_COMPONENT: &str = "E0315";
pub const EMBER_LAYOUT: &str = "E0316";

pub const NOT_COMPILED_YET: &str = "E0401";
pub const NO_MAIN: &str = "E0402";
pub const SERVER_CALL_PLACE: &str = "E0403";
pub const STATIC_PAGE: &str = "E0404";
pub const STATIC_EMBER: &str = "E0405";
pub const STATIC_LINK: &str = "E0406";

pub static ALL: &[ErrorCode] = &[
    ErrorCode {
        code: FILE_ROUTES,
        title: "A filesystem route mount or route file name kiln cannot use",
        explanation: "`app.file_routes(\"pages\")` turns the `.glaze` pages and `route.kiln` files under a project folder into routes when the program is compiled, so its folder, `at:` prefix and `ignore:` list must be written out as literals: a folder below the project root, a URL path starting with `/`, and plain folder names (no paths or globs). In a mounted folder, `[name]` is a whole-segment URL parameter; catch-all `[...name]`, optional `[[name]]` and `(group)` folders are not supported, and names may only use letters, digits, `-`, `_`, `.` and `~`.",
        wrong: "app.file_routes(\"pages\", at: \"admin\", ignore: [\"**/_components\"])\n// pages/(marketing)/about.glaze",
        right: "app.file_routes(\"pages\", at: \"/admin\", ignore: [\"_components\"])\n// pages/marketing/about.glaze",
    },
    ErrorCode {
        code: ROUTE_CONFLICT,
        title: "Filesystem routes that claim the same URLs",
        explanation: "Inside one `app.file_routes` mount, every method and URL has one owner. Two files for the same route (`about.glaze` and `about/index.glaze`), two parameter routes of the same shape (`[id].glaze` and `[slug].glaze`), and two routes that each match URLs the other does not but share one (`[section]/new.glaze` and `docs/[slug].glaze` both match `/docs/new`) are errors; a literal route that is a strict case of a parameter route (`docs/new.glaze` and `docs/[slug].glaze`) is fine and wins. Filesystem routes may not claim kiln's own URLs under `/_kiln`, such as `POST /_kiln/rpc/<name>`, whatever order they are registered in.",
        wrong: "pages/blog/[id].glaze\npages/blog/[slug].glaze",
        right: "pages/blog/[slug].glaze\npages/blog/new.glaze",
    },
    ErrorCode {
        code: ROUTE_FILE,
        title: "A page or route.kiln that does not fit its route",
        explanation: "A filesystem page gets its props from its URL parameters and, with a `route.kiln` beside it, from `pub fn load`, which returns a record or struct whose every field is a prop of the page. `load` and `pub fn get` cannot both answer `GET`; handlers take an optional first `req: web.Request` and then any of the route's parameters by name, typed `String`, `Int`, `Float` or `Bool`, the same wherever a parameter is declared. `pub fn static_params()` lists the pages of a route with parameters, `pub const static_refresh` renders them again after a while in a running server, and `pub const dynamic = true` leaves the route out of a static site; `dynamic` cannot be combined with the other two.",
        wrong: "// pages/blog/[slug]/route.kiln\npub fn load(slug: Int) -> { post: Post, slug: String } ! { ... }\npub fn get() -> web.Response { ... }",
        right: "// pages/blog/[slug]/route.kiln\npub fn load(slug: String) -> { post: Post } ! { ... }",
    },
    ErrorCode {
        code: ROUTE_SHADOWED,
        title: "A route that an earlier one answers first (a warning)",
        explanation: "The router answers a request with the first route, in the order they were added, whose method and pattern match it, whether it was written with `app.get` or found by `app.file_routes`. A route that an earlier one always answers first never runs. This is only a warning, since it may be on purpose; move the registration that should win before the other.",
        wrong: "app.file_routes(\"pages\")   // pages/about.glaze\napp.get(\"/about\", about)",
        right: "app.get(\"/about\", about)\napp.file_routes(\"pages\")",
    },
    ErrorCode {
        code: STATIC_PAGES,
        title: "A static route that lists the wrong pages",
        explanation: "`.static(...)` marks a route whose pages are rendered ahead of time: by `kiln fire --static` into files, or once by a running server. A route with parameters cannot know its pages by itself, so it is given a function that returns the parameters of each one: the parameter's value for a route with one, such as `Int` for `/posts/{id: Int}`, and a record with a field per parameter for more. A route without parameters has one page, so `.static()` takes nothing.",
        wrong: "app.get(\"/posts/{id: Int}\", fn(req, id) { show(id) }).static()",
        right: "app.get(\"/posts/{id: Int}\", fn(req, id) { show(id) }).static(fn() { [1, 2, 3] })",
    },
    ErrorCode {
        code: STATIC_PAGE,
        title: "A page a static site cannot serve",
        explanation: "`kiln fire --static` renders each page once, with an empty request, and writes it to a file that any static host serves as it is. So a page cannot depend on the request: its query string, headers, cookies or body, which differ from visit to visit. Its handler must answer 200 (or redirect), and a page that is not HTML needs a file extension in its path, such as `/feed.xml`, so the host knows its type. Leave a page that needs a server out of the export with `.dynamic()`.",
        wrong: "app.get(\"/search\", fn(req) { results(req.query(\"q\") ?? \"\") })",
        right: "app.get(\"/search\", fn(req) { results(req.query(\"q\") ?? \"\") }).dynamic()",
    },
    ErrorCode {
        code: STATIC_EMBER,
        title: "An ember that needs a server on a static page",
        explanation: "Embers run in the browser, so they work on a static site, but a `server fn` runs on the server, and a static host has none. An ember that calls one cannot be on an exported page: its calls would fail. Move the work into the ember, render the data into the page ahead of time, or leave the page out of the export with `.dynamic()`.",
        wrong: "<button on:click={ count = try add_like(id) }>Like</button>",
        right: "<button on:click={ count += 1 }>Like</button>",
    },
    ErrorCode {
        code: STATIC_LINK,
        title: "A link to a page the static site does not have",
        explanation: "After `kiln fire --static` writes the pages, it follows every link, form and file each one points to on the same site. One that points to a page the export did not write would be broken on the static host: a route with parameters that does not list that page, a `POST` route a form sends to, or a route left out with `.dynamic()`. List the page with `.static(...)`, or point somewhere that exists.",
        wrong: "app.get(\"/posts/{id: Int}\", show).static(fn() { [1] })\n// a page links to /posts/2",
        right: "app.get(\"/posts/{id: Int}\", show).static(fn() { [1, 2] })",
    },
    ErrorCode {
        code: STORE_TYPE,
        title: "A store without a written type or a usable key",
        explanation: "A `store` is application state that embers share, and the browser decodes its value from JSON by its type, so the type is written out: `store cart: Cart = Cart()`. A store kept between reloads with `persist \"key\"` needs a key of letters, digits, `-`, `_` and `.`, which no other store uses; change the key when the store's type changes in a way old values no longer fit.",
        wrong: "pub store cart = Cart()",
        right: "pub store cart: Cart = Cart()",
    },
    ErrorCode {
        code: RESERVED_TYPE,
        title: "A type only the standard library can use",
        explanation: "`Any` and `Rest` let the standard library describe functions Kiln's types cannot express yet, such as `json.encode`, which takes a value of any type with a JSON form. A program that used them would turn off type checking, so they are reserved. A function that works on values of any type is generic instead.",
        wrong: "fn describe(value: Any) -> String {\n    return \"${value}\"\n}",
        right: "fn describe<T>(value: T) -> String {\n    return \"${value}\"\n}",
    },
    ErrorCode {
        code: UNMET_BOUND,
        title: "A type that does not fit an interface",
        explanation: "A generic function's bound, as in `fn label<T: Named>(x: T)`, lists the methods its body calls on `T`. A type fits an interface when it has each of them, with `self` and the same parameter and result types; nothing else declares that it fits. The error names the first method the type is missing or has with another signature.",
        wrong: "interface Named {\n    fn name(self) -> String\n}\n\nstruct Dog {\n    nick: String\n}\n\nfn label<T: Named>(x: T) -> String {\n    return x.name()\n}\n\nfn main() {\n    print(label(Dog(nick: \"Rex\")))\n}",
        right: "interface Named {\n    fn name(self) -> String\n}\n\nstruct Dog {\n    nick: String\n}\n\nimpl Dog {\n    fn name(self) -> String {\n        return self.nick\n    }\n}\n\nfn label<T: Named>(x: T) -> String {\n    return x.name()\n}\n\nfn main() {\n    print(label(Dog(nick: \"Rex\")))\n}",
    },
    ErrorCode {
        code: INTERFACE_MISUSE,
        title: "An interface where a type belongs",
        explanation: "An interface is only a bound on a type parameter: each call of a generic function is compiled for the type it is given, so there are no interface values to pass around. Take a type parameter with the interface as its bound instead. A bound, in turn, names an interface, not a struct or another type.",
        wrong: "fn label(x: Named) -> String {\n    return x.name()\n}",
        right: "fn label<T: Named>(x: T) -> String {\n    return x.name()\n}",
    },
    ErrorCode {
        code: BAD_MODEL,
        title: "Not a model",
        explanation: "A model is one of the program's own structs: its fields are the columns of a table named after it (`Post` to `posts`), and one field, `id` by default, is its key. A field that holds another model, or a list of them, is a relation rather than a column: it needs the key field that links the two, it is loaded only when a query asks for it with `include`, and so it must be optional or a list with a default. Tables, columns and keys that do not follow these conventions are named with `orm.model<T>(...)`.",
        wrong: "struct Post {\n    title: String\n}\n\nlet posts = try orm.find<Post>(db).all()",
        right: "struct Post {\n    id: Int\n    title: String\n}\n\nlet posts = try orm.find<Post>(db).all()",
    },
    ErrorCode {
        code: BAD_QUERY,
        title: "A query the compiler cannot build",
        explanation: "The conditions of `where` and the keys of `order_by` become SQL when the program is compiled, so they are a restricted language: the model's fields, values, comparisons, `and`, `or`, `not`, arithmetic, and `contains`, `starts_with` and `ends_with` on a text column. Anything else is worked out in Kiln before the query and sent as a value, or written as SQL with `db.query`.",
        wrong: "orm.find<Post>(db).where(fn(p) { p.title.words().len() > 3 })",
        right: "orm.find<Post>(db).where(fn(p) { p.title.contains(word) })",
    },
    ErrorCode {
        code: LOCAL_MODULE,
        title: "Invalid local module",
        explanation: "Local folder imports must resolve inside the project root, be acyclic and expose only public declarations. Import paths are case-sensitive.",
        wrong: "// posts/main.kiln\nfn title() -> String { return \"Post\" }\n// main.kiln\nimport posts\nfn main() { print(posts.title()) }",
        right: "// posts/main.kiln\npub fn title() -> String { return \"Post\" }\n// main.kiln\nimport posts\nfn main() { print(posts.title()) }",
    },
    ErrorCode {
        code: EMBER_REQUIRED,
        title: "an Ember component is required",
        explanation: "Only `.ember` files may declare reactive state or event handlers. Hydration hints require an Ember target. Rename an interactive `.glaze` file to `.ember`, or remove the hydration hint from a server-only component. File renames cannot be expressed as a source-text replacement.",
        wrong: "{# counter.ember #}\n{% state count = 0 %}",
        right: "{# counter.ember #}\n{% state count = 0 %}",
    },
    ErrorCode {
        code: AMBIGUOUS_COMPONENT,
        title: "ambiguous component name",
        explanation: "A component tag must resolve to exactly one file. A same-folder pair such as counter.ember and counter.ember is ambiguous. Rename one component and update its tags.",
        wrong: "{# both counter.ember and counter.ember exist #}\n<Counter />",
        right: "{# counter.ember and summary.glaze #}\n<Counter /><Summary />",
    },
    ErrorCode {
        code: EMBER_LAYOUT,
        title: "Ember used as a page layout",
        explanation: "Layouts are server-side .glaze files. An Ember cannot declare a page layout. Place the Ember inside a Glaze page that uses the layout instead.",
        wrong: "{# counter.ember #}\n{% layout \"base.glaze\" %}",
        right: "{# page.glaze #}\n{% layout \"base.glaze\" %}\n<Counter />",
    },
    ErrorCode {
        code: UNEXPECTED_CHAR,
        title: "unexpected character",
        explanation: "The source contains a character that cannot start any token.",
        wrong: "let price = 5 # dollars",
        right: "let price = 5 // dollars",
    },
    ErrorCode {
        code: UNTERMINATED_STRING,
        title: "unterminated string literal",
        explanation: "A string literal was opened with `\"` but never closed on the same line. A `\"` string stays on one line; use `\\n` for a line break, or a `\"\"\"` string, whose text starts on the line after its opening quotes and ends before a line holding only the closing `\"\"\"`. That line's indentation is taken off every line, so no line may be indented less.",
        wrong: "let name = \"Jake",
        right: "let name = \"Jake\"",
    },
    ErrorCode {
        code: UNTERMINATED_COMMENT,
        title: "unterminated block comment",
        explanation: "A `/*` block comment was never closed with `*/`.",
        wrong: "/* TODO\nfn main() {}",
        right: "/* TODO */\nfn main() {}",
    },
    ErrorCode {
        code: INVALID_ESCAPE,
        title: "invalid escape sequence",
        explanation: "Inside a string, `\\` must be followed by one of `n`, `t`, `r`, `0`, `\\`, `\"` or `$`.",
        wrong: "let path = \"C:\\users\"",
        right: "let path = \"C:\\\\users\"",
    },
    ErrorCode {
        code: INVALID_NUMBER,
        title: "invalid number literal",
        explanation: "A number literal is malformed, for example a trailing `_` or a letter inside the digits.",
        wrong: "let n = 12ab",
        right: "let n = 12",
    },
    ErrorCode {
        code: UNCLOSED_INTERPOLATION,
        title: "unclosed string interpolation",
        explanation: "`${` inside a string starts an interpolated expression, which must be closed with `}` on the same line. Write `\\$` for a literal dollar sign before a brace.",
        wrong: "let s = \"Hi ${name\"",
        right: "let s = \"Hi ${name}\"",
    },
    ErrorCode {
        code: EXPECTED_TOKEN,
        title: "expected a specific token",
        explanation: "The parser needed a particular token here, such as a closing bracket or a name.",
        wrong: "fn add(a: Int, b: Int -> Int { a + b }",
        right: "fn add(a: Int, b: Int) -> Int { a + b }",
    },
    ErrorCode {
        code: EXPECTED_EXPRESSION,
        title: "expected an expression",
        explanation: "A value was needed here, such as a literal, a name, a call or a block.",
        wrong: "let total = ",
        right: "let total = 0",
    },
    ErrorCode {
        code: EXPECTED_TYPE,
        title: "expected a type",
        explanation: "A type was needed here. Types are names such as `Int` or `List<Post>`, optionally followed by `?`.",
        wrong: "fn find(id: ) -> Post? { none }",
        right: "fn find(id: Int) -> Post? { none }",
    },
    ErrorCode {
        code: EXPECTED_ITEM,
        title: "expected a declaration",
        explanation: "At the top level of a file only declarations are allowed: `import`, `fn`, `struct`, `enum`, `impl`, `const` and `server fn`. Statements belong inside a function such as `main`.",
        wrong: "print(\"hello\")",
        right: "fn main() {\n    print(\"hello\")\n}",
    },
    ErrorCode {
        code: EXPECTED_PATTERN,
        title: "expected a pattern",
        explanation: "A `match` arm starts with a pattern: `_`, a literal, a name, or an enum variant such as `some(x)`.",
        wrong: "match x {\n    => 1\n}",
        right: "match x {\n    _ => 1\n}",
    },
    ErrorCode {
        code: NO_NULL,
        title: "Kiln has no null",
        explanation: "There is no `null` or `nil`. A value that may be missing has an optional type `T?`, and the missing value is written `none`.",
        wrong: "let user: User? = null",
        right: "let user: User? = none",
    },
    ErrorCode {
        code: NO_SEMICOLONS,
        title: "semicolons are not used",
        explanation: "Statements end at the end of the line. Remove the `;`. To put two statements on one line, put them on separate lines instead.",
        wrong: "let a = 1;",
        right: "let a = 1",
    },
    ErrorCode {
        code: WRONG_KEYWORD,
        title: "keyword from another language",
        explanation: "This word is a keyword in another language. Kiln uses `fn` for functions, `let` and `var` for variables, and `else if` for chained conditions.",
        wrong: "function add(a: Int, b: Int) -> Int { a + b }",
        right: "fn add(a: Int, b: Int) -> Int { a + b }",
    },
    ErrorCode {
        code: NO_STRICT_EQUALITY,
        title: "`===` and `!==` do not exist",
        explanation: "Kiln never converts types implicitly, so `==` is already strict. Use `==` and `!=`.",
        wrong: "if a === b { }",
        right: "if a == b { }",
    },
    ErrorCode {
        code: NO_SYMBOL_LOGIC,
        title: "use `and`, `or` and `not`",
        explanation: "Logical operators are words: `and`, `or` and `not`. The symbols `&&`, `||` and prefix `!` are not used.",
        wrong: "if a && !b { }",
        right: "if a and not b { }",
    },
    ErrorCode {
        code: LET_ELSE_MUST_LEAVE,
        title: "the `else` of `let ... else` must leave",
        explanation: "`let some(x) = value else { ... }` binds `x` for the code after it, so when the value does not match, that code cannot run: the `else` block must end by leaving, with `return`, `throw`, `break`, `continue` or `panic(...)`.",
        wrong: "let some(user) = found else {\n    print(\"missing\")\n}",
        right: "let some(user) = found else {\n    return web.not_found()\n}",
    },
    ErrorCode {
        code: MISPLACED_BOUND,
        title: "a bound on a struct or enum",
        explanation: "Interfaces bound the type parameters of functions and methods, where the code that calls the methods is. A struct or enum keeps its type parameters plain, and the functions that need more of `T` say so.",
        wrong: "struct Shelf<T: Named> {\n    items: List<T>\n}",
        right: "struct Shelf<T> {\n    items: List<T>\n}\n\nfn labels<T: Named>(shelf: Shelf<T>) -> List<String> {\n    return shelf.items.map(fn(x) { x.name() })\n}",
    },
    ErrorCode {
        code: UNDEFINED_NAME,
        title: "undefined name",
        explanation: "The name is not a local variable, function, type or imported module. Check the spelling, declare it, or add the missing `import`.",
        wrong: "fn main() {\n    let app = web.app()\n}",
        right: "import web\n\nfn main() {\n    let app = web.app()\n}",
    },
    ErrorCode {
        code: TYPE_MISMATCH,
        title: "mismatched types",
        explanation: "A value has a different type from the one required here. Kiln never converts between types implicitly; convert explicitly, for example with `to_string()` or string interpolation.",
        wrong: "let label: String = 42",
        right: "let label: String = 42.to_string()",
    },
    ErrorCode {
        code: MISSING_TRY,
        title: "a call that can fail needs `try` or `catch`",
        explanation: "The called function can fail (its signature has `!`). Put `try` in front to pass the error to your caller, or add `catch err { ... }` after it to handle the error here.",
        wrong: "let posts = db.query<Post>(\"select * from posts\")",
        right: "let posts = try db.query<Post>(\"select * from posts\")",
    },
    ErrorCode {
        code: NOTHING_TO_TRY,
        title: "`try` on something that cannot fail",
        explanation: "`try` and `catch` only apply to calls of functions that can fail. Remove them here.",
        wrong: "let n = try names.len()",
        right: "let n = names.len()",
    },
    ErrorCode {
        code: ERROR_TYPE_MISMATCH,
        title: "error type does not match the function's",
        explanation: "A function declared `! E` can only fail with errors of type `E`. Handle other errors with `catch`, or declare the function with a bare `!`, which accepts any error.",
        wrong: "fn load() -> User ! DbError {\n    try auth.require()\n}",
        right: "fn load() -> User ! {\n    try auth.require()\n}",
    },
    ErrorCode {
        code: CANNOT_FAIL,
        title: "this function cannot fail",
        explanation: "`try` and `throw` pass an error to the caller, which is only allowed in a function whose signature has `!`. Add `!` after the return type, or handle the error with `catch`.",
        wrong: "fn main() {\n    try app.listen(\":8080\")\n}",
        right: "fn main() ! {\n    try app.listen(\":8080\")\n}",
    },
    ErrorCode {
        code: ASSIGN_TO_IMMUTABLE,
        title: "cannot assign to an immutable variable",
        explanation: "Variables declared with `let` cannot change. Declare the variable with `var` instead.",
        wrong: "let count = 0\ncount += 1",
        right: "var count = 0\ncount += 1",
    },
    ErrorCode {
        code: NO_SUCH_MEMBER,
        title: "no such field, method or variant",
        explanation: "The type has no field, method or variant with this name. Check the spelling against the type's declaration.",
        wrong: "post.tittle",
        right: "post.title",
    },
    ErrorCode {
        code: BAD_ARGUMENTS,
        title: "wrong arguments",
        explanation: "The call does not match the function's parameters: an argument is missing, there are too many, or a named argument does not exist. Named arguments must use the parameter's name.",
        wrong: "web.json(post, code: 201)",
        right: "web.json(post, status: 201)",
    },
    ErrorCode {
        code: NON_EXHAUSTIVE_MATCH,
        title: "`match` does not cover every case",
        explanation: "Every possible value must be handled by some arm. Add arms for the missing cases, or a final `_ => ...` arm.",
        wrong: "match user {\n    some(u) => u.name\n}",
        right: "match user {\n    some(u) => u.name\n    none => \"guest\"\n}",
    },
    ErrorCode {
        code: ROUTE_PARAMS,
        title: "handler parameters do not match the route",
        explanation: "A route handler takes the request first, then one parameter for each `{name}` in the route pattern, in order. `{id: Int}` gives an `Int`; `{name}` gives a `String`.",
        wrong: "app.get(\"/posts/{id: Int}\", fn(req) { ... })",
        right: "app.get(\"/posts/{id: Int}\", fn(req, id) { ... })",
    },
    ErrorCode {
        code: CANNOT_INFER,
        title: "cannot infer a type",
        explanation: "The compiler cannot work out a type from the surrounding code. Write it out: a type argument such as `req.json<NewPost>()`, or a parameter type such as `fn(n: Int)`.",
        wrong: "let input = try req.json()",
        right: "let input = try req.json<NewPost>()",
    },
    ErrorCode {
        code: DUPLICATE_DEFINITION,
        title: "defined more than once",
        explanation: "Two declarations, fields, variants or parameters in the same place share a name. Rename or remove one of them.",
        wrong: "struct Post {\n    title: String\n    title: String\n}",
        right: "struct Post {\n    title: String\n    subtitle: String\n}",
    },
    ErrorCode {
        code: NOT_CALLABLE,
        title: "not a function",
        explanation: "Only functions, methods, closures, struct names and enum variants with fields can be called with `(...)`.",
        wrong: "let n = count()",
        right: "let n = count",
    },
    ErrorCode {
        code: MISSING_VALUE,
        title: "missing return value",
        explanation: "The function declares a return type, but this path ends without producing a value. End the body with an expression of that type, or use `return value`.",
        wrong: "fn double(n: Int) -> Int {\n    let x = n * 2\n}",
        right: "fn double(n: Int) -> Int {\n    n * 2\n}",
    },
    ErrorCode {
        code: UNKNOWN_TYPE,
        title: "unknown type",
        explanation: "No type with this name is declared or imported. Check the spelling; built-in types are capitalized, as in `String` and `Int`.",
        wrong: "fn greet(name: string) {}",
        right: "fn greet(name: String) {}",
    },
    ErrorCode {
        code: NOT_DISPLAYABLE,
        title: "value has no text form",
        explanation: "`{value}` in a glaze shows text, numbers, booleans, dates and durations. Show a field of a struct, loop over a list with `{% for %}`, unwrap an optional first, and use `{% match %}` to choose text for an enum.",
        wrong: "{% props post: Post, author: User? %}\n<h1>{post}</h1>\n<p>{author.name}</p>",
        right: "{% props post: Post, author: User? %}\n<h1>{post.title}</h1>\n{% if author is some(a) %}<p>{a.name}</p>{% end %}",
    },
    ErrorCode {
        code: SERVER_ONLY,
        title: "server-only module used in the browser",
        explanation: "An ember (a glaze with `{% state %}` or an `on:` handler) runs in the browser, which has no database and no secrets. Modules such as `sql` work only on the server. Move the work into a `server fn` in a `.kiln` file and call it from the ember: the call becomes a typed request to the server.",
        wrong: "{% state posts = sql.open(url).query<Post>(\"select * from posts\") %}",
        right: "// main.kiln\nserver fn recent_posts() -> List<Post> ! { ... }\n\n// feed.glaze\n{% state posts = recent_posts() catch { [] } %}",
    },
    ErrorCode {
        code: BROWSER_ONLY,
        title: "browser-only module used outside a handler",
        explanation: "`browser`, `js` and npm packages run in the browser, so only an ember's `on:` handlers can use them: a glaze's HTML is rendered on the server too, and `.kiln` functions run on the server. Call them from a handler, and keep what they give in `{% state %}` to show it.",
        wrong: "<p>{browser.title()}</p>",
        right: "{% state title = \"\" %}\n<button on:click={ title = browser.title() }>Title</button>\n<p>{title}</p>",
    },
    ErrorCode {
        code: NOT_SERIALIZABLE,
        title: "value cannot cross between server and browser",
        explanation: "An ember's props and state, and the parameters and results of a `server fn`, travel as JSON between the server and the browser. Functions, `js.Value` handles, styles and server handles such as `web.Request` have no JSON form, so they cannot be among them. Pass plain data (numbers, strings, lists, maps, structs and enums of them) and do the rest on its side.",
        wrong: "server fn run(task: fn() -> Int) -> Int { task() }",
        right: "server fn run(task: String) -> Int { ... }",
    },
    ErrorCode {
        code: UNKNOWN_GLAZE,
        title: "unknown glaze or component",
        explanation: "Components, layouts and `web.render` refer to `.glaze` files in the same directory. A component tag uses the file name in PascalCase: `<LikeButton>` is `like_button.ember`. A layout must contain `{% slot %}`.",
        wrong: "web.render(\"home.glaze\", { posts })   // the file is index.glaze",
        right: "web.render(\"index.glaze\", { posts })",
    },
    ErrorCode {
        code: UNCLOSED_GLAZE_TAG,
        title: "element or block never closed",
        explanation: "An HTML element needs a matching closing tag, and `{% if %}`, `{% for %}` and `{% match %}` need a matching `{% end %}`. Void elements such as `<br>`, `<img>` and `<input>` need no closing tag.",
        wrong: "<ul>\n  {% for post in posts %}\n    <li>{post.title}</li>\n</ul>",
        right: "<ul>\n  {% for post in posts %}\n    <li>{post.title}</li>\n  {% end %}\n</ul>",
    },
    ErrorCode {
        code: MISMATCHED_CLOSING_TAG,
        title: "closing tag does not match",
        explanation: "A closing tag must close the innermost open element.",
        wrong: "<p>Hello <b>world</p></b>",
        right: "<p>Hello <b>world</b></p>",
    },
    ErrorCode {
        code: UNKNOWN_GLAZE_TAG,
        title: "unknown glaze tag",
        explanation: "Glazes have a small set of `{% %}` tags: `props`, `state`, `layout`, `slot`, `let`, `if`, `else`, `else if`, `for`, `empty`, `match`, `case` and `end`. Every block ends with `{% end %}`. Other template languages' tags such as `endif`, `elif`, `extends` and `include` are not used.",
        wrong: "{% if user %}Hi{% endif %}",
        right: "{% if user is some(u) %}Hi {u.name}{% end %}",
    },
    ErrorCode {
        code: FOREIGN_TEMPLATE_SYNTAX,
        title: "syntax from another template language",
        explanation: "Glazes output a value with single braces, `{value}`, escaped for where it appears. To output trusted HTML without escaping, write `{raw value}`. Double braces, filters such as `| safe`, and PHP or ERB tags are not used.",
        wrong: "<h1>{{ post.title }}</h1>\n<div>{{ post.html | safe }}</div>",
        right: "<h1>{post.title}</h1>\n<div>{raw post.html}</div>",
    },
    ErrorCode {
        code: MISPLACED_DECLARATION,
        title: "declaration in the wrong place",
        explanation: "`{% props %}`, `{% state %}` and `{% layout %}` describe the whole glaze, so they appear at the top level, outside any element or block. `props` and `layout` appear at most once.",
        wrong: "<div>\n  {% props post: Post %}\n</div>",
        right: "{% props post: Post %}\n<div>{post.title}</div>",
    },
    ErrorCode {
        code: FOREIGN_ATTRIBUTE,
        title: "attribute from another framework",
        explanation: "Glazes use plain HTML attribute names, and event handlers are written `on:event={...}`. React names such as `className`, `htmlFor` and `onClick` are not used.",
        wrong: "<button className=\"primary\" onClick={save()}>Save</button>",
        right: "<button class=\"primary\" on:click={save()}>Save</button>",
    },
    ErrorCode {
        code: UNCLOSED_GLAZE_CODE,
        title: "code in a glaze never closed",
        explanation: "A `{` that starts a value needs a matching `}`, and a `{%` tag needs a matching `%}`. To show a literal brace in text, write `{\"{\"}`.",
        wrong: "<h1>{post.title</h1>",
        right: "<h1>{post.title}</h1>",
    },
    ErrorCode {
        code: STRAY_CLOSING_TAG,
        title: "nothing to close",
        explanation: "This closing tag, `{% end %}`, `{% else %}`, `{% empty %}` or `{% case %}` has no matching opening element or block.",
        wrong: "<p>Hi</p></div>",
        right: "<p>Hi</p>",
    },
    ErrorCode {
        code: BAD_STYLE,
        title: "a style that cannot be compiled",
        explanation: "A style, `ui({ ... })` or a glaze's `ui={{ ... }}`, sets keys such as `p`, `bg` and `hover` to values written in place: a design token such as `space.md`, a keyword such as `.bold`, a number, `true`, or a CSS string such as `\"1rem 2px\"`, which must be valid for the key's CSS property. Code may choose between such values with `if` and `match`, but cannot compute them, because every style becomes CSS when the program is compiled. A style cannot set both `flex` and `grid`.",
        wrong: "ui({ p: \"${n}px\", bg: color.accent })",
        right: "ui({ p: if roomy { space.lg } else { space.sm }, bg: color.accent })",
    },
    ErrorCode {
        code: UNKNOWN_TOKEN,
        title: "a design token or breakpoint that is not declared",
        explanation: "Design tokens are the CSS custom properties that the program's global stylesheets (imported with `{% css \"...\" %}`) declare, named `--<category>-<name>`: `--space-md` is `space.md`, and `--color-on-accent` is `color.on_accent`. The categories are color, space, size, radius, shadow, font, text, weight, leading, tracking, motion and breakpoint. Breakpoints, used in `at: { ... }`, are `--breakpoint-<name>` tokens holding a width.",
        wrong: "ui({ bg: color.surfce })",
        right: "ui({ bg: color.surface })   /* :root { --color-surface: #faf9f6; } */",
    },
    ErrorCode {
        code: BAD_TOKEN,
        title: "a design token with a wrong value, or themes that differ",
        explanation: "A token's value must suit its category: a colour for `--color-*`, a length for `--space-*`, and so on; a breakpoint must be a width without `var()`, the same in every theme. A theme is a rule `.theme-<name>` in a global stylesheet that sets tokens to its own values, and every theme must set the same tokens, so that a page looks complete whichever theme it uses.",
        wrong: ".theme-dark { --color-text: white; }\n.theme-light { --color-text: black; --color-accent: red; }",
        right: ".theme-dark { --color-text: white; --color-accent: orange; }\n.theme-light { --color-text: black; --color-accent: red; }",
    },
    ErrorCode {
        code: UNSAFE_ATTRIBUTE,
        title: "a value where HTML escaping is not enough",
        explanation: "Escaping keeps a value from ending an attribute, but in some attributes the browser reads the text as code: JavaScript in `onclick` and the other `on...` attributes, CSS in `style`, and a whole HTML page in `srcdoc`. A value there could run code or change the page, so glazes accept only numbers in `style` and no values in the others. Use classes and a stylesheet to style, and event handlers (`on:click={...}`) in embers.",
        wrong: "<p style=\"color: {user.colour}\">Hi</p>",
        right: "<p class=\"{user.theme}\">Hi</p>",
    },
    ErrorCode {
        code: ENDLESS_GLAZE,
        title: "a glaze that renders itself without end",
        explanation: "A glaze always renders its layout, and the components it uses outside `{% if %}`, `{% for %}` and `{% match %}` blocks. When following those leads back to the same glaze, rendering it would never finish. A component may use itself inside a block that ends, such as a loop over a tree's children.",
        wrong: "{# base.glaze #}\n{% layout \"base.glaze\" %}\n<main>{% slot %}</main>",
        right: "{# base.glaze #}\n<main>{% slot %}</main>",
    },
    ErrorCode {
        code: MISPLACED_FILL,
        title: "`{% fill %}` in the wrong place",
        explanation: "`{% fill name %} ... {% end %}` gives the content for a layout's or component's `{% slot name %}`. It goes directly inside a component tag, or at the top level of a page that has a `{% layout %}`, and names a slot that glaze has, once.",
        wrong: "{% layout \"base.glaze\" title: \"Home\" %}\n<div>{% fill sidebar %}<p>Links</p>{% end %}</div>",
        right: "{% layout \"base.glaze\" title: \"Home\" %}\n{% fill sidebar %}<p>Links</p>{% end %}\n<div>Content</div>",
    },
    ErrorCode {
        code: BAD_CSS,
        title: "CSS that cannot be used",
        explanation: "A glaze's `<style>`, or a stylesheet it imports with `{% css %}`, is read as CSS when the program is checked. The file must exist inside the program's folder, and the CSS must parse. Files that CSS refers to with `url(...)` or `@import` must exist too, inside the folder. A CSS Module or `<style>` cannot `@import`: import the stylesheet with `{% css %}` instead.",
        wrong: "{% css \"../shared/site.css\" %}",
        right: "{% css \"site.css\" %}",
    },
    ErrorCode {
        code: UNKNOWN_MODIFIER,
        title: "unknown event modifier",
        explanation: "An `on:` handler's event can be followed by modifiers: `.prevent` stops the browser's own action (a form's submit always does), `.stop` keeps the event from reaching outer handlers, and `.latest` lets a new event replace a call that is still waiting instead of being ignored. Other names are not modifiers.",
        wrong: "<form on:submit.preventDefault={ save() }>",
        right: "<form on:submit.prevent={ save() }>",
    },
    ErrorCode {
        code: NOT_COMPILED_YET,
        title: "not supported by the compiler yet",
        explanation: "The program is valid Kiln, but the compiler cannot build this part of it yet. `kiln check` accepts it; `kiln build` and `kiln run` do not. Forms (`req.form`), embers, `sql` and `auth` arrive in later phases. Map keys and set items can be `Int`, `Float`, `Bool`, `Duration` or `String` for now. Printing or comparing a value whose type is a generic parameter also needs a later phase.",
        wrong: "import sql\n\nfn main() ! {\n    let db = try sql.open(\"app.db\")\n}",
        right: "import fs\n\nfn main() ! {\n    let text = try fs.read(\"app.txt\")\n}",
    },
    ErrorCode {
        code: NO_MAIN,
        title: "no `main` function",
        explanation: "A program that is built or run starts at `fn main()`, which takes no parameters. Files without `main` can still be checked with `kiln check`.",
        wrong: "fn greet() {\n    print(\"hi\")\n}",
        right: "fn main() {\n    print(\"hi\")\n}",
    },
    ErrorCode {
        code: SERVER_CALL_PLACE,
        title: "a `server fn` called where the browser cannot wait for it",
        explanation: "A `server fn` runs on the server. Code in an ember calls it with a request, so the call must be a statement of its own in an `on:` handler, where the handler can wait for the answer: `x = f(...)`, `let x = f(...)` or `f(...)`, optionally with `catch`. Elsewhere in browser code, such as in the ember's markup or inside an expression, there is nothing to wait with.",
        wrong: "<p>{like_count(post.id)}</p>",
        right: "{% state count = 0 %}\n<button on:click={ count = like_count(post.id) catch { count } }>{count}</button>",
    },
];

pub fn lookup(code: &str) -> Option<&'static ErrorCode> {
    let code = code.to_ascii_uppercase();
    ALL.iter().find(|c| c.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for c in ALL {
            assert!(seen.insert(c.code), "duplicate code {}", c.code);
            assert!(c.code.len() == 5 && c.code.starts_with('E'));
        }
    }

    #[test]
    fn lookup_ignores_case() {
        assert_eq!(lookup("e0106").unwrap().code, NO_NULL);
    }
}
