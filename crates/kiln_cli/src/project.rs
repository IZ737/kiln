//! Deterministic project discovery. Only explicitly imported folders enter a build.
pub mod bind;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::Item;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub entry: PathBuf,
    pub files: Vec<PathBuf>,
    overlays: BTreeMap<PathBuf, String>,
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
    let root = root(path)?;
    let manifest = root.join("kiln.toml");
    let mut entry = root.clone();
    if manifest.is_file() {
        let text = std::fs::read_to_string(&manifest)?;
        let bad = |key: &str, message: String| {
            let start = text.find(key).unwrap_or(0);
            invalid_at(
                &manifest,
                Span::new(start, (start + key.len()).min(text.len())),
                message,
            )
        };

        let value: toml::Table = toml::from_str(&text).map_err(|e: toml::de::Error| {
            let span = e
                .span()
                .map(|r| Span::new(r.start, r.end))
                .unwrap_or_default();
            invalid_at(&manifest, span, e.to_string())
        })?;
        for key in value.keys() {
            if key != "package" {
                return Err(bad(key, format!("unsupported section `{key}`")));
            }
        }
        let package = value
            .get("package")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| bad("package", "kiln.toml requires a [package] table".into()))?;
        for (key, value) in package {
            if !["name", "kind", "entry"].contains(&key.as_str()) {
                return Err(bad(key, format!("unsupported package key `{key}`")));
            }
            if value.as_str().is_none() {
                return Err(bad(key, format!("package.{key} must be a string")));
            }
        }
        if package
            .get("kind")
            .and_then(toml::Value::as_str)
            .is_some_and(|s| s != "app")
        {
            return Err(bad("kind", "package.kind must be `app`".into()));
        }
        let selected = package
            .get("entry")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| bad("package", "package.entry is required".into()))?;
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
    if path.is_file() {
        entry = path.canonicalize()?.parent().unwrap().to_path_buf();
    }
    let mut project = Project {
        root,
        entry: entry.clone(),
        files: Vec::new(),
        overlays: overlays.clone(),
    };
    let mut visiting = Vec::new();
    let mut visited = BTreeSet::new();
    project.visit(&entry, &mut visiting, &mut visited)?;
    project.files.sort();
    Ok(project)
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
        let mut entries = std::fs::read_dir(dir)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            if let Some(previous) = names.insert(name.to_lowercase(), name.clone()) {
                return Err(invalid(format!(
                    "case collision in {}: `{previous}` and `{name}`",
                    dir.display()
                )));
            }
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "kiln") || crate::is_glaze(&path) {
                if entry.file_type()?.is_symlink() {
                    return Err(invalid(format!(
                        "source symlinks are not supported: {}",
                        path.display()
                    )));
                }
                if path.is_file() {
                    files.push(path);
                }
            }
        }
        files.extend(
            self.overlays
                .keys()
                .filter(|p| p.parent() == Some(dir))
                .cloned(),
        );
        files.sort();
        files.dedup();
        if files.is_empty() {
            return Err(invalid(format!(
                "module {} contains no Kiln source",
                dir.display()
            )));
        }
        let mut imports = BTreeMap::new();
        for file in &files {
            if crate::is_glaze(file) {
                continue;
            }
            let text = match self.overlays.get(file) {
                Some(text) => text.clone(),
                None => std::fs::read_to_string(file)?,
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
                        if kiln_check::std_source(&name).is_some() || name.starts_with("npm.") {
                            if self.root.join(name.replace('.', "/")).is_dir() {
                                return Err(invalid(format!("local folder `{name}` conflicts with a reserved standard module; rename the folder")));
                            }
                            continue;
                        }
                        let mut target = self.root.clone();
                        for part in &path {
                            if part.name.is_empty()
                                || part.name.contains(['/', '\\'])
                                || part.name == ".."
                            {
                                return Err(invalid("invalid module path"));
                            }
                            let exact = std::fs::read_dir(&target)?
                                .filter_map(Result::ok)
                                .any(|e| e.file_name().to_string_lossy() == part.name);
                            target.push(&part.name);
                            if !exact {
                                return Err(invalid_at(file, span, format!("no module `{name}` (paths are case-sensitive); create its folder or correct the import")));
                            }
                            if std::fs::symlink_metadata(&target)?.file_type().is_symlink() {
                                return Err(invalid(format!(
                                    "module symlinks are not supported: {}",
                                    target.display()
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
