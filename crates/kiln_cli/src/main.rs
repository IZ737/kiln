//! The `kiln` command.
//!
//! Phase 0 implements the commands that work on syntax alone: `check`,
//! `parse` and `explain`. Build and run commands arrive with the compiler
//! backend in phase 1.

use clap::{Parser, Subcommand};
use kiln_diagnostics::{codes, Diagnostic, SourceFile};
use std::collections::BTreeMap;
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

    // Files are type-checked with the other files in their directory,
    // which form one app: glazes use the types of the `.kiln` files there.
    let mut dirs: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for path in &files {
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        dirs.entry(dir).or_default().push(path.clone());
    }
    let mut error_count = 0;
    let mut json_diags = Vec::new();
    for (dir, requested) in &dirs {
        let mut members = requested.clone();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                let source = path.extension().is_some_and(|e| e == "kiln") || is_glaze(&path);
                if source && path.is_file() && !members.iter().any(|m| same_file(m, &path)) {
                    members.push(path);
                }
            }
        }
        let app = match check_dir(&members) {
            Ok(app) => app,
            Err((path, err)) => {
                eprintln!("error: cannot read {}: {err}", path.display());
                return ExitCode::from(2);
            }
        };
        for (file, diagnostics) in app {
            if !requested
                .iter()
                .any(|r| r.display().to_string() == file.name)
            {
                continue;
            }
            error_count += diagnostics.len();
            for diag in &diagnostics {
                if json {
                    json_diags.push(diag.to_json(&file));
                } else {
                    eprintln!("{}", diag.render(&file));
                }
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

enum Parsed {
    Kiln(kiln_syntax::ParseResult),
    Glaze(Box<kiln_syntax::glaze::GlazeResult>),
}

/// Parses and type-checks the files of one directory. Type errors in a
/// directory where something does not parse would mostly be noise, so they
/// are reported only when every file parses.
#[allow(clippy::type_complexity)]
fn check_dir(
    paths: &[PathBuf],
) -> Result<Vec<(SourceFile, Vec<Diagnostic>)>, (PathBuf, std::io::Error)> {
    let mut sources = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(path).map_err(|e| (path.clone(), e))?;
        let file = SourceFile::new(path.display().to_string(), text);
        let parsed = if is_glaze(path) {
            Parsed::Glaze(Box::new(kiln_syntax::glaze::parse_glaze(&file.text)))
        } else {
            Parsed::Kiln(kiln_syntax::parse(&file.text))
        };
        sources.push((path, file, parsed));
    }
    let parse_errors = |p: &Parsed| match p {
        Parsed::Kiln(r) => r.diagnostics.clone(),
        Parsed::Glaze(r) => r.diagnostics.clone(),
    };
    if sources.iter().any(|(_, _, p)| !parse_errors(p).is_empty()) {
        return Ok(sources
            .into_iter()
            .map(|(_, file, p)| {
                let diags = parse_errors(&p);
                (file, diags)
            })
            .collect());
    }
    let modules: Vec<&kiln_syntax::ast::Module> = sources
        .iter()
        .filter_map(|(_, _, p)| match p {
            Parsed::Kiln(r) => Some(&r.module),
            Parsed::Glaze(_) => None,
        })
        .collect();
    let names: Vec<String> = sources
        .iter()
        .map(|(path, _, _)| {
            path.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let glazes: Vec<(&str, &kiln_syntax::ast::Glaze)> = sources
        .iter()
        .zip(&names)
        .filter_map(|((_, _, p), name)| match p {
            Parsed::Glaze(r) => Some((name.as_str(), &r.glaze)),
            Parsed::Kiln(_) => None,
        })
        .collect();
    let mut result = kiln_check::check_app(&modules, &glazes);
    let mut module_diags = result.modules.drain(..);
    let mut glaze_diags = result.glazes.drain(..);
    let mut out = Vec::new();
    for (path, file, _) in &sources {
        let diags = if is_glaze(path) {
            glaze_diags.next()
        } else {
            module_diags.next()
        };
        out.push((file.clone(), diags.unwrap_or_default()));
    }
    Ok(out)
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
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
