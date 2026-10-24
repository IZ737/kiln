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
        .arg(examples().join("hello.kiln"))
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
    let out = kiln()
        .arg("build")
        .arg(examples().join("hello.kiln"))
        .arg("-o")
        .arg(&exe)
        .arg("--emit-c")
        .arg(&c)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(std::fs::read_to_string(&c).unwrap().contains("kf_main"));
    let run = Command::new(&exe).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&run.stdout), "Hello, world!\n");
}

#[test]
fn run_reports_features_the_compiler_lacks() {
    let dir = scratch("unsupported");
    let file = dir.join("app.kiln");
    std::fs::write(
        &file,
        "import json\n\nfn main() {\n    print(json.encode(1))\n}\n",
    )
    .unwrap();
    let out = kiln().arg("run").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("E0401"), "{stderr}");
    assert!(stderr.contains("`json.encode`"), "{stderr}");
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
