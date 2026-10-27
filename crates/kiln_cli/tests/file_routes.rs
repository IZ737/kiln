//! `app.file_routes`: the rules of filesystem routes, through `kiln check`
//! (conformance/web_file_routes runs a mounted app, and `fire.rs` exports
//! one).
use serde_json::Value;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

const MAIN: &str = "import web\n\nfn main() ! {\n    let app = web.app()\n    app.file_routes(\"pages\")\n    try app.listen(\":8080\")\n}\n";

struct App(PathBuf);

impl App {
    /// An app whose `main.kiln` mounts `pages/`, with these files.
    fn new(files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "kiln-file-routes-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let app = Self(root);
        if !files.iter().any(|(n, _)| *n == "main.kiln") {
            app.write("main.kiln", MAIN);
        }
        for (name, text) in files {
            app.write(name, text);
        }
        app
    }
    fn write(&self, name: &str, text: &str) {
        let file = self.0.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }
    /// `kiln check --json`: whether it passed, and its diagnostics.
    fn check(&self) -> (bool, Vec<Value>) {
        let out = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["check", "--json"])
            .arg(&self.0)
            .output()
            .unwrap();
        let json: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
            panic!(
                "{}\n{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            )
        });
        (
            out.status.success(),
            json["diagnostics"].as_array().cloned().unwrap_or_default(),
        )
    }
    fn succeeds(&self) -> Vec<Value> {
        let (ok, diagnostics) = self.check();
        assert!(ok, "{diagnostics:#?}");
        diagnostics
    }
    /// Fails with an error of `code` whose message has `message`, in a
    /// file whose path ends with `file`.
    fn fails(&self, code: &str, file: &str, message: &str) {
        let (ok, diagnostics) = self.check();
        assert!(!ok, "expected {code} `{message}`, but it passed");
        assert!(
            diagnostics.iter().any(|d| d["code"] == code
                && d["severity"] == "error"
                && d["message"].as_str().unwrap().contains(message)
                && d["file"]
                    .as_str()
                    .unwrap()
                    .replace('\\', "/")
                    .ends_with(file)),
            "expected {code} in {file}: `{message}`, found {diagnostics:#?}"
        );
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn main_with(call: &str) -> String {
    MAIN.replace("app.file_routes(\"pages\")", call)
}

#[test]
fn a_mount_is_written_out() {
    let page = ("pages/index.glaze", "<h1>Home</h1>\n");
    for (call, message) in [
        (
            "let dir = \"pages\"\n    app.file_routes(dir)",
            "string literal",
        ),
        ("app.file_routes(\"../pages\")", "without `.` or `..`"),
        ("app.file_routes(\".\")", "below the project root"),
        ("app.file_routes(\"missing\")", "no folder `missing/`"),
        (
            "app.file_routes(\"pages\", at: \"admin\")",
            "starting with `/`",
        ),
        (
            "app.file_routes(\"pages\", at: \"/a/{b}\")",
            "without a query",
        ),
        (
            "app.file_routes(\"pages\", ignore: [\"**/_components\"])",
            "not paths",
        ),
        (
            "app.file_routes(\"pages\", ignore: [\"part*\"])",
            "not patterns",
        ),
        (
            "app.file_routes(\"pages\", ignore: [\"\"])",
            "folder's name",
        ),
        (
            "app.file_routes(\"pages\", prefix: \"/x\")",
            "takes a folder",
        ),
    ] {
        let main = main_with(call);
        App::new(&[("main.kiln", &main), page]).fails("E0238", "main.kiln", message);
    }
}

#[test]
fn file_names_that_are_not_routes() {
    for (file, message) in [
        ("pages/[...path].glaze", "catch-all"),
        ("pages/docs/[...path]/index.glaze", "catch-all"),
        ("pages/[[page]].glaze", "optional segment"),
        ("pages/(marketing)/about.glaze", "route group"),
        ("pages/post-[slug].glaze", "whole segment"),
        ("pages/[Slug].glaze", "lowercase Kiln name"),
        ("pages/[].glaze", "lowercase Kiln name"),
        ("pages/[req].glaze", "cannot be a URL parameter"),
        ("pages/a b.glaze", "cannot be part of a URL"),
    ] {
        App::new(&[(file, "<p>x</p>\n")]).fails("E0238", file.rsplit('/').next().unwrap(), message);
    }
    App::new(&[("pages/[a]/[a].glaze", "<p>x</p>\n")]).fails(
        "E0238",
        "[a].glaze",
        "two parameters named `a`",
    );
}

#[test]
fn components_hidden_files_and_embers_are_not_routes() {
    let app = App::new(&[
        (
            "pages/index.glaze",
            "<pages._components.Nav />\n<pages.blog._components.Card />\n",
        ),
        ("pages/_components/nav.glaze", "<nav></nav>\n"),
        (
            "pages/blog/_components/card.glaze",
            "{% props title: String %}\n<p>{title}</p>\n",
        ),
        ("pages/.drafts/[x y].glaze", "<p>draft</p>\n"),
        (
            "pages/counter.ember",
            "{% state n = 0 %}\n<button on:click={ n += 1 }>{n}</button>\n",
        ),
    ]);
    // `card.glaze` needs a prop, which is fine for a component.
    app.fails("E0209", "index.glaze", "title");
    app.write(
        "pages/index.glaze",
        "<pages._components.Nav />\n<pages.blog._components.Card title=\"t\" />\n",
    );
    app.succeeds();
    // With `ignore:` the list is replaced: `_components` holds pages.
    app.write(
        "main.kiln",
        &main_with("app.file_routes(\"pages\", ignore: [\"partials\"])"),
    );
    app.fails("E0240", "card.glaze", "needs the prop `title`");
}

#[test]
fn routes_of_one_mount_do_not_collide() {
    App::new(&[
        ("pages/about.glaze", "<p>a</p>\n"),
        ("pages/about/index.glaze", "<p>b</p>\n"),
    ])
    .fails("E0239", "index.glaze", "defined twice");
    App::new(&[
        ("pages/blog/[id].glaze", "<p>a</p>\n"),
        ("pages/blog/[slug].glaze", "<p>b</p>\n"),
    ])
    .fails(
        "E0239",
        "[slug].glaze",
        "match the same URLs, such as /blog/x",
    );
    App::new(&[
        ("pages/[section]/new.glaze", "<p>a</p>\n"),
        ("pages/docs/[slug].glaze", "<p>b</p>\n"),
    ])
    .fails("E0239", ".glaze", "both match /docs/new");
    // A literal page is a case of the parameter one and wins; typed
    // parameters that cannot meet do not collide.
    App::new(&[
        ("pages/docs/new.glaze", "<p>new</p>\n"),
        ("pages/docs/[slug].glaze", "<p>doc</p>\n"),
        ("pages/n/[id].glaze", "{% props id: Int %}\n<p>{id}</p>\n"),
        ("pages/n/[on].glaze", "{% props on: Bool %}\n<p>{on}</p>\n"),
    ])
    .succeeds();
    // Different methods share a URL.
    App::new(&[
        ("pages/form/index.glaze", "<form method=\"post\"></form>\n"),
        (
            "pages/form/route.kiln",
            "import web\n\npub fn post(req: web.Request) -> web.Response {\n    return web.redirect(\"/form\")\n}\n",
        ),
    ])
    .succeeds();
}

#[test]
fn kilns_own_urls_are_not_routes() {
    App::new(&[("pages/_kiln/app.glaze", "<p>x</p>\n")]).fails(
        "E0239",
        "app.glaze",
        "under /_kiln",
    );
    let main = main_with("app.file_routes(\"pages\", at: \"/_kiln\")");
    App::new(&[("main.kiln", &main), ("pages/index.glaze", "<p>x</p>\n")]).fails(
        "E0239",
        "index.glaze",
        "under /_kiln",
    );
    let post = "import web\n\npub fn post() -> web.Response {\n    return web.text(\"x\")\n}\n";
    App::new(&[("pages/[a]/rpc/[b]/route.kiln", post)]).fails(
        "E0239",
        "route.kiln",
        "kiln's own route POST /_kiln/rpc/{name}",
    );
    // A GET there, or a POST that cannot match, is fine.
    let get = post.replace("pub fn post", "pub fn get");
    App::new(&[
        ("pages/[a]/rpc/[b]/route.kiln", &get),
        ("pages/[a]/rpcs/[b]/route.kiln", post),
    ])
    .succeeds();
}

#[test]
fn pages_get_their_props_from_the_url_and_load() {
    App::new(&[(
        "pages/[slug].glaze",
        "{% props slug: String, title: String %}\n<p>{title}</p>\n",
    )])
    .fails("E0240", "[slug].glaze", "needs the prop `title`");
    App::new(&[(
        "pages/[slug].glaze",
        "{% props slug: List<Int> %}\n<p>x</p>\n",
    )])
    .fails("E0240", "[slug].glaze", "comes from the URL");
    App::new(&[("pages/[n].glaze", "{% props n: Int? %}\n<p>x</p>\n")]).fails(
        "E0240",
        "[n].glaze",
        "not optional",
    );
    let page = (
        "pages/p/[id]/index.glaze",
        "{% props id: Int, title: String %}\n<h1>{id} {title}</h1>\n",
    );
    let route = "pages/p/[id]/route.kiln";
    App::new(&[
        page,
        (
            route,
            "pub fn load(id: Int) -> { title: String } {\n    return { title: \"t\" }\n}\n",
        ),
    ])
    .succeeds();
    for (load, message) in [
        ("pub fn load() -> { title: String, extra: Int } {\n    return { title: \"t\", extra: 1 }\n}\n", "no prop `extra`"),
        ("pub fn load() -> { title: String, id: Int } {\n    return { title: \"t\", id: 1 }\n}\n", "comes from the URL"),
        ("pub fn load() -> { title: String }? {\n    return none\n}\n", "not `none`"),
        ("import web\n\npub fn load() -> web.Response {\n    return web.text(\"x\")\n}\n", "exports `pub fn get`"),
        ("pub fn load(id: String) -> { title: String } {\n    return { title: id }\n}\n", "is `String` here but `Int`"),
        ("pub fn load(slug: String) -> { title: String } {\n    return { title: slug }\n}\n", "not a parameter of /p/{id}"),
        ("pub fn load() -> { other: String } {\n    return { other: \"t\" }\n}\n", "no prop `other`"),
        ("pub fn load() -> Int {\n    return 1\n}\n", "returns the page's props as a record"),
    ] {
        App::new(&[page, (route, load)]).fails("E0240", "route.kiln", message);
    }
    // A struct of the folder or of an imported module: its fields are props.
    App::new(&[
        page,
        (route, "import shared\n\npub fn load() -> shared.Data {\n    return shared.Data(title: \"t\")\n}\n"),
        ("shared/data.kiln", "pub struct Data {\n    title: String\n}\n"),
    ])
    .succeeds();
    App::new(&[
        page,
        (route, "pub struct Data {\n    title: String\n    more: Int\n}\n\npub fn load() -> Data {\n    return Data(title: \"t\", more: 1)\n}\n"),
    ])
    .fails("E0240", "route.kiln", "no prop `more`");
    // A field of the wrong type is an error at `load`, not in kiln's code.
    App::new(&[
        page,
        (
            route,
            "pub fn load() -> { title: Int } {\n    return { title: 1 }\n}\n",
        ),
    ])
    .fails("E0202", "route.kiln", "");
}

#[test]
fn route_files_follow_the_rules() {
    let page = (
        "pages/p/[id]/index.glaze",
        "{% props id: Int %}\n<h1>{id}</h1>\n",
    );
    let route = "pages/p/[id]/route.kiln";
    let web = "import web\n\n";
    for (code, message) in [
        ("pub fn load() -> { } {\n    return { }\n}\n\npub fn get() -> web.Response {\n    return web.text(\"x\")\n}\n", "both answer GET"),
        ("pub fn post(id: Int, req: web.Request) -> web.Response {\n    return web.text(\"x\")\n}\n", "the request comes first"),
        ("pub fn post(id: Int = 1) -> web.Response {\n    return web.text(\"x\")\n}\n", "has no default"),
        ("pub fn post(id: Int) -> web.Response {\n    return web.text(\"x\")\n}\n\npub const dynamic = true\n\npub fn static_params() -> List<Int> {\n    return [1]\n}\n", "keep one"),
        ("pub const static_refresh = 10m\n", "needs `pub fn static_params()`"),
        ("pub const static_refresh = 0s\n\npub fn static_params() -> List<Int> {\n    return [1]\n}\n", "more than 0"),
        ("pub const dynamic = 1\n", "`true` or `false`"),
        ("pub fn static_params(x: Int) -> List<Int> {\n    return [x]\n}\n", "takes nothing"),
    ] {
        App::new(&[page, (route, &format!("{web}{code}"))]).fails("E0240", "route.kiln", message);
    }
    App::new(&[(
        "pages/about/route.kiln",
        "pub fn static_params() -> List<Int> {\n    return [1]\n}\n",
    )])
    .fails("E0240", "route.kiln", "no parameters");
    App::new(&[(
        "pages/api/route.kiln",
        "pub fn load() -> { a: Int } {\n    return { a: 1 }\n}\n",
    )])
    .fails("E0240", "route.kiln", "has no `index.glaze`");
    App::new(&[(
        "pages/api/route.kiln",
        "fn helper() -> Int {\n    return 1\n}\n",
    )])
    .fails("E0240", "route.kiln", "answers nothing");
    // The page's static pages, from route.kiln alone.
    App::new(&[
        page,
        (route, "pub fn static_params() -> List<Int> ! {\n    return [1, 2]\n}\n\npub const static_refresh = 5m\n"),
    ])
    .succeeds();
    // A wrong enumeration type is an error at `static_params`.
    App::new(&[
        page,
        (
            route,
            "pub fn static_params() -> List<String> {\n    return [\"a\"]\n}\n",
        ),
    ])
    .fails("E0202", "route.kiln", "");
    // A private function with a route's name is not part of it.
    let warnings = App::new(&[page, (route, "fn load() -> { } {\n    return { }\n}\n")]).succeeds();
    assert!(
        warnings.iter().any(|d| d["severity"] == "warning"
            && d["message"].as_str().unwrap().contains("`load` is private")),
        "{warnings:#?}"
    );
}

#[test]
fn shadowed_routes_are_warnings() {
    let main = main_with("app.get(\"/about\", fn(req) { web.text(\"x\") })\n    app.file_routes(\"pages\")\n    app.file_routes(\"more\")");
    let diagnostics = App::new(&[
        ("main.kiln", &main),
        ("pages/about.glaze", "<p>a</p>\n"),
        ("pages/[slug].glaze", "<p>s</p>\n"),
        ("more/contact.glaze", "<p>c</p>\n"),
    ])
    .succeeds();
    let messages: Vec<&str> = diagnostics
        .iter()
        .filter(|d| d["code"] == "E0241" && d["severity"] == "warning")
        .map(|d| d["message"].as_str().unwrap())
        .collect();
    assert!(
        messages.iter().any(|m| m.contains(
            "GET /about from pages/about.glaze is answered first by `app.get(\"/about\", ...)`"
        )),
        "{messages:#?}"
    );
    assert!(
        messages.iter().any(|m| m.contains(
            "GET /contact from more/contact.glaze is answered first by pages/[slug].glaze"
        )),
        "{messages:#?}"
    );
}
