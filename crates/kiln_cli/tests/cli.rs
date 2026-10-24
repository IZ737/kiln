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
