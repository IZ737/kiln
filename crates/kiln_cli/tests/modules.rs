//! Folder modules are checked through the same CLI path used by builds.
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};
static NEXT: AtomicUsize = AtomicUsize::new(0);
struct App(PathBuf);
impl App {
    fn new(files: &[(&str, &str)]) -> Self {
        let root = std::env::temp_dir().join(format!(
            "kiln-modules-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        let app = Self(root);
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
    fn check(&self) -> Output {
        Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["check", "--json"])
            .arg(&self.0)
            .output()
            .unwrap()
    }
    fn succeeds(&self) {
        let out = self.check();
        assert!(
            out.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    fn fails(&self, expected: &str) {
        let out = self.check();
        assert!(!out.status.success());
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(
            text.contains(expected),
            "{text}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
impl Drop for App {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn public_api_private_implementation_and_nominal_identity() {
    let app = App::new(&[
        (
            "main.kiln",
            "import a, b\nfn main() { print(a.show(a.Value(n: 2))) }\n",
        ),
        (
            "a/types.kiln",
            "pub struct Value { n: Int }\nfn hidden(n: Int) -> Int { return n + 1 }\n",
        ),
        (
            "a/api.kiln",
            "pub fn show(v: Value) -> Int { return hidden(v.n) }\n",
        ),
        ("b/main.kiln", "pub struct Value { n: Int }\n"),
    ]);
    app.succeeds();
    app.write(
        "main.kiln",
        "import a, b\nfn main() { print(a.show(b.Value(n: 2))) }\n",
    );
    app.fails("E0202");
    app.write("main.kiln", "import a\nfn main() { print(a.hidden(2)) }\n");
    app.fails("is private");
}
#[test]
fn exported_signatures_cannot_leak_private_types() {
    let app = App::new(&[
        ("main.kiln", "import a\nfn main() {}\n"),
        (
            "a/main.kiln",
            "struct Secret { value: Int }\npub fn leak() -> Secret { return Secret(value: 1) }\n",
        ),
    ]);
    app.fails("private type");
}
#[test]
fn imports_are_lexical_and_nested_modules_are_explicit() {
    let app = App::new(&[
        ("main.kiln", "import a.b\nfn main() { print(b.value()) }\n"),
        ("a/b/api.kiln", "pub fn value() -> Int { return 7 }\n"),
        ("other.kiln", "fn other() { print(b.value()) }\n"),
    ]);
    app.fails("E0201");
    app.write("other.kiln", "fn other() {}\n");
    app.succeeds();
}
#[test]
fn cycles_and_case_errors_are_deterministic() {
    let app = App::new(&[
        ("main.kiln", "import a\nfn main() {}\n"),
        ("a/main.kiln", "import b\npub fn a() {}\n"),
        ("b/main.kiln", "import a\npub fn b() {}\n"),
    ]);
    app.fails("a -> b -> a");
    app.write("b/main.kiln", "pub fn b() {}\n");
    app.write("main.kiln", "import A\nfn main() {}\n");
    app.fails("case-sensitive");
}
#[test]
fn manifest_validation() {
    let app = App::new(&[
        ("main.kiln", "fn main() {}\n"),
        (
            "kiln.toml",
            "[package]\nname = \"test\"\nkind = \"app\"\nentry = \"main.kiln\"\n",
        ),
    ]);
    app.succeeds();
    app.write("kiln.toml", "[package]\nentry = \"../main.kiln\"\n");
    app.fails("within the project root");
    app.write("kiln.toml", "[package]\nentry = 42\n");
    app.fails("must be a string");
    app.write("kiln.toml", "[package]\nentry = \"main.kiln\"\n[wares]\n");
    app.fails("unsupported section");
}
#[test]
fn module_local_components_and_css() {
    let app=App::new(&[("main.kiln","import posts\nfn main() {}\n"),("posts/main.kiln","import web\npub fn page() -> web.Response { return web.render(\"index.glaze\", {}) }\n"),("posts/index.glaze","{% css \"style.css\" %}\n<Card />\n"),("posts/card.glaze","<p>Posts</p>\n"),("posts/style.css","p { color: red; }\n"),("card.glaze","{% props missing: String %}\n{missing}\n")]);
    app.succeeds();
}

#[test]
fn manifest_selects_nested_entry_and_explicit_file_overrides_it() {
    let app = App::new(&[
        ("kiln.toml", "[package]\nentry = \"app/start.kiln\"\n"),
        (
            "app/start.kiln",
            "import library\nfn main() { print(library.value()) }\n",
        ),
        ("library/main.kiln", "pub fn value() -> Int { return 42 }\n"),
        ("other/main.kiln", "fn main() { print(7) }\n"),
    ]);
    for (target, expected) in [(&app.0, "42\n"), (&app.0.join("other/main.kiln"), "7\n")] {
        let out = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .arg("run")
            .arg(target)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&out.stdout), expected);
    }
}

#[test]
fn source_maps_keep_original_error_locations_and_private_names_cannot_be_spelled() {
    let app = App::new(&[
        (
            "main.kiln",
            "import posts\nfn main() { print(posts.title(1)) }\n",
        ),
        (
            "posts/api.kiln",
            "pub fn title(s: String) -> String { return s }\nfn hidden() {}\n",
        ),
    ]);
    let out = app.check();
    let json: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let diagnostic = &json["diagnostics"][0];
    assert!(diagnostic.to_string().contains("main.kiln"));
    app.write(
        "main.kiln",
        "import posts\nfn main() { __kiln_module_5_posts_hidden() }\n",
    );
    app.fails("reserved");
}

#[test]
fn unreachable_folders_are_not_implicitly_compiled() {
    let app = App::new(&[
        ("main.kiln", "fn main() {}\n"),
        ("unused/broken.kiln", "this is not valid Kiln\n"),
    ]);
    app.succeeds();
}

#[test]
fn case_collisions_and_reserved_module_folders_fail() {
    let app = App::new(&[
        ("main.kiln", "import web\nfn main() {}\n"),
        ("web/api.kiln", "pub fn value() {}\n"),
    ]);
    app.fails("reserved");
    let app = App::new(&[
        ("main.kiln", "fn main() {}\n"),
        ("posts/a.kiln", "fn a() {}\n"),
        ("Posts/b.kiln", "fn b() {}\n"),
    ]);
    if app.0.join("posts").canonicalize().unwrap() != app.0.join("Posts").canonicalize().unwrap() {
        app.fails("case collision");
    }
}

#[test]
fn inferred_exports_cannot_leak_private_nominal_types() {
    let app = App::new(&[
        ("main.kiln", "import a\nfn main() {}\n"),
        (
            "a/main.kiln",
            "struct Secret { value: Int }\npub const leak = [Secret(value: 1)]\n",
        ),
    ]);
    app.fails("private type");
}

#[test]
fn inferred_receivers_do_not_bypass_method_visibility() {
    let app=App::new(&[("main.kiln","import a\nfn main() { let value = a.Value(n: 1)\nprint(value.hidden()) }\n"),("a/main.kiln","pub struct Value { n: Int }\nimpl Value { fn hidden(self) -> Int { return self.n } }\n")]);
    app.fails("method `hidden` is private");
    app.write("a/main.kiln","pub struct Value { n: Int }\nimpl Value { pub fn hidden(self) -> Int { return self.n } }\n");
    app.succeeds();
}

#[test]
fn fieldnotes_optional_manifest_builds_for_native_c_and_windows() {
    fn copy(source: &std::path::Path, dest: &std::path::Path) {
        std::fs::create_dir_all(dest).unwrap();
        for entry in std::fs::read_dir(source).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let to = dest.join(entry.file_name());
            if path.is_dir() {
                copy(&path, &to);
            } else {
                std::fs::copy(path, to).unwrap();
            }
        }
    }
    let app = App::new(&[]);
    copy(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fieldnotes"),
        &app.0,
    );
    app.write(
        "kiln.toml",
        "[package]\nname = \"fieldnotes\"\nkind = \"app\"\nentry = \"main.kiln\"\n",
    );
    let mut options = vec![vec!["--backend", "c"]];
    if kiln_native::unavailable(kiln_native::Target::Linux).is_none() {
        options.push(vec!["--backend", "native", "--target", "linux"]);
    }
    if kiln_native::unavailable(kiln_native::Target::Windows).is_none() {
        options.push(vec!["--backend", "native", "--target", "windows"]);
    }
    for flags in options {
        let output = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .arg("build")
            .arg(&app.0)
            .args(&flags)
            .arg("-o")
            .arg(app.0.join("app.exe"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{flags:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn browser_reachability_keeps_server_capabilities_out_across_modules() {
    let app=App::new(&[
        ("main.kiln","import web, secrets\nfn read() -> String ! { return try secrets.read() }\nfn main() ! { let app = web.app()\napp.get(\"/\", fn(req) { return web.render(\"index.glaze\", {}) })\ntry app.listen(\":8080\") }\n"),
        ("secrets/api.kiln","import fs\npub fn read() -> String ! { return try fs.read(\"secret.txt\") }\n"),
        ("index.glaze","<Editor />"),
        ("editor.ember","{% state message = \"\" %}\n<button on:click={ message = read() catch err { err.message } }>Read</button>\n"),
    ]);
    let output = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .arg("build")
        .arg(&app.0)
        .arg("-o")
        .arg(app.0.join("app"))
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(error.contains("E0218"), "{error}");
    assert!(error.contains("secrets/api.kiln"), "{error}");
    assert!(!error.contains("__kiln_module_"), "{error}");
}
