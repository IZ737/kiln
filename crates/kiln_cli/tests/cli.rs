use std::path::Path;
use std::process::Command;

fn kiln() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kiln"))
}

fn examples() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples")
}

#[test]
fn check_passes_on_examples() {
    let out = kiln().arg("check").arg(examples()).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn check_json_reports_errors_with_fixes() {
    let dir = std::env::temp_dir().join(format!("kiln-cli-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bad.kiln");
    std::fs::write(&file, "fn main() {\n    let x = null\n}\n").unwrap();

    let out = kiln()
        .args(["check", "--json"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["error_count"], 1);
    let diag = &report["diagnostics"][0];
    assert_eq!(diag["code"], "E0106");
    assert_eq!(diag["start"]["line"], 2);
    assert_eq!(diag["suggestion"]["replacement"], "none");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_reports_type_errors() {
    let dir = std::env::temp_dir().join(format!("kiln-cli-types-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bad.kiln");
    std::fs::write(&file, "fn main() {\n    let n: Int = \"seven\"\n}\n").unwrap();

    let out = kiln()
        .args(["check", "--json"])
        .arg(&file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["error_count"], 1);
    assert_eq!(report["diagnostics"][0]["code"], "E0202");

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_type_checks_glazes_with_their_directory() {
    let dir = std::env::temp_dir().join(format!("kiln-cli-glaze-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.kiln"),
        "import web\n\nstruct Post {\n    title: String\n}\n\nfn page() -> web.Response {\n    web.render(\"post.glaze\", {})\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("post.glaze"),
        "{% props post: Post %}\n<h1>{post.titel}</h1>\n",
    )
    .unwrap();

    // Checking only the glaze still uses the types in main.kiln.
    let out = kiln()
        .args(["check", "--json"])
        .arg(dir.join("post.glaze"))
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["error_count"], 1);
    assert_eq!(report["diagnostics"][0]["code"], "E0208");

    // Checking the directory also finds the missing prop in main.kiln.
    let out = kiln().args(["check", "--json"]).arg(&dir).output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let codes: Vec<_> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(codes, ["E0209", "E0208"]);

    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn explain_prints_examples() {
    let out = kiln().args(["explain", "E0107"]).output().unwrap();
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("Wrong:") && text.contains("Right:"));
}

#[test]
fn explain_rejects_unknown_codes() {
    let out = kiln().args(["explain", "E9999"]).output().unwrap();
    assert!(!out.status.success());
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-cli-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn run_compiles_and_runs_hello() {
    let out = kiln()
        .arg("run")
        .arg(examples().join("hello"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Hello, world!\n");
}

#[test]
fn build_writes_an_executable() {
    let dir = scratch("build");
    let exe = dir.join("hello");
    let c = dir.join("hello.c");
    let ir = dir.join("hello.ir");
    let out = kiln()
        .arg("build")
        .arg(examples().join("hello"))
        .arg("-o")
        .arg(&exe)
        .arg("--emit-c")
        .arg(&c)
        .arg("--emit-ir")
        .arg(&ir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(&c).unwrap().contains("kf_main"));
    assert!(std::fs::read_to_string(&ir).unwrap().contains("fn main()"));
    let run = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, world!\n");
}

#[test]
fn run_reports_features_the_compiler_lacks() {
    let dir = scratch("unsupported");
    let file = dir.join("app.kiln");
    std::fs::write(
        &file,
        "fn main() {\n    let big = 99999999999999999999\n    print(big)\n}\n",
    )
    .unwrap();
    let out = kiln().arg("run").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E0401"), "{stderr}");
    assert!(stderr.contains("an integer this large"), "{stderr}");
}

#[test]
fn run_needs_main() {
    let dir = scratch("no-main");
    let file = dir.join("lib.kiln");
    std::fs::write(&file, "fn helper() -> Int {\n    1\n}\n").unwrap();
    let out = kiln().arg("run").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("E0402"));
}

#[test]
fn run_reports_type_errors_before_compiling() {
    let dir = scratch("type-error");
    let file = dir.join("bad.kiln");
    std::fs::write(&file, "fn main() {\n    let x: Int = \"no\"\n}\n").unwrap();
    let out = kiln().arg("run").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("E0202"));
}

#[test]
fn run_compiles_every_file_in_the_folder() {
    let dir = scratch("multi");
    std::fs::write(
        dir.join("main.kiln"),
        "fn main() {\n    print(greet(\"Kiln\"))\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("greet.kiln"),
        "fn greet(name: String) -> String {\n    \"Hello, ${name}!\"\n}\n",
    )
    .unwrap();
    let out = kiln().arg("run").arg(&dir).output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "Hello, Kiln!\n");
}

#[test]
fn compile_errors_point_into_the_right_file() {
    let dir = scratch("multi-error");
    std::fs::write(dir.join("a.kiln"), "fn main() {\n    print(count())\n}\n").unwrap();
    std::fs::write(
        dir.join("b.kiln"),
        "fn count() -> Int {\n    let big = 99999999999999999999\n    big\n}\n",
    )
    .unwrap();
    let out = kiln().arg("run").arg(dir.join("a.kiln")).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E0401"), "{stderr}");
    assert!(stderr.contains("b.kiln:2:15"), "{stderr}");
}

#[cfg(unix)]
#[test]
fn run_reports_a_program_stopped_by_a_signal() {
    let dir = scratch("signal");
    std::fs::write(
        dir.join("main.kiln"),
        "fn down(n: Int) -> Int {\n    down(n + 1) + 1\n}\n\nfn main() {\n    print(down(0))\n}\n",
    )
    .unwrap();
    let out = kiln().arg("run").arg(&dir).output().unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("stopped by signal"), "{stderr}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_reports_css_errors_in_glazes() {
    let dir = scratch("css-errors");
    let app = dir.join("app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        app.join("main.kiln"),
        "import web\n\nfn page() -> web.Response {\n    web.render(\"page.glaze\", {})\n}\n",
    )
    .unwrap();
    std::fs::write(app.join("card.module.css"), ".card { padding: 1px; }\n").unwrap();
    std::fs::write(dir.join("outside.css"), "a { color: red; }\n").unwrap();
    std::fs::write(
        app.join("page.glaze"),
        "{% css \"missing.css\" %}\n{% css \"../outside.css\" %}\n{% css module card from \"card.module.css\" %}\n<p class={card.nope}>hi</p>\n<style>\na { color: red; }}\n</style>\n",
    )
    .unwrap();
    let out = kiln().args(["check", "--json"]).arg(&app).output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let found: Vec<_> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_string(),
                d["start"]["line"].as_u64().unwrap(),
            )
        })
        .collect();
    let codes = |c: &str, l: u64| (c.to_string(), l);
    assert_eq!(
        found,
        [
            codes("E0312", 1),
            codes("E0312", 2),
            codes("E0208", 4),
            codes("E0312", 7)
        ],
        "{report}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_reports_bad_tokens_at_their_import() {
    let dir = scratch("bad-tokens");
    std::fs::write(
        dir.join("main.kiln"),
        "import web\n\nfn page() -> web.Response {\n    web.render(\"page.glaze\", {})\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("theme.css"),
        ".theme-light { --color-accent: #c65a34; --color-text: #222; }\n.theme-dark { --color-accent: 2rem; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("page.glaze"),
        "<p>hi</p>\n{% css \"theme.css\" %}\n",
    )
    .unwrap();
    let out = kiln().args(["check", "--json"]).arg(&dir).output().unwrap();
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let found: Vec<_> = report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            (
                d["code"].as_str().unwrap().to_string(),
                d["start"]["line"].as_u64().unwrap(),
                d["message"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        found,
        [
            (
                "E0222".to_string(),
                2,
                "`--color-accent: 2rem` is not a valid `color` value".to_string()
            ),
            (
                "E0222".to_string(),
                2,
                "theme `dark` does not set `--color-text`, which theme `light` sets".to_string()
            ),
        ],
        "{report}"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn programs_serve_their_files_byte_for_byte() {
    use std::io::{Read, Write};
    let program = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance/web_styles");
    let dir = scratch("assets");
    let exe = dir.join(if cfg!(windows) { "app.exe" } else { "app" });
    let out = kiln()
        .arg("build")
        .arg(&program)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Command::new(&exe)
        .env("PORT", port.to_string())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let get = |path: &str| -> Vec<u8> {
        for _ in 0..100 {
            if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", port)) {
                write!(
                    s,
                    "GET {path} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n"
                )
                .unwrap();
                let mut response = Vec::new();
                s.read_to_end(&mut response).unwrap();
                let body = response.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                return response[body..].to_vec();
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        panic!("the server never started");
    };
    let gif = get("/static/img/pixel.gif");
    let _ = child.kill();
    let _ = child.wait();
    assert_eq!(
        gif,
        std::fs::read(program.join("static/img/pixel.gif")).unwrap()
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn unused_ember_files_do_not_emit_browser_assets() {
    let dir = std::env::temp_dir().join(format!("kiln-unused-ember-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.kiln"),
        "import web\nfn main() { print(web.render(\"page.glaze\").body) }\n",
    )
    .unwrap();
    std::fs::write(dir.join("page.glaze"), "<p>Static</p>").unwrap();
    std::fs::write(
        dir.join("unused.ember"),
        "{% state n = 0 %}<button on:click={ n += 1 }>{n}</button>",
    )
    .unwrap();
    let ir = dir.join("program.ir");
    let output = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .arg("build")
        .arg(&dir)
        .arg("--emit-ir")
        .arg(&ir)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = std::fs::read_to_string(&ir).unwrap();
    assert!(
        !text.contains("/_kiln/ember-"),
        "unused Ember emitted assets"
    );
    // A stateless Ember is still an island when referenced.
    std::fs::write(dir.join("quiet.ember"), "<p>Quiet</p>").unwrap();
    std::fs::write(dir.join("page.glaze"), "<Quiet />").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .arg("build")
        .arg(&dir)
        .arg("--emit-ir")
        .arg(&ir)
        .arg("-o")
        .arg(dir.join("app"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(std::fs::read_to_string(&ir)
        .unwrap()
        .contains("/_kiln/ember-"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn ember_diagnostics_keep_the_actual_filename_and_line() {
    let dir = std::env::temp_dir().join(format!("kiln-ember-spans-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.kiln"), "fn main() {}\n").unwrap();
    let file = dir.join("counter.ember");
    std::fs::write(
        &file,
        "{% state n = 0 %}\n<button on:click={ missing() }>{n}</button>",
    )
    .unwrap();
    let out = kiln()
        .args(["check", "--json"])
        .arg(&file)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["error_count"], 1, "{report}");
    let error = &report["diagnostics"][0];
    assert_eq!(error["code"], "E0201");
    assert_eq!(error["start"]["line"], 2);
    assert!(error["file"].as_str().unwrap().ends_with("counter.ember"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn explicit_embers_cannot_read_server_capabilities_in_the_browser() {
    let dir = std::env::temp_dir().join(format!("kiln-ember-boundary-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.kiln"),
        "import web, fs\nfn main() { print(web.render(\"index.glaze\").body) }\n",
    )
    .unwrap();
    std::fs::write(dir.join("index.glaze"), "<Counter />").unwrap();
    for handler in [
        "env(\"SECRET\")",
        "fs.read(\"secret\") catch { \"\" }",
        "web.redirect(\"/\")",
    ] {
        std::fs::write(
            dir.join("counter.ember"),
            format!("<button on:click={{ {handler} }}>Go</button>"),
        )
        .unwrap();
        let output = kiln()
            .arg("build")
            .arg(&dir)
            .arg("-o")
            .arg(dir.join("app"))
            .output()
            .unwrap();
        assert!(!output.status.success(), "browser accepted {handler}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("E0218"), "{handler}: {stderr}");
        assert!(stderr.contains("counter.ember:1:"), "{handler}: {stderr}");
    }
    let _ = std::fs::remove_dir_all(dir);
}
