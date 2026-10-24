//! The `kiln` command: `check`, `build`, `run`, `lsp`, `parse` and `explain`.

mod lsp;

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
    /// Compile a program into an executable. A program is a folder: all of
    /// its `.kiln` files, one of which has `fn main()`.
    Build {
        /// The program's folder, or any file in it.
        file: PathBuf,
        /// Where to write the executable. Defaults to the folder's name.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Also write the generated C to this path.
        #[arg(long)]
        emit_c: Option<PathBuf>,
        /// Also write the program's IR, in its readable form, to this path.
        #[arg(long)]
        emit_ir: Option<PathBuf>,
    },
    /// Compile a program and run it.
    Run {
        /// The program's folder, or any file in it.
        file: PathBuf,
        /// Arguments passed to the program.
        #[arg(trailing_var_arg = true)]
        args: Vec<String>,
    },
    /// Run the language server, for editors, over standard input and output.
    Lsp,
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
        Command::Build {
            file,
            output,
            emit_c,
            emit_ir,
        } => {
            let out = output.unwrap_or_else(|| default_output(&file));
            let emit = Emit {
                c: emit_c.as_deref(),
                ir: emit_ir.as_deref(),
            };
            match build(&file, &out, emit) {
                Ok(()) => ExitCode::SUCCESS,
                Err(code) => code,
            }
        }
        Command::Run { file, args } => run(&file, &args),
        Command::Lsp => ExitCode::from(lsp::run() as u8),
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
    let mut texts = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(path).map_err(|e| (path.clone(), e))?;
        texts.push((path.clone(), text));
    }
    Ok(check_sources(texts))
}

/// Parses and type-checks the files of one directory, given their text.
pub(crate) fn check_sources(texts: Vec<(PathBuf, String)>) -> Vec<(SourceFile, Vec<Diagnostic>)> {
    let mut sources = Vec::new();
    for (path, text) in &texts {
        let file = SourceFile::new(path.display().to_string(), text.clone());
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
        return sources
            .into_iter()
            .map(|(_, file, p)| {
                let diags = parse_errors(&p);
                (file, diags)
            })
            .collect();
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
    out
}

/// The folder a program lives in, and its `.kiln` and `.glaze` files. A
/// program is a whole folder; naming one of its files means that folder.
fn app_files(path: &Path) -> std::io::Result<(PathBuf, Vec<PathBuf>)> {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        std::fs::metadata(path)?;
        match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        }
    };
    let mut files = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let p = entry?.path();
        let source = p.extension().is_some_and(|e| e == "kiln") || is_glaze(&p);
        if source && p.is_file() {
            files.push(p);
        }
    }
    files.sort();
    Ok((dir, files))
}

/// Where `kiln build` writes the executable by default: the program's
/// folder name, in the current directory.
fn default_output(path: &Path) -> PathBuf {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let name = dir
        .canonicalize()
        .ok()
        .and_then(|d| d.file_name().map(|n| n.to_owned()))
        .unwrap_or_else(|| "program".into());
    PathBuf::from(name)
}

/// Where `build` also writes the intermediate forms of a program.
#[derive(Clone, Copy, Default)]
struct Emit<'a> {
    c: Option<&'a Path>,
    ir: Option<&'a Path>,
}

/// Checks and compiles the program in a folder into an executable at `out`.
fn build(path: &Path, out: &Path, emit: Emit) -> Result<(), ExitCode> {
    let (dir, files) = app_files(path).map_err(|err| {
        eprintln!("error: cannot read {}: {err}", path.display());
        ExitCode::from(2)
    })?;
    let failed = |count: usize| {
        eprintln!("{count} error(s). Run `kiln explain <code>` for details.");
        ExitCode::FAILURE
    };
    let checked = check_dir(&files).map_err(|(p, err)| {
        eprintln!("error: cannot read {}: {err}", p.display());
        ExitCode::from(2)
    })?;
    let count: usize = checked.iter().map(|(_, d)| d.len()).sum();
    if count > 0 {
        for (file, diagnostics) in &checked {
            for diag in diagnostics {
                eprintln!("{}", diag.render(file));
            }
        }
        return Err(failed(count));
    }

    // The files of a folder share one namespace, so the compiler sees them
    // as one text. Diagnostics are mapped back to the file they point into.
    let kiln_files: Vec<&SourceFile> = checked
        .iter()
        .map(|(f, _)| f)
        .filter(|f| !is_glaze(Path::new(&f.name)))
        .collect();
    if kiln_files.is_empty() {
        eprintln!("error: there are no `.kiln` files in {}", dir.display());
        return Err(ExitCode::from(2));
    }
    let mut combined = String::new();
    let mut offsets = Vec::new();
    for file in &kiln_files {
        offsets.push(combined.len() as u32);
        combined.push_str(&file.text);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }
    let report = |diagnostics: Vec<Diagnostic>| {
        let count = diagnostics.len();
        for mut diag in diagnostics {
            let i = offsets
                .iter()
                .rposition(|&o| o <= diag.span.start)
                .unwrap_or(0);
            let shift = |s: &mut kiln_diagnostics::Span| {
                s.start = s.start.saturating_sub(offsets[i]);
                s.end = s.end.saturating_sub(offsets[i]);
            };
            shift(&mut diag.span);
            if let Some(s) = &mut diag.suggestion {
                shift(&mut s.span);
            }
            eprintln!("{}", diag.render(kiln_files[i]));
        }
        failed(count)
    };
    let parsed = kiln_syntax::parse(&combined);
    if !parsed.diagnostics.is_empty() {
        return Err(report(parsed.diagnostics));
    }
    let program = kiln_check::check_program(&parsed.module);
    if !program.diagnostics.is_empty() {
        return Err(report(program.diagnostics));
    }
    let write = |path: &Path, text: &str| {
        std::fs::write(path, text).map_err(|err| {
            eprintln!("error: cannot write {}: {err}", path.display());
            ExitCode::from(2)
        })
    };
    let ir = kiln_ir::lower(&parsed.module, &program).map_err(report)?;
    if let Some(ir_path) = emit.ir {
        write(ir_path, &ir.to_string())?;
    }
    let c = kiln_codegen::emit_c(&ir).map_err(report)?;
    if let Some(c_path) = emit.c {
        write(c_path, &c)?;
    }
    kiln_codegen::compile_c(&c, out).map_err(|err| {
        eprintln!("error: {err}");
        ExitCode::from(2)
    })
}

fn run(path: &Path, args: &[String]) -> ExitCode {
    let dir = match kiln_codegen::scratch_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };
    let exe = dir.join("program");
    if let Err(code) = build(path, &exe, Emit::default()) {
        let _ = std::fs::remove_dir_all(&dir);
        return code;
    }
    let status = std::process::Command::new(&exe).args(args).status();
    let _ = std::fs::remove_dir_all(&dir);
    match status {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(code.clamp(0, 255) as u8),
            None => ExitCode::FAILURE,
        },
        Err(err) => {
            eprintln!("error: cannot run the program: {err}");
            ExitCode::from(2)
        }
    }
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

pub(crate) fn is_glaze(path: &Path) -> bool {
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
