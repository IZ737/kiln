//! Resolve folder symbols before the existing checker/lowerer. Edits carry a
//! reversible source map; generated names never replace diagnostic locations.
use kiln_diagnostics::{Diagnostic, Span};
use kiln_syntax::ast::*;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Edit {
    pub span: Span,
    pub text: String,
}
#[derive(Clone, Debug, Default)]
pub struct SourceMap {
    pub edits: Vec<Edit>,
}
impl SourceMap {
    pub fn apply(&mut self, source: &str) -> String {
        self.edits.sort_by_key(|e| (e.span.start, e.span.end));
        self.edits.dedup_by(|a, b| a.span == b.span);
        let mut out = String::new();
        let mut cursor = 0;
        for edit in &self.edits {
            let start = edit.span.start as usize;
            assert!(start >= cursor, "overlapping module edits");
            out.push_str(&source[cursor..start]);
            out.push_str(&edit.text);
            cursor = edit.span.end as usize;
        }
        out.push_str(&source[cursor..]);
        out
    }
    pub fn generated(&self, position: usize) -> usize {
        let mut delta = 0isize;
        for edit in &self.edits {
            if position < edit.span.start as usize {
                break;
            }
            if position < edit.span.end as usize {
                return (edit.span.start as isize + delta) as usize;
            }
            delta += edit.text.len() as isize - edit.span.len() as isize;
        }
        (position as isize + delta) as usize
    }
    pub fn original(&self, span: Span) -> Span {
        let offset = |position: u32, end: bool| {
            let mut delta = 0i64;
            for e in &self.edits {
                let start = e.span.start as i64 + delta;
                let stop = start + e.text.len() as i64;
                if (position as i64) < start {
                    break;
                }
                if (position as i64) < stop || ((position as i64) == stop && end) {
                    return if end { e.span.end } else { e.span.start };
                }
                delta += e.text.len() as i64 - e.span.len() as i64;
            }
            (position as i64 - delta).max(0) as u32
        };
        Span {
            start: offset(span.start, false),
            end: offset(span.end, true),
        }
    }
}
#[derive(Clone)]
struct Symbol {
    qualified: String,
    public: bool,
    nominal: bool,
}
type Symbols = BTreeMap<String, BTreeMap<String, Symbol>>;

pub struct Bound {
    pub names: BTreeMap<String, String>,
    pub texts: Vec<(PathBuf, String)>,
    pub maps: Vec<SourceMap>,
    pub diagnostics: Vec<Vec<Diagnostic>>,
    /// Warnings, which do not stop a build, by file.
    pub warnings: Vec<Vec<Diagnostic>>,
    /// Code `app.file_routes` mounts wrote into a file's text: the file,
    /// the range in its bound text, and the file and span an error there
    /// is about.
    pub regions: Vec<Region>,
    /// The routes the mounts add.
    pub routes: Vec<super::routes::RouteInfo>,
}

#[derive(Clone, Debug)]
pub struct Region {
    pub file: usize,
    pub start: usize,
    pub end: usize,
    pub target: usize,
    pub span: Span,
}

impl Bound {
    /// The file and span of the original source that a span in a file's
    /// bound text stands for.
    pub fn original(&self, file: usize, span: Span) -> (usize, Span) {
        let start = span.start as usize;
        if let Some(r) = self
            .regions
            .iter()
            .filter(|r| r.file == file && r.start <= start && start < r.end)
            .min_by_key(|r| r.end - r.start)
        {
            return (r.target, r.span);
        }
        (file, self.maps[file].original(span))
    }
}
fn owner(root: &Path, path: &Path) -> String {
    path.parent()
        .unwrap_or(root)
        .strip_prefix(root)
        .unwrap_or(Path::new(""))
        .to_string_lossy()
        .replace(['/', '\\'], ".")
}
fn qualified(module: &str, name: &str, nominal: bool) -> String {
    if module.is_empty() {
        return name.to_string();
    }
    let prefix = if nominal {
        "KilnModule"
    } else {
        "__kiln_module"
    };
    format!(
        "{prefix}_{}_symbol_{}",
        module.split('.').map(segment).collect::<Vec<_>>().join("_"),
        name
    )
}

/// A folder's part of a module symbol: `5_posts`. A folder name that is
/// not an identifier, such as a route folder `[slug]`, is written in hex
/// after an `e` (`12e_5b736c75675d`), which no other folder's part has.
fn segment(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        format!("{}_{}", s.len(), s)
    } else {
        let hex: String = s.bytes().map(|b| format!("{b:02x}")).collect();
        format!("{}e_{hex}", hex.len())
    }
}

/// The symbol a folder module gives one of its functions or constants.
pub fn module_symbol(module: &str, name: &str) -> String {
    qualified(module, name, false)
}

pub fn bind(root: &Path, texts: &[(PathBuf, String)]) -> Bound {
    bind_entry(root, root, texts)
}
pub fn bind_entry(root: &Path, entry: &Path, texts: &[(PathBuf, String)]) -> Bound {
    let parsed: Vec<_> = texts
        .iter()
        .map(|(p, t)| (!crate::is_glaze(p)).then(|| kiln_syntax::parse(t).module))
        .collect();
    let packages = super::packages(root);
    let mut symbols = Symbols::new();
    for ((path, _), module) in texts.iter().zip(&parsed) {
        let owner = owner(root, path);
        let names = symbols.entry(owner.clone()).or_default();
        let Some(module) = module else { continue };
        // A test file's `test_` functions are visible to the test harness.
        let test_file = path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("test_"));
        for item in &module.items {
            let (name, public, nominal) = match item {
                Item::Fn(f) => (
                    &f.name,
                    f.is_pub || (test_file && f.name.name.starts_with("test_")),
                    false,
                ),
                Item::Struct(s) => (&s.name, s.is_pub, true),
                Item::Enum(e) => (&e.name, e.is_pub, true),
                Item::Interface(i) => (&i.name, i.is_pub, true),
                Item::Const(c) => (&c.name, c.is_pub, false),
                _ => continue,
            };
            names.entry(name.name.clone()).or_insert_with(|| Symbol {
                qualified: if matches!(item, Item::Fn(_))
                    && name.name == "main"
                    && path.parent() == Some(entry)
                {
                    "main".to_string()
                } else if matches!(item, Item::Fn(_)) && name.name == "main" && owner.is_empty() {
                    // The program's own `main`, in a test build whose entry
                    // is the test harness.
                    "__kiln_app_main".to_string()
                } else {
                    qualified(&owner, &name.name, nominal)
                },
                public,
                nominal,
            });
        }
    }
    let mut result = Bound {
        names: symbols
            .iter()
            .flat_map(|(module, names)| {
                names
                    .iter()
                    .filter(|(_, s)| {
                        s.qualified.starts_with("__kiln_module_")
                            || s.qualified.starts_with("KilnModule_")
                    })
                    .map(move |(name, s)| (s.qualified.clone(), format!("{module}.{name}")))
            })
            .collect(),
        texts: Vec::new(),
        maps: Vec::new(),
        diagnostics: Vec::new(),
        warnings: Vec::new(),
        regions: Vec::new(),
        routes: Vec::new(),
    };
    // `app.file_routes` mounts become functions that add their routes.
    let mut plan = super::routes::analyze(&super::routes::Inputs {
        root,
        texts,
        modules: &parsed,
        packages: &packages,
    });
    let mut route_diagnostics: BTreeMap<usize, Vec<Diagnostic>> = BTreeMap::new();
    for (file, d) in plan.diagnostics.drain(..) {
        route_diagnostics.entry(file).or_default().push(d);
    }
    let mut route_warnings: BTreeMap<usize, Vec<Diagnostic>> = BTreeMap::new();
    for (file, d) in plan.warnings.drain(..) {
        route_warnings.entry(file).or_default().push(d);
    }
    result.routes = std::mem::take(&mut plan.routes);
    // A glaze has no imports of its own: it sees the folder modules its
    // folder's `.kiln` files import, so a page can take `shared.Post`.
    let mut folder_imports: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for ((path, _), module) in texts.iter().zip(&parsed) {
        let Some(module) = module else { continue };
        let own = owner(root, path);
        let seen = folder_imports.entry(own.clone()).or_default();
        for item in &module.items {
            if let Item::Import(i) = item {
                for path in &i.paths {
                    let full = path
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let key = super::translate_in(&packages, &own, &full);
                    if symbols.contains_key(&key) {
                        seen.insert(path.last().unwrap().name.clone(), key);
                    }
                }
            }
        }
    }
    for (index, ((path, text), module)) in texts.iter().zip(&parsed).enumerate() {
        let own = owner(root, path);
        let mut resolver = Resolver {
            packages: &packages,
            owner: &own,
            source: text,
            symbols: &symbols,
            imports: BTreeMap::new(),
            components: BTreeMap::new(),
            locals: vec![BTreeSet::new()],
            generics: vec![BTreeSet::new()],
            map: SourceMap::default(),
            diagnostics: Vec::new(),
        };
        if let Some(module) = module {
            let mut aliases = BTreeSet::new();
            for item in &module.items {
                if let Item::Import(i) = item {
                    let mut retained = Vec::new();
                    for path in &i.paths {
                        let full = path
                            .iter()
                            .map(|p| p.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        let alias = &path.last().unwrap().name;
                        if !aliases.insert(alias.clone())
                            || symbols
                                .get(&own)
                                .is_some_and(|names| names.contains_key(alias))
                        {
                            resolver.diagnostics.push(super::diagnostic(format!("module alias `{alias}` conflicts with another import or declaration"),path.last().unwrap().span));
                        }

                        let key = super::translate_in(&packages, &own, &full);
                        if symbols.contains_key(&key) {
                            if resolver.imports.insert(alias.clone(), key).is_some() {
                                resolver.diagnostics.push(super::diagnostic(
                                    format!("duplicate module alias `{alias}`"),
                                    i.span,
                                ));
                            }
                        } else {
                            retained.push(full);
                        }
                    }
                    if retained.len() != i.paths.len() {
                        resolver.edit(
                            i.span,
                            if retained.is_empty() {
                                String::new()
                            } else {
                                format!("import {}", retained.join(", "))
                            },
                        );
                    }
                }
            }
            for item in &module.items {
                resolver.item(item);
            }
        } else {
            let glaze = kiln_syntax::glaze::parse_component(text, &path.to_string_lossy()).glaze;
            resolver.imports = folder_imports.get(&own).cloned().unwrap_or_default();
            resolver.glaze_imports(&glaze, root, texts);
            resolver.glaze(&glaze);
        }
        let file_plan = plan.files.remove(&index).unwrap_or_default();
        for (span, text) in file_plan.edits {
            resolver.edit(span, text);
        }
        let mut rewritten = resolver.map.apply(text);
        let base = rewritten.len();
        rewritten.push_str(&file_plan.appended);
        result.regions.extend(
            file_plan
                .regions
                .into_iter()
                .map(|(start, end, target, span)| Region {
                    file: index,
                    start: base + start,
                    end: base + end,
                    target,
                    span,
                }),
        );
        result.texts.push((path.clone(), rewritten));
        result.maps.push(resolver.map);
        resolver
            .diagnostics
            .extend(route_diagnostics.remove(&index).unwrap_or_default());
        result.diagnostics.push(resolver.diagnostics);
        result
            .warnings
            .push(route_warnings.remove(&index).unwrap_or_default());
    }
    result
}
struct Resolver<'a> {
    packages: &'a super::Packages,
    owner: &'a str,
    source: &'a str,
    symbols: &'a Symbols,
    imports: BTreeMap<String, String>,
    /// Components a glaze imports, by tag name: the tag to write instead,
    /// such as `components.Card`.
    components: BTreeMap<String, String>,
    locals: Vec<BTreeSet<String>>,
    generics: Vec<BTreeSet<String>>,
    map: SourceMap,
    diagnostics: Vec<Diagnostic>,
}
impl Resolver<'_> {
    fn edit(&mut self, span: Span, text: String) {
        self.map.edits.push(Edit { span, text });
    }
    fn local(&self, name: &str) -> bool {
        self.locals.iter().rev().any(|s| s.contains(name))
    }
    fn declare(&mut self, name: &str) {
        self.locals.last_mut().unwrap().insert(name.to_string());
    }
    fn resolve(
        &mut self,
        path: &[(&str, Span)],
        exported: bool,
        type_position: bool,
    ) -> Option<(String, usize)> {
        let (first, first_span) = *path.first()?;
        if first.starts_with("__kiln_module_") || first.starts_with("KilnModule_") {
            self.diagnostics.push(super::diagnostic(
                "this name is reserved for compiler module symbols",
                first_span,
            ));
            return None;
        }

        if if type_position {
            self.generics.iter().any(|g| g.contains(first))
        } else {
            self.local(first)
        } {
            return None;
        }
        if path.len() == 1
            && self.owner == "__test"
            && !self
                .symbols
                .get("__test")
                .is_some_and(|m| m.contains_key(first))
        {
            // The test harness calls the tests of the root folder.
            if let Some(symbol) = self.symbols.get("").and_then(|m| m.get(first)) {
                return Some((symbol.qualified.clone(), 1));
            }
        }
        let (module, name, count) = if let Some(module) = self.imports.get(first) {
            if path.len() < 2 {
                return None;
            }
            (module.as_str(), path[1].0, 2)
        } else {
            (self.owner, first, 1)
        };
        if let Some(symbol) = self.symbols.get(module).and_then(|m| m.get(name)) {
            let span = first_span.to(path[count - 1].1);
            if module != self.owner && !symbol.public {
                self.diagnostics.push(super::diagnostic(
                    format!("`{module}.{name}` is private; add `pub` to its declaration"),
                    span,
                ));
            }
            if exported && symbol.nominal && !symbol.public {
                self.diagnostics.push(super::diagnostic(
                    format!("public signature exposes private type `{name}`; make the type public"),
                    span,
                ));
            }
            return Some((symbol.qualified.clone(), count));
        }
        if count == 2 {
            self.diagnostics.push(super::diagnostic(
                format!("module `{module}` has no exported member `{name}`"),
                first_span.to(path[1].1),
            ));
        } else if !self.owner.is_empty()
            && self.symbols.get("").is_some_and(|m| m.contains_key(name))
        {
            // The root's names are kept for compatibility with the entrypoint.
            // They must not become implicit imports in child modules.
            self.diagnostics.push(super::diagnostic(
                format!("`{name}` is not declared in module `{}`", self.owner),
                first_span,
            ));
        }
        None
    }
    fn path(&mut self, path: &[Ident], exported: bool) {
        let parts: Vec<_> = path.iter().map(|i| (i.name.as_str(), i.span)).collect();
        if let Some((name, count)) = self.resolve(&parts, exported, true) {
            if count > 1 || name != path[0].name {
                self.edit(path[0].span.to(path[count - 1].span), name);
            }
        }
    }
    fn ty(&mut self, t: &TypeExpr, exported: bool) {
        match &t.kind {
            TypeKind::Named { path, args } => {
                self.path(path, exported);
                for a in args {
                    self.ty(a, exported);
                }
            }
            TypeKind::Optional(t) => self.ty(t, exported),
            TypeKind::Fn {
                params,
                ret,
                throws,
            } => {
                for p in params {
                    self.ty(p, exported);
                }
                if let Some(t) = ret {
                    self.ty(t, exported);
                }
                self.throws(throws, exported);
            }
            TypeKind::Record(fields) => {
                for (_, t) in fields {
                    self.ty(t, exported);
                }
            }
            TypeKind::Error => {}
        }
    }
    fn throws(&mut self, t: &Throws, exported: bool) {
        if let Throws::Type(t) = t {
            self.ty(t, exported);
        }
    }
    fn params(&mut self, params: &[Param], exported: bool) {
        for p in params {
            if let Some(t) = &p.ty {
                self.ty(t, exported);
            }
            if let Some(e) = &p.default {
                self.expr(e);
            }
            self.declare(&p.name.name);
        }
    }
    fn function(&mut self, f: &FnDecl, method: bool) {
        if !method {
            self.path(std::slice::from_ref(&f.name), false);
        }
        self.locals.push(BTreeSet::new());
        self.generics
            .push(f.generics.iter().map(|g| g.name.clone()).collect());
        for b in &f.bounds {
            for t in &b.interfaces {
                self.ty(t, f.is_pub);
            }
        }
        self.params(&f.params, f.is_pub);
        if let Some(t) = &f.ret {
            self.ty(t, f.is_pub);
        }
        self.throws(&f.throws, f.is_pub);
        self.block(&f.body);
        self.locals.pop();
        self.generics.pop();
    }
    fn item(&mut self, i: &Item) {
        match i {
            Item::Fn(f) => self.function(f, false),
            Item::Struct(s) => {
                self.path(std::slice::from_ref(&s.name), false);
                self.generics
                    .push(s.generics.iter().map(|g| g.name.clone()).collect());
                for f in &s.fields {
                    self.ty(&f.ty, s.is_pub);
                    if let Some(e) = &f.default {
                        self.expr(e);
                    }
                }
                self.generics.pop();
            }
            Item::Enum(e) => {
                self.path(std::slice::from_ref(&e.name), false);
                self.generics
                    .push(e.generics.iter().map(|g| g.name.clone()).collect());
                for v in &e.variants {
                    for f in &v.fields {
                        self.ty(&f.ty, e.is_pub);
                    }
                }
                self.generics.pop();
            }
            Item::Const(c) => {
                self.path(std::slice::from_ref(&c.name), false);
                if let Some(t) = &c.ty {
                    self.ty(t, c.is_pub);
                }
                self.expr(&c.value);
            }
            Item::Impl(i) => {
                let parameters = match &i.target.kind {
                    TypeKind::Named { args, .. } => args
                        .iter()
                        .filter_map(|t| match &t.kind {
                            TypeKind::Named { path, .. } if path.len() == 1 => {
                                Some(path[0].name.clone())
                            }
                            _ => None,
                        })
                        .collect(),
                    _ => BTreeSet::new(),
                };
                self.generics.push(parameters);
                self.ty(&i.target, false);
                for f in &i.methods {
                    self.function(f, true);
                }
                self.generics.pop();
            }
            Item::Interface(i) => {
                self.path(std::slice::from_ref(&i.name), false);
                for m in &i.methods {
                    self.function(m, true);
                }
            }
            Item::Import(_) => {}
        }
    }
    fn block(&mut self, b: &Block) {
        self.locals.push(BTreeSet::new());
        for s in &b.stmts {
            match s {
                Stmt::Let {
                    name, ty, value, ..
                } => {
                    if let Some(t) = ty {
                        self.ty(t, false);
                    }
                    self.expr(value);
                    self.declare(&name.name);
                }
                Stmt::Assign { target, value, .. } => {
                    self.expr(target);
                    self.expr(value);
                }
                Stmt::Return { value, .. } => {
                    if let Some(e) = value {
                        self.expr(e);
                    }
                }
                Stmt::For {
                    binding,
                    iter,
                    filter,
                    body,
                    ..
                } => {
                    self.expr(iter);
                    self.locals.push(BTreeSet::from([binding.name.clone()]));
                    if let Some(e) = filter {
                        self.expr(e);
                    }
                    self.block(body);
                    self.locals.pop();
                }
                Stmt::While { cond, body, .. } => {
                    self.locals.push(BTreeSet::new());
                    self.expr(cond);
                    self.block(body);
                    self.locals.pop();
                }
                Stmt::Expr(e) => self.expr(e),
                Stmt::Break(_) | Stmt::Continue(_) => {}
            }
        }
        self.locals.pop();
    }
    fn pattern(&mut self, p: &Pattern) {
        match &p.kind {
            PatternKind::Binding(s) => self.declare(s),
            PatternKind::Literal(e) => self.expr(e),
            PatternKind::Variant { path, args } => {
                self.path(path, false);
                for a in args {
                    self.pattern(a);
                }
            }
            _ => {}
        }
    }
    fn expr(&mut self, e: &Expr) {
        fn parts<'a>(e: &'a Expr, out: &mut Vec<(&'a str, Span)>) -> bool {
            match &e.kind {
                ExprKind::Ident(s) => {
                    out.push((s, e.span));
                    true
                }
                ExprKind::Field { expr, name } => {
                    if !parts(expr, out) {
                        return false;
                    }
                    out.push((&name.name, name.span));
                    true
                }
                _ => false,
            }
        }
        let mut path = Vec::new();
        if parts(e, &mut path) {
            if let Some((name, count)) = self.resolve(&path, false, false) {
                if count > 1 || name != path[0].0 {
                    self.edit(path[0].1.to(path[count - 1].1), name);
                }
            }
            return;
        }
        match &e.kind {
            ExprKind::Str(pieces) => {
                for p in pieces {
                    if let StrPiece::Expr(e) = p {
                        self.expr(e);
                    }
                }
            }
            ExprKind::List(xs) => {
                for x in xs {
                    self.expr(x);
                }
            }
            ExprKind::Record(fs) => {
                for f in fs {
                    if let Some(e) = &f.value {
                        self.expr(e);
                    } else {
                        let path = [(f.name.name.as_str(), f.name.span)];
                        if let Some((name, _)) = self.resolve(&path, false, false) {
                            if name != f.name.name {
                                self.edit(f.name.span, format!("{}: {name}", f.name.name));
                            }
                        }
                    }
                }
            }
            ExprKind::Unary { expr, .. } | ExprKind::Try(expr) | ExprKind::Throw(expr) => {
                self.expr(expr)
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.expr(lhs);
                self.expr(rhs);
            }
            ExprKind::Call {
                callee,
                type_args,
                args,
            } => {
                // `app.file_routes(...)` is replaced by a call of the
                // function its routes are written in; its arguments are
                // literals and stay as they are.
                if let ExprKind::Field { expr, name } = &callee.kind {
                    if name.name == "file_routes" {
                        self.expr(expr);
                        return;
                    }
                }
                if matches!(&callee.kind, ExprKind::Field {expr,name} if name.name=="render" && matches!(&expr.kind, ExprKind::Ident(n) if n=="web"))
                {
                    if let Some(a) = args.first() {
                        self.rebase(&a.value);
                    }
                }
                self.expr(callee);
                for t in type_args {
                    self.ty(t, false);
                }
                for a in args {
                    self.expr(&a.value);
                }
            }
            ExprKind::Field { expr, .. } => self.expr(expr),
            ExprKind::Index { expr, index } => {
                self.expr(expr);
                self.expr(index);
            }
            ExprKind::Closure { params, ret, body } => {
                self.locals.push(BTreeSet::new());
                self.params(params, false);
                if let Some(t) = ret {
                    self.ty(t, false);
                }
                self.block(body);
                self.locals.pop();
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.locals.push(BTreeSet::new());
                self.expr(cond);
                self.block(then);
                self.locals.pop();
                if let Some(e) = otherwise {
                    self.expr(e);
                }
            }
            ExprKind::Block(b) => self.block(b),
            ExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for a in arms {
                    self.locals.push(BTreeSet::new());
                    self.pattern(&a.pattern);
                    if let Some(g) = &a.guard {
                        self.expr(g);
                    }
                    self.expr(&a.body);
                    self.locals.pop();
                }
            }
            ExprKind::Catch {
                expr,
                binding,
                handler,
            } => {
                self.expr(expr);
                self.locals.push(BTreeSet::new());
                if let Some(b) = binding {
                    self.declare(&b.name);
                }
                self.block(handler);
                self.locals.pop();
            }
            ExprKind::Is { expr, pattern } => {
                self.expr(expr);
                self.pattern(pattern);
            }
            _ => {}
        }
    }
    /// `{% import "..." %}` in a glaze or ember: a component becomes a tag
    /// name, a folder a module name. The tag itself is removed.
    fn glaze_imports(&mut self, g: &Glaze, root: &Path, texts: &[(PathBuf, String)]) {
        for import in &g.imports {
            // A comment, so the line still renders as nothing.
            self.edit(import.span, "{# import #}".to_string());
            let Some(spec) = super::routes::literal(&import.path) else {
                self.diagnostics.push(super::diagnostic(
                    "an import's path is a string literal, such as \"../components/card.glaze\"",
                    import.path.span,
                ));
                continue;
            };
            let target = match self.locate(&spec) {
                Ok(target) => target,
                Err(message) => {
                    self.diagnostics
                        .push(super::diagnostic(message, import.path.span));
                    continue;
                }
            };
            let exists = |path: &Path| texts.iter().any(|(p, _)| p == path) || path.exists();
            let alias = import.alias.as_ref();
            if target.ends_with(".glaze") || target.ends_with(".ember") {
                if !exists(&root.join(&target)) {
                    self.diagnostics.push(super::diagnostic(
                        format!("`{spec}` names `{target}`, which is not in the project"),
                        import.path.span,
                    ));
                    continue;
                }
                let (folder, file) = target.rsplit_once('/').unwrap_or(("", &target));
                let stem = file.trim_end_matches(".glaze").trim_end_matches(".ember");
                let component = pascal_case(stem);
                let name = alias.map_or(component.clone(), |a| a.name.clone());
                if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
                    self.diagnostics.push(super::diagnostic(
                        format!(
                            "a component's name is capitalized: `as {}`",
                            pascal_case(&name)
                        ),
                        alias.map_or(import.path.span, |a| a.span),
                    ));
                    continue;
                }
                let tag = if folder.is_empty() {
                    component
                } else {
                    format!("{}.{component}", folder.replace('/', "."))
                };
                if self.components.insert(name.clone(), tag).is_some() {
                    self.diagnostics.push(super::diagnostic(
                        format!("`<{name}>` is imported twice; rename one with `as`"),
                        import.span,
                    ));
                }
            } else {
                let key = target.replace('/', ".");
                if !self.symbols.contains_key(&key) {
                    let message = if root.join(&target).is_dir() {
                        format!("`{spec}` names the folder `{target}`, which has no Kiln source")
                    } else {
                        format!("`{spec}` names `{target}`, which is not a folder in the project; a component's path ends in `.glaze` or `.ember`")
                    };
                    self.diagnostics
                        .push(super::diagnostic(message, import.path.span));
                    continue;
                }
                let name = alias.map_or_else(
                    || target.rsplit('/').next().unwrap_or(&target).to_string(),
                    |a| a.name.clone(),
                );
                if let Some(previous) = self.imports.insert(name.clone(), key.clone()) {
                    if previous != key {
                        self.diagnostics.push(super::diagnostic(
                            format!("module name `{name}` is already this folder's import of `{previous}`; rename this one with `as`"),
                            import.span,
                        ));
                    }
                }
            }
        }
    }
    /// The project-relative path a `./`, `../`, `@/` or `@alias/` path in
    /// this file names. In a Ware, `@/` is the Ware's root and there are
    /// no aliases.
    fn locate(&self, spec: &str) -> Result<String, String> {
        super::paths::locate(self.owner, spec, &self.packages.aliases)
    }
    /// `{% css "@/styles/site.css" %}`: CSS paths are relative to the
    /// glaze, so a rooted one is written as one from the glaze's folder.
    fn rebase_css(&mut self, e: &Expr) {
        let ExprKind::Str(parts) = &e.kind else {
            return;
        };
        let [StrPiece::Text(path)] = parts.as_slice() else {
            return;
        };
        if !path.starts_with('@') {
            return;
        }
        match self.locate(path) {
            Ok(target) => {
                let up = self.owner.split('.').filter(|p| !p.is_empty()).count();
                self.edit(e.span, format!("\"{}{target}\"", "../".repeat(up)));
            }
            Err(message) => self.diagnostics.push(super::diagnostic(message, e.span)),
        }
    }
    fn rebase(&mut self, e: &Expr) {
        // `@/...` and `@alias/...` are from the project root.
        if let ExprKind::Str(parts) = &e.kind {
            if let [StrPiece::Text(path)] = parts.as_slice() {
                if path.starts_with('@') {
                    match self.locate(path) {
                        Ok(path) => self.edit(e.span, format!("\"{path}\"")),
                        Err(message) => self.diagnostics.push(super::diagnostic(message, e.span)),
                    }
                    return;
                }
            }
        }
        if self.owner.is_empty() {
            return;
        }
        if let ExprKind::Str(parts) = &e.kind {
            if let [StrPiece::Text(path)] = parts.as_slice() {
                if !path.contains('/') && !path.contains('\\') {
                    self.edit(
                        e.span,
                        format!("\"{}/{}\"", self.owner.replace('.', "/"), path),
                    );
                } else if let Some(ware) = super::ware_of(self.owner) {
                    // A Ware's paths are relative to the Ware's root.
                    self.edit(e.span, format!("\"{}/{}\"", ware.replace('.', "/"), path));
                }
            }
        }
    }
    fn glaze(&mut self, g: &Glaze) {
        if let Some(p) = &g.props {
            self.params(&p.params, false);
        }
        for s in &g.states {
            if let Some(t) = &s.ty {
                self.ty(t, false);
            }
            self.expr(&s.value);
            self.declare(&s.name.name);
        }
        for c in &g.css {
            self.rebase_css(&c.path);
        }
        if let Some(l) = &g.layout {
            self.rebase(&l.path);
            for a in &l.args {
                self.expr(&a.value);
            }
        }
        self.nodes(&g.nodes);
    }
    fn nodes(&mut self, nodes: &[Node]) {
        self.locals.push(BTreeSet::new());
        for n in nodes {
            match n {
                Node::Expr { expr, .. } => self.expr(expr),
                Node::Element(e) => {
                    if e.name.name.starts_with("__") {
                        self.diagnostics.push(super::diagnostic(
                            "names starting with `__` are reserved for the compiler",
                            e.name.span,
                        ));
                    }
                    let imported = self.components.get(&e.name.name).cloned();
                    let qualified = e
                        .name
                        .name
                        .rsplit_once('.')
                        .map(|(module, component)| {
                            (
                                super::translate_in(self.packages, self.owner, module),
                                module,
                                component,
                            )
                        })
                        .filter(|(key, module, _)| key != module)
                        .map(|(key, _, component)| format!("{key}.{component}"));
                    if e.is_component()
                        && (imported.is_some()
                            || qualified.is_some()
                            || (!self.owner.is_empty() && !e.name.name.contains('.')))
                    {
                        let name = imported
                            .or(qualified)
                            .unwrap_or_else(|| format!("{}.{}", self.owner, e.name.name));
                        self.edit(e.name.span, name.clone());
                        let element = &self.source[e.span.start as usize..e.span.end as usize];
                        if let Some(close) = element.rfind("</") {
                            let start = e.span.start as usize + close + 2;
                            if self.source[start..].starts_with(&e.name.name) {
                                self.edit(Span::new(start, start + e.name.name.len()), name);
                            }
                        }
                    }
                    for a in &e.attrs {
                        match &a.value {
                            AttrValue::Expr(e) => self.expr(e),
                            AttrValue::Handler(b) => self.block(b),
                            AttrValue::Text(parts) => {
                                for p in parts {
                                    if let AttrPart::Expr(e) = p {
                                        self.expr(e);
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                    self.nodes(&e.children);
                }
                Node::If {
                    branches,
                    otherwise,
                    ..
                } => {
                    for b in branches {
                        self.locals.push(BTreeSet::new());
                        self.expr(&b.cond);
                        self.nodes(&b.body);
                        self.locals.pop();
                    }
                    if let Some(ns) = otherwise {
                        self.nodes(ns);
                    }
                }
                Node::For {
                    binding,
                    iter,
                    filter,
                    body,
                    empty,
                    ..
                } => {
                    self.expr(iter);
                    self.locals.push(BTreeSet::from([binding.name.clone()]));
                    if let Some(e) = filter {
                        self.expr(e);
                    }
                    self.nodes(body);
                    self.locals.pop();
                    if let Some(ns) = empty {
                        self.nodes(ns);
                    }
                }
                Node::Match { expr, cases, .. } => {
                    self.expr(expr);
                    for c in cases {
                        self.locals.push(BTreeSet::new());
                        self.pattern(&c.pattern);
                        if let Some(e) = &c.guard {
                            self.expr(e);
                        }
                        self.nodes(&c.body);
                        self.locals.pop();
                    }
                }
                Node::Let(l) => {
                    if let Some(t) = &l.ty {
                        self.ty(t, false);
                    }
                    self.expr(&l.value);
                    self.declare(&l.name.name);
                }
                Node::Fill { body, .. } => self.nodes(body),
                _ => {}
            }
        }
        self.locals.pop();
    }
}

/// `like_button` is `LikeButton`, as component tags are named.
fn pascal_case(name: &str) -> String {
    name.split(['_', '-'])
        .map(|w| {
            let mut c = w.chars();
            c.next()
                .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                .unwrap_or_default()
        })
        .collect()
}
