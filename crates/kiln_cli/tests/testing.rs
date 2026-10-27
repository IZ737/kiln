//! `kiln test`: discovery, failures and reports, and the journal and
//! guestbook examples' own unit, integration and browser tests. The
//! browser tests run when a Chromium is found (`KILN_BROWSER=1` makes a missing one a failure).
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn kiln_test(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_kiln"))
        .arg("test")
        .arg(dir)
        .args(args)
        .env("KILN_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap()
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(files: &[(&str, &str)]) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "kiln-testing-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        for (name, body) in files {
            let path = dir.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        Scratch(dir)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn browser_available() -> bool {
    let found = std::env::var_os("KILN_TEST_BROWSER").is_some()
        || [
            "/opt/pw-browsers",
            "/usr/bin/chromium",
            "/usr/bin/google-chrome",
        ]
        .iter()
        .any(|p| Path::new(p).exists())
        || std::env::var_os("HOME")
            .is_some_and(|h| Path::new(&h).join(".cache/ms-playwright").is_dir());
    if !found {
        assert!(
            std::env::var_os("KILN_BROWSER").is_none(),
            "KILN_BROWSER is set, but no Chromium was found"
        );
        eprintln!("skipping the browser tests: no Chromium");
    }
    found
}

#[test]
fn failures_point_at_the_line_and_set_the_exit_code() {
    let s = Scratch::new(&[
        ("main.kiln", "fn double(n: Int) -> Int {\n    return n * 2\n}\n\nfn main() {\n    print(double(2))\n}\n"),
        (
            "test_double.kiln",
            "import test\n\nfn test_doubles() {\n    test.equal(double(2), 4)\n}\n\nfn test_wrong() {\n    print(\"some output\")\n    test.equal(double(2), 5)\n}\n\nfn test_throws() ! {\n    throw \"broken\"\n}\n",
        ),
    ]);
    let out = kiln_test(&s.0, &[]);
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(t.contains("ok    test_doubles"), "{t}");
    assert!(t.contains("test_double.kiln:9:5: the values differ"), "{t}");
    assert!(
        t.contains("expected: 5") && t.contains("actual:   4"),
        "{t}"
    );
    assert!(t.contains("some output"), "{t}");
    assert!(t.contains("the test failed: broken"), "{t}");

    let out = kiln_test(&s.0, &["--json", "--filter", "wrong"]);
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["version"], 1);
    assert_eq!(report["summary"]["failed"], 1);
    assert_eq!(report["tests"][0]["id"], "test_wrong");
    assert_eq!(report["tests"][0]["failure"]["at"], "test_double.kiln:9:5");
    assert_eq!(report["tests"][0]["failure"]["expected"], "5");

    let out = kiln_test(&s.0, &["--list"]);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).lines().count(),
        3,
        "{}",
        text(&out)
    );
    assert!(kiln_test(&s.0, &["--filter", "doubles"]).status.success());
    // An ordinary build leaves the test files out.
    let run = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(["check"])
        .arg(&s.0)
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", text(&run));
}

#[test]
fn junit_reports_and_timeouts() {
    let s = Scratch::new(&[
        ("main.kiln", "fn main() {}\n"),
        (
            "test_slow.kiln",
            "import test\n\nfn test_forever() {\n    var n = 0\n    while true {\n        n += 1\n    }\n}\n",
        ),
    ]);
    let xml = s.0.join("report.xml");
    let out = kiln_test(&s.0, &["--timeout", "1", "--junit", xml.to_str().unwrap()]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("longer than 1 s"), "{}", text(&out));
    let report = std::fs::read_to_string(&xml).unwrap();
    assert!(
        report.contains("<testcase classname=\"\" name=\"test_forever\""),
        "{report}"
    );
    assert!(report.contains("<failure"), "{report}");
}

#[test]
fn the_journal_tests_pass() {
    examples_tests_pass("examples/journal");
}

/// The app docs/guides/first-site.md builds, with its Ware locked.
#[test]
fn the_guestbook_tests_pass() {
    examples_tests_pass("examples/guestbook");
}

fn examples_tests_pass(dir: &str) {
    let mut args = vec!["--unit", "--integration", "--locked"];
    if browser_available() {
        args.push("--browser");
    }
    let out = kiln_test(&root().join(dir), &args);
    assert!(out.status.success(), "{}", text(&out));
    assert!(text(&out).contains("0 failed"), "{}", text(&out));
}

#[test]
fn a_failing_browser_test_saves_a_screenshot() {
    if !browser_available() {
        return;
    }
    let s = Scratch::new(&[
        (
            "main.kiln",
            "import web\n\nfn main() ! {\n    let app = web.app()\n    app.get(\"/\", fn(req) {\n        return web.html(\"<!doctype html><title>Hi</title><h1>Hello</h1><button>Go</button>\")\n    })\n    try app.listen(\":8080\")\n}\n",
        ),
        (
            "tests/browser/test_page.kiln",
            "import test\n\nfn test_missing_button() {\n    let page = test.page()\n    page.goto(\"/\")\n    test.equal(page.role(\"heading\", name: \"Hello\").text(), \"Hello\")\n    page.role(\"button\", name: \"Go\").click()\n    page.role(\"button\", name: \"Stop\").click()\n}\n",
        ),
    ]);
    let out = kiln_test(&s.0, &["--timeout", "30"]);
    let t = text(&out);
    assert_eq!(out.status.code(), Some(1), "{t}");
    assert!(
        t.contains("test_page.kiln:8:5: no element matches button \"Stop\""),
        "{t}"
    );
    let shot =
        s.0.join(".kiln/test-results/tests.browser.test_missing_button/failure-1.png");
    assert!(shot.is_file(), "{t}");
}
