//! `kiln fmt`: formats `.kiln` files, and the code in glazes' and embers'
//! `{% %}` tags, with the formatter the language server uses.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The formatted text of a file, by its kind.
pub fn format_file(path: &Path, text: &str) -> Result<String, String> {
    if crate::is_glaze(path) {
        kiln_syntax::format::format_glaze(text)
    } else {
        kiln_syntax::format::format_kiln(text)
    }
}

fn collect(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_file() {
        out.push(path.to_path_buf());
        return;
    }
    let Ok(entries) = std::fs::read_dir(path) else {
        return;
    };
    let mut entries: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    entries.sort();
    for p in entries {
        let name = p.file_name().unwrap_or_default().to_string_lossy().to_string();
        if p.is_dir() {
            if !name.starts_with('.') && !name.starts_with("__") && name != "target" && name != "node_modules" {
                collect(&p, out);
            }
        } else if p.extension().is_some_and(|e| e == "kiln") || crate::is_glaze(&p) {
            out.push(p);
        }
    }
}

pub fn run(paths: Vec<PathBuf>, check: bool) -> ExitCode {
    let paths = if paths.is_empty() { vec![PathBuf::from(".")] } else { paths };
    let mut files = Vec::new();
    for p in &paths {
        if !p.exists() {
            eprintln!("error: {} does not exist", p.display());
            return ExitCode::from(2);
        }
        collect(p, &mut files);
    }
    let mut changed = Vec::new();
    let mut failed = 0;
    for file in &files {
        let Ok(text) = std::fs::read_to_string(file) else {
            eprintln!("error: cannot read {}", file.display());
            failed += 1;
            continue;
        };
        match format_file(file, &text) {
            Ok(out) if out == text => {}
            Ok(out) => {
                if check {
                    println!("{}", file.display());
                } else if let Err(e) = std::fs::write(file, out) {
                    eprintln!("error: cannot write {}: {e}", file.display());
                    failed += 1;
                    continue;
                }
                changed.push(file);
            }
            Err(e) => {
                eprintln!("error: {}: {e}", file.display());
                failed += 1;
            }
        }
    }
    if check {
        if !changed.is_empty() {
            eprintln!("{} of {} file(s) are not formatted; run `kiln fmt`", changed.len(), files.len());
        }
    } else {
        eprintln!("formatted {} file(s); {} were already formatted", changed.len(), files.len() - changed.len() - failed);
    }
    if failed > 0 {
        ExitCode::from(2)
    } else if check && !changed.is_empty() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
