//! The agent benchmark's tasks (`bench/agents/tasks`) must stay fair: each
//! task's starting project fails its hidden checks, and its reference
//! solution passes them, with this compiler.
use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Copies a folder's files into `to`, writing the path of kiln/ui where a
/// file says `{{KILN_UI}}`. `run/` holds expected runs, not project files.
fn overlay(from: &Path, to: &Path, ui: &str) {
    for entry in std::fs::read_dir(from).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap();
        let target = to.join(name);
        if path.is_dir() {
            if name == "run" && from.ends_with("check") {
                continue;
            }
            std::fs::create_dir_all(&target).unwrap();
            overlay(&path, &target, ui);
        } else {
            let text = std::fs::read_to_string(&path).unwrap();
            std::fs::write(&target, text.replace("{{KILN_UI}}", ui)).unwrap();
        }
    }
}

fn kiln(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(args)
        .current_dir(dir)
        .env("KILN_NO_UPDATE_CHECK", "1")
        .output()
        .unwrap()
}

fn project(task: &Path, layers: &[&str], label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "kiln-bench-{}-{label}-{}",
        task.file_name().unwrap().to_string_lossy(),
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let ui = root().join("wares/ui").canonicalize().unwrap();
    for layer in layers {
        overlay(&task.join(layer), &dir, &ui.to_string_lossy());
    }
    dir
}

#[test]
fn every_task_is_fair() {
    let tasks = root().join("bench/agents/tasks");
    let mut names: Vec<PathBuf> = std::fs::read_dir(&tasks)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    names.sort();
    assert!(names.len() >= 7, "{names:?}");
    for task in names {
        for part in ["task.md", "start", "solution", "check"] {
            assert!(task.join(part).exists(), "{} has no {part}", task.display());
        }

        let unsolved = project(&task, &["start", "check"], "start");
        let out = kiln(&unsolved, &["test", "."]);
        assert_ne!(
            out.status.code(),
            Some(0),
            "{}: the starting project already passes",
            task.display()
        );
        let _ = std::fs::remove_dir_all(&unsolved);

        let solved = project(&task, &["start", "solution", "check"], "solution");
        let out = kiln(&solved, &["test", "."]);
        assert!(
            out.status.success(),
            "{}: the solution fails its checks:\n{}{}",
            task.display(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            kiln(&solved, &["check", "."]).status.success(),
            "{}",
            task.display()
        );
        // Expected runs: `run/NAME.args` gives the arguments, `.out` the
        // output and `.code` the exit code (0 when absent).
        let runs = task.join("check/run");
        if runs.is_dir() {
            for entry in std::fs::read_dir(&runs).unwrap() {
                let path = entry.unwrap().path();
                if path.extension().is_none_or(|e| e != "args") {
                    continue;
                }
                let args = std::fs::read_to_string(&path).unwrap();
                let mut command = vec!["run", ".", "--"];
                command.extend(args.split_whitespace());
                let out = kiln(&solved, &command);
                let code: i32 = std::fs::read_to_string(path.with_extension("code"))
                    .map(|c| c.trim().parse().unwrap())
                    .unwrap_or(0);
                assert_eq!(out.status.code(), Some(code), "{}", path.display());
                if let Ok(expected) = std::fs::read_to_string(path.with_extension("out")) {
                    assert_eq!(
                        String::from_utf8_lossy(&out.stdout),
                        expected,
                        "{}",
                        path.display()
                    );
                }
            }
        }
        let _ = std::fs::remove_dir_all(&solved);
    }
}
