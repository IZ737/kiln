//! `kiln fire`: release builds, and static sites with `--static`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn kiln() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kiln"))
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-fire-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

fn fire_static(app: &Path, out: &Path) -> Output {
    kiln()
        .args(["fire", "--static", "-o"])
        .arg(out)
        .arg(app)
        .output()
        .unwrap()
}

#[test]
fn a_site_is_written_as_files() {
    let out = temp("blog");
    let fired = fire_static(&root().join("examples/static_blog"), &out);
    assert!(fired.status.success(), "{}", stderr(&fired));
    let read = |file: &str| {
        std::fs::read_to_string(out.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"))
    };
    assert!(read("index.html").contains("Notes from the kiln"));
    assert!(read("about/index.html").contains("<h1>About</h1>"));
    // The pages `.static` lists, by their parameter.
    assert!(read("posts/first-firing/index.html").contains("The first firing"));
    assert!(read("posts/glazes-and-embers/index.html").contains("Glazes &amp; embers"));
    assert!(read("feed.xml").starts_with("<?xml"));
    assert!(read("about-us/index.html").contains("url=/about"));
    assert!(read("404.html").contains("This page is not here"));
    assert!(out.join("static/kiln.svg").is_file());
    let assets: Vec<String> = std::fs::read_dir(out.join("_kiln"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(assets.iter().any(|a| a.ends_with(".wasm")), "{assets:?}");
    assert!(assets.iter().any(|a| a.ends_with(".css")), "{assets:?}");

    // kiln replaces a site it wrote, but not a folder it did not.
    let again = fire_static(&root().join("examples/static_blog"), &out);
    assert!(again.status.success(), "{}", stderr(&again));
    let other = temp("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("keep.txt"), "mine").unwrap();
    let refused = fire_static(&root().join("examples/static_blog"), &other);
    assert!(!refused.status.success());
    assert!(
        stderr(&refused).contains("kiln did not write it"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(
        std::fs::read_to_string(other.join("keep.txt")).unwrap(),
        "mine"
    );
    let _ = std::fs::remove_dir_all(&out);
    let _ = std::fs::remove_dir_all(&other);
    let _ = std::fs::remove_file(root().join("examples/static_blog/.kiln/fire-static.json"));
    let _ = std::fs::remove_dir(root().join("examples/static_blog/.kiln"));
}

/// Each thing a static site cannot have is an error at its source, and
/// links to a route already reported are not reported again.
#[test]
fn what_a_static_site_cannot_have_is_reported() {
    let app = temp("bad-app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("main.kiln"),
        r#"import web

server fn like(n: Int) -> Int {
    n + 1
}

fn main() ! {
    let app = web.app()
    app.get("/", fn(req) { web.render("home.glaze") })
    app.get("/tags/{tag}", fn(req, tag) { web.html("<p>${tag}</p>") })
    app.get("/search", fn(req) { web.text(req.query("q") ?? "") })
    app.get("/data", fn(req) { web.json([1, 2]) })
    app.get("/broken", fn(req) { throw web.error(500, "no") })
    app.get("/hidden", fn(req) { web.html("<p>hidden</p>") }).dynamic()
    app.post("/send", fn(req) { web.text("sent") })
    try app.listen("127.0.0.1:8080")
}
"#,
    )
    .unwrap();
    std::fs::write(
        app.join("home.glaze"),
        r#"<!doctype html>
<html><head><title>Home</title></head><body>
<a href="/tags/kiln">a tag</a>
<a href="/hidden">hidden</a>
<a href="https://example.com/">elsewhere</a>
<form method="post" action="/send"><button>Send</button></form>
<Liker />
</body></html>
"#,
    )
    .unwrap();
    std::fs::write(
        app.join("liker.ember"),
        "{% state n = 0 %}\n<button on:click={ n = like(n) catch err { n } }>{n}</button>\n",
    )
    .unwrap();
    let out = temp("bad-out");
    let fired = fire_static(&app, &out);
    let text = stderr(&fired);
    assert!(!fired.status.success(), "{text}");
    for expected in [
        "error[E0237]: the route /tags/{tag} has parameters",
        "error[E0404]: the page /search reads the query string",
        "error[E0404]: /data answers `application/json",
        "error[E0404]: /broken answered 500",
        "error[E0406]: a link points to /hidden, which the static site does not have",
        "error[E0406]: a form sends a POST to /send",
        "error[E0405]: this ember calls the server function `like`",
        "liker.ember:2:",
        "main.kiln:10:",
    ] {
        assert!(text.contains(expected), "no {expected:?} in:\n{text}");
    }
    // /tags/kiln is a page of the route reported above.
    assert!(!text.contains("points to /tags/kiln"), "{text}");
    assert!(!text.contains("example.com"), "{text}");
    let _ = std::fs::remove_dir_all(&app);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn a_program_without_a_server_has_no_site() {
    let app = temp("no-server");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(app.join("main.kiln"), "fn main() {\n    print(\"hi\")\n}\n").unwrap();
    let out = temp("no-server-out");
    let fired = fire_static(&app, &out);
    assert!(!fired.status.success());
    assert!(
        stderr(&fired).contains("without calling `app.listen`"),
        "{}",
        stderr(&fired)
    );
    let _ = std::fs::remove_dir_all(&app);
    let _ = std::fs::remove_dir_all(&out);
}

#[test]
fn fire_without_static_builds_an_executable() {
    let dir = temp("exe");
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("hello{}", std::env::consts::EXE_SUFFIX));
    let fired = kiln()
        .args(["fire", "-o"])
        .arg(&exe)
        .arg(root().join("examples/hello"))
        .output()
        .unwrap();
    assert!(fired.status.success(), "{}", stderr(&fired));
    assert!(exe.is_file());
    let _ = std::fs::remove_dir_all(&dir);
}
