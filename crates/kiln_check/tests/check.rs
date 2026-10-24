use kiln_diagnostics::{codes, Diagnostic};

fn check(src: &str) -> Vec<Diagnostic> {
    let result = kiln_syntax::parse(src);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected parse errors: {:#?}",
        result.diagnostics
    );
    kiln_check::check(&result.module)
}

fn error_codes(src: &str) -> Vec<&'static str> {
    check(src).iter().map(|d| d.code).collect()
}

#[track_caller]
fn ok(src: &str) {
    let diags = check(src);
    assert!(diags.is_empty(), "unexpected diagnostics: {diags:#?}");
}

/// Applies a diagnostic's suggested fix to `src`.
fn apply_fix(src: &str, diag: &Diagnostic) -> String {
    let fix = diag.suggestion.as_ref().expect("a suggested fix");
    let mut out = src.to_string();
    out.replace_range(
        fix.span.start as usize..fix.span.end as usize,
        &fix.replacement,
    );
    out
}

#[test]
fn inference_and_generics() {
    ok(r#"
fn main() {
    let names = ["Ada", "Grace"]
    let lengths = names.map(fn(n) { n.len() })
    let total: Int = lengths.len() + 1
    let first: String? = names.find(fn(n) { n.starts_with("A") })
    let shown = first ?? "nobody"
    print("${shown} ${total}")
}
"#);
}

#[test]
fn undefined_names_suggest_a_fix() {
    let src = "fn main() {\n    let title = \"x\"\n    print(tittle)\n}\n";
    let diags = check(src);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].code, codes::UNDEFINED_NAME);
    ok(&apply_fix(src, &diags[0]));
}

#[test]
fn missing_import_suggests_the_import() {
    let src = "fn main() {\n    log.info(\"hi\")\n}\n";
    let diags = check(src);
    assert_eq!(error_codes(src), [codes::UNDEFINED_NAME]);
    let fixed = apply_fix(src, &diags[0]);
    assert!(fixed.starts_with("import log\n"), "{fixed}");
    ok(&fixed);
}

#[test]
fn type_mismatch() {
    assert_eq!(
        error_codes("fn main() {\n    let n: Int = \"seven\"\n}\n"),
        [codes::TYPE_MISMATCH]
    );
    assert_eq!(
        error_codes("fn f() -> String {\n    1 + 2\n}\n"),
        [codes::TYPE_MISMATCH]
    );
    assert_eq!(
        error_codes("fn main() {\n    let xs = [1, \"two\"]\n}\n"),
        [codes::TYPE_MISMATCH]
    );
}

#[test]
fn optionals_must_be_unwrapped() {
    assert_eq!(
        error_codes("fn f(name: String?) -> String {\n    name\n}\n"),
        [codes::TYPE_MISMATCH]
    );
    ok("fn f(name: String?) -> String {\n    name ?? \"anonymous\"\n}\n");
    ok("fn f(name: String) -> String? {\n    name\n}\n");
}

#[test]
fn calls_that_can_fail_need_try_or_catch() {
    let src = "import sql\n\nfn main() ! {\n    let db = sql.open(\"x\")\n}\n";
    let diags = check(src);
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].code, codes::MISSING_TRY);
    ok(&apply_fix(src, &diags[0]));
}

#[test]
fn try_on_something_that_cannot_fail() {
    let src = "fn one() -> Int {\n    1\n}\n\nfn main() {\n    let n = try one()\n}\n";
    let diags = check(src);
    assert_eq!(error_codes(src), [codes::NOTHING_TO_TRY]);
    ok(&apply_fix(src, &diags[0]));
}

#[test]
fn propagating_from_a_function_that_cannot_fail() {
    let src = "import sql\n\nfn main() {\n    let db = try sql.open(\"x\")\n}\n";
    let diags = check(src);
    assert_eq!(error_codes(src), [codes::CANNOT_FAIL]);
    ok(&apply_fix(src, &diags[0]));
}

#[test]
fn error_types_must_match() {
    let src = r#"
import sql

enum AppError {
    Broken
}

fn open() -> sql.Db ! AppError {
    try sql.open("x")
}
"#;
    assert_eq!(error_codes(src), [codes::ERROR_TYPE_MISMATCH]);
    ok(r#"
import sql

enum AppError {
    Broken(reason: String)
}

fn open() -> sql.Db ! AppError {
    sql.open("x") catch err {
        throw AppError.Broken(err.message)
    }
}
"#);
}

#[test]
fn closures_infer_whether_they_fail() {
    ok(r#"
import web, sql

fn main() ! {
    let app = web.app()
    app.get("/", fn(req) {
        let db = try sql.open("x")
        web.json(try db.query<Int>("select 1"))
    })
    try app.listen(":8080")
}
"#);
}

#[test]
fn assigning_to_a_let_suggests_var() {
    let src = "fn main() {\n    let n = 1\n    n += 1\n}\n";
    let diags = check(src);
    assert_eq!(error_codes(src), [codes::ASSIGN_TO_IMMUTABLE]);
    ok(&apply_fix(src, &diags[0]));
}

#[test]
fn unknown_fields_and_methods() {
    let src = r#"
struct Post {
    title: String
}

fn f(p: Post) -> String {
    p.tittle
}
"#;
    let diags = check(src);
    assert_eq!(error_codes(src), [codes::NO_SUCH_MEMBER]);
    ok(&apply_fix(src, &diags[0]));
    assert_eq!(
        error_codes("fn main() {\n    \"x\".shout()\n}\n"),
        [codes::NO_SUCH_MEMBER]
    );
}

#[test]
fn argument_counts_and_names() {
    let src = "fn add(a: Int, b: Int) -> Int {\n    a + b\n}\n";
    assert_eq!(
        error_codes(&format!("{src}\nfn main() {{\n    add(1)\n}}\n")),
        [codes::BAD_ARGUMENTS]
    );
    assert_eq!(
        error_codes(&format!("{src}\nfn main() {{\n    add(1, 2, 3)\n}}\n")),
        [codes::BAD_ARGUMENTS]
    );
    assert_eq!(
        error_codes(&format!("{src}\nfn main() {{\n    add(1, c: 2)\n}}\n")),
        [codes::BAD_ARGUMENTS]
    );
    ok(&format!("{src}\nfn main() {{\n    add(b: 1, a: 2)\n}}\n"));
}

#[test]
fn struct_construction() {
    let src = "struct Post {\n    title: String\n    published: Bool = false\n}\n";
    ok(&format!(
        "{src}\nfn main() {{\n    let p = Post(title: \"Hi\")\n}}\n"
    ));
    assert_eq!(
        error_codes(&format!("{src}\nfn main() {{\n    let p = Post()\n}}\n")),
        [codes::BAD_ARGUMENTS]
    );
}

#[test]
fn match_must_be_exhaustive() {
    let src = r#"
enum Role {
    Admin
    Editor
    Viewer
}

fn label(r: Role) -> String {
    match r {
        Admin => "admin"
        Editor => "editor"
    }
}
"#;
    let diags = check(src);
    assert_eq!(error_codes(src), [codes::NON_EXHAUSTIVE_MATCH]);
    assert!(diags[0].message.contains("Viewer"), "{}", diags[0].message);
    assert_eq!(
        error_codes("fn f(x: Int?) -> Int {\n    match x {\n        some(n) => n\n    }\n}\n"),
        [codes::NON_EXHAUSTIVE_MATCH]
    );
    assert_eq!(
        error_codes("fn f(b: Bool) -> Int {\n    match b {\n        true => 1\n    }\n}\n"),
        [codes::NON_EXHAUSTIVE_MATCH]
    );
    ok("fn f(n: Int) -> Int {\n    match n {\n        0 => 1\n        _ => n\n    }\n}\n");
}

#[test]
fn match_arms_must_agree() {
    assert_eq!(
        error_codes("fn f(b: Bool) -> Int {\n    match b {\n        true => 1\n        false => \"no\"\n    }\n}\n"),
        [codes::TYPE_MISMATCH]
    );
}

#[test]
fn route_parameters_are_typed() {
    ok(r#"
import web

fn main() ! {
    let app = web.app()
    app.get("/posts/{id: Int}", fn(req, id) {
        let next: Int = id + 1
        web.text("${next}")
    })
    app.get("/hello/{name}", fn(req, name) { web.text(name.upper()) })
    try app.listen(":8080")
}
"#);
    let src = r#"
import web

fn main() {
    let app = web.app()
    app.get("/posts/{id: Int}", fn(req) { web.text("x") })
}
"#;
    assert_eq!(error_codes(src), [codes::ROUTE_PARAMS]);
    let src = r#"
import web

fn main() {
    let app = web.app()
    app.get("/posts/{id: Int}", fn(req, id) { web.text(id) })
}
"#;
    assert_eq!(error_codes(src), [codes::TYPE_MISMATCH]);
}

#[test]
fn generic_calls_need_a_type() {
    assert_eq!(
        error_codes("import json\n\nfn main() ! {\n    let v = try json.decode(\"1\")\n}\n"),
        [codes::CANNOT_INFER]
    );
    ok("import json\n\nfn main() ! {\n    let v = try json.decode<Int>(\"1\")\n}\n");
    ok("import json\n\nfn main() ! {\n    let v: Int = try json.decode(\"1\")\n}\n");
}

#[test]
fn duplicates() {
    assert_eq!(
        error_codes("fn a() {\n}\n\nfn a() {\n}\n"),
        [codes::DUPLICATE_DEFINITION]
    );
    assert_eq!(
        error_codes("struct P {\n    x: Int\n    x: Int\n}\n"),
        [codes::DUPLICATE_DEFINITION]
    );
}

#[test]
fn calling_a_non_function() {
    assert_eq!(
        error_codes("fn main() {\n    let n = 1\n    n()\n}\n"),
        [codes::NOT_CALLABLE]
    );
}

#[test]
fn missing_return_value() {
    assert_eq!(
        error_codes("fn f(b: Bool) -> Int {\n    if b {\n        return 1\n    }\n}\n"),
        [codes::MISSING_VALUE]
    );
}

#[test]
fn unknown_types_suggest_a_fix() {
    let src = "fn f(n: Integer) {\n}\n";
    let diags = check(src);
    assert_eq!(error_codes(src), [codes::UNKNOWN_TYPE]);
    assert!(apply_fix(src, &diags[0]).contains("n: Int"));
    assert_eq!(error_codes("fn f(xs: List) {\n}\n"), [codes::TYPE_MISMATCH]);
}

#[test]
fn unknown_module() {
    assert_eq!(error_codes("import nope\n"), [codes::UNDEFINED_NAME]);
}

#[test]
fn one_mistake_one_error() {
    // An undefined name should not also produce errors where its value is used.
    let src = "fn main() {\n    let x = missing.thing(1)\n    let y: Int = x + 1\n    print(try x.go())\n}\n";
    assert_eq!(error_codes(src), [codes::UNDEFINED_NAME]);
}

#[test]
fn methods_without_self_are_called_on_the_type() {
    let src = r#"
struct Post {
    title: String
}

impl Post {
    fn untitled() -> Post {
        Post(title: "Untitled")
    }

    fn shout(self) -> String {
        self.title.upper()
    }
}

fn main() {
    let p = Post.untitled()
    print(p.shout())
}
"#;
    ok(src);
    let bad = src.replace("Post.untitled()", "Post(title: \"x\").untitled()");
    let diags = check(&bad);
    assert_eq!(error_codes(&bad), [codes::NO_SUCH_MEMBER]);
    ok(&apply_fix(&bad, &diags[0]));
}
