//! The `kiln` command: `check`, `build`, `run`, `lsp`, `parse`, `explain` and
//! `upgrade`.

mod db;
mod dev;
mod fmt;
mod lsp;
mod mcp;
mod notice;
mod npm;
mod project;
mod styles;
mod testing;
mod upgrade;
mod ware;

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
    /// Use kiln.lock as it is: fail instead of updating it.
    #[arg(long, global = true)]
    locked: bool,
    /// Make no network requests: Wares come from kiln.lock and the cache.
    #[arg(long, global = true)]
    offline: bool,
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
        /// `.kiln`, `.glaze` and `.ember` files. Defaults to the current directory.
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
    /// Run the project's tests: `fn test_*()` in files named `test_*.kiln`.
    /// Tests under `tests/integration/` and `tests/browser/` can start the
    /// app, use a database of their own and drive a real browser.
    Test {
        /// The project. Defaults to the current directory.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Only unit tests (any kind flags combine).
        #[arg(long)]
        unit: bool,
        /// Only integration tests.
        #[arg(long)]
        integration: bool,
        /// Only browser tests.
        #[arg(long)]
        browser: bool,
        /// Only tests whose name contains this, such as `login`.
        #[arg(long)]
        filter: Option<String>,
        /// Print the results as one JSON document (version 1).
        #[arg(long)]
        json: bool,
        /// List the tests without running them.
        #[arg(long)]
        list: bool,
        /// Also write the results as JUnit XML.
        #[arg(long)]
        junit: Option<PathBuf>,
        /// Run the tests again whenever a file of the project changes.
        #[arg(long)]
        watch: bool,
        /// How many tests run at once. Defaults to the number of CPUs.
        #[arg(long)]
        jobs: Option<usize>,
        /// Seconds a test may take. Defaults to `[test] timeout`, or 60.
        #[arg(long)]
        timeout: Option<u64>,
        /// Run a failing test up to this many more times; a test that
        /// passes on a retry is reported as flaky.
        #[arg(long, default_value_t = 0)]
        retries: u32,
        /// Show the browser instead of running it headless.
        #[arg(long)]
        headed: bool,
        /// Download the pinned browser that browser tests use, and stop.
        #[arg(long)]
        install_browser: bool,
        /// How to generate code, as for `kiln build`.
        #[arg(long, value_enum)]
        backend: Option<Backend>,
    },
    /// Format `.kiln` files, and the code in glazes' `{% %}` tags. Markup
    /// and CSS are left as they are.
    Fmt {
        /// Files or folders. Defaults to the current directory.
        paths: Vec<PathBuf>,
        /// Change nothing: list the files that are not formatted, and fail
        /// if there are any.
        #[arg(long)]
        check: bool,
    },
    /// Run a program's server while you work on it: kiln builds again and
    /// restarts it when a file changes, keeps the last good build running
    /// when a build fails, and reloads open pages.
    Dev {
        /// The program's folder. Defaults to the current directory.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// The port pages are served on.
        #[arg(long, default_value_t = 8080)]
        port: u16,
        /// The address to listen on. Only this machine can reach the
        /// default.
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// How to generate code, as for `kiln build`.
        #[arg(long, value_enum)]
        backend: Option<Backend>,
    },
    /// Run the language server, for editors, over standard input and output.
    Lsp,
    /// Run a Model Context Protocol server over standard input and output,
    /// so a coding agent can read the project graph, check, explain, format
    /// and (when allowed) run tests with this compiler.
    Mcp {
        /// A folder the agent may read. Defaults to the current directory;
        /// give it more than once for more folders.
        #[arg(long = "root")]
        roots: Vec<PathBuf>,
        /// Let the agent run the project's tests, which executes its code.
        #[arg(long)]
        allow_exec: bool,
        /// Let the agent write files: format them, and build executables
        /// into `.kiln/mcp/`.
        #[arg(long)]
        allow_write: bool,
    },
    /// Print Kiln's documentation for this version.
    Docs {
        /// The guide for coding agents, as one Markdown document.
        #[arg(long)]
        llm: bool,
    },
    /// Print the syntax tree of a file.
    Parse { file: PathBuf },
    /// Explain an error code, with a wrong and a right example.
    Explain {
        /// An error code such as E0106.
        code: String,
    },
    /// Add a Ware, such as `kiln/ui` or `kiln/ui@0.1`, to kiln.toml and
    /// kiln.lock; or an npm package for embers to call, such as
    /// `npm:nanoid@5.0.9`, whose files are kept under `npm/` with `npm.lock`.
    Add {
        /// `publisher/name[@version]`, or `npm:name@version`.
        package: String,
        /// The program's folder. Defaults to the current directory.
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// A version requirement, such as `0.2` or `>=1.1, <2`.
        #[arg(long)]
        version: Option<String>,
        /// A Ware in a local folder, whose source is used as it is.
        #[arg(long)]
        path: Option<PathBuf>,
        /// A Ware in a Git repository.
        #[arg(long)]
        git: Option<String>,
        /// With `--git`: a commit (40 hex digits).
        #[arg(long)]
        rev: Option<String>,
        /// With `--git`: a branch, locked at its current commit.
        #[arg(long)]
        branch: Option<String>,
        /// With `--git`: a tag, locked at its commit.
        #[arg(long)]
        tag: Option<String>,
        /// The registry, if not the one kiln.toml names.
        #[arg(long)]
        registry: Option<String>,
        /// The name to import it by. Defaults to the Ware's own name.
        #[arg(long = "as")]
        alias: Option<String>,
        /// Accept Wares that need capabilities without asking.
        #[arg(long)]
        allow_capabilities: bool,
    },
    /// Remove a Ware from kiln.toml and kiln.lock.
    Remove {
        /// The name it is imported by, or `publisher/name`.
        ware: String,
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Choose the newest releases kiln.toml allows and write kiln.lock.
    /// (`kiln upgrade` upgrades kiln itself.)
    Update {
        /// Only these Wares; defaults to all.
        wares: Vec<String>,
        #[arg(long, default_value = ".")]
        dir: PathBuf,
        /// Accept Wares that gain capabilities without asking.
        #[arg(long)]
        allow_capabilities: bool,
    },
    /// Show the Wares kiln.lock holds, and why each is there.
    Tree {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// Show the dependency chains that bring in this Ware.
        #[arg(long)]
        why: Option<String>,
    },
    /// Download every locked Ware into the cache, for offline builds.
    Fetch {
        #[arg(default_value = ".")]
        dir: PathBuf,
    },
    /// Publish the Ware in a folder to a registry.
    Publish {
        #[arg(default_value = ".")]
        dir: PathBuf,
        /// The registry. Defaults to the one kiln.toml names.
        #[arg(long)]
        registry: Option<String>,
        /// Show what would be published, and send nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Save a registry's publish token, read from standard input.
    Login {
        #[arg(long)]
        registry: String,
    },
    /// Stop new resolutions from choosing a release (`publisher/name@1.2.3`).
    /// Lockfiles that already hold it keep working.
    Yank {
        release: String,
        #[arg(long)]
        registry: Option<String>,
        /// Make the release choosable again.
        #[arg(long)]
        undo: bool,
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// Look after the per-user Ware cache.
    Cache {
        #[command(subcommand)]
        action: CacheCommand,
    },
    /// Copy components of kiln/ui into your project, to own and change.
    Ui {
        #[command(subcommand)]
        action: UiCommand,
    },
    /// Make a new web app in a new folder: a manifest, a page, a module and
    /// tests. Nothing is fetched or run.
    New { dir: PathBuf },
    /// Check a locked Ware's source against kiln.lock and its capabilities.
    Ware {
        #[command(subcommand)]
        action: WareCommand,
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
    /// Work with the database a program's models describe: its schema,
    /// its migrations and the models of a database that already exists.
    Db {
        #[command(subcommand)]
        action: DbCommand,
        /// The program's folder. Defaults to the current directory.
        #[arg(long, global = true, default_value = ".")]
        path: PathBuf,
        /// The database, such as `sqlite:./data/app.db`. Defaults to
        /// `DATABASE_URL`.
        #[arg(long, global = true)]
        url: Option<String>,
    },
    /// Record the latest release for the notice that a newer kiln is out.
    /// Other commands start it in the background once an hour.
    #[command(hide = true)]
    UpdateCheck,
}

#[derive(Subcommand)]
enum CacheCommand {
    /// Print where the cache is.
    Dir,
    /// How much it holds.
    Stats,
    /// Check every cached Ware against its checksum.
    Verify,
    /// Remove Wares not used for a while, and leftovers of interrupted
    /// downloads.
    Prune {
        /// Show what would be removed.
        #[arg(long)]
        dry_run: bool,
        /// Remove Wares unused for this many days.
        #[arg(long, default_value_t = 30)]
        older_than: u64,
    },
}

#[derive(Subcommand)]
enum WareCommand {
    /// Check a locked Ware: its checksum, and its declared capabilities.
    Verify {
        ware: String,
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum UiCommand {
    /// Copy components, such as `button input`, or `all`. Files that exist
    /// are never overwritten.
    Add {
        #[arg(required = true)]
        components: Vec<String>,
        /// The folder module to copy them into.
        #[arg(long, default_value = "components")]
        into: String,
        #[arg(long, default_value = ".")]
        dir: PathBuf,
    },
    /// List the components.
    List,
}

#[derive(Subcommand)]
enum DbCommand {
    /// Show which migrations have been applied, and whether the database
    /// still matches them.
    Status,
    /// Write a migration for what the models changed since the last one.
    /// It is SQL to read, not to run: nothing is applied.
    Diff {
        /// What the migration does, such as `add_post_summary`.
        #[arg(long)]
        name: String,
        /// Write a migration that drops or rewrites data.
        #[arg(long)]
        allow_destructive: bool,
    },
    /// Apply the migrations that have not run yet, in order.
    Migrate {
        /// Print what would run, and change nothing.
        #[arg(long)]
        dry_run: bool,
    },
    /// Check that the migrations are whole and readable.
    Verify,
    /// Write the models of a database that already exists.
    Pull {
        /// Where to write them. Defaults to `models.kiln` in the folder.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Only report what the database has that the models do not.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> ExitCode {
    kiln_diagnostics::set_path_display(project::shown);
    let cli = Cli::parse();
    ware::set_mode(ware::Mode {
        locked: cli.locked,
        offline: cli.offline,
        ..Default::default()
    });
    let command = cli.command;
    // The language server talks to an editor, `upgrade` says itself what
    // is available, and JSON output is for programs.
    let notice = !matches!(
        command,
        Command::Lsp
            | Command::Mcp { .. }
            | Command::Docs { .. }
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

/// The guide for coding agents, as this version of kiln knows it.
pub(crate) const AGENT_GUIDE: &str = include_str!("../../../docs/guides/agents.md");

fn dispatch(command: Command) -> ExitCode {
    match command {
        Command::Check { paths, json } => check(paths, json),
        Command::Fmt { paths, check } => fmt::run(paths, check),
        Command::Dev {
            dir,
            port,
            host,
            backend,
        } => {
            let target = Target::host();
            let backend = Backend::or_default(backend, target);
            let builder = move |p: &project::Project, out: &Path| {
                build_project(p, out, Emit::default(), backend, target).is_ok()
            };
            dev::run(dev::Options { dir, host, port }, &builder)
        }
        Command::Test {
            dir,
            unit,
            integration,
            browser,
            filter,
            json,
            list,
            junit,
            watch,
            jobs,
            timeout,
            retries,
            headed,
            install_browser,
            backend,
        } => {
            if install_browser {
                return report(testing::browser::install());
            }
            let mut kinds = Vec::new();
            for (on, kind) in [
                (unit, testing::Kind::Unit),
                (integration, testing::Kind::Integration),
                (browser, testing::Kind::Browser),
            ] {
                if on {
                    kinds.push(kind);
                }
            }
            let config = project::root(&dir)
                .ok()
                .and_then(|r| ware::manifest::Manifest::load(&r).ok().flatten())
                .map(|m| m.test)
                .unwrap_or_default();
            let options = testing::Options {
                path: dir,
                kinds,
                filter,
                json,
                list,
                junit,
                watch,
                jobs: jobs
                    .unwrap_or_else(|| std::thread::available_parallelism().map_or(2, |n| n.get())),
                timeout: std::time::Duration::from_secs(timeout.or(config.timeout).unwrap_or(60)),
                retries,
                headed,
                env: config.env,
            };
            let target = Target::host();
            let backend = Backend::or_default(backend, target);
            let builder = move |p: &project::Project, out: &Path| {
                build_project(p, out, Emit::default(), backend, target).is_ok()
            };
            testing::run(&options, &builder)
        }
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
        Command::Mcp {
            roots,
            allow_exec,
            allow_write,
        } => mcp::run(mcp::Options {
            roots: if roots.is_empty() {
                vec![PathBuf::from(".")]
            } else {
                roots
            },
            allow_exec,
            allow_write,
        }),
        Command::Docs { llm } => {
            if llm {
                print!("{AGENT_GUIDE}");
            } else {
                println!("Kiln {}: the documentation is at https://github.com/IZ737/kiln/tree/main/docs; `kiln docs --llm` prints the guide for coding agents.", env!("CARGO_PKG_VERSION"));
            }
            ExitCode::SUCCESS
        }
        Command::Parse { file } => parse(&file),
        Command::Explain { code } => explain(&code),
        Command::Add {
            package,
            dir,
            version,
            path,
            git,
            rev,
            branch,
            tag,
            registry,
            alias,
            allow_capabilities,
        } => report(if package.starts_with("npm:") {
            npm::add(&package, &dir)
        } else {
            ware::commands::add(
                &package,
                &dir,
                ware::commands::AddOptions {
                    version,
                    path,
                    git,
                    rev,
                    branch,
                    tag,
                    registry,
                    alias,
                },
                allow_capabilities,
            )
        }),
        Command::Remove { ware: w, dir } => report(ware::commands::remove(&w, &dir)),
        Command::Update {
            wares,
            dir,
            allow_capabilities,
        } => report(ware::commands::update(&wares, &dir, allow_capabilities)),
        Command::Tree { dir, why } => report(ware::commands::tree(&dir, why.as_deref())),
        Command::Fetch { dir } => report(ware::commands::fetch(&dir)),
        Command::Publish {
            dir,
            registry,
            dry_run,
        } => report(ware::commands::publish(&dir, registry.as_deref(), dry_run)),
        Command::Login { registry } => report(ware::commands::login(&registry)),
        Command::Yank {
            release,
            registry,
            undo,
            dir,
        } => report(ware::commands::yank(
            &release,
            registry.as_deref(),
            &dir,
            undo,
        )),
        Command::Cache { action } => report(ware::commands::cache_command(match action {
            CacheCommand::Dir => ware::commands::CacheAction::Dir,
            CacheCommand::Stats => ware::commands::CacheAction::Stats,
            CacheCommand::Verify => ware::commands::CacheAction::Verify,
            CacheCommand::Prune {
                dry_run,
                older_than,
            } => ware::commands::CacheAction::Prune {
                dry_run,
                days: older_than,
            },
        })),
        Command::Ui {
            action:
                UiCommand::Add {
                    components,
                    into,
                    dir,
                },
        } => report(ware::ui::add(&dir, &into, &components)),
        Command::Ui {
            action: UiCommand::List,
        } => report(Ok(ware::ui::names().join("\n"))),
        Command::New { dir } => report(ware::ui::new_project(&dir)),
        Command::Ware {
            action: WareCommand::Verify { ware: w, dir },
        } => report(ware::commands::verify_ware(&w, &dir)),
        Command::Upgrade {
            version,
            check,
            force,
        } => upgrade::run(upgrade::Options {
            version,
            check,
            force,
        }),
        Command::Db { action, path, url } => {
            let action = match action {
                DbCommand::Status => db::Action::Status,
                DbCommand::Diff {
                    name,
                    allow_destructive,
                } => db::Action::Diff {
                    name,
                    allow_destructive,
                },
                DbCommand::Migrate { dry_run } => db::Action::Migrate { dry_run },
                DbCommand::Verify => db::Action::Verify,
                DbCommand::Pull { out, check } => db::Action::Pull { out, check },
            };
            db::run(action, &path, url)
        }
        Command::UpdateCheck => notice::update_check(),
    }
}

/// Prints a command's summary, or its error.
fn report(result: Result<String, String>) -> ExitCode {
    match result {
        Ok(summary) => {
            println!("{summary}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

fn check(paths: Vec<PathBuf>, json: bool) -> ExitCode {
    let report = match check_paths(paths) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };
    if json {
        let diagnostics: Vec<_> = report
            .diagnostics
            .iter()
            .map(|(file, diag)| diag.to_json(file))
            .collect();
        let out = serde_json::json!({
            "files_checked": report.files_checked,
            "error_count": report.diagnostics.len(),
            "diagnostics": diagnostics,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    } else {
        for (file, diag) in &report.diagnostics {
            eprintln!("{}", diag.render(file));
        }
        if report.diagnostics.is_empty() {
            eprintln!("checked {} file(s): no errors", report.files_checked);
        } else {
            eprintln!(
                "checked {} file(s): {} error(s). Run `kiln explain <code>` for details.",
                report.files_checked,
                report.diagnostics.len()
            );
        }
    }
    if report.diagnostics.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

/// What `kiln check` found: how many files it read, and each error with
/// its file.
pub(crate) struct CheckReport {
    pub files_checked: usize,
    pub diagnostics: Vec<(SourceFile, Diagnostic)>,
}

/// Checks files and folders as `kiln check` does, for it and `kiln mcp`.
pub(crate) fn check_paths(paths: Vec<PathBuf>) -> Result<CheckReport, String> {
    let paths = if paths.is_empty() {
        vec![PathBuf::from(".")]
    } else {
        paths
    };
    let mut files = Vec::new();
    for path in &paths {
        collect_sources(path, &mut files)
            .map_err(|err| format!("cannot read {}: {err}", path.display()))?;
    }
    files.sort();
    // Tests are built by `kiln test`, with the module they test and the
    // harness, so a folder of them alone is not a program to check.
    let named: Vec<&PathBuf> = paths.iter().filter(|p| p.is_file()).collect();
    files.retain(|f| {
        named.contains(&f)
            || !(f.extension().is_some_and(|e| e == "kiln")
                && f.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with("test_")))
    });
    // Files are type-checked with the other files in their directory,
    // which form one app: glazes use the types of the `.kiln` files there.
    let mut dirs: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for path in &files {
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        dirs.entry(dir).or_default().push(path.clone());
    }
    let selected: Vec<_> = paths
        .iter()
        .filter(|p| {
            p.is_dir()
                && (p.join("kiln.toml").is_file()
                    || std::fs::read_dir(p).is_ok_and(|mut entries| {
                        entries.any(|e| {
                            e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "kiln"))
                        })
                    }))
        })
        .cloned()
        .collect();
    dirs.retain(|dir, _| {
        !selected
            .iter()
            .any(|root| dir != root && dir.starts_with(root))
    });
    for root in selected {
        dirs.entry(root).or_default();
    }
    let mut found = Vec::new();
    let mut visited = std::collections::HashSet::new();
    for dir in dirs.keys() {
        let canonical = dir.canonicalize().unwrap_or_else(|_| dir.clone());
        if visited.contains(&canonical) {
            continue;
        }
        let target = paths
            .iter()
            .find(|p| {
                p.is_file()
                    && p.parent()
                        .is_some_and(|parent| parent.canonicalize().is_ok_and(|p| p == canonical))
            })
            .unwrap_or(dir);
        let project = match project::discover(target) {
            Ok(project) => project,
            Err(err) => {
                let (file, diag) = project::error_diagnostic(&err, &dir.join("kiln.toml"));
                found.push((file, diag));
                continue;
            }
        };
        for file in &project.files {
            if let Some(parent) = file.parent() {
                visited.insert(parent.to_path_buf());
            }
        }
        let texts = project
            .files
            .iter()
            .map(|p| project::read(p).map(|text| (p.clone(), text)))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|err| err.to_string())?;
        let app = check_project_sources_entry(&project.root, &project.entry, texts).0;
        for (file, diagnostics) in app {
            if paths.iter().all(|p| p.is_file())
                && Path::new(&file.name).parent() == Some(canonical.as_path())
                && !paths
                    .iter()
                    .any(|p| p.canonicalize().is_ok_and(|p| p == Path::new(&file.name)))
            {
                continue;
            }
            for diag in diagnostics {
                found.push((file.clone(), diag));
            }
        }
    }
    Ok(CheckReport {
        files_checked: files.len(),
        diagnostics: found,
    })
}

enum Parsed {
    Kiln(kiln_syntax::ParseResult),
    Glaze(Box<kiln_syntax::glaze::GlazeResult>),
}

/// Parses and type-checks the files of one directory, given their text.
pub(crate) fn check_sources(texts: Vec<(PathBuf, String)>) -> Vec<(SourceFile, Vec<Diagnostic>)> {
    let root = project::common_root(texts.iter().map(|(p, _)| p.as_path()));
    let overlays = texts.iter().cloned().collect();
    if root.is_dir() {
        if let Err(error) = project::discover_overlay(&root, &overlays) {
            let (file, diag) = project::error_diagnostic(&error, &root.join("kiln.toml"));
            let target = texts
                .iter()
                .position(|(p, _)| p.display().to_string() == file.name)
                .unwrap_or(0);
            return texts
                .into_iter()
                .enumerate()
                .map(|(i, (p, t))| {
                    (
                        SourceFile::new(p.display().to_string(), t),
                        if i == target {
                            vec![diag.clone()]
                        } else {
                            Vec::new()
                        },
                    )
                })
                .collect();
        }
    }
    check_project_sources(&root, texts).0
}

/// What `kiln build` needs besides the checked files: what each glaze's
/// values are, and the app's CSS.
#[derive(Default)]
struct AppInfo {
    glaze_values: GlazeValues,
    glaze_states: Vec<Vec<(String, String)>>,
    styles: styles::AppStyles,
    /// The npm packages of `npm.lock`, or why they cannot be used.
    npm: Vec<npm::Package>,
    npm_error: Option<String>,
    source_maps: HashMap<String, (SourceFile, project::bind::SourceMap)>,
    symbol_names: BTreeMap<String, String>,
}

/// What the checker found each value a glaze outputs to be, for each glaze
/// in the order given.
type GlazeValues = Vec<HashMap<Span, kiln_check::ValueKind>>;

fn check_project_sources(
    root: &Path,
    texts: Vec<(PathBuf, String)>,
) -> (Vec<(SourceFile, Vec<Diagnostic>)>, AppInfo) {
    check_project_sources_entry(root, root, texts)
}
fn check_project_sources_entry(
    root: &Path,
    entry: &Path,
    texts: Vec<(PathBuf, String)>,
) -> (Vec<(SourceFile, Vec<Diagnostic>)>, AppInfo) {
    let bound = project::bind::bind_entry(root, entry, &texts);
    if bound.diagnostics.iter().any(|d| !d.is_empty()) {
        return (
            texts
                .into_iter()
                .zip(bound.diagnostics)
                .map(|((p, t), d)| (SourceFile::new(p.display().to_string(), t), d))
                .collect(),
            AppInfo::default(),
        );
    }
    let (mut checked, mut info) = check_app_sources_at(root, bound.texts);
    info.source_maps = texts
        .iter()
        .zip(&bound.maps)
        .map(|((p, t), m)| {
            (
                p.display().to_string(),
                (
                    SourceFile::new(p.display().to_string(), t.clone()),
                    m.clone(),
                ),
            )
        })
        .collect();
    info.symbol_names = bound.names;

    for (i, (file, diagnostics)) in checked.iter_mut().enumerate() {
        if diagnostics.is_empty() {
            continue;
        }
        for diagnostic in diagnostics {
            diagnostic.message = display_symbols(&diagnostic.message, &info.symbol_names);
            diagnostic.span = bound.maps[i].original(diagnostic.span);
            if let Some(s) = &mut diagnostic.suggestion {
                s.span = bound.maps[i].original(s.span);
            }
        }
        *file = SourceFile::new(texts[i].0.display().to_string(), texts[i].1.clone());
    }
    (checked, info)
}

/// [`check_sources`], also returning what the compiler needs to know about
/// the glazes when the files have no errors. The CSS the glazes import is
/// read from the folder of the files.
fn check_app_sources_at(
    dir: &Path,
    texts: Vec<(PathBuf, String)>,
) -> (Vec<(SourceFile, Vec<Diagnostic>)>, AppInfo) {
    // The npm packages are modules for the embers' handlers.
    let (npm, npm_error) = match npm::load(dir) {
        Ok(packages) => (packages, None),
        Err(err) => (Vec::new(), Some(err)),
    };
    kiln_check::set_foreign_modules(npm.iter().map(|p| p.bindings.clone()).collect());
    let mut sources = Vec::new();
    for (path, text) in &texts {
        let file = SourceFile::new(path.display().to_string(), text.clone());
        let parsed = if is_glaze(path) {
            Parsed::Glaze(Box::new(kiln_syntax::glaze::parse_component(
                &file.text, &file.name,
            )))
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
            path.strip_prefix(dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
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
    let mut styles = styles::load(dir, &glaze_files);
    let mut result = kiln_check::check_app_with_css(&modules, &glazes, &styles.styling);
    project::check_const_exports(&modules, &glazes, &styles.styling, &mut result);
    let owners: Vec<_> = sources
        .iter()
        .filter(|(p, _, _)| !is_glaze(p))
        .map(|(p, _, _)| p.parent().unwrap_or(dir))
        .collect();
    let glaze_owners: Vec<_> = sources
        .iter()
        .filter(|(p, _, _)| is_glaze(p))
        .map(|(p, _, _)| p.parent().unwrap_or(dir))
        .collect();
    project::check_method_access(
        &modules,
        &owners,
        &glazes,
        &glaze_owners,
        &styles.styling,
        &mut result,
    );

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
        npm,
        npm_error,
        source_maps: HashMap::new(),
        symbol_names: BTreeMap::new(),
    };
    (out, info)
}

/// Checks a program and returns what the checker made of it, for the
/// tools that read a program rather than build it, such as `kiln db`.
/// Glazes are left out: they hold no models.
fn check_program_of(path: &Path) -> Result<(kiln_check::Program, PathBuf), String> {
    let project = project::discover(path).map_err(|err| {
        let (file, diag) = project::error_diagnostic(&err, &path.join("kiln.toml"));
        diag.render(&file)
    })?;
    let mut texts = Vec::new();
    for file in &project.files {
        let text =
            project::read(file).map_err(|err| format!("cannot read {}: {err}", file.display()))?;
        texts.push((file.clone(), text));
    }
    let bound = project::bind::bind_entry(&project.root, &project.entry, &texts);
    let broken = "the program has errors: run `kiln check` and fix them first";
    if bound.diagnostics.iter().any(|d| !d.is_empty()) {
        return Err(broken.into());
    }
    let mut combined = String::new();
    for ((path, _), (_, text)) in texts.iter().zip(&bound.texts) {
        if is_glaze(path) {
            continue;
        }
        combined.push_str(text);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }
    let parsed = kiln_syntax::parse(&combined);
    if !parsed.diagnostics.is_empty() {
        return Err(broken.into());
    }
    let program = kiln_check::check_program(&parsed.module);
    if !program.diagnostics.is_empty() {
        return Err(broken.into());
    }
    Ok((program, project.root))
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
    let project = project::discover(path).map_err(|err| {
        let (file, diag) = project::error_diagnostic(&err, &path.join("kiln.toml"));
        eprintln!("{}", diag.render(&file));
        ExitCode::from(2)
    })?;
    build_project(&project, out, emit, backend, target)
}

/// [`build`], for a project already found, such as a test harness.
fn build_project(
    project: &project::Project,
    out: &Path,
    emit: Emit,
    backend: Backend,
    target: Target,
) -> Result<(), ExitCode> {
    let dir = project.root.clone();
    let files = project.files.clone();
    let failed = |count: usize| {
        eprintln!("{count} error(s). Run `kiln explain <code>` for details.");
        ExitCode::FAILURE
    };
    let mut texts = Vec::new();
    for path in &files {
        let text = project::read(path).map_err(|err| {
            eprintln!("error: cannot read {}: {err}", path.display());
            ExitCode::from(2)
        })?;
        texts.push((path.clone(), text));
    }
    let (checked, info) = check_project_sources_entry(&dir, &project.entry, texts);
    let glaze_values = &info.glaze_values;
    let glaze_states = &info.glaze_states;
    let styles = &info.styles;
    if let Some(err) = &info.npm_error {
        eprintln!("error: {err}");
        return Err(ExitCode::from(2));
    }
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
    let mut kiln_files = kiln_files;
    for file in &kiln_files {
        offsets.push(combined.len() as u32);
        combined.push_str(&file.text);
        if !combined.ends_with('\n') {
            combined.push('\n');
        }
    }

    // The admin's pages are written from the models the program asks for
    // with `admin.pages`, and are part of its own text, before the
    // glazes. Errors in them point at a text of their own.
    let admin = kiln_ir::admin::generate(&kiln_syntax::parse(&combined).module)
        .map(|text| SourceFile::new("<admin pages>", text));
    if let Some(file) = &admin {
        offsets.push(combined.len() as u32);
        combined.push_str(&file.text);
        kiln_files.push(file);
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
        .map(|f| kiln_syntax::glaze::parse_component(&f.text, &f.name).glaze)
        .collect();
    let glaze_names: Vec<String> = glaze_files
        .iter()
        .map(|f| {
            Path::new(&f.name)
                .strip_prefix(&dir)
                .unwrap_or(Path::new(&f.name))
                .to_string_lossy()
                .replace('\\', "/")
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
    let asset_dirs: std::collections::BTreeSet<_> =
        files.iter().filter_map(|p| p.parent()).collect();
    for asset_dir in asset_dirs {
        let mut local = styles::static_files(asset_dir).map_err(|err| {
            eprintln!("error: {err}");
            ExitCode::from(2)
        })?;
        if asset_dir != dir {
            let module = asset_dir
                .strip_prefix(&dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            for asset in &mut local {
                asset.path = asset
                    .path
                    .replacen("/static/", &format!("/static/{module}/"), 1);
            }
        }
        for asset in local {
            if assets.iter().any(|a| a.path == asset.path) {
                eprintln!("error: duplicate static asset URL {}", asset.path);
                return Err(ExitCode::from(2));
            }
            assets.push(asset);
        }
    }
    let generated_at = combined.len() as u32;
    let glaze_refs: Vec<(&str, &kiln_syntax::ast::Glaze)> = glaze_names
        .iter()
        .map(String::as_str)
        .zip(&parsed_glazes)
        .collect();
    let failed_in_glazes = |diagnostics: &[(usize, Diagnostic)]| {
        for (i, diag) in diagnostics {
            eprintln!("{}", render_original(diag, glaze_files[*i], &info));
        }
        failed(diagnostics.len())
    };

    // The embers run in the browser: their code, with the program's, is
    // compiled to WebAssembly, which the server serves with the bridge.
    let mut ember_script = None;
    let mut server_fns = Vec::new();
    let mut rpc_version = String::new();
    let mut rpc_guard = false;
    let user = kiln_syntax::parse(&combined);
    let reachable = reachable_components(&user.module, &glaze_refs, &styles.styling);
    if glaze_refs
        .iter()
        .any(|(name, g)| g.is_ember() && reachable.contains(*name))
    {
        (server_fns, rpc_guard) = server_fns_of(&user.module, &combined);
        // Handlers wait for the npm functions that return promises as
        // they do for `server fn`s.
        let mut waits = server_fns.clone();
        waits.extend(npm::waits(&info.npm));
        let client_app = kiln_ir::glaze::App {
            reachable_components: Some(&reachable),
            server_fns: &waits,
            ..Default::default()
        };
        let client = kiln_ir::glaze::generate_client(&inputs, &client_app);
        if !client.diagnostics.is_empty() {
            return Err(failed_in_glazes(&client.diagnostics));
        }
        let text = format!("{combined}{}{}", client.text, npm::calls_source(&info.npm));
        let report = |d| {
            report_diagnostics(
                d,
                &kiln_files,
                &offsets,
                &glaze_files,
                Some((&client, generated_at)),
                &info,
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
        let js = kiln_wasm::browser_script(&wasm_url, &rpc_version, &npm::modules(&info.npm));
        for package in &info.npm {
            for (path, body) in &package.files {
                assets.push(kiln_ir::glaze::Asset {
                    path: path.clone(),
                    content_type: "text/javascript; charset=utf-8",
                    body: body.clone(),
                    immutable: true,
                });
            }
        }
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
        reachable_components: Some(&reachable),
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
            &info,
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
    kiln_ir::set_caller_location(Some(caller_locations(&kiln_files, &offsets, &info, &dir)));
    let ir = kiln_ir::lower(&parsed.module, &program);
    kiln_ir::set_caller_location(None);
    let ir = ir.map_err(report)?;
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

/// How a call is shown to a person, as `path:line:column` in its original
/// file (relative to the project), from its offset in the program's text.
fn caller_locations(
    kiln_files: &[&SourceFile],
    offsets: &[u32],
    info: &AppInfo,
    root: &Path,
) -> Box<dyn Fn(u32) -> String + Send> {
    type Placed = (
        u32,
        SourceFile,
        Option<(SourceFile, project::bind::SourceMap)>,
    );
    let files: Vec<Placed> = kiln_files
        .iter()
        .zip(offsets)
        .map(|(f, &o)| ((*f).clone(), o, info.source_maps.get(&f.name).cloned()))
        .map(|(f, o, m)| (o, f, m))
        .collect();
    let root = project::real(root);
    Box::new(move |offset| {
        let Some(i) = files.iter().rposition(|(o, _, _)| *o <= offset) else {
            return String::new();
        };
        let (start, file, map) = &files[i];
        let local = offset - start;
        let (file, local) = match map {
            Some((original, map)) => (
                original,
                map.original(Span::new(local as usize, local as usize))
                    .start,
            ),
            None => (file, local),
        };
        let at = file.line_col(local);
        let path = PathBuf::from(project::shown(&file.name));
        let path = path.strip_prefix(&root).unwrap_or(&path);
        format!(
            "{}:{}:{}",
            path.display().to_string().replace('\\', "/"),
            at.line,
            at.column
        )
    })
}

/// Follow explicit render calls, layouts and component references. All source
/// functions are conservatively roots: a component with no reference is never
/// included merely because its file is present in the folder.
fn reachable_components(
    module: &kiln_syntax::ast::Module,
    glazes: &[(&str, &kiln_syntax::ast::Glaze)],
    styling: &kiln_check::Styling,
) -> std::collections::HashSet<String> {
    use kiln_check::ide::{analyze_app, Ref, Target};
    let references = |target| {
        analyze_app(&[module], glazes, target, styling)
            .references
            .into_iter()
            .filter_map(|(_, r)| match r {
                Ref::Component(file) => Some(file),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let mut pending = references(Target::Module(0));
    let mut reached = std::collections::HashSet::new();
    while let Some(file) = pending.pop() {
        if reached.insert(file.clone()) {
            if let Some(i) = glazes.iter().position(|(name, _)| *name == file) {
                pending.extend(references(Target::Glaze(i)));
            }
        }
    }
    reached
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
    info: &AppInfo,
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
                        eprintln!("{}", render_original(&diag, glaze_files[i], info));
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
        eprintln!("{}", render_original(&diag, kiln_files[i], info));
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

pub(crate) fn is_glaze(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext == "glaze" || ext == "ember")
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
        let result = kiln_syntax::glaze::parse_component(&file.text, &file.name);
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

fn display_symbols(text: &str, names: &BTreeMap<String, String>) -> String {
    let mut out = text.to_string();
    let mut names = names.iter().collect::<Vec<_>>();
    names.sort_by_key(|(name, _)| std::cmp::Reverse(name.len()));
    for (name, display) in names {
        out = out.replace(name, display);
    }
    ware_names(&out)
}

/// `__ware.kiln.ui.Button` as `kiln/ui.Button`, as people write Wares.
fn ware_names(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("__ware.") {
        out.push_str(&rest[..i]);
        let after = &rest[i + "__ware.".len()..];
        let mut parts = after.splitn(3, '.');
        match (parts.next(), parts.next()) {
            (Some(p), Some(n))
                if !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                let len = n
                    .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
                    .unwrap_or(n.len());
                out.push_str(&format!("{p}/{}", &n[..len]));
                rest = &after[p.len() + 1 + len..];
            }
            _ => {
                out.push_str("__ware.");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
fn render_original(diag: &Diagnostic, file: &SourceFile, info: &AppInfo) -> String {
    let mut diag = diag.clone();
    diag.message = display_symbols(&diag.message, &info.symbol_names);
    if let Some((original, map)) = info.source_maps.get(&file.name) {
        diag.span = map.original(diag.span);
        if let Some(s) = &mut diag.suggestion {
            s.span = map.original(s.span);
            s.replacement = display_symbols(&s.replacement, &info.symbol_names);
        }
        diag.render(original)
    } else {
        diag.render(file)
    }
}
