//! Runs every program in `conformance/` (one folder each) with `kiln run`,
//! once with each backend (the native one when this kiln has its runtime),
//! and compares its output with the expected files in the folder:
//!
//! - `main.out`: the expected standard output.
//! - `main.err`: the expected standard error, if the program fails. The
//!   exit code must then be 101 for a panic and 1 for an error that left
//!   `main`. Without this file, the program must exit with 0 and print
//!   nothing on standard error.
//! - `main.args`: the program's arguments, one per line.
//! - `main.in`: the program's standard input.
//! - `main.code`: the expected exit code, for a program that ends itself
//!   with `os.exit` or writes to standard error and succeeds.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

#[test]
fn conformance_programs_produce_expected_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let mut programs: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    programs.sort();
    assert!(
        programs.len() >= 5,
        "expected programs in {}",
        root.display()
    );
    let mut backends = vec!["c"];
    if kiln_native::unavailable().is_none() {
        backends.push("native");
    }
    let mut failures = Vec::new();
    for backend in backends {
        for program in &programs {
            run_program(program, backend, &mut failures);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Runs one program and records how its behaviour differs from the
/// expected files.
fn run_program(program: &Path, backend: &str, failures: &mut Vec<String>) {
    let read = |ext: &str| std::fs::read_to_string(program.join("main").with_extension(ext)).ok();
    let args = read("args").unwrap_or_default();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(program)
        .args(args.lines())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = read("in").unwrap_or_default();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let out = child.wait_with_output().unwrap();
    writer.join().unwrap();
    let expected_out = read("out").unwrap_or_default();
    let expected_err = read("err");
    let expected_code = match &expected_err {
        _ if read("code").is_some() => read("code").unwrap().trim().parse().unwrap(),
        None => 0,
        Some(err) if err.lines().last().is_some_and(|l| l.starts_with("panic:")) => 101,
        Some(_) => 1,
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let name = format!(
        "{} ({backend})",
        program.file_name().unwrap().to_string_lossy()
    );
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
