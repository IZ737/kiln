//! `kiln test`: finds the tests of a project, builds them into one program
//! with a `main` kiln writes, and runs each test in a process of its own.
//!
//! A test is a function named `test_*`, with no parameters, in a file named
//! `test_*.kiln`. Test files belong to their folder's module (so they can
//! call its private functions), but only `kiln test` builds them. Where a
//! test file sits says what kind of test it is: `tests/integration/` and
//! `tests/browser/` hold integration and browser tests, and every other
//! test is a unit test.
//!
//! A test talks to the runner through its standard streams: a line that
//! starts with `@@kiln-test@@` is a request (a failure, a database, the
//! app's server, an HTTP request, a browser action), answered with one
//! line of JSON. Everything else the test prints is its output.

pub mod browser;
pub mod http;

use crate::project;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const MARKER: &str = "@@kiln-test@@ ";

/// The JSON report's version.
pub const REPORT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Unit,
    Integration,
    Browser,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Unit => "unit",
            Kind::Integration => "integration",
            Kind::Browser => "browser",
        }
    }
}

/// One test function.
#[derive(Clone, Debug)]
pub struct Case {
    /// `notes.test_left` or, in the root folder, `test_left`.
    pub id: String,
    /// The folder module, `notes` or `tests.integration`; `""` is the root.
    pub module: String,
    pub func: String,
    /// The file, relative to the project.
    pub file: String,
    pub line: usize,
    pub column: usize,
    pub throws: bool,
    pub kind: Kind,
}

impl Case {
    fn at(&self) -> String {
        format!("{}:{}:{}", self.file, self.line, self.column)
    }
}

pub struct Options {
    pub path: PathBuf,
    /// The kinds to run; empty runs every kind.
    pub kinds: Vec<Kind>,
    pub filter: Option<String>,
    pub json: bool,
    pub list: bool,
    pub junit: Option<PathBuf>,
    pub watch: bool,
    pub jobs: usize,
    pub timeout: Duration,
    pub retries: u32,
    pub headed: bool,
    /// `[test] env`: given to the tests and to the app.
    pub env: BTreeMap<String, String>,
}

/// Builds a project into an executable.
pub type Builder<'a> = &'a (dyn Fn(&project::Project, &Path) -> bool + Sync);

/// Folders that never hold tests.
fn skipped(name: &str) -> bool {
    name.starts_with('.')
        || name.starts_with("__")
        || matches!(name, "target" | "node_modules" | "migrations" | "static")
}

/// Every test of the project at `root`, in a fixed order.
pub fn discover(root: &Path) -> Result<Vec<Case>, String> {
    let mut cases = Vec::new();
    walk(root, root, &mut cases)?;
    cases.sort_by(|a, b| (a.kind, &a.id).cmp(&(b.kind, &b.id)));
    Ok(cases)
}

fn walk(root: &Path, dir: &Path, cases: &mut Vec<Case>) -> Result<(), String> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        if kind.is_dir() {
            if !skipped(&name) {
                walk(root, &path, cases)?;
            }
            continue;
        }
        if !(name.starts_with("test_") && name.ends_with(".kiln")) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let module = Path::new(&rel)
            .parent()
            .map(|p| p.to_string_lossy().replace('/', "."))
            .unwrap_or_default();
        let kind = if rel.starts_with("tests/browser/") {
            Kind::Browser
        } else if rel.starts_with("tests/integration/") {
            Kind::Integration
        } else {
            Kind::Unit
        };
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let file = kiln_diagnostics::SourceFile::new(rel.clone(), text);
        for item in kiln_syntax::parse(&file.text).module.items {
            let kiln_syntax::ast::Item::Fn(f) = item else {
                continue;
            };
            if !f.name.name.starts_with("test_") || !f.params.is_empty() {
                continue;
            }
            let at = file.line_col(f.span.start);
            let id = if module.is_empty() {
                f.name.name.clone()
            } else {
                format!("{module}.{}", f.name.name)
            };
            cases.push(Case {
                id,
                module: module.clone(),
                func: f.name.name.clone(),
                file: rel.clone(),
                line: at.line,
                column: at.column,
                throws: !matches!(f.throws, kiln_syntax::ast::Throws::Never),
                kind,
            });
        }
    }
    Ok(())
}

/// The `main` that runs the test `KILN_TEST` names.
pub fn harness(cases: &[Case]) -> Result<String, String> {
    let mut modules: BTreeMap<String, String> = BTreeMap::new();
    for c in cases.iter().filter(|c| !c.module.is_empty()) {
        let last = c.module.rsplit('.').next().unwrap().to_string();
        if let Some(other) = modules.get(&last) {
            if *other != c.module {
                return Err(format!(
                    "the test modules `{other}` and `{}` both end in `{last}`; rename one of the folders",
                    c.module
                ));
            }
        }
        modules.insert(last, c.module.clone());
    }
    let mut imports = vec!["os".to_string(), "io".to_string(), "test".to_string()];
    for m in modules.values() {
        if !imports.contains(m) {
            imports.push(m.clone());
        }
    }
    let mut out = format!(
        "// Written by `kiln test`: runs the test that KILN_TEST names.\nimport {}\n\nfn main() {{\n    let name = os.env(\"KILN_TEST\") ?? \"\"\n",
        imports.join(", ")
    );
    for c in cases {
        let call = if c.module.is_empty() {
            format!("{}()", c.func)
        } else {
            format!("{}.{}()", c.module.rsplit('.').next().unwrap(), c.func)
        };
        let body = if c.throws {
            format!(
                "{call} catch err {{\n            test.fail(\"the test failed: ${{err.message}}\", at: {})\n        }}",
                json!(c.at())
            )
        } else {
            call
        };
        out.push_str(&format!(
            "    if name == {} {{\n        {body}\n        return\n    }}\n",
            json!(c.id)
        ));
    }
    out.push_str("    io.print_err(\"kiln test: no test ${name}\")\n    os.exit(3)\n}\n");
    Ok(out)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Passed,
    Failed,
    /// The runner could not run the test: no browser, a build that failed.
    Error,
}

#[derive(Clone, Debug)]
pub struct Outcome {
    pub case: Case,
    pub status: Status,
    pub millis: u128,
    pub message: String,
    pub at: String,
    pub expected: String,
    pub actual: String,
    pub output: String,
    pub artifacts: Vec<String>,
    /// Runs before this one that failed, with `--retries`.
    pub retries: u32,
}

pub fn run(options: &Options, builder: Builder) -> ExitCode {
    if !options.watch {
        return once(options, builder);
    }
    let mut last = snapshot(&options.path);
    let mut code = once(options, builder);
    loop {
        std::thread::sleep(Duration::from_millis(400));
        let now = snapshot(&options.path);
        if now != last {
            // Let a burst of saves settle.
            std::thread::sleep(Duration::from_millis(150));
            last = snapshot(&options.path);
            eprintln!("\nkiln test: files changed, running again");
            code = once(options, builder);
        }
        if std::env::var_os("KILN_TEST_WATCH_ONCE").is_some() {
            return code;
        }
    }
}

/// The source files of a project and when each changed.
pub fn snapshot(root: &Path) -> BTreeMap<PathBuf, std::time::SystemTime> {
    fn go(dir: &Path, out: &mut BTreeMap<PathBuf, std::time::SystemTime>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if path.is_dir() {
                if !name.starts_with('.') && name != "target" && name != "node_modules" {
                    go(&path, out);
                }
            } else if let Ok(m) = e.metadata().and_then(|m| m.modified()) {
                out.insert(path, m);
            }
        }
    }
    let mut out = BTreeMap::new();
    go(root, &mut out);
    out
}

fn once(options: &Options, builder: Builder) -> ExitCode {
    let root = match project::root(&options.path) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {}: {e}", options.path.display());
            return ExitCode::from(2);
        }
    };
    let all = match discover(&root) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    let cases: Vec<Case> = all
        .into_iter()
        .filter(|c| options.kinds.is_empty() || options.kinds.contains(&c.kind))
        .filter(|c| {
            options
                .filter
                .as_ref()
                .is_none_or(|f| c.id.contains(f.as_str()))
        })
        .collect();
    if options.list {
        if options.json {
            let list: Vec<Value> = cases.iter().map(case_json).collect();
            println!("{}", json!({ "version": REPORT_VERSION, "tests": list }));
        } else {
            for c in &cases {
                println!("{}  {} ({})", c.kind.name(), c.id, c.at());
            }
        }
        return ExitCode::SUCCESS;
    }
    if cases.is_empty() {
        if options.json {
            println!("{}", report_json(&[], 0));
        } else {
            eprintln!(
                "kiln test: no tests in {}. A test is a `fn test_...()` in a file named `test_*.kiln`",
                root.display()
            );
        }
        return ExitCode::SUCCESS;
    }
    let started = Instant::now();
    let work = std::env::temp_dir().join(format!(
        "kiln-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&work);
    let source = work.join("harness");
    if let Err(e) = std::fs::create_dir_all(&source) {
        eprintln!("error: cannot make {}: {e}", source.display());
        return ExitCode::from(3);
    }
    let text = match harness(&cases) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(2);
        }
    };
    std::fs::write(source.join("main.kiln"), text).ok();
    let include_root = cases.iter().any(|c| c.module.is_empty());
    let project = match project::discover_with(
        &root,
        &BTreeMap::new(),
        Some(&project::Harness {
            dir: source.clone(),
            include_root,
        }),
    ) {
        Ok(p) => p,
        Err(err) => {
            let (file, diag) = project::error_diagnostic(&err, &root.join("kiln.toml"));
            eprintln!("{}", diag.render(&file));
            return ExitCode::from(2);
        }
    };
    let exe = work.join(crate::Target::host().executable("tests"));
    if !builder(&project, &exe) {
        let _ = std::fs::remove_dir_all(&work);
        return ExitCode::from(2);
    }
    let shared = Shared {
        root: root.clone(),
        work: work.clone(),
        exe,
        builder,
        app: OnceLock::new(),
        browser: OnceLock::new(),
        options,
        results: root.join(".kiln").join("test-results"),
    };
    let _ = std::fs::remove_dir_all(&shared.results);
    if !options.json {
        let count = |k: Kind| cases.iter().filter(|c| c.kind == k).count();
        eprintln!(
            "kiln test: {} test(s): {} unit, {} integration, {} browser",
            cases.len(),
            count(Kind::Unit),
            count(Kind::Integration),
            count(Kind::Browser)
        );
    }
    let queue = Mutex::new(cases.iter().enumerate().rev().collect::<Vec<_>>());
    let outcomes: Mutex<Vec<(usize, Outcome)>> = Mutex::new(Vec::new());
    let jobs = options.jobs.max(1).min(cases.len());
    std::thread::scope(|scope| {
        for _ in 0..jobs {
            scope.spawn(|| loop {
                let next = queue.lock().unwrap().pop();
                let Some((i, case)) = next.map(|(i, c)| (i, c.clone())) else {
                    break;
                };
                let outcome = shared.run_with_retries(case, i);
                if !options.json {
                    print_outcome(&outcome);
                }
                outcomes.lock().unwrap().push((i, outcome));
            });
        }
    });
    let mut outcomes = outcomes.into_inner().unwrap();
    outcomes.sort_by_key(|(i, _)| *i);
    let outcomes: Vec<Outcome> = outcomes.into_iter().map(|(_, o)| o).collect();
    if let Some(Ok(b)) = shared.browser.get() {
        b.close();
    }
    let _ = std::fs::remove_dir_all(&work);
    let millis = started.elapsed().as_millis();
    if let Some(path) = &options.junit {
        if let Err(e) = std::fs::write(path, junit(&outcomes, millis)) {
            eprintln!("error: cannot write {}: {e}", path.display());
        }
    }
    if options.json {
        println!("{}", report_json(&outcomes, millis));
    } else {
        print_summary(&outcomes, millis);
    }
    if outcomes.iter().any(|o| o.status == Status::Error) {
        ExitCode::from(3)
    } else if outcomes.iter().any(|o| o.status == Status::Failed) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// What every test shares: the built tests, the app, the browser.
struct Shared<'a> {
    root: PathBuf,
    work: PathBuf,
    exe: PathBuf,
    builder: Builder<'a>,
    /// The app's own program, built when a test first starts it.
    app: OnceLock<Result<PathBuf, String>>,
    browser: OnceLock<Result<browser::Browser, String>>,
    options: &'a Options,
    results: PathBuf,
}

impl Shared<'_> {
    fn run_with_retries(&self, case: Case, index: usize) -> Outcome {
        let mut retries = 0;
        loop {
            let mut outcome = self.run_case(&case, index, retries);
            outcome.retries = retries;
            if outcome.status != Status::Failed || retries >= self.options.retries {
                return outcome;
            }
            retries += 1;
        }
    }

    fn app(&self) -> Result<PathBuf, String> {
        self.app
            .get_or_init(|| {
                let project = project::discover(&self.root).map_err(|err| {
                    let (file, diag) =
                        project::error_diagnostic(&err, &self.root.join("kiln.toml"));
                    diag.render(&file)
                })?;
                let exe = self.work.join(crate::Target::host().executable("app"));
                if (self.builder)(&project, &exe) {
                    Ok(exe)
                } else {
                    Err("the app does not build; its errors are above".into())
                }
            })
            .clone()
    }

    fn browser(&self) -> Result<&browser::Browser, String> {
        self.browser
            .get_or_init(|| browser::Browser::launch(&self.root, self.options.headed))
            .as_ref()
            .map_err(Clone::clone)
    }

    fn run_case(&self, case: &Case, index: usize, attempt: u32) -> Outcome {
        let started = Instant::now();
        let tmp = self.work.join(format!("t{index}-{attempt}"));
        let _ = std::fs::create_dir_all(&tmp);
        let mut outcome = Outcome {
            case: case.clone(),
            status: Status::Passed,
            millis: 0,
            message: String::new(),
            at: String::new(),
            expected: String::new(),
            actual: String::new(),
            output: String::new(),
            artifacts: Vec::new(),
            retries: 0,
        };
        let mut env = Env {
            shared: self,
            case,
            tmp: tmp.clone(),
            database: None,
            server: None,
            cookies: BTreeMap::new(),
            pages: Vec::new(),
            infrastructure: None,
        };
        let child = Command::new(&self.exe)
            .envs(&self.options.env)
            .env("KILN_TEST", &case.id)
            .env("KILN_TEST_TMP", &tmp)
            .current_dir(&self.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                outcome.status = Status::Error;
                outcome.message = format!("cannot start the tests: {e}");
                return outcome;
            }
        };
        let (tx, rx) = mpsc::channel::<Option<String>>();
        let stdout = child.stdout.take().unwrap();
        let out_tx = tx.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if out_tx.send(Some(line)).is_err() {
                    break;
                }
            }
            let _ = out_tx.send(None);
        });
        let stderr = child.stderr.take().unwrap();
        let errors = Arc::new(Mutex::new(String::new()));
        let errors_in = errors.clone();
        let err_thread = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                let mut e = errors_in.lock().unwrap();
                e.push_str(&line);
                e.push('\n');
            }
        });
        let mut stdin = child.stdin.take().unwrap();
        let deadline = started + self.options.timeout;
        let mut failed = None;
        let mut timed_out = false;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match rx.recv_timeout(left) {
                Ok(Some(line)) => {
                    let Some(request) = line.strip_prefix(MARKER) else {
                        outcome.output.push_str(&line);
                        outcome.output.push('\n');
                        continue;
                    };
                    let request: Value = serde_json::from_str(request).unwrap_or(Value::Null);
                    let op = request["op"].as_str().unwrap_or("");
                    if op == "fail" {
                        failed = Some(request);
                        continue;
                    }
                    let reply = match env.handle(op, &request, deadline) {
                        Ok(value) => json!({ "ok": true, "value": value, "error": "" }),
                        Err(error) => json!({ "ok": false, "value": "", "error": error }),
                    };
                    if writeln!(stdin, "{reply}")
                        .and_then(|_| stdin.flush())
                        .is_err()
                    {
                        break;
                    }
                }
                Ok(None) => break,
                Err(_) => {
                    timed_out = true;
                    break;
                }
            }
        }
        if timed_out {
            let _ = child.kill();
        }
        let status = wait(&mut child, Duration::from_secs(5));
        let _ = err_thread.join();
        let errors = errors.lock().unwrap().clone();
        if let Some(problem) = env.infrastructure.take() {
            outcome.status = Status::Error;
            outcome.message = problem;
        } else if let Some(f) = failed {
            outcome.status = Status::Failed;
            outcome.message = f["message"].as_str().unwrap_or("").to_string();
            outcome.at = f["at"].as_str().unwrap_or("").to_string();
            outcome.expected = f["expected"].as_str().unwrap_or("").to_string();
            outcome.actual = f["actual"].as_str().unwrap_or("").to_string();
        } else if timed_out {
            outcome.status = Status::Failed;
            outcome.message = format!(
                "the test took longer than {} s; raise it with --timeout",
                self.options.timeout.as_secs()
            );
        } else if status != Some(0) {
            outcome.status = Status::Failed;
            outcome.message = match status {
                Some(code) => format!("the test stopped with exit code {code}"),
                None => "the test crashed".into(),
            };
        }
        if outcome.status == Status::Error && outcome.at.is_empty() {
            outcome.at = case.at();
        }
        if outcome.status != Status::Passed && outcome.at.is_empty() {
            outcome.at = case.at();
        }
        if !errors.trim().is_empty() {
            outcome.output.push_str(&errors);
        }
        if outcome.status != Status::Passed {
            outcome.artifacts = env.keep_evidence();
        }
        env.finish();
        let _ = std::fs::remove_dir_all(&tmp);
        outcome.millis = started.elapsed().as_millis();
        outcome
    }
}

/// Waits for a child, killing it after `limit`.
fn wait(child: &mut Child, limit: Duration) -> Option<i32> {
    let until = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return s.code(),
            Ok(None) if Instant::now() < until => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                return child.wait().ok().and_then(|s| s.code());
            }
        }
    }
}

/// The app under test, running for one test.
struct Server {
    child: Child,
    url: String,
    log: Arc<Mutex<String>>,
}

/// One test's resources.
struct Env<'a, 'b> {
    shared: &'a Shared<'b>,
    case: &'a Case,
    tmp: PathBuf,
    database: Option<String>,
    server: Option<Server>,
    cookies: BTreeMap<String, String>,
    pages: Vec<browser::Page>,
    /// Why the runner could not do what the test asked, which is not the
    /// test's failure.
    infrastructure: Option<String>,
}

impl Env<'_, '_> {
    fn handle(&mut self, op: &str, r: &Value, deadline: Instant) -> Result<String, String> {
        let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
        match op {
            "database" => self.database(),
            "server" => self.server(),
            "encode" => Ok(http::encode(&s("text"))),
            "http" => {
                let base = self.server()?;
                let headers: BTreeMap<String, String> =
                    serde_json::from_str(&s("headers")).unwrap_or_default();
                let left = deadline.saturating_duration_since(Instant::now());
                let res = http::send(
                    &base,
                    &s("method"),
                    &s("path"),
                    &headers,
                    &s("body"),
                    &mut self.cookies,
                    left.max(Duration::from_millis(100)),
                )?;
                Ok(
                    json!({ "status": res.status, "headers": res.headers, "body": res.body })
                        .to_string(),
                )
            }
            "page" => {
                let base = self.server()?;
                let browser = match self.shared.browser() {
                    Ok(b) => b,
                    Err(e) => {
                        self.infrastructure = Some(e.clone());
                        return Err(e);
                    }
                };
                let width = s("width").parse().unwrap_or(1280);
                let height = s("height").parse().unwrap_or(800);
                let page = browser.page(&base, width, height)?;
                let id = page.id.clone();
                self.pages.push(page);
                Ok(id)
            }
            _ => {
                let id = s("page");
                let page = self
                    .pages
                    .iter()
                    .find(|p| p.id == id)
                    .ok_or_else(|| format!("no page `{id}` in this test"))?;
                let browser = self.shared.browser()?;
                let name_dir = self.shared.results.join(&self.case.id);
                browser.act(page, op, r, deadline, &name_dir)
            }
        }
    }

    fn database(&mut self) -> Result<String, String> {
        if let Some(url) = &self.database {
            return Ok(url.clone());
        }
        let file = self.tmp.join("test.db");
        let url = format!("sqlite:{}", file.display());
        crate::db::prepare_test_database(&self.shared.root, &url)
            .map_err(|e| format!("cannot make the test's database: {e}"))?;
        self.database = Some(url.clone());
        Ok(url)
    }

    fn server(&mut self) -> Result<String, String> {
        if let Some(s) = &self.server {
            return Ok(s.url.clone());
        }
        let exe = self.shared.app().inspect_err(|e| {
            self.infrastructure = Some(e.clone());
        })?;
        let database = self.database()?;
        let mut child = Command::new(&exe)
            .envs(&self.shared.options.env)
            .env("KILN_LISTEN", "127.0.0.1:0")
            .env("DATABASE_URL", &database)
            .env("KILN_TEST_TMP", &self.tmp)
            .current_dir(&self.shared.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start the app: {e}"))?;
        let log = Arc::new(Mutex::new(String::new()));
        let (tx, rx) = mpsc::channel();
        let stderr = child.stderr.take().unwrap();
        let log_in = log.clone();
        std::thread::spawn(move || {
            let mut sent = false;
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                if !sent {
                    if let Some(url) = line.strip_prefix("info listening on ") {
                        let _ = tx.send(url.trim().replace("localhost", "127.0.0.1"));
                        sent = true;
                    }
                }
                let mut l = log_in.lock().unwrap();
                l.push_str(&line);
                l.push('\n');
            }
        });
        let stdout = child.stdout.take().unwrap();
        let log_out = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                let mut l = log_out.lock().unwrap();
                l.push_str(&line);
                l.push('\n');
            }
        });
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(url) => {
                self.server = Some(Server {
                    child,
                    url: url.clone(),
                    log,
                });
                Ok(url)
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let log = log.lock().unwrap().clone();
                Err(format!(
                    "the app did not start listening; it must call `app.listen`. Its output:\n{log}"
                ))
            }
        }
    }

    /// Screenshots of the open pages and the app's log, for a failed test.
    fn keep_evidence(&mut self) -> Vec<String> {
        let dir = self.shared.results.join(&self.case.id);
        let mut kept = Vec::new();
        // Only a test that opened pages has a browser to ask; asking
        // otherwise would start one.
        if let Some(Ok(browser)) = self.shared.browser.get() {
            if !self.pages.is_empty() {
                for (i, page) in self.pages.iter().enumerate() {
                    if let Ok(path) = browser.screenshot(page, &dir, &format!("failure-{}", i + 1))
                    {
                        kept.push(path);
                    }
                    let errors = browser.errors(page);
                    if !errors.is_empty() {
                        let _ = std::fs::create_dir_all(&dir);
                        let file = dir.join(format!("page-{}-errors.txt", i + 1));
                        if std::fs::write(&file, errors.join("\n")).is_ok() {
                            kept.push(file.display().to_string());
                        }
                    }
                }
            }
        }
        if let Some(server) = &self.server {
            let log = server.log.lock().unwrap().clone();
            if !log.is_empty() {
                let _ = std::fs::create_dir_all(&dir);
                let file = dir.join("server.log");
                if std::fs::write(&file, log).is_ok() {
                    kept.push(file.display().to_string());
                }
            }
        }
        let cwd = std::env::current_dir().unwrap_or_default();
        kept.into_iter()
            .map(|p| {
                Path::new(&p)
                    .strip_prefix(&cwd)
                    .map(|r| r.display().to_string())
                    .unwrap_or(p)
            })
            .collect()
    }

    fn finish(&mut self) {
        if let Some(Ok(browser)) = self.shared.browser.get() {
            for page in self.pages.drain(..) {
                browser.close_page(&page);
            }
        }
        if let Some(mut s) = self.server.take() {
            let _ = s.child.kill();
            let _ = s.child.wait();
        }
    }
}

fn print_outcome(o: &Outcome) {
    let mut err = std::io::stderr().lock();
    let retried = if o.retries > 0 {
        format!(", passed after {} failed run(s): flaky", o.retries)
    } else {
        String::new()
    };
    match o.status {
        Status::Passed => {
            let _ = writeln!(err, "ok    {} ({} ms{retried})", o.case.id, o.millis);
        }
        Status::Failed | Status::Error => {
            let word = if o.status == Status::Failed {
                "FAIL "
            } else {
                "ERROR"
            };
            let _ = writeln!(err, "{word} {} ({} ms)", o.case.id, o.millis);
            let _ = writeln!(err, "  {}: {}", o.at, o.message);
            if !o.expected.is_empty() || !o.actual.is_empty() {
                let _ = writeln!(err, "    expected: {}", o.expected);
                let _ = writeln!(err, "    actual:   {}", o.actual);
            }
            if !o.output.trim().is_empty() {
                let _ = writeln!(err, "  output:");
                for line in o.output.lines().take(40) {
                    let _ = writeln!(err, "    {line}");
                }
            }
            for a in &o.artifacts {
                let _ = writeln!(err, "  saved {a}");
            }
        }
    }
}

fn print_summary(outcomes: &[Outcome], millis: u128) {
    let passed = outcomes
        .iter()
        .filter(|o| o.status == Status::Passed)
        .count();
    let failed = outcomes
        .iter()
        .filter(|o| o.status == Status::Failed)
        .count();
    let errors = outcomes
        .iter()
        .filter(|o| o.status == Status::Error)
        .count();
    let flaky = outcomes
        .iter()
        .filter(|o| o.status == Status::Passed && o.retries > 0)
        .count();
    let mut line = format!(
        "\n{} test(s): {passed} passed, {failed} failed",
        outcomes.len()
    );
    if errors > 0 {
        line.push_str(&format!(", {errors} could not run"));
    }
    if flaky > 0 {
        line.push_str(&format!(", {flaky} flaky"));
    }
    line.push_str(&format!(" ({:.1} s)", millis as f64 / 1000.0));
    eprintln!("{line}");
    if failed + errors > 0 {
        eprintln!("rerun one with `kiln test --filter <name>`");
    }
}

fn case_json(c: &Case) -> Value {
    json!({
        "id": c.id,
        "kind": c.kind.name(),
        "module": c.module,
        "function": c.func,
        "file": c.file,
        "line": c.line,
        "column": c.column,
    })
}

fn status_name(s: &Status) -> &'static str {
    match s {
        Status::Passed => "passed",
        Status::Failed => "failed",
        Status::Error => "error",
    }
}

pub fn report_json(outcomes: &[Outcome], millis: u128) -> Value {
    let tests: Vec<Value> = outcomes
        .iter()
        .map(|o| {
            let mut v = case_json(&o.case);
            v["status"] = json!(status_name(&o.status));
            v["duration_ms"] = json!(o.millis as u64);
            v["retries"] = json!(o.retries);
            if o.status != Status::Passed {
                v["failure"] = json!({
                    "message": o.message,
                    "at": o.at,
                    "expected": o.expected,
                    "actual": o.actual,
                });
            }
            v["output"] = json!(o.output);
            v["artifacts"] = json!(o.artifacts);
            v
        })
        .collect();
    let count = |s: Status| outcomes.iter().filter(|o| o.status == s).count();
    json!({
        "version": REPORT_VERSION,
        "tests": tests,
        "summary": {
            "total": outcomes.len(),
            "passed": count(Status::Passed),
            "failed": count(Status::Failed),
            "errors": count(Status::Error),
            "duration_ms": millis as u64,
        },
    })
}

fn xml(text: &str) -> String {
    text.chars()
        .filter(|c| *c == '\n' || *c == '\t' || !c.is_control())
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The results as JUnit XML, which most CI systems read.
pub fn junit(outcomes: &[Outcome], millis: u128) -> String {
    let failed = outcomes
        .iter()
        .filter(|o| o.status == Status::Failed)
        .count();
    let errors = outcomes
        .iter()
        .filter(|o| o.status == Status::Error)
        .count();
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"kiln test\" tests=\"{}\" failures=\"{failed}\" errors=\"{errors}\" time=\"{:.3}\">\n",
        outcomes.len(),
        millis as f64 / 1000.0
    );
    for kind in [Kind::Unit, Kind::Integration, Kind::Browser] {
        let of: Vec<&Outcome> = outcomes.iter().filter(|o| o.case.kind == kind).collect();
        if of.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "  <testsuite name=\"{}\" tests=\"{}\">\n",
            kind.name(),
            of.len()
        ));
        for o in of {
            out.push_str(&format!(
                "    <testcase classname=\"{}\" name=\"{}\" file=\"{}\" line=\"{}\" time=\"{:.3}\"",
                xml(&o.case.module),
                xml(&o.case.func),
                xml(&o.case.file),
                o.case.line,
                o.millis as f64 / 1000.0
            ));
            if o.status == Status::Passed {
                out.push_str(" />\n");
                continue;
            }
            let tag = if o.status == Status::Failed {
                "failure"
            } else {
                "error"
            };
            let mut detail = format!("{}: {}", o.at, o.message);
            if !o.expected.is_empty() || !o.actual.is_empty() {
                detail.push_str(&format!(
                    "\nexpected: {}\nactual:   {}",
                    o.expected, o.actual
                ));
            }
            out.push_str(&format!(
                ">\n      <{tag} message=\"{}\">{}</{tag}>\n      <system-out>{}</system-out>\n    </testcase>\n",
                xml(&o.message),
                xml(&detail),
                xml(&o.output)
            ));
        }
        out.push_str("  </testsuite>\n");
    }
    out.push_str("</testsuites>\n");
    out
}
