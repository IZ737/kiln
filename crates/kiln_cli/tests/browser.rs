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
    let Some(node) = node() else { return };
    let dir = std::env::temp_dir().join(format!("kiln-embers-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join(if cfg!(windows) {
        "embers.exe"
    } else {
        "embers"
    });
    let built = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(["build", "-o"])
        .arg(&exe)
        .arg(root().join("crates/kiln_cli/tests/browser/embers"))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "the embers app does not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut server = Command::new(&exe)
        .env("PORT", port.to_string())
        .spawn()
        .unwrap();
    let address = format!("127.0.0.1:{port}");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while std::net::TcpStream::connect(&address).is_err() {
        assert!(
            std::time::Instant::now() < deadline,
            "the embers app never listened"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = Command::new(&node)
        .arg(root().join("crates/kiln_cli/tests/browser/embers.mjs"))
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
    eprintln!("embers: {}", result["metrics"]);
    let failures: Vec<&str> = result["failures"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f.as_str())
        .collect();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
