//! Filesystem routes: `app.file_routes("pages", at: "/", ignore: [...])`.
//!
//! A mount is expanded when the program is compiled, never by scanning
//! folders while it runs. Project discovery brings the mounted folder into
//! the build ([`mount_dirs`]); then, before the checker sees the program,
//! [`analyze`] turns its `.glaze` pages and `route.kiln` files into
//! ordinary `app.get`/`app.post`/... registrations: the call becomes a
//! call of a function written at the end of its file, which adds the
//! routes in the order the router should try them. So filesystem routes
//! are the same routes as written ones, with the same hooks, errors,
//! static export and pre-rendering.
//!
//! The rules (docs/plans/phase-6.7.md, docs/guides/file-routes.md): a
//! `.glaze` file is a page (`index.glaze` the folder's), `[name]` a URL
//! parameter that is a whole segment, and a `route.kiln` beside a page
//! may export `load` (the page's props), HTTP method handlers,
//! `static_params`, `static_refresh` and `dynamic`. Errors point at the
//! file they are about.

use super::bind::module_symbol;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::*;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The HTTP methods a `route.kiln` can answer, as its functions are named.
const METHODS: [&str; 5] = ["get", "post", "put", "patch", "delete"];
/// The folders a mount leaves out when it gives no `ignore:`.
const DEFAULT_IGNORE: &str = "_components";
/// The file that holds a route folder's code.
const ROUTE_FILE: &str = "route.kiln";

/// An `app.file_routes(...)` call, as written.
#[derive(Clone, Debug)]
pub struct MountCall {
    /// The whole call, and the expression it is called on.
    pub span: Span,
    pub receiver: Span,
    /// The folder, relative to the project root, when it is usable.
    pub dir: Option<(String, Span)>,
    /// The URL prefix, normalized (`/` or `/admin`).
    pub at: String,
    /// The folder names it leaves out.
    pub ignore: Vec<String>,
    pub errors: Vec<Diagnostic>,
}

fn error(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(codes::FILE_ROUTES, message, span)
}

pub(super) fn literal(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Str(pieces) => {
            let mut out = String::new();
            for p in pieces {
                match p {
                    StrPiece::Text(t) => out.push_str(t),
                    StrPiece::Expr(_) => return None,
                }
            }
            Some(out)
        }
        _ => None,
    }
}

/// Whether a folder of a mount is usable: a relative path inside the
/// project, below its root.
fn check_dir(dir: &str) -> Result<(), String> {
    if dir.is_empty() || dir == "." || dir == "./" {
        return Err("mount a folder below the project root, such as `\"pages\"`".into());
    }
    if dir.starts_with('/') || dir.contains('\\') || dir.contains(':') {
        return Err("the folder is relative to the project root and written with `/`".into());
    }
    for part in dir.trim_end_matches('/').split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("the folder must be a path inside the project, without `.` or `..`".into());
        }
        if part.starts_with("__") {
            return Err("folder names starting with `__` are reserved".into());
        }
    }
    Ok(())
}

/// The URL prefix of a mount, normalized, or why it is not one.
fn check_at(at: &str) -> Result<String, String> {
    if !at.starts_with('/') {
        return Err(format!(
            "`at:` is a URL path starting with `/`, such as `\"/{}\"`",
            at.trim_start_matches('/')
        ));
    }
    if at.contains(['?', '#', '{', '}', '\\', ' ']) {
        return Err(
            "`at:` is a plain URL path, without a query, fragment, parameters or spaces".into(),
        );
    }
    let trimmed = at.trim_end_matches('/');
    if trimmed.is_empty() {
        return Ok("/".into());
    }
    for part in trimmed[1..].split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("`at:` cannot have empty, `.` or `..` segments".into());
        }
    }
    Ok(trimmed.to_string())
}

/// Whether an `ignore:` entry is a plain folder name.
fn check_ignore(name: &str) -> Result<(), String> {
    if name.is_empty() || name == "." || name == ".." {
        return Err("an ignored folder is a folder's name".into());
    }
    if name.contains(['/', '\\']) {
        return Err(format!(
            "`ignore:` takes folder names, which are left out at any depth, not paths; write `\"{}\"`",
            name.rsplit(['/', '\\']).next().unwrap_or(name)
        ));
    }
    if name.contains(['*', '?', '[', ']', '{', '}', '!']) {
        return Err("`ignore:` takes plain folder names, not patterns".into());
    }
    Ok(())
}

/// Reads an `app.file_routes(...)` call.
fn mount_call(e: &Expr) -> Option<MountCall> {
    let ExprKind::Call { callee, args, .. } = &e.kind else {
        return None;
    };
    let ExprKind::Field { expr, name } = &callee.kind else {
        return None;
    };
    if name.name != "file_routes" {
        return None;
    }
    let mut call = MountCall {
        span: e.span,
        receiver: expr.span,
        dir: None,
        at: "/".into(),
        ignore: vec![DEFAULT_IGNORE.into()],
        errors: Vec::new(),
    };
    let mut dir_given = false;
    for (i, arg) in args.iter().enumerate() {
        let key = arg.name.as_ref().map(|n| n.name.as_str());
        match key {
            None | Some("dir") if !dir_given && (key.is_some() || i == 0) => {
                dir_given = true;
                match literal(&arg.value) {
                    Some(d) => match check_dir(&d) {
                        Ok(()) => {
                            call.dir = Some((d.trim_end_matches('/').to_string(), arg.value.span))
                        }
                        Err(m) => call.errors.push(error(m, arg.value.span)),
                    },
                    None => call.errors.push(
                        error(
                            "the folder of `file_routes` is found when the program is compiled, so it is a string literal",
                            arg.value.span,
                        )
                        .with_help("write the folder out, as in `app.file_routes(\"pages\")`"),
                    ),
                }
            }
            Some("at") => match literal(&arg.value) {
                Some(a) => match check_at(&a) {
                    Ok(a) => call.at = a,
                    Err(m) => call.errors.push(error(m, arg.value.span)),
                },
                None => call.errors.push(error(
                    "`at:` is found when the program is compiled, so it is a string literal",
                    arg.value.span,
                )),
            },
            Some("ignore") => match &arg.value.kind {
                ExprKind::List(items) => {
                    call.ignore.clear();
                    for item in items {
                        match literal(item) {
                            Some(n) => match check_ignore(&n) {
                                Ok(()) => call.ignore.push(n),
                                Err(m) => call.errors.push(error(m, item.span)),
                            },
                            None => call.errors.push(error(
                                "an ignored folder is a string literal",
                                item.span,
                            )),
                        }
                    }
                }
                _ => call.errors.push(error(
                    "`ignore:` is a list of folder names written out, such as `[\"_components\", \"partials\"]`",
                    arg.value.span,
                )),
            },
            _ => call.errors.push(error(
                "`file_routes` takes a folder, `at:` and `ignore:`",
                arg.name.as_ref().map_or(arg.value.span, |n| n.span),
            )),
        }
    }
    if !dir_given {
        call.errors.push(error(
            "`file_routes` needs the folder its pages are in, as in `app.file_routes(\"pages\")`",
            e.span,
        ));
    }
    Some(call)
}

pub(super) fn each_block(b: &Block, f: &mut dyn FnMut(&Expr)) {
    for s in &b.stmts {
        match s {
            Stmt::Let { value, .. } => each_expr(value, f),
            Stmt::Assign { target, value, .. } => {
                each_expr(target, f);
                each_expr(value, f);
            }
            Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    each_expr(v, f);
                }
            }
            Stmt::For {
                iter, filter, body, ..
            } => {
                each_expr(iter, f);
                if let Some(x) = filter {
                    each_expr(x, f);
                }
                each_block(body, f);
            }
            Stmt::While { cond, body, .. } => {
                each_expr(cond, f);
                each_block(body, f);
            }
            Stmt::Expr(e) => each_expr(e, f),
            Stmt::Break(_) | Stmt::Continue(_) => {}
        }
    }
}

/// Calls `f` on `e` and every expression inside it, outer ones first.
fn each_expr(e: &Expr, f: &mut dyn FnMut(&Expr)) {
    f(e);
    match &e.kind {
        ExprKind::Str(pieces) => {
            for p in pieces {
                if let StrPiece::Expr(x) = p {
                    each_expr(x, f);
                }
            }
        }
        ExprKind::List(xs) => xs.iter().for_each(|x| each_expr(x, f)),
        ExprKind::Map(kv) => {
            for (k, v) in kv {
                each_expr(k, f);
                each_expr(v, f);
            }
        }
        ExprKind::Spread(x)
        | ExprKind::Unary { expr: x, .. }
        | ExprKind::Try(x)
        | ExprKind::Throw(x)
        | ExprKind::Field { expr: x, .. } => each_expr(x, f),
        ExprKind::Record(fs) => {
            for field in fs {
                if let Some(v) = &field.value {
                    each_expr(v, f);
                }
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            each_expr(lhs, f);
            each_expr(rhs, f);
        }
        ExprKind::Call { callee, args, .. } => {
            each_expr(callee, f);
            for a in args {
                each_expr(&a.value, f);
            }
        }
        ExprKind::Index { expr, index } => {
            each_expr(expr, f);
            each_expr(index, f);
        }
        ExprKind::Closure { body, .. } | ExprKind::Block(body) => each_block(body, f),
        ExprKind::If {
            cond,
            then,
            otherwise,
        } => {
            each_expr(cond, f);
            each_block(then, f);
            if let Some(o) = otherwise {
                each_expr(o, f);
            }
        }
        ExprKind::Match { scrutinee, arms } => {
            each_expr(scrutinee, f);
            for a in arms {
                if let Some(g) = &a.guard {
                    each_expr(g, f);
                }
                each_expr(&a.body, f);
            }
        }
        ExprKind::Catch { expr, handler, .. } => {
            each_expr(expr, f);
            each_block(handler, f);
        }
        ExprKind::Is { expr, .. } => each_expr(expr, f),
        _ => {}
    }
}

/// The bodies of a module's functions and methods, and its constants.
pub(super) fn each_body(module: &Module, f: &mut dyn FnMut(&Block)) {
    for item in &module.items {
        match item {
            Item::Fn(func) => f(&func.body),
            Item::Impl(imp) => imp.methods.iter().for_each(|m| f(&m.body)),
            _ => {}
        }
    }
}

/// The `app.file_routes(...)` calls of a module, in the order of its text.
pub fn mount_calls(module: &Module) -> Vec<MountCall> {
    let mut calls = Vec::new();
    each_body(module, &mut |body| {
        each_block(body, &mut |e| calls.extend(mount_call(e)));
    });
    for item in &module.items {
        if let Item::Const(c) = item {
            each_expr(&c.value, &mut |e| calls.extend(mount_call(e)));
        }
    }
    calls.sort_by_key(|c| c.span.start);
    calls
}

/// The folders a module's mounts bring into the build: each folder below
/// a mounted one that holds source, ignored ones too (their components
/// stay usable). Folders whose names start with `.` are not part of it.
pub fn mount_dirs(root: &Path, module: &Module) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for call in mount_calls(module) {
        let Some((dir, _)) = call.dir else { continue };
        let start = root.join(&dir);
        let mut stack = vec![start];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(super::real(&dir)) else {
                continue;
            };
            let mut has_source = false;
            let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
            entries.sort_by_key(|e| e.file_name());
            for entry in entries {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') {
                    continue;
                }
                let Ok(kind) = entry.file_type() else {
                    continue;
                };
                let path = dir.join(&name);
                if kind.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "kiln") || crate::is_glaze(&path) {
                    has_source = true;
                }
            }
            if has_source {
                out.push(dir);
            }
        }
    }
    out.sort();
    out
}

/// What the type of a URL parameter is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    Bool,
    Int,
    Float,
    String,
}

impl Kind {
    fn of(name: &str) -> Option<Kind> {
        Some(match name {
            "String" => Kind::String,
            "Int" => Kind::Int,
            "Float" => Kind::Float,
            "Bool" => Kind::Bool,
            _ => return None,
        })
    }
    fn name(self) -> &'static str {
        match self {
            Kind::String => "String",
            Kind::Int => "Int",
            Kind::Float => "Float",
            Kind::Bool => "Bool",
        }
    }
    /// Whether a URL segment is a value of this kind, as the router reads
    /// it (`k_web_param` in kiln.c).
    fn accepts(self, s: &str) -> bool {
        match self {
            Kind::String => !s.is_empty(),
            Kind::Bool => s == "true" || s == "false",
            Kind::Int => {
                s.strip_prefix('-').unwrap_or(s).parse::<u64>().is_ok() && s.parse::<i64>().is_ok()
            }
            Kind::Float => {
                s.starts_with(|c: char| c == '-' || c == '.' || c.is_ascii_digit())
                    && s.parse::<f64>().is_ok_and(f64::is_finite)
            }
        }
    }
    /// Whether every segment this kind accepts the other does too.
    fn within(self, other: Kind) -> bool {
        self == other || other == Kind::String || (self == Kind::Int && other == Kind::Float)
    }
    fn meets(self, other: Kind) -> bool {
        !matches!(
            (self, other),
            (Kind::Bool, Kind::Int | Kind::Float) | (Kind::Int | Kind::Float, Kind::Bool)
        )
    }
    /// A segment both kinds accept.
    fn sample(self, other: Kind) -> &'static str {
        match (self, other) {
            (Kind::Bool, _) | (_, Kind::Bool) => "true",
            (Kind::Int, _) | (_, Kind::Int) => "1",
            (Kind::Float, _) | (_, Kind::Float) => "1.5",
            _ => "x",
        }
    }
}

/// A segment of a route's URL.
#[derive(Clone, Debug, PartialEq)]
enum Seg {
    Lit(String),
    Param(String, Kind),
}

impl Seg {
    fn within(&self, other: &Seg) -> bool {
        match (self, other) {
            (Seg::Lit(a), Seg::Lit(b)) => a == b,
            (Seg::Lit(a), Seg::Param(_, k)) => k.accepts(a),
            (Seg::Param(..), Seg::Lit(_)) => false,
            (Seg::Param(_, a), Seg::Param(_, b)) => a.within(*b),
        }
    }
    fn meets(&self, other: &Seg) -> Option<String> {
        match (self, other) {
            (Seg::Lit(a), Seg::Lit(b)) => (a == b).then(|| a.clone()),
            (Seg::Lit(a), Seg::Param(_, k)) | (Seg::Param(_, k), Seg::Lit(a)) => {
                k.accepts(a).then(|| a.clone())
            }
            (Seg::Param(_, a), Seg::Param(_, b)) => a.meets(*b).then(|| a.sample(*b).to_string()),
        }
    }
    /// How general the segment is, to try specific routes first.
    fn rank(&self) -> u8 {
        match self {
            Seg::Lit(_) => 0,
            Seg::Param(_, Kind::Bool | Kind::Int) => 1,
            Seg::Param(_, Kind::Float) => 2,
            Seg::Param(_, Kind::String) => 3,
        }
    }
}

fn within(a: &[Seg], b: &[Seg]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.within(y))
}

/// A URL both patterns match, if there is one.
fn meet(a: &[Seg], b: &[Seg]) -> Option<String> {
    if a.len() != b.len() {
        return None;
    }
    let mut parts = Vec::new();
    for (x, y) in a.iter().zip(b) {
        parts.push(x.meets(y)?);
    }
    Some(url_of(&parts))
}

fn url_of(parts: &[String]) -> String {
    let path = parts.join("/");
    if path.is_empty() {
        "/".into()
    } else {
        format!("/{path}")
    }
}

/// The pattern `app.get` takes for these segments.
fn pattern(segs: &[Seg]) -> String {
    let parts: Vec<String> = segs
        .iter()
        .map(|s| match s {
            Seg::Lit(l) => l.clone(),
            Seg::Param(n, Kind::String) => format!("{{{n}}}"),
            Seg::Param(n, k) => format!("{{{n}: {}}}", k.name()),
        })
        .collect();
    url_of(&parts)
}

/// A file or folder name's URL segment: `[name]` is a parameter.
fn segment(name: &str) -> Result<Seg, String> {
    if name.starts_with("[...") || name.starts_with("[[...") {
        return Err(format!(
            "`{name}` would be a catch-all route, which `file_routes` does not have: `[name]` matches one segment; register a route by hand for deeper paths"
        ));
    }
    if name.starts_with("[[") && name.ends_with("]]") {
        let inner = &name[2..name.len() - 2];
        return Err(format!(
            "`{name}` would be an optional segment, which `file_routes` does not have; make two pages, `index.glaze` and `[{inner}].glaze`"
        ));
    }
    if name.starts_with('(') && name.ends_with(')') {
        let inner = &name[1..name.len() - 1];
        return Err(format!(
            "`{name}` would be a route group, which `file_routes` does not have: a folder is a URL segment; put its pages one folder up, in `{inner}/`, or mount the folder with `app.file_routes(..., at: ...)`"
        ));
    }
    if let Some(inner) = name.strip_prefix('[').and_then(|n| n.strip_suffix(']')) {
        let ident = inner.starts_with(|c: char| c.is_ascii_lowercase() || c == '_')
            && inner
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if inner.is_empty() || !ident {
            return Err(format!(
                "`{name}`: a URL parameter's name is a lowercase Kiln name, such as `[slug]`"
            ));
        }
        if inner.starts_with("__")
            || kiln_syntax::token::TokenKind::keyword(inner).is_some()
            || inner == "req"
        {
            return Err(format!(
                "`{inner}` cannot be a URL parameter's name; rename it"
            ));
        }
        return Ok(Seg::Param(inner.to_string(), Kind::String));
    }
    if name.contains(['[', ']']) {
        return Err(format!(
            "`{name}`: a URL parameter is a whole segment, `[name]`, not part of one"
        ));
    }
    let fits =
        |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') || !c.is_ascii();
    if name.is_empty() || !name.chars().all(fits) {
        return Err(format!(
            "`{name}` cannot be part of a URL: a route's folder and file names use letters, digits, `-`, `_`, `.` and `~`"
        ));
    }
    Ok(Seg::Lit(name.to_string()))
}

/// What the plan adds to one file of the program.
#[derive(Default, Debug)]
pub struct FilePlan {
    /// Replacements in its text: each mount call becomes a call of the
    /// function that adds its routes.
    pub edits: Vec<(Span, String)>,
    /// What is written after its text: those functions.
    pub appended: String,
    /// Where parts of `appended` come from, to point errors in them at
    /// the file and span they are about: (start, end, file, span).
    pub regions: Vec<(usize, usize, usize, Span)>,
}

/// A route a mount adds, for the editor, `kiln fire --static` and tests.
#[derive(Clone, Debug)]
pub struct RouteInfo {
    pub method: &'static str,
    pub pattern: String,
    /// The file that answers it, and where in it.
    pub file: usize,
    pub span: Span,
    /// The file with the `file_routes` call and its span.
    pub mount: (usize, Span),
    pub dynamic: bool,
    /// A folder with a `route.kiln` and no page and no `GET`, which a
    /// static site cannot have, and which says nothing about it.
    pub unexported: bool,
}

/// What [`analyze`] makes of a program's mounts.
#[derive(Default, Debug)]
pub struct Plan {
    pub files: BTreeMap<usize, FilePlan>,
    pub diagnostics: Vec<(usize, Diagnostic)>,
    pub warnings: Vec<(usize, Diagnostic)>,
    pub routes: Vec<RouteInfo>,
}

/// A source of a route parameter's type, for errors about it.
struct Declared {
    kind: Kind,
    file: usize,
    span: Span,
    what: String,
}

/// A route a mount found, before its parameters' types are known.
struct Candidate {
    /// The URL below the mount, with parameters' names.
    segs: Vec<Seg>,
    /// The folder's module, which owns `route.kiln`.
    owner: String,
    glaze: Option<usize>,
    route: Option<usize>,
    /// The file to point at when the route as a whole is wrong.
    anchor: usize,
}

/// A `route.kiln`'s exports.
#[derive(Default)]
struct Exports<'a> {
    load: Option<&'a FnDecl>,
    methods: BTreeMap<&'static str, &'a FnDecl>,
    static_params: Option<&'a FnDecl>,
    static_refresh: Option<&'a ConstDecl>,
    dynamic: Option<(bool, Span)>,
}

/// One argument a handler or loader takes.
enum Binding {
    Request,
    Param(usize),
}

/// A route ready to write: its method, segments and the Kiln code of its
/// handler and of what follows the registration.
struct Built {
    method: &'static str,
    segs: Vec<Seg>,
    file: usize,
    span: Span,
    /// Parts of the generated handler, each with what errors in it are
    /// about.
    handler: Vec<(String, Option<(usize, Span)>)>,
    suffix: Vec<(String, Option<(usize, Span)>)>,
    /// The file the route comes from, as shown in errors.
    source: String,
    dynamic: bool,
    unexported: bool,
}

pub struct Inputs<'a> {
    pub root: &'a Path,
    pub texts: &'a [(PathBuf, String)],
    pub modules: &'a [Option<Module>],
    pub packages: &'a super::Packages,
}

impl Inputs<'_> {
    fn rel(&self, i: usize) -> String {
        self.texts[i]
            .0
            .strip_prefix(self.root)
            .unwrap_or(&self.texts[i].0)
            .to_string_lossy()
            .replace('\\', "/")
    }
    fn owner(&self, i: usize) -> String {
        self.texts[i]
            .0
            .parent()
            .unwrap_or(self.root)
            .strip_prefix(self.root)
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .replace(['/', '\\'], ".")
    }
}

/// Turns a program's `app.file_routes` mounts into routes. `texts` are
/// all the files of the build and `modules` the parsed `.kiln` ones.
pub fn analyze(inputs: &Inputs) -> Plan {
    let mut plan = Plan::default();
    let mut glazes: BTreeMap<usize, Option<Glaze>> = BTreeMap::new();
    for (i, module) in inputs.modules.iter().enumerate() {
        let Some(module) = module else { continue };
        let calls = mount_calls(module);
        if calls.is_empty() {
            continue;
        }
        let mut file = FilePlan::default();
        let imports_web = module.items.iter().any(|item| {
            matches!(item, Item::Import(imp) if imp.paths.iter().any(|p| p.len() == 1 && p[0].name == "web"))
        });
        if !imports_web {
            file.edits.push((Span::new(0, 0), "import web\n".into()));
        }
        // Each mount's registrations and explicit ones, to warn about
        // routes that earlier ones shadow.
        let mut order: Vec<(usize, Registered)> = explicit_routes(module)
            .into_iter()
            .map(|r| (r.span.start as usize, r))
            .collect();
        for (k, call) in calls.iter().enumerate() {
            let name = format!("__kiln_routes_{i}_{k}");
            file.edits.push((
                Span::new(call.span.start as usize, call.receiver.start as usize),
                format!("{name}("),
            ));
            file.edits.push((
                Span::new(call.receiver.end as usize, call.span.end as usize),
                ")".into(),
            ));
            for d in &call.errors {
                plan.diagnostics.push((i, d.clone()));
            }
            let built = match (&call.dir, call.errors.is_empty()) {
                (Some(dir), true) => mount(inputs, i, call, dir, &mut glazes, &mut plan),
                _ => Vec::new(),
            };
            order.push((
                call.span.start as usize,
                Registered {
                    routes: built
                        .iter()
                        .map(|b| (b.method, b.segs.clone(), b.source.clone()))
                        .collect(),
                    span: call.span,
                    mount: true,
                },
            ));
            write_mount(&mut file, &name, i, call, &built);
            for b in &built {
                plan.routes.push(RouteInfo {
                    method: b.method,
                    pattern: pattern(&b.segs),
                    file: b.file,
                    span: b.span,
                    mount: (i, call.span),
                    dynamic: b.dynamic,
                    unexported: b.unexported,
                });
            }
        }
        order.sort_by_key(|(at, _)| *at);
        shadowing(&order, i, &mut plan);
        plan.files.insert(i, file);
    }
    plan
}

/// A registration in a function, for [`shadowing`]: an `app.get("/x",
/// ...)` written out, or the routes of a mount.
struct Registered {
    routes: Vec<(&'static str, Vec<Seg>, String)>,
    span: Span,
    mount: bool,
}

/// The routes a module registers with a literal pattern.
fn explicit_routes(module: &Module) -> Vec<Registered> {
    let mut out = Vec::new();
    each_body(module, &mut |body| {
        each_block(body, &mut |e| {
            let ExprKind::Call { callee, args, .. } = &e.kind else {
                return;
            };
            let ExprKind::Field { name, .. } = &callee.kind else {
                return;
            };
            let Some(method) = METHODS.iter().find(|m| **m == name.name) else {
                return;
            };
            let Some(first) = args.first().filter(|a| a.name.is_none()) else {
                return;
            };
            let Some(text) = literal(&first.value) else {
                return;
            };
            if !text.starts_with('/') || args.len() < 2 {
                return;
            }
            let segs = text
                .trim_start_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .map(
                    |s| match s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                        Some(inner) => {
                            let (n, t) = inner.split_once(':').unwrap_or((inner, "String"));
                            Seg::Param(n.trim().into(), Kind::of(t.trim()).unwrap_or(Kind::String))
                        }
                        None => Seg::Lit(s.to_string()),
                    },
                )
                .collect();
            out.push(Registered {
                routes: vec![(method, segs, format!("`app.{method}(\"{text}\", ...)`"))],
                span: first.value.span,
                mount: false,
            });
        });
    });
    out
}

/// Warns about filesystem routes an earlier registration in the same file
/// always answers first, and written routes an earlier mount answers.
fn shadowing(order: &[(usize, Registered)], file: usize, plan: &mut Plan) {
    for (b_index, (_, b)) in order.iter().enumerate() {
        for (_, a) in &order[..b_index] {
            if !a.mount && !b.mount {
                continue;
            }
            for (method, segs, source) in &b.routes {
                let Some((_, _, by)) = a
                    .routes
                    .iter()
                    .find(|(m, s, _)| m == method && within(segs, s))
                else {
                    continue;
                };
                let what = if b.mount {
                    format!("{} {} from {source}", method.to_uppercase(), pattern(segs))
                } else {
                    format!("{} {}", method.to_uppercase(), pattern(segs))
                };
                plan.warnings.push((
                    file,
                    Diagnostic::warning(
                        codes::ROUTE_SHADOWED,
                        format!(
                            "{what} is answered first by {by}, registered earlier, so this route never runs"
                        ),
                        b.span,
                    )
                    .with_help("the first route that matches answers; register this one first if it should win"),
                ));
            }
        }
    }
}

/// The routes of one mount, in the order to register them.
fn mount(
    inputs: &Inputs,
    call_file: usize,
    call: &MountCall,
    (dir, dir_span): &(String, Span),
    glazes: &mut BTreeMap<usize, Option<Glaze>>,
    plan: &mut Plan,
) -> Vec<Built> {
    let base = inputs.root.join(dir);
    if !super::real(&base).is_dir() {
        plan.diagnostics.push((
            call_file,
            error(
                format!("there is no folder `{dir}/` in the project"),
                *dir_span,
            ),
        ));
        return Vec::new();
    }
    let prefix: Vec<Seg> = call
        .at
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .map(|s| Seg::Lit(s.to_string()))
        .collect();
    // The files below the mount, by folder (relative parts).
    let mut folders: BTreeMap<Vec<String>, Vec<usize>> = BTreeMap::new();
    for (i, (path, _)) in inputs.texts.iter().enumerate() {
        let Ok(rest) = path.strip_prefix(&base) else {
            continue;
        };
        let parts: Vec<String> = rest
            .parent()
            .unwrap_or(Path::new(""))
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect();
        if parts
            .iter()
            .any(|p| p.starts_with('.') || call.ignore.contains(p))
        {
            continue;
        }
        folders.entry(parts).or_default().push(i);
    }
    let mut candidates = Vec::new();
    let mut bad_folders = std::collections::BTreeSet::new();
    for (parts, files) in &folders {
        let mut segs = Vec::new();
        let mut ok = true;
        for part in parts {
            match segment(part) {
                Ok(s) => segs.push(s),
                Err(m) => {
                    ok = false;
                    let folder = format!("{dir}/{}", parts.join("/"));
                    if bad_folders.insert(folder.clone()) {
                        plan.diagnostics.push((
                            files[0],
                            error(
                                format!("in the routes of `{folder}/`: {m}"),
                                Span::new(0, 0),
                            ),
                        ));
                    }
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let owner = inputs.owner(files[0]);
        let mut index = None;
        let mut route = None;
        for &i in files {
            let path = &inputs.texts[i].0;
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            if name == ROUTE_FILE {
                route = Some(i);
            } else if let Some(stem) = name.strip_suffix(".glaze") {
                if stem == "index" {
                    index = Some(i);
                    continue;
                }
                match segment(stem) {
                    Ok(s) => {
                        let mut segs = segs.clone();
                        segs.push(s);
                        candidates.push(Candidate {
                            segs,
                            owner: owner.clone(),
                            glaze: Some(i),
                            route: None,
                            anchor: i,
                        });
                    }
                    Err(m) => plan.diagnostics.push((i, error(m, Span::new(0, 0)))),
                }
            }
        }
        if index.is_some() || route.is_some() {
            candidates.push(Candidate {
                segs: segs.clone(),
                owner,
                glaze: index,
                route,
                anchor: index.or(route).unwrap(),
            });
        }
    }
    let mut built = Vec::new();
    for c in &candidates {
        let mut segs = prefix.clone();
        segs.extend(c.segs.iter().cloned());
        let names: Vec<&str> = segs
            .iter()
            .filter_map(|s| match s {
                Seg::Param(n, _) => Some(n.as_str()),
                _ => None,
            })
            .collect();
        if let Some(dup) = names
            .iter()
            .enumerate()
            .find(|(k, n)| names[..*k].contains(n))
            .map(|(_, n)| *n)
        {
            plan.diagnostics.push((
                c.anchor,
                Diagnostic::error(
                    codes::FILE_ROUTES,
                    format!(
                        "the URL {} has two parameters named `{dup}`; rename one",
                        pattern(&segs)
                    ),
                    Span::new(0, 0),
                ),
            ));
            continue;
        }
        if let Some(g) = c.glaze {
            glazes.entry(g).or_insert_with(|| {
                let parsed = kiln_syntax::glaze::parse_component(
                    &inputs.texts[g].1,
                    &inputs.texts[g].0.to_string_lossy(),
                );
                parsed.diagnostics.is_empty().then_some(parsed.glaze)
            });
        }
        // A page with syntax errors is left to the checker, which reports
        // them; its route cannot be made yet.
        let glaze = match c.glaze {
            Some(g) => match glazes[&g].clone() {
                Some(glaze) => Some(glaze),
                None => continue,
            },
            None => None,
        };
        built.extend(candidate(inputs, c, segs, glaze.as_ref(), plan));
    }
    conflicts(inputs, &built, plan);
    // Specific routes first: literal segments before parameters, narrow
    // types before wide ones; ties keep the order of the files.
    built.sort_by(|a, b| {
        let ra: Vec<u8> = a.segs.iter().map(Seg::rank).collect();
        let rb: Vec<u8> = b.segs.iter().map(Seg::rank).collect();
        ra.cmp(&rb).then(a.source.cmp(&b.source))
    });
    built
}

/// Errors for routes of one mount that claim the same URLs, and for those
/// that claim kiln's own.
fn conflicts(inputs: &Inputs, built: &[Built], plan: &mut Plan) {
    for (k, b) in built.iter().enumerate() {
        if let Some(Seg::Lit(first)) = b.segs.first() {
            if format!("/{first}") == kiln_ir::glaze::RESERVED_PREFIX {
                plan.diagnostics.push((
                    b.file,
                    Diagnostic::error(
                        codes::ROUTE_CONFLICT,
                        format!(
                            "{} {} is under {}, where kiln serves its own files and calls",
                            b.method.to_uppercase(),
                            pattern(&b.segs),
                            kiln_ir::glaze::RESERVED_PREFIX
                        ),
                        b.span,
                    )
                    .with_help("move the page, or mount the folder with another `at:`"),
                ));
                continue;
            }
        }
        for (method, reserved) in kiln_ir::glaze::RESERVED_ROUTES {
            if !method.eq_ignore_ascii_case(b.method) {
                continue;
            }
            let theirs: Vec<Seg> = reserved
                .trim_start_matches('/')
                .split('/')
                .map(
                    |s| match s.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
                        Some(n) => Seg::Param(n.into(), Kind::String),
                        None => Seg::Lit(s.into()),
                    },
                )
                .collect();
            if let Some(url) = meet(&b.segs, &theirs) {
                plan.diagnostics.push((
                    b.file,
                    Diagnostic::error(
                        codes::ROUTE_CONFLICT,
                        format!(
                            "{} {} could answer {url}, which is kiln's own route {method} {reserved}",
                            b.method.to_uppercase(),
                            pattern(&b.segs),
                        ),
                        b.span,
                    )
                    .with_help("rename a folder so the route cannot match it, or mount the folder with another `at:`"),
                ));
            }
        }
        for a in &built[..k] {
            if a.method != b.method {
                continue;
            }
            let Some(url) = meet(&a.segs, &b.segs) else {
                continue;
            };
            let (ab, ba) = (within(&a.segs, &b.segs), within(&b.segs, &a.segs));
            let message = if ab && ba {
                if a.segs == b.segs || a.file == b.file {
                    format!(
                        "{} {} is defined twice, by {} and {}",
                        b.method.to_uppercase(),
                        pattern(&b.segs),
                        a.source,
                        b.source
                    )
                } else {
                    format!(
                        "{} {} and {} {} match the same URLs, such as {url}, so neither can win",
                        a.method.to_uppercase(),
                        pattern(&a.segs),
                        b.method.to_uppercase(),
                        pattern(&b.segs),
                    )
                }
            } else if ab || ba {
                continue;
            } else {
                format!(
                    "{} {} ({}) and {} ({}) both match {url}, and each matches URLs the other does not, so neither can win",
                    b.method.to_uppercase(),
                    pattern(&a.segs),
                    a.source,
                    pattern(&b.segs),
                    b.source
                )
            };
            let _ = inputs;
            plan.diagnostics.push((
                b.file,
                Diagnostic::error(codes::ROUTE_CONFLICT, message, b.span).with_help(format!(
                    "rename or move one of them, or add a page for {url} itself"
                )),
            ));
        }
    }
}

fn route_error(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(codes::ROUTE_FILE, message, span)
}

/// `web.Request`, as a parameter's type.
fn is_request(t: &TypeExpr) -> bool {
    matches!(&t.kind, TypeKind::Named { path, args } if args.is_empty()
        && path.iter().map(|p| p.name.as_str()).collect::<Vec<_>>() == ["web", "Request"])
}

fn is_response(t: &TypeExpr) -> bool {
    matches!(&t.kind, TypeKind::Named { path, args } if args.is_empty()
        && path.iter().map(|p| p.name.as_str()).collect::<Vec<_>>() == ["web", "Response"])
}

/// The URL parameter type a type expression names.
fn kind_of(t: &TypeExpr) -> Result<Kind, String> {
    match &t.kind {
        TypeKind::Named { path, args } if path.len() == 1 && args.is_empty() => {
            Kind::of(&path[0].name).ok_or_else(|| {
                format!(
                    "a URL parameter is a `String`, `Int`, `Float` or `Bool`, not `{}`",
                    path[0].name
                )
            })
        }
        TypeKind::Optional(_) => Err(
            "a URL parameter is always in the URL, so its type is not optional; remove the `?`"
                .into(),
        ),
        _ => Err("a URL parameter is a `String`, `Int`, `Float` or `Bool`".into()),
    }
}

/// The exports of a `route.kiln`, with errors and warnings about them.
fn exports<'a>(module: &'a Module, file: usize, plan: &mut Plan) -> Exports<'a> {
    let mut out = Exports::default();
    for item in &module.items {
        match item {
            Item::Fn(f) => {
                let name = f.name.name.as_str();
                let known = name == "load" || name == "static_params" || METHODS.contains(&name);
                if !known {
                    continue;
                }
                if !f.is_pub {
                    plan.warnings.push((
                        file,
                        Diagnostic::warning(
                            codes::ROUTE_FILE,
                            format!("`{name}` is private, so it is not part of the route; write `pub fn {name}` if it should be"),
                            f.name.span,
                        ),
                    ));
                    continue;
                }
                if !f.generics.is_empty() {
                    plan.diagnostics.push((
                        file,
                        route_error(
                            format!("`{name}` is part of the route, so it cannot be generic"),
                            f.name.span,
                        ),
                    ));
                    continue;
                }
                match name {
                    "load" => out.load = Some(f),
                    "static_params" => out.static_params = Some(f),
                    m => {
                        let m = METHODS.iter().find(|x| **x == m).unwrap();
                        out.methods.insert(m, f);
                    }
                }
            }
            Item::Const(c) if c.store.is_none() => {
                let name = c.name.name.as_str();
                if name != "dynamic" && name != "static_refresh" {
                    continue;
                }
                if !c.is_pub {
                    plan.warnings.push((
                        file,
                        Diagnostic::warning(
                            codes::ROUTE_FILE,
                            format!("`{name}` is private, so the route does not read it; write `pub const {name}` if it should"),
                            c.name.span,
                        ),
                    ));
                    continue;
                }
                if name == "dynamic" {
                    match &c.value.kind {
                        ExprKind::Bool(b) => out.dynamic = Some((*b, c.name.span)),
                        _ => plan.diagnostics.push((
                            file,
                            route_error(
                                "`dynamic` is `true` or `false`, written out",
                                c.value.span,
                            ),
                        )),
                    }
                } else {
                    match &c.value.kind {
                        ExprKind::Duration(d)
                            if d.chars()
                                .take_while(|c| c.is_ascii_digit() || *c == '_' || *c == '.')
                                .any(|c| c.is_ascii_digit() && c != '0') =>
                        {
                            out.static_refresh = Some(c)
                        }
                        ExprKind::Duration(_) => plan.diagnostics.push((
                            file,
                            route_error(
                                "`static_refresh` is how long a page stays, so it is more than 0",
                                c.value.span,
                            ),
                        )),
                        _ => plan.diagnostics.push((
                            file,
                            route_error(
                                "`static_refresh` is a duration written out, such as `10m`",
                                c.value.span,
                            ),
                        )),
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// What a handler or loader takes: an optional first `req: web.Request`,
/// then route parameters by name.
fn bindings(
    f: &FnDecl,
    params: &[(String, Kind)],
    url: &str,
    file: usize,
    declared: &mut [Vec<Declared>],
    plan: &mut Plan,
) -> Option<Vec<Binding>> {
    let mut out = Vec::new();
    let mut ok = true;
    for (k, p) in f.params.iter().enumerate() {
        let ty = p.ty.as_ref();
        if ty.is_some_and(is_request) {
            if k != 0 {
                plan.diagnostics.push((
                    file,
                    route_error(
                        "the request comes first: `req: web.Request`, then route parameters",
                        p.span,
                    ),
                ));
                ok = false;
            }
            out.push(Binding::Request);
            continue;
        }
        let Some(index) = params.iter().position(|(n, _)| *n == p.name.name) else {
            let names: Vec<String> = params.iter().map(|(n, _)| format!("`{n}`")).collect();
            let has = if names.is_empty() {
                "has no parameters".to_string()
            } else {
                format!("has {}", names.join(", "))
            };
            plan.diagnostics.push((
                file,
                route_error(
                    format!("`{}` is not a parameter of {url}, which {has}; a route function takes an optional `req: web.Request` and parameters by name", p.name.name),
                    p.name.span,
                ),
            ));
            ok = false;
            continue;
        };
        if p.default.is_some() {
            plan.diagnostics.push((
                file,
                route_error(
                    "a route parameter is always in the URL, so it has no default",
                    p.span,
                ),
            ));
            ok = false;
        }
        match ty.map(kind_of) {
            Some(Ok(kind)) => declared[index].push(Declared {
                kind,
                file,
                span: p.span,
                what: format!("`{}` in `{}`", p.name.name, f.name.name),
            }),
            Some(Err(m)) => {
                plan.diagnostics
                    .push((file, route_error(m, ty.unwrap().span)));
                ok = false;
            }
            None => {}
        }
        out.push(Binding::Param(index));
    }
    ok.then_some(out)
}

/// The fields of what `load` returns, as written: a record type or a
/// struct of the folder or of a module it imports.
fn load_fields<'a>(
    inputs: &'a Inputs,
    owner: &str,
    route: usize,
    ret: &'a TypeExpr,
) -> Result<Vec<(&'a Ident, &'a TypeExpr)>, String> {
    let returns =
        "`load` returns the page's props as a record, such as `{ post: Post }`, or a struct";
    match &ret.kind {
        TypeKind::Record(fields) => Ok(fields.iter().map(|(n, t)| (n, t)).collect()),
        TypeKind::Optional(_) => Err(
            "`load` returns the page's props, not `none`: throw `web.error(404, ...)` when there is no page"
                .into(),
        ),
        TypeKind::Named { .. } if is_response(ret) => Err(
            "`load` returns the page's props; a route that makes its own response exports `pub fn get` instead"
                .into(),
        ),
        TypeKind::Named { path, args } if args.is_empty() && path.len() <= 2 => {
            let module = if path.len() == 1 {
                owner.to_string()
            } else {
                let Some(Some(m)) = inputs.modules.get(route) else {
                    return Err(returns.into());
                };
                let alias = &path[0].name;
                let full = m.items.iter().find_map(|item| match item {
                    Item::Import(i) => i.paths.iter().find(|p| p.last().unwrap().name == *alias).map(|p| {
                        p.iter().map(|x| x.name.as_str()).collect::<Vec<_>>().join(".")
                    }),
                    _ => None,
                });
                let Some(full) = full else {
                    return Err(returns.into());
                };
                super::translate_in(inputs.packages, owner, &full)
            };
            let name = &path.last().unwrap().name;
            for (i, m) in inputs.modules.iter().enumerate() {
                let Some(m) = m else { continue };
                if inputs.owner(i) != module {
                    continue;
                }
                for item in &m.items {
                    if let Item::Struct(s) = item {
                        if s.name.name == *name {
                            if !s.generics.is_empty() {
                                return Err(format!("`load` cannot return the generic struct `{name}`"));
                            }
                            return Ok(s.fields.iter().map(|f| (&f.name, &f.ty)).collect());
                        }
                    }
                }
            }
            Err(returns.into())
        }
        _ => Err(returns.into()),
    }
}

/// The routes of one page or route folder.
fn candidate(
    inputs: &Inputs,
    c: &Candidate,
    segs: Vec<Seg>,
    glaze: Option<&Glaze>,
    plan: &mut Plan,
) -> Vec<Built> {
    let url = pattern(&segs);
    let mut params: Vec<(String, Kind)> = segs
        .iter()
        .filter_map(|s| match s {
            Seg::Param(n, k) => Some((n.clone(), *k)),
            _ => None,
        })
        .collect();
    let mut declared: Vec<Vec<Declared>> = params.iter().map(|_| Vec::new()).collect();
    let before = plan.diagnostics.len();
    let route_module = c
        .route
        .and_then(|r| inputs.modules.get(r).and_then(|m| m.as_ref()));
    let ex = match (c.route, route_module) {
        (Some(r), Some(m)) => exports(m, r, plan),
        _ => Exports::default(),
    };
    let route_file = c.route.unwrap_or(c.anchor);
    // The page's props, and which of them are route parameters.
    let props: Vec<&Param> = glaze
        .and_then(|g| g.props.as_ref())
        .map(|p| p.params.iter().collect())
        .unwrap_or_default();
    if let Some(g) = c.glaze {
        for p in &props {
            if let Some(index) = params.iter().position(|(n, _)| *n == p.name.name) {
                match p.ty.as_ref().map(kind_of) {
                    Some(Ok(kind)) => declared[index].push(Declared {
                        kind,
                        file: g,
                        span: p.span,
                        what: format!("the prop `{}`", p.name.name),
                    }),
                    Some(Err(m)) => plan.diagnostics.push((
                        g,
                        route_error(
                            format!("the prop `{}` comes from the URL: {m}", p.name.name),
                            p.span,
                        ),
                    )),
                    None => {}
                }
            }
        }
    }
    let load = ex.load.map(|f| {
        (
            f,
            bindings(f, &params, &url, route_file, &mut declared, plan),
        )
    });
    let mut methods = Vec::new();
    for (m, f) in &ex.methods {
        methods.push((
            *m,
            *f,
            bindings(f, &params, &url, route_file, &mut declared, plan),
        ));
    }
    // Each parameter's type: the same wherever it is written, `String`
    // where it is not.
    for (index, decls) in declared.iter().enumerate() {
        let Some(first) = decls.first() else { continue };
        for d in &decls[1..] {
            if d.kind != first.kind {
                plan.diagnostics.push((
                    d.file,
                    route_error(
                        format!(
                            "the URL parameter `{}` is `{}` here but `{}` in {}{}; give it one type",
                            params[index].0,
                            d.kind.name(),
                            first.kind.name(),
                            first.what,
                            if first.file == d.file {
                                String::new()
                            } else {
                                format!(" ({})", inputs.rel(first.file))
                            }
                        ),
                        d.span,
                    ),
                ));
            }
        }
        params[index].1 = first.kind;
    }
    let segs: Vec<Seg> = {
        let mut k = 0;
        segs.into_iter()
            .map(|s| match s {
                Seg::Param(n, _) => {
                    k += 1;
                    Seg::Param(n, params[k - 1].1)
                }
                lit => lit,
            })
            .collect()
    };
    let url = pattern(&segs);
    // What the route does.
    if let (Some((load, _)), Some(get)) = (&load, ex.methods.get("get")) {
        let _ = load;
        plan.diagnostics.push((
            route_file,
            route_error(
                "`load` and `get` both answer GET: `load` gives the page its props and kiln renders it, while `get` makes the whole response",
                get.name.span,
            )
            .with_help("remove `get` to keep the page, or remove `load` and render the page in `get`"),
        ));
    }
    if let Some((load, _)) = &load {
        if c.glaze.is_none() {
            plan.diagnostics.push((
                route_file,
                route_error(
                    "`load` gives a page its props, and this folder has no `index.glaze`",
                    load.name.span,
                )
                .with_help("add an `index.glaze` beside this file, or replace `load` with `pub fn get` that makes the response"),
            ));
        }
    }
    let gets = c.glaze.is_some() || ex.methods.contains_key("get");
    if let Some(sp) = ex.static_params {
        if !sp.params.is_empty() {
            plan.diagnostics.push((
                route_file,
                route_error(
                    "`static_params` takes nothing and returns the parameters of each page",
                    sp.params[0].span,
                ),
            ));
        }
        if params.is_empty() {
            plan.diagnostics.push((
                route_file,
                route_error(
                    format!("{url} has no parameters, so it has one page and nothing to list"),
                    sp.name.span,
                )
                .with_help("remove `static_params`; `static_refresh` alone renders the page when the server starts"),
            ));
        }
        if !gets {
            plan.diagnostics.push((
                route_file,
                route_error(
                    "`static_params` lists the pages of a GET route, and this folder has no `index.glaze` or `pub fn get`",
                    sp.name.span,
                ),
            ));
        }
    }
    if let Some(refresh) = ex.static_refresh {
        if !params.is_empty() && ex.static_params.is_none() {
            plan.diagnostics.push((
                route_file,
                route_error(
                    format!("{url} has parameters, so `static_refresh` needs `pub fn static_params()` to list the pages it renders"),
                    refresh.name.span,
                ),
            ));
        }
        if !gets {
            plan.diagnostics.push((
                route_file,
                route_error(
                    "`static_refresh` renders a GET page again, and this folder has no `index.glaze` or `pub fn get`",
                    refresh.name.span,
                ),
            ));
        }
    }
    let dynamic = ex.dynamic.is_some_and(|(d, _)| d);
    if let Some((true, span)) = ex.dynamic {
        for (what, given) in [
            ("static_params", ex.static_params.is_some()),
            ("static_refresh", ex.static_refresh.is_some()),
        ] {
            if given {
                plan.diagnostics.push((
                    route_file,
                    route_error(
                        format!("`dynamic` leaves the route out of static pages, and `{what}` renders it ahead of time; keep one"),
                        span,
                    ),
                ));
            }
        }
    }
    if let (Some(r), None, true) = (c.route, c.glaze, ex.methods.is_empty()) {
        if ex.load.is_none() {
            let span = Span::new(0, 0);
            plan.diagnostics.push((
                r,
                route_error(
                    "this route.kiln answers nothing: export `pub fn get`, `post`, `put`, `patch` or `delete`, or add an `index.glaze` beside it",
                    span,
                ),
            ));
        }
    }
    // The page's props: from the URL, from `load`, defaults and the request.
    let mut loaded: Vec<String> = Vec::new();
    let render_page = c.glaze.is_some() && !ex.methods.contains_key("get");
    if let (Some(g), true) = (c.glaze, render_page) {
        if let Some((load, _)) = &load {
            match load.ret.as_ref() {
                None => plan.diagnostics.push((
                    route_file,
                    route_error(
                        "`load` returns the page's props, such as `-> { post: Post }`",
                        load.name.span,
                    ),
                )),
                Some(ret) => {
                    match load_fields(inputs, &c.owner, route_file, ret) {
                        Err(m) => plan
                            .diagnostics
                            .push((route_file, route_error(m, ret.span))),
                        Ok(fields) => {
                            for (name, _) in fields {
                                if params.iter().any(|(n, _)| *n == name.name) {
                                    plan.diagnostics.push((
                                    route_file,
                                    route_error(
                                        format!("`{}` comes from the URL, so `load` does not return it", name.name),
                                        name.span,
                                    ),
                                ));
                                } else if !props.iter().any(|p| p.name.name == name.name) {
                                    plan.diagnostics.push((
                                    route_file,
                                    route_error(
                                        format!(
                                            "`{}` has no prop `{}`; every field `load` returns is a prop of the page",
                                            inputs.rel(g),
                                            name.name
                                        ),
                                        name.span,
                                    )
                                    .with_help(format!("declare it in the page's `{{% props %}}`, or stop returning `{}`", name.name)),
                                ));
                                } else {
                                    loaded.push(name.name.clone());
                                }
                            }
                        }
                    }
                }
            }
        }
        if plan.diagnostics.len() == before {
            for p in &props {
                let filled = p.default.is_some()
                    || (p.name.name == "request" && p.ty.as_ref().is_some_and(is_request))
                    || params.iter().any(|(n, _)| *n == p.name.name)
                    || loaded.contains(&p.name.name);
                if !filled {
                    let help = if load.is_some() {
                        format!(
                            "return `{}` from `load`, or give the prop a default",
                            p.name.name
                        )
                    } else if c.route.is_some() {
                        format!("return `{}` from `pub fn load` in route.kiln, or give the prop a default", p.name.name)
                    } else {
                        "give the prop a default, or move the page to a folder with an `index.glaze` and a route.kiln whose `pub fn load` returns it".into()
                    };
                    plan.diagnostics.push((
                        g,
                        route_error(
                            format!(
                                "the page {url} needs the prop `{}`, which neither its URL nor a `load` gives",
                                p.name.name
                            ),
                            p.span,
                        )
                        .with_help(help),
                    ));
                }
            }
        }
    }
    if plan.diagnostics.len() > before {
        return Vec::new();
    }
    // The handlers.
    let call = |owner: &str, f: &FnDecl, binds: &[Binding]| {
        let args: Vec<String> = binds
            .iter()
            .map(|b| match b {
                Binding::Request => "__req".to_string(),
                Binding::Param(k) => format!("__p{k}"),
            })
            .collect();
        format!(
            "{}{}({})",
            if matches!(f.throws, Throws::Never) {
                ""
            } else {
                "try "
            },
            module_symbol(owner, &f.name.name),
            args.join(", ")
        )
    };
    let mut out = Vec::new();
    let anchor_span = |file: usize| -> Span {
        if Some(file) == c.glaze {
            glaze
                .and_then(|g| g.props.as_ref())
                .map(|p| p.span)
                .unwrap_or_default()
        } else {
            Span::new(0, 0)
        }
    };
    let mut suffix_for_get = Vec::new();
    if let Some(sp) = ex.static_params {
        let closure = format!("fn() {{ {} }}", call(&c.owner, sp, &[]));
        let refresh = ex
            .static_refresh
            .map(|r| format!(", refresh: {}", module_symbol(&c.owner, &r.name.name)))
            .unwrap_or_default();
        suffix_for_get.push((
            format!(".static({closure}{refresh})"),
            Some((route_file, sp.name.span)),
        ));
    } else if let Some(r) = ex.static_refresh {
        suffix_for_get.push((
            format!(
                ".static(refresh: {})",
                module_symbol(&c.owner, &r.name.name)
            ),
            Some((route_file, r.name.span)),
        ));
    }
    if dynamic {
        suffix_for_get.push((".dynamic()".into(), None));
    }
    if render_page {
        let g = c.glaze.unwrap();
        let mut handler = Vec::new();
        let mut data: Vec<String> = props
            .iter()
            .filter_map(|p| {
                params
                    .iter()
                    .position(|(n, _)| *n == p.name.name)
                    .map(|k| format!("{}: __p{k}", p.name.name))
            })
            .collect();
        if let Some((load_fn, Some(binds))) = &load {
            handler.push((
                format!("let __page = {}\n", call(&c.owner, load_fn, binds)),
                Some((route_file, load_fn.name.span)),
            ));
            data.extend(loaded.iter().map(|n| format!("{n}: __page.{n}")));
        }
        let render = if data.is_empty() {
            format!(
                "web.render({})",
                kiln_ir::glaze::kiln_string(&inputs.rel(g))
            )
        } else {
            format!(
                "web.render({}, {{ {} }})",
                kiln_ir::glaze::kiln_string(&inputs.rel(g)),
                data.join(", ")
            )
        };
        let render_target = match &load {
            Some((f, _)) => (route_file, f.ret.as_ref().map_or(f.name.span, |r| r.span)),
            None => (g, anchor_span(g)),
        };
        handler.push((render, Some(render_target)));
        out.push(Built {
            method: "get",
            segs: segs.clone(),
            file: g,
            span: anchor_span(g),
            handler,
            suffix: suffix_for_get.clone(),
            source: inputs.rel(g),
            dynamic,
            unexported: false,
        });
    }
    for (m, f, binds) in &methods {
        let Some(binds) = binds else { continue };
        let is_get = *m == "get";
        out.push(Built {
            method: m,
            segs: segs.clone(),
            file: route_file,
            span: f.name.span,
            handler: vec![(call(&c.owner, f, binds), Some((route_file, f.name.span)))],
            suffix: if is_get {
                suffix_for_get.clone()
            } else {
                Vec::new()
            },
            source: format!("`{}` in {}", m, inputs.rel(route_file)),
            dynamic,
            unexported: c.glaze.is_none() && !ex.methods.contains_key("get") && !dynamic,
        });
    }
    out
}

/// Writes a mount's function: `fn NAME(__app: web.App) { ... }`.
fn write_mount(
    file: &mut FilePlan,
    name: &str,
    call_file: usize,
    call: &MountCall,
    built: &[Built],
) {
    let mut text = String::new();
    let mut regions = Vec::new();
    let mut push = |text: &mut String, part: &str, target: Option<(usize, Span)>| {
        let start = text.len();
        text.push_str(part);
        let (f, s) = target.unwrap_or((call_file, call.span));
        regions.push((start, text.len(), f, s));
    };
    let dir = call.dir.as_ref().map_or("", |(d, _)| d.as_str());
    push(
        &mut text,
        &format!("\n\n// The routes `app.file_routes` finds in `{dir}/`.\nfn {name}(__app: web.App) {{\n"),
        None,
    );
    for b in built {
        let params: Vec<String> = b
            .segs
            .iter()
            .filter(|s| matches!(s, Seg::Param(..)))
            .enumerate()
            .map(|(k, _)| format!(", __p{k}"))
            .collect();
        push(
            &mut text,
            &format!(
                "    __app.{}({}, fn(__req{}) {{\n",
                b.method,
                kiln_ir::glaze::kiln_string(&pattern(&b.segs)),
                params.concat()
            ),
            None,
        );
        for (part, target) in &b.handler {
            push(&mut text, "        ", None);
            push(&mut text, part.trim_end_matches('\n'), *target);
            push(&mut text, "\n", None);
        }
        push(&mut text, "    })", None);
        for (part, target) in &b.suffix {
            push(&mut text, part, *target);
        }
        push(&mut text, "\n", None);
    }
    push(&mut text, "}\n", None);
    let offset = file.appended.len();
    file.appended.push_str(&text);
    file.regions.extend(
        regions
            .into_iter()
            .map(|(s, e, f, sp)| (s + offset, e + offset, f, sp)),
    );
}
