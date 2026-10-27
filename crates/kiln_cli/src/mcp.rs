//! `kiln mcp`: a Model Context Protocol server, over standard input and
//! output, that gives a coding agent the compiler's own answers: the
//! project's modules and exports, its Wares, diagnostics, explanations,
//! formatting and tests. It is the same code as `kiln check`, `kiln fmt`
//! and `kiln test`, not another compiler.
//!
//! Reading and checking are always allowed. Running tests executes the
//! project's code, so it needs `--allow-exec`; writing files (formatting,
//! building) needs `--allow-write`. Publishing, deploying and migrating
//! are not offered at all. Every path must be inside a workspace root, and
//! nothing reads the network or changes kiln.lock.

use crate::project;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{BufRead, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// The version of the tools' schemas and results. A change that could
/// break an agent's use of them makes a new version.
pub const SCHEMA_VERSION: u32 = 1;

/// The MCP revisions this server speaks, newest first.
const PROTOCOLS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

/// The most text one result carries; longer text is cut and says so.
const MAX_TEXT: usize = 100_000;

pub struct Options {
    pub roots: Vec<PathBuf>,
    pub allow_exec: bool,
    pub allow_write: bool,
}

struct Server {
    roots: Vec<PathBuf>,
    allow_exec: bool,
    allow_write: bool,
    /// Messages read while a test run was waiting, to answer next.
    queued: VecDeque<Value>,
    incoming: mpsc::Receiver<Value>,
}

pub fn run(options: Options) -> ExitCode {
    let mut roots = Vec::new();
    for root in &options.roots {
        match root.canonicalize() {
            Ok(r) if r.is_dir() => roots.push(r),
            _ => {
                eprintln!("error: {} is not a folder", root.display());
                return ExitCode::from(2);
            }
        }
    }
    // Checking must not write kiln.lock or reach the network.
    crate::ware::set_mode(crate::ware::Mode {
        offline: true,
        frozen: true,
        ..Default::default()
    });
    let (tx, incoming) = mpsc::channel();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            if line.trim().is_empty() {
                continue;
            }
            let message = serde_json::from_str(&line)
                .unwrap_or_else(|e| json!({ "__invalid": e.to_string() }));
            if tx.send(message).is_err() {
                break;
            }
        }
    });
    let mut server = Server {
        roots,
        allow_exec: options.allow_exec,
        allow_write: options.allow_write,
        queued: VecDeque::new(),
        incoming,
    };
    loop {
        let message = match server.queued.pop_front() {
            Some(m) => m,
            None => match server.incoming.recv() {
                Ok(m) => m,
                Err(_) => return ExitCode::SUCCESS,
            },
        };
        if let Some(reply) = server.handle(message) {
            send(&reply);
        }
    }
}

fn send(message: &Value) {
    let mut out = std::io::stdout().lock();
    let _ = writeln!(out, "{message}");
    let _ = out.flush();
}

fn error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// A tool's answer: text for the agent to read, and the same as data.
fn result(data: Value, text: String, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": bounded(text) }],
        "structuredContent": data,
        "isError": is_error,
    })
}

fn failure(message: impl Into<String>) -> Value {
    let message = message.into();
    result(
        json!({ "schema": SCHEMA_VERSION, "error": message }),
        message,
        true,
    )
}

fn bounded(mut text: String) -> String {
    if text.len() > MAX_TEXT {
        let mut cut = MAX_TEXT;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
        text.push_str("\n… (cut: the result is longer than kiln mcp returns at once)");
    }
    text
}

impl Server {
    fn handle(&mut self, message: Value) -> Option<Value> {
        if let Some(e) = message.get("__invalid") {
            return Some(error(
                Value::Null,
                -32700,
                &format!("not JSON: {}", e.as_str().unwrap_or("")),
            ));
        }
        let method = message.get("method").and_then(Value::as_str);
        let Some(id) = message.get("id").cloned() else {
            // A notification: nothing to answer.
            return None;
        };
        let Some(method) = method else {
            // A response to a request this server never sends.
            return None;
        };
        let params = message.get("params").cloned().unwrap_or(json!({}));
        let answer = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.tools() })),
            "tools/call" => self.call(&id, &params),
            _ => Err((-32601, format!("kiln mcp has no method `{method}`"))),
        };
        Some(match answer {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => error(id, code, &message),
        })
    }

    fn initialize(&self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let protocol = asked
            .filter(|v| PROTOCOLS.contains(v))
            .unwrap_or(PROTOCOLS[0]);
        let roots: Vec<String> = self.roots.iter().map(|r| shown(r)).collect();
        json!({
            "protocolVersion": protocol,
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "kiln", "version": env!("CARGO_PKG_VERSION") },
            "instructions": format!(
                "Kiln's compiler and tools for the projects in {}. Start with `project_graph`, \
                 change code, then `check` until it reports no errors and `run_tests` (if allowed). \
                 `guide` returns the Kiln guide for agents. Paths are relative to the first root. \
                 Running tests is {}; writing files is {}.",
                roots.join(", "),
                if self.allow_exec { "allowed" } else { "not allowed (start kiln mcp with --allow-exec)" },
                if self.allow_write { "allowed" } else { "not allowed (start kiln mcp with --allow-write)" },
            ),
        })
    }

    fn tools(&self) -> Vec<Value> {
        let path = json!({
            "type": "string",
            "description": "A file or folder inside a workspace root. Defaults to the first root."
        });
        let read_only = json!({ "readOnlyHint": true, "openWorldHint": false });
        vec![
            json!({
                "name": "version",
                "description": "The kiln version, the tool schema version and what this server is allowed to do.",
                "inputSchema": { "type": "object", "properties": {} },
                "annotations": read_only,
            }),
            json!({
                "name": "project_graph",
                "description": "The project's modules (folders), their files, imports and `pub` exports with signatures, its entry and its Wares. Reads source; runs nothing.",
                "inputSchema": { "type": "object", "properties": { "path": path } },
                "annotations": read_only,
            }),
            json!({
                "name": "check",
                "description": "Type-check files or folders as `kiln check` does. Returns structured diagnostics: code, message, file, line and column, help and an exact suggested fix when there is one. Runs nothing.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "paths": { "type": "array", "items": { "type": "string" }, "description": "Files or folders. Defaults to the first root." },
                        "offset": { "type": "integer", "minimum": 0, "description": "Skip this many diagnostics (for paging)." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 500, "description": "At most this many diagnostics. Defaults to 100." }
                    }
                },
                "annotations": read_only,
            }),
            json!({
                "name": "explain",
                "description": "Explain an error code such as E0201, with a wrong and a right example.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "code": { "type": "string" } },
                    "required": ["code"]
                },
                "annotations": read_only,
            }),
            json!({
                "name": "ware_tree",
                "description": "The Wares kiln.lock holds, with versions, sources and capabilities; with `why`, the chains that bring one in.",
                "inputSchema": {
                    "type": "object",
                    "properties": { "path": path, "why": { "type": "string" } }
                },
                "annotations": read_only,
            }),
            json!({
                "name": "format_preview",
                "description": "Which files `kiln fmt` would change, and the formatted text of each (or of one file). Writes nothing.",
                "inputSchema": { "type": "object", "properties": { "path": path } },
                "annotations": read_only,
            }),
            json!({
                "name": "list_tests",
                "description": "The project's tests (unit, integration and browser) without running them.",
                "inputSchema": { "type": "object", "properties": { "path": path } },
                "annotations": read_only,
            }),
            json!({
                "name": "guide",
                "description": "The Kiln guide for coding agents: a minimal app, modules, glazes and styling, SQL/ORM and auth, Wares, testing, and common mistakes.",
                "inputSchema": { "type": "object", "properties": {} },
                "annotations": read_only,
            }),
            json!({
                "name": "run_tests",
                "description": "Run the project's tests with `kiln test` and return the results. EXECUTES THE PROJECT'S CODE: tests build and start the app, use a database of their own and may drive a browser. Needs kiln mcp --allow-exec.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "path": path,
                        "filter": { "type": "string", "description": "Only tests whose name contains this." },
                        "kinds": { "type": "array", "items": { "enum": ["unit", "integration", "browser"] } },
                        "timeout": { "type": "integer", "minimum": 1, "description": "Seconds each test may take." }
                    }
                },
                "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": true },
            }),
            json!({
                "name": "format_apply",
                "description": "Format files in place with `kiln fmt`. Changes only layout; refuses a file it cannot format without changing its meaning. Needs kiln mcp --allow-write.",
                "inputSchema": { "type": "object", "properties": { "path": path } },
                "annotations": { "readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false },
            }),
            json!({
                "name": "build",
                "description": "Compile the program into an executable inside the project's `.kiln/mcp/` folder. Runs nothing. Needs kiln mcp --allow-write.",
                "inputSchema": { "type": "object", "properties": { "path": path } },
                "annotations": { "readOnlyHint": false, "destructiveHint": false, "openWorldHint": false },
            }),
        ]
    }

    fn call(&mut self, id: &Value, params: &Value) -> Result<Value, (i64, String)> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "tools/call needs a tool `name`".to_string()))?;
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        Ok(match name {
            "version" => self.version(),
            "project_graph" => self.path(&args).map_or_else(failure, |p| graph(&p)),
            "check" => self.check(&args),
            "explain" => explain(args.get("code").and_then(Value::as_str).unwrap_or("")),
            "ware_tree" => self.path(&args).map_or_else(failure, |p| {
                let why = args.get("why").and_then(Value::as_str);
                match crate::ware::commands::tree(&p, why) {
                    Ok(text) => result(
                        json!({ "schema": SCHEMA_VERSION, "tree": text }),
                        text,
                        false,
                    ),
                    Err(e) => failure(e),
                }
            }),
            "format_preview" => self.path(&args).map_or_else(failure, |p| format(&p, false)),
            "list_tests" => self
                .path(&args)
                .map_or_else(failure, |p| self.list_tests(&p)),
            "guide" => {
                let text = crate::AGENT_GUIDE.to_string();
                result(
                    json!({ "schema": SCHEMA_VERSION, "guide": text }),
                    text,
                    false,
                )
            }
            "run_tests" if !self.allow_exec => failure(
                "run_tests executes the project's code, and this server was not allowed to: \
                 the user can start it with `kiln mcp --allow-exec`",
            ),
            "run_tests" => self
                .path(&args)
                .map_or_else(failure, |p| self.run_tests(id, &p, &args)),
            "format_apply" | "build" if !self.allow_write => failure(format!(
                "{name} writes files, and this server was not allowed to: \
                 the user can start it with `kiln mcp --allow-write`"
            )),
            "format_apply" => self.path(&args).map_or_else(failure, |p| format(&p, true)),
            "build" => self.path(&args).map_or_else(failure, |p| self.build(&p)),
            _ => return Err((-32602, format!("kiln mcp has no tool `{name}`"))),
        })
    }

    fn version(&self) -> Value {
        let data = json!({
            "schema": SCHEMA_VERSION,
            "kiln": env!("CARGO_PKG_VERSION"),
            "roots": self.roots.iter().map(|r| shown(r)).collect::<Vec<_>>(),
            "allow_exec": self.allow_exec,
            "allow_write": self.allow_write,
        });
        let text =
            format!(
            "kiln {} (tool schema {SCHEMA_VERSION}); roots: {}; running tests {}; writing files {}",
            env!("CARGO_PKG_VERSION"),
            data["roots"],
            if self.allow_exec { "allowed" } else { "not allowed" },
            if self.allow_write { "allowed" } else { "not allowed" },
        );
        result(data, text, false)
    }

    /// The `path` argument, which must be inside a root.
    fn path(&self, args: &Value) -> Result<PathBuf, String> {
        self.inside(args.get("path").and_then(Value::as_str))
    }

    fn inside(&self, path: Option<&str>) -> Result<PathBuf, String> {
        let first = &self.roots[0];
        let Some(path) = path.filter(|p| !p.is_empty()) else {
            return Ok(first.clone());
        };
        let joined = first.join(path);
        let real = joined
            .canonicalize()
            .map_err(|_| format!("{path} does not exist"))?;
        if self.roots.iter().any(|r| real.starts_with(r)) {
            Ok(real)
        } else {
            Err(format!(
                "{path} is outside the workspace roots kiln mcp was started with"
            ))
        }
    }

    fn check(&self, args: &Value) -> Value {
        let mut paths = Vec::new();
        match args.get("paths").and_then(Value::as_array) {
            Some(list) if !list.is_empty() => {
                for p in list {
                    match self.inside(p.as_str()) {
                        Ok(p) => paths.push(p),
                        Err(e) => return failure(e),
                    }
                }
            }
            _ => paths.push(self.roots[0].clone()),
        }
        let report = match crate::check_paths(paths) {
            Ok(r) => r,
            Err(e) => return failure(e),
        };
        let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .clamp(1, 500) as usize;
        let total = report.diagnostics.len();
        let page: Vec<_> = report.diagnostics.iter().skip(offset).take(limit).collect();
        let diagnostics: Vec<Value> = page.iter().map(|(f, d)| d.to_json(f)).collect();
        let mut text = if total == 0 {
            format!("checked {} file(s): no errors", report.files_checked)
        } else {
            format!(
                "checked {} file(s): {total} error(s){}\n\n",
                report.files_checked,
                if page.len() < total {
                    format!(", showing {} from {}", page.len(), offset + 1)
                } else {
                    String::new()
                }
            )
        };
        for (file, diag) in &page {
            text.push_str(&diag.render(file));
            text.push('\n');
        }
        let next = (offset + page.len() < total).then_some(offset + page.len());
        result(
            json!({
                "schema": SCHEMA_VERSION,
                "files_checked": report.files_checked,
                "error_count": total,
                "diagnostics": diagnostics,
                "next_offset": next,
            }),
            text,
            false,
        )
    }

    fn list_tests(&self, path: &Path) -> Value {
        let out = Command::new(exe())
            .args(["test", "--list", "--json"])
            .arg(path)
            .stdin(Stdio::null())
            .output();
        match out {
            Ok(out) => match serde_json::from_slice::<Value>(&out.stdout) {
                Ok(list) => {
                    let text = list["tests"]
                        .as_array()
                        .map(|tests| {
                            tests
                                .iter()
                                .map(|t| {
                                    format!(
                                        "{} {}",
                                        t["kind"].as_str().unwrap_or(""),
                                        t["id"].as_str().unwrap_or("")
                                    )
                                })
                                .collect::<Vec<_>>()
                                .join("\n")
                        })
                        .unwrap_or_default();
                    let text = if text.is_empty() {
                        "no tests".into()
                    } else {
                        text
                    };
                    result(
                        json!({ "schema": SCHEMA_VERSION, "tests": list["tests"] }),
                        text,
                        false,
                    )
                }
                Err(_) => failure(String::from_utf8_lossy(&out.stderr).trim().to_string()),
            },
            Err(e) => failure(format!("cannot run kiln test: {e}")),
        }
    }

    /// Runs `kiln test --json`, which an agent can cancel.
    fn run_tests(&mut self, id: &Value, path: &Path, args: &Value) -> Value {
        let mut command = Command::new(exe());
        command.args(["test", "--json"]).arg(path);
        if let Some(filter) = args.get("filter").and_then(Value::as_str) {
            command.args(["--filter", filter]);
        }
        for kind in args
            .get("kinds")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            match kind.as_str() {
                Some(k @ ("unit" | "integration" | "browser")) => {
                    command.arg(format!("--{k}"));
                }
                _ => return failure("kinds are `unit`, `integration` and `browser`"),
            }
        }
        if let Some(t) = args.get("timeout").and_then(Value::as_u64) {
            command.args(["--timeout", &t.to_string()]);
        }
        let mut child = match command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => return failure(format!("cannot run kiln test: {e}")),
        };
        let collect = |mut r: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut s = Vec::new();
                let _ = r.read_to_end(&mut s);
                String::from_utf8_lossy(&s).into_owned()
            })
        };
        let stdout = collect(Box::new(child.stdout.take().unwrap()));
        let stderr = collect(Box::new(child.stderr.take().unwrap()));
        let status = loop {
            if let Ok(Some(status)) = child.try_wait() {
                break Some(status);
            }
            match self.incoming.recv_timeout(Duration::from_millis(100)) {
                Ok(m) => {
                    let cancels = m.get("method").and_then(Value::as_str)
                        == Some("notifications/cancelled")
                        && m.pointer("/params/requestId") == Some(id);
                    if cancels {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    if m.get("method").and_then(Value::as_str) == Some("ping") {
                        if let Some(reply) = self.handle(m) {
                            send(&reply);
                        }
                    } else {
                        self.queued.push_back(m);
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
        };
        let out = stdout.join().unwrap_or_default();
        let err = stderr.join().unwrap_or_default();
        let Some(status) = status else {
            return failure("the test run was cancelled");
        };
        let Ok(report) = serde_json::from_str::<Value>(out.trim()) else {
            return failure(format!(
                "kiln test failed before running tests ({status}):\n{}",
                err.trim()
            ));
        };
        let mut text = String::new();
        for t in report["tests"].as_array().into_iter().flatten() {
            let status = t["status"].as_str().unwrap_or("");
            text.push_str(&format!("{status} {}\n", t["id"].as_str().unwrap_or("")));
            if let Some(message) = t.pointer("/failure/message").and_then(Value::as_str) {
                let at = t
                    .pointer("/failure/at")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                text.push_str(&format!("    {at}: {}\n", message.replace('\n', "\n    ")));
            }
        }
        text.push_str(&format!("\n{}", report["summary"]));
        // Exit code 1 is failing tests, which is a result, not an error.
        let broken = status.code() != Some(0) && status.code() != Some(1);
        result(
            json!({ "schema": SCHEMA_VERSION, "exit_code": status.code(), "report": report }),
            text,
            broken,
        )
    }

    fn build(&self, path: &Path) -> Value {
        let root = match project::root(path) {
            Ok(r) => r,
            Err(e) => return failure(e.to_string()),
        };
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".into());
        let out = root
            .join(".kiln")
            .join("mcp")
            .join(crate::Target::host().executable(&name));
        let output = Command::new(exe())
            .args(["--offline", "build"])
            .arg(&root)
            .arg("-o")
            .arg(&out)
            .stdin(Stdio::null())
            .output();
        match output {
            Ok(o) if o.status.success() => {
                let text = format!("built {}", shown(&out));
                result(
                    json!({ "schema": SCHEMA_VERSION, "executable": shown(&out) }),
                    text,
                    false,
                )
            }
            Ok(o) => failure(String::from_utf8_lossy(&o.stderr).trim().to_string()),
            Err(e) => failure(format!("cannot run kiln build: {e}")),
        }
    }
}

fn exe() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("kiln"))
}

fn shown(path: &Path) -> String {
    path.display().to_string()
}

fn explain(code: &str) -> Value {
    let Some(entry) = kiln_diagnostics::codes::lookup(code.trim()) else {
        return failure(format!("unknown error code `{code}`"));
    };
    let text = format!(
        "{}: {}\n\n{}\n\nWrong:\n\n{}\n\nRight:\n\n{}",
        entry.code, entry.title, entry.explanation, entry.wrong, entry.right
    );
    result(
        json!({
            "schema": SCHEMA_VERSION,
            "code": entry.code,
            "title": entry.title,
            "explanation": entry.explanation,
            "wrong": entry.wrong,
            "right": entry.right,
        }),
        text,
        false,
    )
}

/// The project's modules, files, imports and exports.
fn graph(path: &Path) -> Value {
    use kiln_syntax::ast::Item;
    let project = match project::discover(path) {
        Ok(p) => p,
        Err(e) => {
            let (file, diag) = project::error_diagnostic(&e, &path.join("kiln.toml"));
            return failure(diag.render(&file));
        }
    };
    let root = &project.root;
    let mut modules: std::collections::BTreeMap<String, Value> = Default::default();
    for file in &project.files {
        let rel = file.strip_prefix(root).unwrap_or(file);
        let module = rel
            .parent()
            .map(|p| p.to_string_lossy().replace(['/', '\\'], "."))
            .unwrap_or_default();
        if module.starts_with("__test") {
            continue;
        }
        let entry = modules.entry(module.clone()).or_insert_with(|| {
            json!({
                "module": module,
                "ware": module.starts_with("__ware."),
                "files": [],
                "imports": [],
                "exports": [],
            })
        });
        entry["files"]
            .as_array_mut()
            .unwrap()
            .push(json!(rel.to_string_lossy().replace('\\', "/")));
        if crate::is_glaze(file) {
            continue;
        }
        let Ok(text) = project::read(file) else {
            continue;
        };
        let source = kiln_diagnostics::SourceFile::new(file.display().to_string(), text.clone());
        let parsed = kiln_syntax::parse(&text).module;
        for item in &parsed.items {
            let (kind, public, name, span) = match item {
                Item::Import(i) => {
                    for path in &i.paths {
                        let name = path
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        let imports = entry["imports"].as_array_mut().unwrap();
                        if !imports.contains(&json!(name)) {
                            imports.push(json!(name));
                        }
                    }
                    continue;
                }
                Item::Fn(f) => ("fn", f.is_pub, &f.name.name, f.span),
                Item::Struct(s) => ("struct", s.is_pub, &s.name.name, s.span),
                Item::Enum(e) => ("enum", e.is_pub, &e.name.name, e.span),
                Item::Const(c) => ("const", c.is_pub, &c.name.name, c.span),
                Item::Impl(_) => continue,
            };
            if !public {
                continue;
            }
            let item_text = &text[span.start as usize..span.end as usize];
            let signature = match kind {
                "fn" => item_text.split('{').next().unwrap_or(item_text).trim(),
                _ => item_text
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim_end_matches('{')
                    .trim(),
            };
            entry["exports"].as_array_mut().unwrap().push(json!({
                "kind": kind,
                "name": name,
                "signature": signature,
                "file": rel.to_string_lossy().replace('\\', "/"),
                "line": source.line_col(span.start).line,
            }));
        }
    }
    let modules: Vec<Value> = modules.into_values().collect();
    let entry = project
        .entry
        .strip_prefix(root)
        .unwrap_or(&project.entry)
        .to_string_lossy()
        .replace('\\', "/");
    let mut text = format!("project {}\n", shown(root));
    for m in &modules {
        let name = m["module"].as_str().unwrap_or("");
        text.push_str(&format!(
            "\nmodule {}{}\n",
            if name.is_empty() { "(root)" } else { name },
            if m["ware"] == json!(true) {
                " (Ware)"
            } else {
                ""
            }
        ));
        let imports: Vec<&str> = m["imports"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        if !imports.is_empty() {
            text.push_str(&format!("  imports {}\n", imports.join(", ")));
        }
        for e in m["exports"].as_array().unwrap() {
            text.push_str(&format!("  {}\n", e["signature"].as_str().unwrap_or("")));
        }
    }
    result(
        json!({
            "schema": SCHEMA_VERSION,
            "root": shown(root),
            "entry": entry,
            "modules": modules,
        }),
        text,
        false,
    )
}

/// Formats a file or folder: shows what would change, or writes it.
fn format(path: &Path, write: bool) -> Value {
    let mut files = Vec::new();
    crate::fmt::collect(path, &mut files);
    let mut changed = Vec::new();
    let mut problems = Vec::new();
    let mut text = String::new();
    for file in &files {
        let Ok(before) = std::fs::read_to_string(file) else {
            continue;
        };
        match crate::fmt::format_file(file, &before) {
            Ok(after) if after == before => {}
            Ok(after) => {
                if write {
                    if let Err(e) = std::fs::write(file, &after) {
                        problems.push(json!({ "file": shown(file), "error": e.to_string() }));
                        continue;
                    }
                } else if files.len() == 1 || changed.len() < 20 {
                    text.push_str(&format!("--- {}\n{after}\n", shown(file)));
                }
                changed.push(json!({
                    "file": shown(file),
                    "formatted": (!write).then_some(after),
                }));
            }
            Err(e) => problems.push(json!({ "file": shown(file), "error": e })),
        }
    }
    let summary = match (changed.len(), write) {
        (0, _) => format!("{} file(s) already formatted", files.len()),
        (n, true) => format!("formatted {n} file(s)"),
        (n, false) => format!("{n} file(s) would change"),
    };
    let text = format!("{summary}\n\n{text}");
    let is_error = !problems.is_empty();
    result(
        json!({ "schema": SCHEMA_VERSION, "changed": changed, "problems": problems }),
        text,
        is_error,
    )
}
