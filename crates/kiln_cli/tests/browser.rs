//! Tests in a real browser: Chromium, driven by Playwright from Node.js.
//! They run when Node.js 22 or later and Playwright are installed (CI
//! installs both), and are skipped otherwise; `KILN_BROWSER=1` makes a
//! missing browser a failure instead.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Node.js with Playwright, or `None` when the browser tests cannot run.
fn node() -> Option<String> {
    let node = std::env::var("KILN_NODE").unwrap_or_else(|_| "node".into());
    let ok = Command::new(&node)
        .args(["-e", "require('playwright')"])
        .output()
        .is_ok_and(|o| o.status.success());
    if !ok {
        assert!(
            std::env::var_os("KILN_BROWSER").is_none(),
            "KILN_BROWSER is set, but Node.js with Playwright is not available"
        );
        eprintln!("skipping the browser tests: no Node.js with Playwright");
        return None;
    }
    Some(node)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn glaze_urls_cannot_assemble_executable_schemes() {
    let Some(node) = node() else { return };
    for backend in ["c", "native"] {
        if backend == "native" && kiln_native::unavailable(kiln_native::Target::host()).is_some() {
            continue;
        }
        let rendered = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["run", "--backend", backend])
            .arg(root().join("conformance/glaze_url_safety"))
            .output()
            .unwrap();
        assert!(
            rendered.status.success(),
            "{}",
            String::from_utf8_lossy(&rendered.stderr)
        );
        let html = std::env::temp_dir().join(format!(
            "kiln-glaze-urls-{}-{backend}.html",
            std::process::id()
        ));
        std::fs::write(&html, rendered.stdout).unwrap();
        let out = Command::new(&node)
            .arg(root().join("crates/kiln_cli/tests/browser/glaze_urls.mjs"))
            .arg(&html)
            .output()
            .unwrap();
        let _ = std::fs::remove_file(&html);
        assert!(
            out.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// Programs from `conformance/` that need nothing but the browser: their
/// WebAssembly build runs in Chromium and prints what the native one does.
#[test]
fn wasm_programs_run_in_a_real_browser() {
    let Some(node) = node() else { return };
    let programs = [
        "basics",
        "closures",
        "collections",
        "compound_keys",
        "errors",
        "generics",
        "json",
        "maps",
        "panic",
        "records",
        "string_building",
        "types",
    ];
    let dir = std::env::temp_dir().join(format!("kiln-browser-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut scripts = Vec::new();
    for name in programs {
        let script = dir.join(format!("{name}.mjs"));
        let built = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["build", "--target", "wasm", "-o"])
            .arg(&script)
            .arg(root().join("conformance").join(name))
            .output()
            .unwrap();
        assert!(
            built.status.success(),
            "{name} does not build for wasm:\n{}",
            String::from_utf8_lossy(&built.stderr)
        );
        scripts.push(script);
    }
    let out = Command::new(&node)
        .arg(root().join("crates/kiln_cli/tests/browser/wasm.mjs"))
        .args(&scripts)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "the browser did not run:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let results: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
    let mut failures = Vec::new();
    for (name, result) in programs.iter().zip(&results) {
        let read = |ext: &str| {
            std::fs::read_to_string(
                root()
                    .join("conformance")
                    .join(name)
                    .join("main")
                    .with_extension(ext),
            )
            .unwrap_or_default()
        };
        let expected_err = read("err");
        let expected_code = if expected_err.is_empty() {
            0
        } else if expected_err
            .lines()
            .last()
            .is_some_and(|l| l.starts_with("panic:"))
        {
            101
        } else {
            1
        };
        if result["out"] != read("out").as_str() {
            failures.push(format!(
                "{name}: output differs:\n{}",
                result["out"].as_str().unwrap_or("")
            ));
        }
        if result["err"] != expected_err.as_str() {
            failures.push(format!(
                "{name}: standard error differs:\n{}",
                result["err"]
            ));
        }
        if result["code"] != expected_code {
            failures.push(format!(
                "{name}: exit code {}, expected {expected_code}",
                result["code"]
            ));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// The app in `tests/browser/embers`, a server with a page of embers, built
/// natively and driven in Chromium by `tests/browser/embers.mjs`:
/// hydration hints, state and events, lists, forms, nested embers, calls
/// of `server fn`s with their pending, cancelled and failed states, and
/// pages that work without JavaScript or WasmGC.
#[test]
fn embers_work_in_a_real_browser() {
    drive(
        "embers",
        "crates/kiln_cli/tests/browser/embers",
        "embers.mjs",
    );
}

/// The app in `tests/browser/stores`: two unrelated embers share a store
/// from a folder module, changes batch and keep focus, client navigation
/// keeps the store while a reload starts it again, a store with a key
/// outlives the page, and overlapping requests each see their own.
#[test]
fn stores_work_in_a_real_browser() {
    drive(
        "stores",
        "crates/kiln_cli/tests/browser/stores",
        "stores.mjs",
    );
}

/// The documentation site's embers: the search box and the copy buttons.
#[test]
fn the_sites_embers_work_in_a_real_browser() {
    drive("site", "site", "site.mjs");
}

/// `examples/static_blog`, exported with `kiln fire --static` and served
/// by a plain file server with no Kiln program running: its pages, styles,
/// image and feed load, a redirect page and the 404 page work, and its
/// ember hydrates and runs.
#[test]
fn a_static_export_works_in_a_real_browser() {
    let Some(node) = node() else { return };
    let dir = std::env::temp_dir().join(format!("kiln-static-blog-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let fired = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(["fire", "--static", "-o"])
        .arg(&dir)
        .arg(root().join("examples/static_blog"))
        .output()
        .unwrap();
    assert!(
        fired.status.success(),
        "the export failed:\n{}",
        String::from_utf8_lossy(&fired.stderr)
    );
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let site = dir.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let site = site.clone();
            std::thread::spawn(move || serve_file(stream, &site));
        }
    });
    let out = Command::new(&node)
        .arg(root().join("crates/kiln_cli/tests/browser/static_blog.mjs"))
        .arg(format!("http://{address}"))
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the browser did not run:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    eprintln!("static_blog: {}", result["metrics"]);
    let failures: Vec<&str> = result["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f.as_str())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Answers one request as a plain static host does: the file at the path,
/// a folder's `index.html` (after a redirect to the path with a `/`), or
/// `404.html` with status 404.
fn serve_file(mut stream: std::net::TcpStream, site: &Path) {
    use std::io::{BufRead, BufReader, Write};
    let mut line = String::new();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    if reader.read_line(&mut line).is_err() {
        return;
    }
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() || header.trim().is_empty() {
            break;
        }
    }
    let target = line.split_whitespace().nth(1).unwrap_or("/");
    let path = target.split(['?', '#']).next().unwrap_or("/");
    let mut decoded = Vec::new();
    let bytes = path.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&path[i + 1..i + 3], 16) {
                decoded.push(b);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    let path = String::from_utf8_lossy(&decoded).into_owned();
    let mut file = site.join(path.trim_start_matches('/'));
    if path.split('/').any(|p| p == "..") {
        file = site.join("missing");
    }
    let respond = |stream: &mut std::net::TcpStream, status: &str, kind: &str, extra: &str, body: &[u8]| {
        let head = format!(
            "HTTP/1.1 {status}\r\ncontent-type: {kind}\r\ncontent-length: {}\r\n{extra}connection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
    };
    if file.is_dir() {
        if !path.ends_with('/') {
            let extra = format!("location: {path}/\r\n");
            respond(&mut stream, "301 Moved Permanently", "text/plain", &extra, b"");
            return;
        }
        file = file.join("index.html");
    }
    let kind = match file.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css",
        Some("js" | "mjs") => "text/javascript",
        Some("wasm") => "application/wasm",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("xml") => "application/xml",
        Some("json") => "application/json",
        _ => "application/octet-stream",
    };
    match std::fs::read(&file) {
        Ok(body) => respond(&mut stream, "200 OK", kind, "", &body),
        Err(_) => {
            let body = std::fs::read(site.join("404.html")).unwrap_or_default();
            respond(&mut stream, "404 Not Found", "text/html; charset=utf-8", "", &body);
        }
    }
}

/// Builds the app in `app`, runs it, and drives it in Chromium with the
/// script `script` in `tests/browser`, which prints the checks that
/// failed.
fn drive(name: &str, app: &str, script: &str) {
    drive_backend(name, app, script, None);
}
fn drive_backend(name: &str, app: &str, script: &str, backend: Option<&str>) {
    drive_with(name, app, script, backend, &|_| Vec::new());
}

/// [`drive_backend`], with `setup` run first in the test's own folder: it
/// returns the environment the app runs with.
fn drive_with(
    name: &str,
    app: &str,
    script: &str,
    backend: Option<&str>,
    setup: &dyn Fn(&Path) -> Vec<(String, String)>,
) {
    let Some(node) = node() else { return };
    let dir = std::env::temp_dir().join(format!("kiln-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    let mut command = Command::new(env!("CARGO_BIN_EXE_kiln"));
    command
        .args(["build", "-o"])
        .arg(&exe)
        .arg(root().join(app));
    if let Some(backend) = backend {
        command.args(["--backend", backend]);
    }
    let built = command.output().unwrap();
    assert!(
        built.status.success(),
        "{app} does not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Command::new(&exe)
        .env("PORT", port.to_string())
        .envs(setup(&dir))
        .spawn()
        .unwrap();
    let address = format!("127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::net::TcpStream::connect(&address).is_err() {
        assert!(std::time::Instant::now() < deadline, "{app} never listened");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = Command::new(&node)
        .arg(root().join("crates/kiln_cli/tests/browser").join(script))
        .arg(format!("http://{address}"))
        .output()
        .unwrap();
    let _ = server.kill();
    let _ = server.wait();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(
        out.status.success(),
        "the browser did not run:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|_| {
        panic!(
            "the test printed no result:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    });
    eprintln!("{name}: {}", result["metrics"]);
    let failures: Vec<&str> = result["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f.as_str())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn multi_folder_fieldnotes_works_in_a_real_browser() {
    drive(
        "fieldnotes",
        "crates/kiln_cli/tests/fieldnotes",
        "fieldnotes.mjs",
    );
    drive_backend(
        "fieldnotes-c",
        "crates/kiln_cli/tests/fieldnotes",
        "fieldnotes.mjs",
        Some("c"),
    );
}

/// The dogfood app `examples/journal`, in folders, on a database its
/// committed migrations make: signing up and in, posts with per-record
/// permissions, and the ember that adds comments through an ORM-backed
/// `server fn` that knows who is calling, with hostile calls turned away.
/// It runs on SQLite with both backends, and natively on PostgreSQL and
/// MySQL when `KILN_POSTGRES_URL` and `KILN_MYSQL_URL` are set.
#[test]
fn journal_works_in_a_real_browser() {
    let mut runs: Vec<(&str, Option<&str>, Option<String>)> =
        vec![("journal", None, None), ("journal-c", Some("c"), None)];
    for (name, var) in [
        ("journal-postgres", "KILN_POSTGRES_URL"),
        ("journal-mysql", "KILN_MYSQL_URL"),
    ] {
        if let Ok(url) = std::env::var(var) {
            runs.push((name, None, Some(url)));
        }
    }
    for (name, backend, server) in runs {
        drive_with(name, "examples/journal", "journal.mjs", backend, &|dir| {
            let url = match &server {
                Some(url) => {
                    // A server keeps what the last run left: start again.
                    let mut db = kiln_dbwire::open(url, None).unwrap();
                    for table in [
                        "comments",
                        "posts",
                        "users",
                        "kiln_sessions",
                        "kiln_migrations",
                    ] {
                        db.exec(&format!("drop table if exists {table}"), &[])
                            .unwrap();
                    }
                    url.clone()
                }
                None => format!("sqlite:{}", dir.join("journal.db").display()),
            };
            let migrated = Command::new(env!("CARGO_BIN_EXE_kiln"))
                .args(["db", "migrate", "--url", &url, "--path"])
                .arg(root().join("examples/journal"))
                .output()
                .unwrap();
            assert!(
                migrated.status.success(),
                "{}",
                String::from_utf8_lossy(&migrated.stderr)
            );
            vec![
                ("DATABASE_URL".into(), url),
                ("JOURNAL_HTTP".into(), "1".into()),
                ("JOURNAL_ADMIN".into(), "ada@example.com".into()),
            ]
        });
    }
}
