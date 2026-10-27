//! Deterministic project discovery. Only explicitly imported folders enter a build.
pub mod bind;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::Item;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub entry: PathBuf,
    pub files: Vec<PathBuf>,
    overlays: BTreeMap<PathBuf, String>,
    tests: bool,
}

fn invalid(message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message.into())
}

#[derive(Debug)]
struct SourceError {
    path: PathBuf,
    span: Span,
    message: String,
}
impl std::fmt::Display for SourceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.path.display(), self.message)
    }
}
impl std::error::Error for SourceError {}
fn invalid_at(path: &Path, span: Span, message: impl Into<String>) -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        SourceError {
            path: path.to_path_buf(),
            span,
            message: message.into(),
        },
    )
}
pub fn error_diagnostic(
    error: &std::io::Error,
    fallback: &Path,
) -> (kiln_diagnostics::SourceFile, Diagnostic) {
    let source = error
        .get_ref()
        .and_then(|e| e.downcast_ref::<SourceError>());
    let path = source.map(|s| s.path.as_path()).unwrap_or(fallback);
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let span = source.map(|s| s.span).unwrap_or_default();
    (
        kiln_diagnostics::SourceFile::new(path.display().to_string(), text),
        diagnostic(
            source
                .map(|s| s.message.clone())
                .unwrap_or_else(|| error.to_string()),
            span,
        ),
    )
}

/// An explicit directory bounds discovery. A file can use its nearest manifest.
pub fn root(path: &Path) -> std::io::Result<PathBuf> {
    let path = path.canonicalize()?;
    if path.is_dir() {
        return Ok(path);
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid("source has no parent folder"))?;
    for dir in parent.ancestors() {
        if dir.join("kiln.toml").is_file() {
            return Ok(dir.to_path_buf());
        }
    }
    Ok(parent.to_path_buf())
}

pub fn discover(path: &Path) -> std::io::Result<Project> {
    discover_overlay(path, &BTreeMap::new())
}
pub fn discover_overlay(
    path: &Path,
    overlays: &BTreeMap<PathBuf, String>,
) -> std::io::Result<Project> {
    discover_with(path, overlays, None)
}

/// A test build: kiln's generated `main`, in a folder of its own that
/// appears as `__test/` in the project, and whether the root folder (whose
/// own tests it calls) is part of the build.
pub struct Harness {
    pub dir: PathBuf,
    pub include_root: bool,
}

/// Discovers a project. In a test build, files named `test_*.kiln` are
/// part of their folders, and `import test` names kiln's test module.
pub fn discover_with(
    path: &Path,
    overlays: &BTreeMap<PathBuf, String>,
    harness: Option<&Harness>,
) -> std::io::Result<Project> {
    let tests = harness.is_some();
    let root = root(path)?;
    let mut entry = root.clone();
    let manifest = crate::ware::manifest::Manifest::load(&root)
        .map_err(|(file, (span, message))| invalid_at(&file, span, message))?;
    if let Some(manifest) = &manifest {
        let bad = |key: &str, message: String| {
            invalid_at(
                &manifest.path,
                crate::ware::manifest::key_span(&manifest.text, key),
                message,
            )
        };
        if let Some(selected) = &manifest.entry {
            let selected_path = Path::new(selected);
            if selected_path.is_absolute()
                || selected_path
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                return Err(bad(
                    "entry",
                    "entry must be a relative path within the project root".into(),
                ));
            }
            let selected = root
                .join(selected_path)
                .canonicalize()
                .map_err(|e| bad("entry", format!("cannot read entry `{selected}`: {e}")))?;
            if !selected.starts_with(&root)
                || selected.extension().is_none_or(|e| e != "kiln")
                || !selected.is_file()
            {
                return Err(bad(
                    "entry",
                    "entry must be a .kiln file within the project root".into(),
                ));
            }
            entry = selected.parent().unwrap().to_path_buf();
        }
    }
    let mut packages = Packages::default();
    if let Some(manifest) = &manifest {
        let graph = crate::ware::prepare(&root, manifest).map_err(|message| {
            invalid_at(
                &manifest.path,
                crate::ware::manifest::key_span(&manifest.text, "dependencies"),
                message,
            )
        })?;
        if let Some(graph) = graph {
            packages = mount(&root, &graph);
        }
    }
    if tests {
        let (name, dir) = crate::ware::builtin_test().map_err(invalid)?;
        add_mount(root.join(name.mount()), dir);
        packages.wares.insert(
            name.module_key(),
            WarePackage {
                deps: BTreeMap::new(),
                cached: true,
            },
        );
        packages.root.insert("test".into(), name.module_key());
    }
    PACKAGES.lock().unwrap().insert(root.clone(), packages);
    if path.is_file() {
        entry = path.canonicalize()?.parent().unwrap().to_path_buf();
    }
    if let Some(h) = harness {
        entry = root.join("__test");
        add_mount(entry.clone(), h.dir.clone());
    }
    let mut project = Project {
        root,
        entry: entry.clone(),
        files: Vec::new(),
        overlays: overlays.clone(),
        tests,
    };
    let mut visiting = Vec::new();
    let mut visited = BTreeSet::new();
    project.visit(&entry, &mut visiting, &mut visited)?;
    if harness.is_some_and(|h| h.include_root) {
        let root = project.root.clone();
        project.visit(&root, &mut visiting, &mut visited)?;
    }
    project.files.sort();
    Ok(project)
}

/// Where each Ware's source appears in a build: its folder under
/// `__ware/` in the project, which reads from the real folder.
static MOUNTS: Mutex<BTreeMap<PathBuf, PathBuf>> = Mutex::new(BTreeMap::new());
/// Each project's dependency names, by root.
static PACKAGES: Mutex<BTreeMap<PathBuf, Packages>> = Mutex::new(BTreeMap::new());

#[derive(Clone, Debug, Default)]
pub struct Packages {
    /// The project's dependencies: import name to module key.
    pub root: BTreeMap<String, String>,
    /// Each Ware, by module key (`__ware.kiln.ui`).
    pub wares: BTreeMap<String, WarePackage>,
}

#[derive(Clone, Debug, Default)]
pub struct WarePackage {
    pub deps: BTreeMap<String, String>,
    pub cached: bool,
}

fn add_mount(virtual_dir: PathBuf, real: PathBuf) {
    MOUNTS.lock().unwrap().insert(virtual_dir, real);
}

fn mount(root: &Path, graph: &crate::ware::Graph) -> Packages {
    let mut packages = Packages::default();
    for (alias, name) in &graph.root {
        packages.root.insert(alias.clone(), name.module_key());
    }
    for (name, ware) in &graph.wares {
        add_mount(root.join(name.mount()), ware.dir.clone());
        packages.wares.insert(
            name.module_key(),
            WarePackage {
                deps: ware
                    .deps
                    .iter()
                    .map(|(a, n)| (a.clone(), n.module_key()))
                    .collect(),
                cached: ware.cached,
            },
        );
    }
    packages
}

pub fn packages(root: &Path) -> Packages {
    PACKAGES
        .lock()
        .unwrap()
        .get(root)
        .cloned()
        .unwrap_or_default()
}

/// The file or folder a path in a build stands for: a Ware's source is
/// read from the cache or its local folder.
pub fn real(path: &Path) -> PathBuf {
    let mounts = MOUNTS.lock().unwrap();
    for (virtual_dir, real) in mounts.iter().rev() {
        if let Ok(rest) = path.strip_prefix(virtual_dir) {
            return if rest.as_os_str().is_empty() {
                real.clone()
            } else {
                real.join(rest)
            };
        }
    }
    path.to_path_buf()
}

/// The path a person should see for a file in a build.
pub fn shown(name: &str) -> String {
    real(Path::new(name)).display().to_string()
}

/// Reads a source file of a build.
pub fn read(path: &Path) -> std::io::Result<String> {
    std::fs::read_to_string(real(path))
}

/// Whether a file in a build is a cached Ware's, which must not be edited.
pub fn is_cached_ware(root: &Path, path: &Path) -> bool {
    let Ok(rest) = path.strip_prefix(root) else {
        return false;
    };
    let parts: Vec<String> = rest
        .components()
        .take(3)
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.len() == 3
        && parts[0] == "__ware"
        && packages(root)
            .wares
            .get(&format!("__ware.{}.{}", parts[1], parts[2]))
            .is_some_and(|w| w.cached)
}

/// The module a written import names, seen from a module (`owner`): a
/// dependency's name maps to its Ware, and a Ware's own folders are
/// relative to the Ware.
pub fn translate_in(packages: &Packages, owner: &str, full: &str) -> String {
    resolve_import(packages, owner, full).0
}

/// [`translate_in`], also saying whether the first name was a dependency's.
pub fn resolve_import(packages: &Packages, owner: &str, full: &str) -> (String, bool) {
    let (prefix, deps) = match ware_of(owner) {
        Some(key) => (key.clone(), packages.wares.get(&key).map(|w| &w.deps)),
        None => (String::new(), Some(&packages.root)),
    };
    let (first, rest) = match full.split_once('.') {
        Some((f, r)) => (f, Some(r)),
        None => (full, None),
    };
    if let Some(dep) = deps.and_then(|d| d.get(first)) {
        return (
            match rest {
                Some(r) => format!("{dep}.{r}"),
                None => dep.clone(),
            },
            true,
        );
    }
    if prefix.is_empty() {
        (full.to_string(), false)
    } else {
        (format!("{prefix}.{full}"), false)
    }
}

/// The Ware a module belongs to: `__ware.kiln.ui` for `__ware.kiln.ui.forms`.
pub fn ware_of(owner: &str) -> Option<String> {
    let parts: Vec<&str> = owner.split('.').collect();
    (parts.len() >= 3 && parts[0] == "__ware").then(|| parts[..3].join("."))
}

fn is_test_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "kiln")
        && path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("test_"))
}

impl Project {
    fn visit(
        &mut self,
        dir: &Path,
        visiting: &mut Vec<PathBuf>,
        visited: &mut BTreeSet<PathBuf>,
    ) -> std::io::Result<()> {
        if let Some(start) = visiting.iter().position(|p| p == dir) {
            let chain = visiting[start..]
                .iter()
                .chain(std::iter::once(&dir.to_path_buf()))
                .map(|p| {
                    p.strip_prefix(&self.root)
                        .unwrap_or(p)
                        .display()
                        .to_string()
                })
                .collect::<Vec<_>>()
                .join(" -> ");
            return Err(invalid(format!("local module import cycle: {chain}")));
        }
        if visited.contains(dir) {
            return Ok(());
        }
        visiting.push(dir.to_path_buf());
        let mut files = Vec::new();
        let mut names = BTreeMap::new();
        let real_dir = real(dir);
        let mut entries = std::fs::read_dir(&real_dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(previous) = names.insert(name.to_lowercase(), name.clone()) {
                return Err(invalid(format!(
                    "case collision in {}: `{previous}` and `{name}`",
                    real_dir.display()
                )));
            }
            let path = dir.join(&name);
            if path.extension().is_some_and(|e| e == "kiln") || crate::is_glaze(&path) {
                if entry.file_type()?.is_symlink() {
                    return Err(invalid(format!(
                        "source symlinks are not supported: {}",
                        entry.path().display()
                    )));
                }
                if is_test_file(&path) && !self.tests {
                    continue;
                }
                if entry.path().is_file() {
                    files.push(path);
                }
            }
        }
        files.extend(
            self.overlays
                .keys()
                .filter(|p| p.parent() == Some(dir) && (self.tests || !is_test_file(p)))
                .cloned(),
        );
        files.sort();
        files.dedup();
        if files.is_empty() {
            return Err(invalid(format!(
                "module {} contains no Kiln source",
                real_dir.display()
            )));
        }
        let owner = dir
            .strip_prefix(&self.root)
            .unwrap_or(Path::new(""))
            .to_string_lossy()
            .replace(['/', '\\'], ".");
        let packages = packages(&self.root);
        let package_root = match ware_of(&owner) {
            Some(key) => self.root.join(key.replace('.', "/")),
            None => self.root.clone(),
        };
        let mut imports = BTreeMap::new();
        for file in &files {
            if crate::is_glaze(file) {
                continue;
            }
            let text = match self.overlays.get(file) {
                Some(text) => text.clone(),
                None => read(file)?,
            };
            for item in kiln_syntax::parse(&text).module.items {
                if let Item::Import(import) = item {
                    for path in import.paths {
                        let span = path.first().unwrap().span.to(path.last().unwrap().span);
                        let name = path
                            .iter()
                            .map(|i| i.name.as_str())
                            .collect::<Vec<_>>()
                            .join(".");
                        let (key, dependency) = resolve_import(&packages, &owner, &name);
                        if !dependency
                            && (kiln_check::std_source(&name).is_some() || name.starts_with("npm."))
                        {
                            if package_root.join(name.replace('.', "/")).is_dir() {
                                return Err(invalid(format!("local folder `{name}` conflicts with a reserved standard module; rename the folder")));
                            }
                            continue;
                        }
                        if path[0].name.starts_with("__") {
                            return Err(invalid_at(
                                file,
                                span,
                                "module names starting with `__` are reserved",
                            ));
                        }
                        if dependency && package_root.join(&path[0].name).is_dir() {
                            return Err(invalid_at(file, span, format!("the dependency `{}` and the folder `{}/` have the same name; rename one", path[0].name, path[0].name)));
                        }
                        let mut target = self.root.clone();
                        let parts: Vec<&str> = key.split('.').collect();
                        // A Ware's root is mounted by kiln; its folders are checked.
                        let mounted = if key.starts_with("__ware.") { 3 } else { 0 };
                        for (i, part) in parts.iter().enumerate() {
                            if part.is_empty() || part.contains(['/', '\\']) || *part == ".." {
                                return Err(invalid("invalid module path"));
                            }
                            if i < mounted {
                                target.push(part);
                                continue;
                            }
                            let exact = std::fs::read_dir(real(&target))?
                                .filter_map(Result::ok)
                                .any(|e| e.file_name().to_string_lossy() == *part);
                            target.push(part);
                            if !exact {
                                return Err(invalid_at(file, span, format!("no module `{name}` (paths are case-sensitive); create its folder or correct the import")));
                            }
                            if std::fs::symlink_metadata(real(&target))?
                                .file_type()
                                .is_symlink()
                            {
                                return Err(invalid(format!(
                                    "module symlinks are not supported: {}",
                                    real(&target).display()
                                )));
                            }
                        }
                        imports.insert(target, (file.clone(), span));
                    }
                }
            }
        }
        self.files.extend(files);
        for (import, (file, span)) in imports {
            self.visit(&import, visiting, visited).map_err(|error| {
                if error.get_ref().is_some_and(|e| e.is::<SourceError>()) {
                    error
                } else {
                    invalid_at(&file, span, error.to_string())
                }
            })?;
        }
        visiting.pop();
        visited.insert(dir.to_path_buf());
        Ok(())
    }
}

pub fn diagnostic(message: impl Into<String>, span: Span) -> Diagnostic {
    Diagnostic::error(codes::LOCAL_MODULE, message, span)
}

/// The nearest app folder within an editor workspace. A selected workspace
/// bounds ancestor lookup; nested independent apps remain independent.
pub fn editor_root(path: &Path, roots: &[PathBuf]) -> PathBuf {
    let dir = path.parent().unwrap_or(Path::new("."));
    let boundary = roots
        .iter()
        .filter(|r| path.starts_with(r))
        .max_by_key(|r| r.components().count());
    for ancestor in dir.ancestors() {
        if boundary.is_some_and(|b| !ancestor.starts_with(b)) {
            break;
        }
        if ancestor.join("kiln.toml").is_file() {
            return ancestor.to_path_buf();
        }
    }
    for ancestor in dir.ancestors().collect::<Vec<_>>().into_iter().rev() {
        if boundary.is_some_and(|b| !ancestor.starts_with(b)) {
            continue;
        }
        if ancestor.join("kiln.toml").is_file() || ancestor.join("main.kiln").is_file() {
            return ancestor.to_path_buf();
        }
    }
    boundary.cloned().unwrap_or_else(|| dir.to_path_buf())
}

pub fn common_root<'a>(paths: impl Iterator<Item = &'a Path>) -> PathBuf {
    let mut paths = paths;
    let mut root = paths
        .next()
        .and_then(Path::parent)
        .unwrap_or(Path::new("."))
        .to_path_buf();
    for path in paths {
        while !path.starts_with(&root) && root.pop() {}
    }
    root
}

/// Inferred exported constants must obey the same visibility rule as written
/// signatures, including nominal types nested inside containers and records.
pub fn check_const_exports(
    modules: &[&kiln_syntax::ast::Module],
    glazes: &[(&str, &kiln_syntax::ast::Glaze)],
    styling: &kiln_check::Styling,
    result: &mut kiln_check::AppDiagnostics,
) {
    use kiln_check::{ide, types::Ty};
    let private: BTreeSet<_> = modules
        .iter()
        .flat_map(|m| m.items.iter())
        .filter_map(|i| match i {
            Item::Struct(s) if !s.is_pub => Some(s.name.name.clone()),
            Item::Enum(e) if !e.is_pub => Some(e.name.name.clone()),
            _ => None,
        })
        .collect();
    fn leaks(ty: &Ty, a: &ide::Analysis, private: &BTreeSet<String>) -> bool {
        match ty {
            Ty::Adt(id, args) => {
                private.contains(&a.adts[*id].name) || args.iter().any(|t| leaks(t, a, private))
            }
            Ty::Opt(t) => leaks(t, a, private),
            Ty::Record(fs) => fs.iter().any(|(_, t)| leaks(t, a, private)),
            Ty::Fn(f) => {
                f.params.iter().any(|t| leaks(t, a, private))
                    || leaks(&f.ret, a, private)
                    || f.throws.as_ref().is_some_and(|t| leaks(t, a, private))
            }
            _ => false,
        }
    }
    for (i, module) in modules.iter().enumerate() {
        let constants: Vec<_> = module
            .items
            .iter()
            .filter_map(|item| match item {
                Item::Const(c) if c.is_pub && c.ty.is_none() => Some(c),
                _ => None,
            })
            .collect();
        if constants.is_empty() || !result.modules[i].is_empty() {
            continue;
        }
        let analysis = ide::analyze_app(modules, glazes, ide::Target::Module(i), styling);
        for c in constants {
            if analysis
                .consts
                .get(&c.name.name)
                .is_some_and(|t| leaks(t, &analysis, &private))
            {
                result.modules[i].push(diagnostic(
                    "public constant exposes a private type; make the type public",
                    c.name.span,
                ));
            }
        }
    }
}

/// Method declarations keep their defining folder's visibility, even when a
/// public nominal value reaches another module through inference.
pub fn check_method_access(
    modules: &[&kiln_syntax::ast::Module],
    owners: &[&Path],
    glazes: &[(&str, &kiln_syntax::ast::Glaze)],
    glaze_owners: &[&Path],
    styling: &kiln_check::Styling,
    result: &mut kiln_check::AppDiagnostics,
) {
    use kiln_check::ide::{self, Ref, Target};
    use kiln_syntax::ast::TypeKind;
    let mut private = BTreeMap::new();
    for (module, owner) in modules.iter().zip(owners) {
        for item in &module.items {
            if let Item::Impl(imp) = item {
                if let TypeKind::Named { path, .. } = &imp.target.kind {
                    if let Some(ty) = path.last() {
                        for method in &imp.methods {
                            if !method.is_pub {
                                private.insert((ty.name.clone(), method.name.name.clone()), *owner);
                            }
                        }
                    }
                }
            }
        }
    }
    if private.is_empty() {
        return;
    }
    for (target, owner) in owners
        .iter()
        .enumerate()
        .map(|(i, p)| (Target::Module(i), p))
        .chain(
            glaze_owners
                .iter()
                .enumerate()
                .map(|(i, p)| (Target::Glaze(i), p)),
        )
    {
        let diagnostics = match target {
            Target::Module(i) => &mut result.modules[i],
            Target::Glaze(i) => &mut result.glazes[i],
        };
        if !diagnostics.is_empty() {
            continue;
        }
        let analysis = ide::analyze_app(modules, glazes, target, styling);
        for (span, reference) in &analysis.references {
            if let Ref::Method(id, name) = reference {
                if private
                    .get(&(analysis.adts[*id].name.clone(), name.clone()))
                    .is_some_and(|p| p != owner)
                {
                    diagnostics.push(diagnostic(
                        format!("method `{name}` is private; add `pub` to its declaration"),
                        *span,
                    ));
                }
            }
        }
    }
}
