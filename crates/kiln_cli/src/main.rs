//! The `kiln` command: `check`, `build`, `run`, `lsp`, `parse`, `explain` and
//! `upgrade`.

mod lsp;
mod notice;
mod styles;
mod upgrade;

use clap::{Parser, Subcommand};
use kiln_diagnostics::{codes, Diagnostic, SourceFile, Span};
use kiln_native::Target;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "kiln", version, about = "The Kiln language toolchain")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// The code generators `build` and `run` can use.
#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Backend {
    /// Machine code from Cranelift, made into an executable by kiln
    /// itself.
    Native,
    /// C, compiled by the system's C compiler.
    C,
}

impl Backend {
    /// The native backend where this kiln carries the target's runtime,
    /// otherwise C.
    fn or_default(backend: Option<Backend>, target: Target) -> Backend {
        backend.unwrap_or(if kiln_native::unavailable(target).is_none() {
            Backend::Native
        } else {
            Backend::C
        })
    }
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
        /// How to generate code. `native` needs no outside tools; `c`
        /// needs a C compiler and the Boehm GC. Defaults to `native` where
        /// this kiln supports it.
        #[arg(long, value_enum)]
        backend: Option<Backend>,
        /// The system to build for: `linux` or `windows` (x86-64), or
        /// `wasm` for a WebAssembly module run by Node.js (22 or later),
        /// which tests the browser's backend. Defaults to the one kiln runs
        /// on. Only the native backend builds for other systems.
        #[arg(long, value_parser = parse_target)]
        target: Option<BuildTarget>,
    },
    /// Compile a program and run it.
    Run {
        /// The program's folder, or any file in it.
        file: PathBuf,
        /// How to generate code, as for `kiln build`.
        #[arg(long, value_enum)]
        backend: Option<Backend>,
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
    /// Replace this kiln with the latest release, or the one you name.
    Upgrade {
        /// The release to install, such as v0.2.0. Defaults to the latest.
        #[arg(long)]
        version: Option<String>,
        /// Only say whether a newer release is available.
        #[arg(long)]
        check: bool,
        /// Install the release even if this kiln is already that version.
        #[arg(long)]
        force: bool,
    },
    /// Record the latest release for the notice that a newer kiln is out.
    /// Other commands start it in the background once an hour.
    #[command(hide = true)]
    UpdateCheck,
}

fn main() -> ExitCode {
    let command = Cli::parse().command;
    // The language server talks to an editor, `upgrade` says itself what
    // is available, and JSON output is for programs.
    let notice = !matches!(
        command,
        Command::Lsp
            | Command::Upgrade { .. }
            | Command::UpdateCheck
            | Command::Check { json: true, .. }
    );
    let code = dispatch(command);
    if notice {
        notice::show();
    }
    code
}

fn dispatch(command: Command) -> ExitCode {
    match command {
        Command::Check { paths, json } => check(paths, json),
        Command::Build {
            file,
            output,
            emit_c,
            emit_ir,
            backend,
            target,
        } => {
            let emit = Emit {
                c: emit_c.as_deref(),
                ir: emit_ir.as_deref(),
                wasm: false,
            };
            let target = match target {
                Some(BuildTarget::Wasm) => {
                    let out = output.unwrap_or_else(|| {
                        default_output(&file, Target::Linux).with_extension("mjs")
                    });
                    let emit = Emit { wasm: true, ..emit };
                    return match build(&file, &out, emit, Backend::Native, Target::host()) {
                        Ok(()) => ExitCode::SUCCESS,
                        Err(code) => code,
                    };
                }
                Some(BuildTarget::System(t)) => t,
                None => Target::host(),
            };
            let out = output.unwrap_or_else(|| default_output(&file, target));
            let backend = Backend::or_default(backend, target);
            match build(&file, &out, emit, backend, target) {
                Ok(()) => ExitCode::SUCCESS,
                Err(code) => code,
            }
        }
        Command::Run {
            file,
            backend,
            args,
        } => run(&file, &args, Backend::or_default(backend, Target::host())),
        Command::Lsp => ExitCode::from(lsp::run() as u8),
        Command::Parse { file } => parse(&file),
        Command::Explain { code } => explain(&code),
        Command::Upgrade {
            version,
            check,
            force,
        } => upgrade::run(upgrade::Options {
            version,
            check,
            force,
        }),
        Command::UpdateCheck => notice::update_check(),
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
    check_app_sources(texts).0
}

/// What `kiln build` needs besides the checked files: what each glaze's
/// values are, and the app's CSS.
#[derive(Default)]
struct AppInfo {
    glaze_values: GlazeValues,
    glaze_states: Vec<Vec<(String, String)>>,
    styles: styles::AppStyles,
}

/// What the checker found each value a glaze outputs to be, for each glaze
/// in the order given.
type GlazeValues = Vec<HashMap<Span, kiln_check::ValueKind>>;

/// [`check_sources`], also returning what the compiler needs to know about
/// the glazes when the files have no errors. The CSS the glazes import is
/// read from the folder of the files.
fn check_app_sources(
    texts: Vec<(PathBuf, String)>,
) -> (Vec<(SourceFile, Vec<Diagnostic>)>, AppInfo) {
    let dir = texts
        .first()
        .and_then(|(p, _)| p.parent())
        .map(|p| {
            if p.as_os_str().is_empty() {
                Path::new(".")
            } else {
                p
            }
        })
        .unwrap_or(Path::new("."))
        .to_path_buf();
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
        let files = sources
            .into_iter()
            .map(|(_, file, p)| {
                let diags = parse_errors(&p);
                (file, diags)
            })
            .collect();
        return (files, AppInfo::default());
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
    let glaze_files: Vec<styles::GlazeFile> = sources
        .iter()
        .zip(&names)
        .filter_map(|((_, _, p), name)| match p {
            Parsed::Glaze(r) => Some(styles::GlazeFile {
                name,
                glaze: &r.glaze,
            }),
            Parsed::Kiln(_) => None,
        })
        .collect();
    let mut styles = styles::load(&dir, &glaze_files);
    let mut result = kiln_check::check_app_with_css(&modules, &glazes, &styles.styling);
    styles.finish(&kiln_check::style::stylesheet(
        &result.ui_rules,
        &styles.styling.tokens,
    ));
    let mut module_diags = result.modules.drain(..);
    let mut glaze_diags = result.glazes.drain(..);
    let mut out = Vec::new();
    for ((path, file, _), name) in sources.iter().zip(&names) {
        let diags = if is_glaze(path) {
            glaze_diags.next().map(|mut d| {
                d.extend(styles.diagnostics.remove(name).unwrap_or_default());
                d.sort_by_key(|d| d.span.start);
                d
            })
        } else {
            module_diags.next()
        };
        out.push((file.clone(), diags.unwrap_or_default()));
    }
    let info = AppInfo {
        glaze_values: result.glaze_values,
        glaze_states: result.glaze_states,
        styles,
    };
    (out, info)
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
fn parse_target(name: &str) -> Result<BuildTarget, String> {
    if name == "wasm" {
        return Ok(BuildTarget::Wasm);
    }
    Target::from_name(name)
        .map(BuildTarget::System)
        .ok_or_else(|| {
            let mut names: Vec<&str> = Target::ALL.iter().map(|t| t.name()).collect();
            names.push("wasm");
            format!("expected one of: {}", names.join(", "))
        })
}

/// What `kiln build --target` names.
#[derive(Clone, Copy)]
enum BuildTarget {
    System(Target),
    /// A WebAssembly module and its runtime, as one script for Node.js.
    Wasm,
}

/// The executable `kiln build` writes when no `--output` is given: the
/// program folder's name, with `.exe` for Windows.
fn default_output(path: &Path, target: Target) -> PathBuf {
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let name = dir
        .canonicalize()
        .ok()
        .and_then(|d| d.file_name().map(|n| n.to_owned()))
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "program".into());
    PathBuf::from(target.executable(&name))
}

/// Where `build` also writes the intermediate forms of a program.
#[derive(Clone, Copy, Default)]
struct Emit<'a> {
    c: Option<&'a Path>,
    ir: Option<&'a Path>,
    /// Build the program for WebAssembly instead, as a Node.js script.
    wasm: bool,
}

/// Checks and compiles the program in a folder into an executable at `out`.
fn build(
    path: &Path,
    out: &Path,
    emit: Emit,
    backend: Backend,
    target: Target,
) -> Result<(), ExitCode> {
    if backend == Backend::C && target != Target::host() {
        eprintln!(
            "error: the C backend builds only for the system kiln runs on ({}); use `--backend native` for {}",
            Target::host().name(),
            target.name()
        );
        return Err(ExitCode::from(2));
    }
    let (dir, files) = app_files(path).map_err(|err| {
        eprintln!("error: cannot read {}: {err}", path.display());
        ExitCode::from(2)
    })?;
    let failed = |count: usize| {
        eprintln!("{count} error(s). Run `kiln explain <code>` for details.");
        ExitCode::FAILURE
    };
    let mut texts = Vec::new();
    for path in &files {
        let text = std::fs::read_to_string(path).map_err(|err| {
            eprintln!("error: cannot read {}: {err}", path.display());
            ExitCode::from(2)
        })?;
        texts.push((path.clone(), text));
    }
    let (checked, info) = check_app_sources(texts);
    let glaze_values = &info.glaze_values;
    let glaze_states = &info.glaze_states;
    let styles = &info.styles;
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

    // Glazes are compiled into Kiln functions, which follow the files'
    // text. Errors in them are mapped back to the glazes.
    let glaze_files: Vec<&SourceFile> = checked
        .iter()
        .map(|(f, _)| f)
        .filter(|f| is_glaze(Path::new(&f.name)))
        .collect();
    let parsed_glazes: Vec<kiln_syntax::ast::Glaze> = glaze_files
        .iter()
        .map(|f| kiln_syntax::glaze::parse_glaze(&f.text).glaze)
        .collect();
    let glaze_names: Vec<String> = glaze_files
        .iter()
        .map(|f| {
            Path::new(&f.name)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    let inputs: Vec<kiln_ir::glaze::GlazeInput> = (0..glaze_files.len())
        .map(|i| kiln_ir::glaze::GlazeInput {
            file: &glaze_names[i],
            src: &glaze_files[i].text,
            glaze: &parsed_glazes[i],
            values: &glaze_values[i],
            scope: styles.scopes.get(&glaze_names[i]).map(String::as_str),
            modules: styles.exports.get(&glaze_names[i]),
            states: glaze_states.get(i).map_or(&[][..], Vec::as_slice),
        })
        .collect();
    let mut assets = styles.assets.clone();
    assets.extend(styles::static_files(&dir).map_err(|err| {
        eprintln!("error: {err}");
        ExitCode::from(2)
    })?);
    let generated_at = combined.len() as u32;
    let glaze_refs: Vec<(&str, &kiln_syntax::ast::Glaze)> = glaze_names
        .iter()
        .map(String::as_str)
        .zip(&parsed_glazes)
        .collect();
    let failed_in_glazes = |diagnostics: &[(usize, Diagnostic)]| {
        for (i, diag) in diagnostics {
            eprintln!("{}", diag.render(glaze_files[*i]));
        }
        failed(diagnostics.len())
    };

    // The embers run in the browser: their code, with the program's, is
    // compiled to WebAssembly, which the server serves with the bridge.
    let mut ember_script = None;
    let mut server_fns = Vec::new();
    let mut rpc_version = String::new();
    let mut rpc_guard = false;
    if parsed_glazes.iter().any(|g| g.is_ember()) {
        let user = kiln_syntax::parse(&combined);
        (server_fns, rpc_guard) = server_fns_of(&user.module, &combined);
        let client_app = kiln_ir::glaze::App {
            server_fns: &server_fns,
            ..Default::default()
        };
        let client = kiln_ir::glaze::generate_client(&inputs, &client_app);
        if !client.diagnostics.is_empty() {
            return Err(failed_in_glazes(&client.diagnostics));
        }
        let text = format!("{combined}{}", client.text);
        let report = |d| {
            report_diagnostics(
                d,
                &kiln_files,
                &offsets,
                &glaze_files,
                Some((&client, generated_at)),
            )
        };
        let parsed = kiln_syntax::parse(&text);
        if !parsed.diagnostics.is_empty() {
            return Err(report(parsed.diagnostics));
        }
        let program =
            kiln_check::check_browser_program(&parsed.module, &glaze_refs, &styles.styling.tokens);
        if !program.diagnostics.is_empty() {
            return Err(report(program.diagnostics));
        }
        let ir = kiln_ir::lower(&parsed.module, &program).map_err(report)?;
        let roots = kiln_ir::glaze::CLIENT_EXPORTS
            .iter()
            .filter_map(|name| {
                let id = ir.funcs.iter().position(|f| f.name == *name)?;
                Some((name.to_string(), id))
            })
            .collect();
        let module = kiln_wasm::compile(&ir, &kiln_wasm::Roots::Exports(roots)).map_err(report)?;
        // Calls stay valid while the functions' signatures do.
        rpc_version = kiln_css::short_hash(format!("{server_fns:?}").as_bytes());
        let wasm_url = format!("/_kiln/ember-{}.wasm", kiln_css::short_hash(&module.wasm));
        let js = kiln_wasm::browser_script(&wasm_url, &rpc_version, &[]);
        let js_url = format!("/_kiln/ember-{}.js", kiln_css::short_hash(js.as_bytes()));
        assets.push(kiln_ir::glaze::Asset {
            path: wasm_url,
            content_type: "application/wasm",
            body: module.wasm,
            immutable: true,
        });
        assets.push(kiln_ir::glaze::Asset {
            path: js_url.clone(),
            content_type: "text/javascript; charset=utf-8",
            body: js.into_bytes(),
            immutable: true,
        });
        ember_script = Some(js_url);
    }

    let app = kiln_ir::glaze::App {
        stylesheet: styles.stylesheet.as_deref(),
        assets: &assets,
        ember_script: ember_script.as_deref(),
        server_fns: &server_fns,
        rpc_version: &rpc_version,
        rpc_guard,
    };
    let generated = kiln_ir::glaze::generate(&inputs, &app);
    if let Some(g) = &generated {
        if !g.diagnostics.is_empty() {
            return Err(failed_in_glazes(&g.diagnostics));
        }
        combined.push_str(&g.text);
    }
    let report = |diagnostics: Vec<Diagnostic>| {
        report_diagnostics(
            diagnostics,
            &kiln_files,
            &offsets,
            &glaze_files,
            generated.as_ref().map(|g| (g, generated_at)),
        )
    };
    let parsed = kiln_syntax::parse(&combined);
    if !parsed.diagnostics.is_empty() {
        return Err(report(parsed.diagnostics));
    }
    let program =
        kiln_check::check_program_with_tokens(&parsed.module, &glaze_refs, &styles.styling.tokens);
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
    if emit.wasm {
        let module = kiln_wasm::compile(&ir, &kiln_wasm::Roots::Main).map_err(report)?;
        return write(out, &kiln_wasm::node_runner(&module));
    }
    let c = match (backend, emit.c) {
        (Backend::C, _) | (_, Some(_)) => Some(kiln_codegen::emit_c(&ir).map_err(report)?),
        _ => None,
    };
    if let (Some(c_path), Some(c)) = (emit.c, &c) {
        write(c_path, c)?;
    }
    let linked = match (backend, c) {
        (Backend::C, Some(c)) => kiln_codegen::compile_c(&c, out),
        _ => {
            let object = kiln_native::compile(&ir, target).map_err(report)?;
            kiln_native::link(&object, target, out)
        }
    };
    linked.map_err(|err| {
        eprintln!("error: {err}");
        ExitCode::from(2)
    })
}

/// Prints diagnostics of a program's combined text, each in the file it
/// points into: a `.kiln` file (which start at `offsets`), or a glaze
/// whose code was generated at the offset given with `generated`.
fn report_diagnostics(
    diagnostics: Vec<Diagnostic>,
    kiln_files: &[&SourceFile],
    offsets: &[u32],
    glaze_files: &[&SourceFile],
    generated: Option<(&kiln_ir::glaze::GlazeSource, u32)>,
) -> ExitCode {
    let count = diagnostics.len();
    for mut diag in diagnostics {
        if let Some((g, generated_at)) = generated {
            if diag.span.start >= generated_at {
                diag.span = Span {
                    start: diag.span.start - generated_at,
                    end: diag.span.end.saturating_sub(generated_at),
                };
                diag.suggestion = None;
                match g.locate(diag.span) {
                    Some((i, span)) => {
                        diag.span = span;
                        eprintln!("{}", diag.render(glaze_files[i]));
                    }
                    None => {
                        let file = SourceFile::new("<glazes>", g.text.clone());
                        eprintln!("{}", diag.render(&file));
                    }
                }
                continue;
            }
        }
        let i = offsets
            .iter()
            .rposition(|&o| o <= diag.span.start)
            .unwrap_or(0);
        let shift = |s: &mut Span| {
            s.start = s.start.saturating_sub(offsets[i]);
            s.end = s.end.saturating_sub(offsets[i]);
        };
        shift(&mut diag.span);
        if let Some(s) = &mut diag.suggestion {
            shift(&mut s.span);
        }
        eprintln!("{}", diag.render(kiln_files[i]));
    }
    eprintln!("{count} error(s). Run `kiln explain <code>` for details.");
    ExitCode::FAILURE
}

fn run(path: &Path, args: &[String], backend: Backend) -> ExitCode {
    let dir = match kiln_codegen::scratch_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };
    let exe = dir.join(Target::host().executable("program"));
    if let Err(code) = build(path, &exe, Emit::default(), backend, Target::host()) {
        let _ = std::fs::remove_dir_all(&dir);
        return code;
    }
    let status = std::process::Command::new(&exe).args(args).status();
    let _ = std::fs::remove_dir_all(&dir);
    match status {
        Ok(status) => match status.code() {
            Some(code) => ExitCode::from(code.clamp(0, 255) as u8),
            None => {
                report_signal(&status);
                ExitCode::FAILURE
            }
        },
        Err(err) => {
            eprintln!("error: cannot run the program: {err}");
            ExitCode::from(2)
        }
    }
}

/// Explains a program that was stopped by a signal, which otherwise ends
/// without a word.
fn report_signal(status: &std::process::ExitStatus) {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        let Some(signal) = status.signal() else {
            return;
        };
        let detail = match signal {
            11 => " (a crash: a stack overflow from deep recursion, or a compiler bug; please report it)",
            9 => " (killed)",
            6 => " (aborted)",
            _ => "",
        };
        eprintln!("error: the program was stopped by signal {signal}{detail}");
    }
    #[cfg(not(unix))]
    eprintln!("error: the program stopped abnormally ({status})");
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

/// The program's `server fn`s, as written in `text`, and whether it
/// defines `rpc_guard`, which every call of one passes first.
fn server_fns_of(
    module: &kiln_syntax::ast::Module,
    text: &str,
) -> (Vec<kiln_ir::glaze::ServerFn>, bool) {
    use kiln_syntax::ast::{Item, Throws};
    let at = |s: Span| text[s.start as usize..s.end as usize].to_string();
    let mut fns = Vec::new();
    let mut guard = false;
    for item in &module.items {
        let Item::Fn(f) = item else { continue };
        guard |= f.name.name == "rpc_guard";
        if !f.is_server {
            continue;
        }
        fns.push(kiln_ir::glaze::ServerFn {
            module: None,
            name: f.name.name.clone(),
            params: f
                .params
                .iter()
                .map(|p| {
                    (
                        p.name.name.clone(),
                        p.ty.as_ref().map_or_else(|| "_".into(), |t| at(t.span)),
                        p.default.as_ref().map(|d| at(d.span)),
                    )
                })
                .collect(),
            ret: f.ret.as_ref().map(|t| at(t.span)),
            throws: !matches!(f.throws, Throws::Never),
        });
    }
    (fns, guard)
}
