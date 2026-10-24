//! Runs every program in `conformance/` with `kiln run` and compares its
//! output with the expected files next to it:
//!
//! - `name.out`: the expected standard output.
//! - `name.err`: the expected standard error, if the program fails. The
//!   exit code must then be 101 for a panic and 1 for an error that left
//!   `main`. Without this file, the program must exit with 0 and print
//!   nothing on standard error.

use std::path::Path;
use std::process::Command;

#[test]
fn conformance_programs_produce_expected_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let mut programs: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "kiln"))
        .collect();
    programs.sort();
    assert!(
        programs.len() >= 5,
        "expected programs in {}",
        root.display()
    );
    let mut failures = Vec::new();
    for program in &programs {
        let out = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .arg("run")
            .arg(program)
            .output()
            .unwrap();
        let read = |ext: &str| std::fs::read_to_string(program.with_extension(ext)).ok();
        let expected_out = read("out").unwrap_or_default();
        let expected_err = read("err");
        let expected_code = match &expected_err {
            None => 0,
            Some(err) if err.lines().last().is_some_and(|l| l.starts_with("panic:")) => 101,
            Some(_) => 1,
        };
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        let name = program.file_name().unwrap().to_string_lossy();
        if stdout != expected_out {
            failures.push(format!(
                "{name}: standard output differs\n--- expected\n{expected_out}--- found\n{stdout}"
            ));
        }
        if stderr != expected_err.clone().unwrap_or_default() {
            failures.push(format!("{name}: standard error differs:\n{stderr}"));
        }
        if out.status.code() != Some(expected_code) {
            failures.push(format!(
                "{name}: exit code {:?}, expected {expected_code}",
                out.status.code()
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
