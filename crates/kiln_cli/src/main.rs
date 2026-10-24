//! The `kiln` command.
//!
//! Phase 0 implements the commands that work on syntax alone: `check`,
//! `parse` and `explain`. Build and run commands arrive with the compiler
//! backend in phase 1.

use clap::{Parser, Subcommand};
use kiln_diagnostics::{codes, SourceFile};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "kiln", version, about = "The Kiln language toolchain")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check source files for errors without building.
    Check {
        /// Files or directories to check. Directories are searched for
        /// `.kiln` and `.glaze` files. Defaults to the current directory.
        paths: Vec<PathBuf>,
        /// Print diagnostics as JSON, for editors and coding agents.
        #[arg(long)]
        json: bool,
    },
    /// Print the syntax tree of a file.
    Parse { file: PathBuf },
    /// Explain an error code, with a wrong and a right example.
    Explain {
        /// An error code such as E0106.
        code: String,
    },
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Check { paths, json } => check(paths, json),
        Command::Parse { file } => parse(&file),
        Command::Explain { code } => explain(&code),
    }
}

fn check(paths: Vec<PathBuf>, json: bool) -> ExitCode {
    let paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths
    };
    let mut files = Vec::new();
    for path in &paths {
        if let Err(err) = collect_sources(path, &mut files) {
            eprintln!("error: cannot read {}: {err}", path.display());
            return ExitCode::from(2);
        }
    }
    files.sort();

    let mut error_count = 0;
    let mut json_diags = Vec::new();
    for path in &files {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                return ExitCode::from(2);
            }
        };
        let file = SourceFile::new(path.display().to_string(), text);
        let diagnostics = if is_glaze(path) {
            // Glazes are parsed; type-checking them comes next.
            kiln_syntax::glaze::parse_glaze(&file.text).diagnostics
        } else {
            let result = kiln_syntax::parse(&file.text);
            // Type errors in a file that does not parse would mostly be noise.
            if result.diagnostics.is_empty() {
                kiln_check::check(&result.module)
            } else {
                result.diagnostics
            }
        };
        error_count += diagnostics.len();
        for diag in &diagnostics {
            if json {
                json_diags.push(diag.to_json(&file));
            } else {
                eprintln!("{}", diag.render(&file));
            }
        }
    }

    if json {
        let report = serde_json::json!({
            "files_checked": files.len(),
            "error_count": error_count,
            "diagnostics": json_diags,
        });
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else if error_count == 0 {
        eprintln!("checked {} file(s): no errors", files.len());
    } else {
        eprintln!(
            "checked {} file(s): {error_count} error(s). Run `kiln explain <code>` for details.",
            files.len()
        );
    }
    if error_count == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn is_glaze(path: &Path) -> bool {
    path.extension().is_some_and(|ext| ext == "glaze")
}

fn collect_sources(path: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
    if path.is_file() {
        out.push(path.to_path_buf());
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let path = entry?.path();
        let hidden_or_build = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('.') || n == "target");
        if hidden_or_build {
            continue;
        }
        if path.is_dir() {
            collect_sources(&path, out)?;
        } else if path.extension().is_some_and(|ext| ext == "kiln") || is_glaze(&path) {
            out.push(path);
        }
    }
    Ok(())
}

fn parse(path: &Path) -> ExitCode {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(err) => {
            eprintln!("error: cannot read {}: {err}", path.display());
            return ExitCode::from(2);
        }
    };
    let file = SourceFile::new(path.display().to_string(), text);
    let diagnostics = if is_glaze(path) {
        let result = kiln_syntax::glaze::parse_glaze(&file.text);
        println!("{:#?}", result.glaze);
        result.diagnostics
    } else {
        let result = kiln_syntax::parse(&file.text);
        println!("{:#?}", result.module);
        result.diagnostics
    };
    for diag in &diagnostics {
        eprintln!("{}", diag.render(&file));
    }
    if diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn explain(code: &str) -> ExitCode {
    let Some(entry) = codes::lookup(code) else {
        eprintln!("error: unknown error code `{code}`");
        return ExitCode::FAILURE;
    };
    println!("{}: {}\n", entry.code, entry.title);
    println!("{}\n", entry.explanation);
    println!("Wrong:\n\n{}\n", indent(entry.wrong));
    println!("Right:\n\n{}", indent(entry.right));
    ExitCode::SUCCESS
}

fn indent(text: &str) -> String {
    text.lines()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
