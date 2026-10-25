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

pub const NOT_COMPILED_YET: &str = "E0401";
pub const NO_MAIN: &str = "E0402";

pub static ALL: &[ErrorCode] = &[
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
        explanation: "A string literal was opened with `\"` but never closed on the same line. Strings cannot span lines; use `\\n` for a line break.",
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
        code: UNKNOWN_GLAZE,
        title: "unknown glaze or component",
        explanation: "Components, layouts and `web.render` refer to `.glaze` files in the same directory. A component tag uses the file name in PascalCase: `<LikeButton>` is `like_button.glaze`. A layout must contain `{% slot %}`.",
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
