//! `kiln.toml`: the optional manifest. Phase 4.2's `[package]` selects an
//! entry; Wares add `[dependencies]`, `[registry]`, `[policy]` and `[test]`.
//! Every key is checked, and an error points at the key it is about.

use super::name::{check_alias, WareName};
use kiln_diagnostics::Span;
use semver::{Version, VersionReq};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What a Ware may do beyond computing: the effects its code can reach.
/// They are declared in the Ware's manifest and checked against the
/// standard modules its source imports (see `policy.rs`).
pub const CAPABILITIES: &[(&str, &str)] = &[
    ("filesystem", "reads and writes files (`fs`)"),
    ("network", "serves or makes network connections (`web`)"),
    (
        "database",
        "opens databases (`sql`, `orm`, `auth`, `admin`)",
    ),
    (
        "environment",
        "reads environment variables, arguments or exits the process (`os`, `env`)",
    ),
    ("foreign", "calls JavaScript or npm packages (`js`, `npm`)"),
];

/// Trust tiers a registry can state about a release, weakest first.
pub const TRUST: &[&str] = &["unverified", "verified-publisher", "peer-reviewed"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Kind {
    #[default]
    App,
    Ware,
}

#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub path: PathBuf,
    pub text: String,
    pub name: Option<String>,
    pub kind: Kind,
    /// The entry file, relative to the root, as written.
    pub entry: Option<String>,
    pub version: Option<Version>,
    pub description: Option<String>,
    pub license: Option<String>,
    pub capabilities: BTreeSet<String>,
    /// By the name the project imports each one with.
    pub dependencies: BTreeMap<String, Dependency>,
    pub registry: Option<String>,
    pub policy: Option<Policy>,
    pub test: TestConfig,
    /// `[aliases]`: `ui = "components"` lets source paths start `@ui/`.
    /// The folders are relative to the root, as written.
    pub aliases: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Dependency {
    pub name: WareName,
    pub source: DepSource,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DepSource {
    Registry {
        req: VersionReq,
        registry: Option<String>,
    },
    /// Relative to the manifest's folder, as written.
    Path(String),
    Git {
        url: String,
        reference: GitRef,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum GitRef {
    Rev(String),
    Branch(String),
    Tag(String),
    Default,
}

#[derive(Clone, Debug, Default)]
pub struct Policy {
    pub publishers: Option<BTreeSet<String>>,
    pub sources: Option<BTreeSet<String>>,
    pub capabilities: Option<BTreeSet<String>>,
    pub trust: Option<String>,
}

#[derive(Clone, Debug, Default)]
pub struct TestConfig {
    /// Seconds a test may run before it is stopped.
    pub timeout: Option<u64>,
    pub env: BTreeMap<String, String>,
    /// The browser executable for browser tests.
    pub browser: Option<String>,
}

/// A manifest error: what is wrong, and where in the text.
pub type Error = (Span, String);

impl Manifest {
    pub fn load(root: &Path) -> Result<Option<Manifest>, (PathBuf, Error)> {
        let path = root.join("kiln.toml");
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| (path.clone(), (Span::default(), format!("cannot read: {e}"))))?;
        Manifest::parse(&path, &text)
            .map(Some)
            .map_err(|e| (path, e))
    }

    pub fn parse(path: &Path, text: &str) -> Result<Manifest, Error> {
        let at = |key: &str| key_span(text, key);
        let bad = |key: &str, message: String| (at(key), message);
        let value: toml::Table = toml::from_str(text).map_err(|e: toml::de::Error| {
            let span = e
                .span()
                .map(|r| Span::new(r.start, r.end))
                .unwrap_or_default();
            (span, e.message().to_string())
        })?;
        let mut m = Manifest {
            path: path.to_path_buf(),
            text: text.to_string(),
            ..Default::default()
        };
        for key in value.keys() {
            if ![
                "package",
                "dependencies",
                "registry",
                "policy",
                "test",
                "aliases",
            ]
            .contains(&key.as_str())
            {
                return Err(bad(key, format!("unsupported section `{key}`")));
            }
        }
        let package = value
            .get("package")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| bad("package", "kiln.toml requires a [package] table".into()))?;
        for (key, value) in package {
            match key.as_str() {
                "name" | "kind" | "entry" | "version" | "description" | "license" => {
                    if value.as_str().is_none() {
                        return Err(bad(key, format!("package.{key} must be a string")));
                    }
                }
                "capabilities" => {}
                _ => return Err(bad(key, format!("unsupported package key `{key}`"))),
            }
        }
        let string = |key: &str| {
            package
                .get(key)
                .and_then(toml::Value::as_str)
                .map(String::from)
        };
        m.name = string("name");
        m.kind = match string("kind").as_deref() {
            None | Some("app") => Kind::App,
            Some("ware") => Kind::Ware,
            Some(_) => return Err(bad("kind", "package.kind must be `app` or `ware`".into())),
        };
        m.entry = string("entry");
        m.description = string("description");
        m.license = string("license");
        if let Some(v) = string("version") {
            m.version = Some(
                Version::parse(&v)
                    .map_err(|e| bad("version", format!("package.version `{v}`: {e}")))?,
            );
        }
        if let Some(caps) = package.get("capabilities") {
            m.capabilities = capability_list(caps, "package.capabilities")
                .map_err(|msg| bad("capabilities", msg))?;
        }
        if m.kind == Kind::Ware {
            let name = m
                .name
                .as_deref()
                .ok_or_else(|| bad("package", "a Ware's package.name is required".into()))?;
            WareName::parse(name).map_err(|msg| bad("name", msg))?;
            if m.version.is_none() {
                return Err(bad(
                    "package",
                    "a Ware's package.version is required".into(),
                ));
            }
            if m.entry.is_some() {
                return Err(bad(
                    "entry",
                    "a Ware has no entry: its root folder is the module projects import".into(),
                ));
            }
        } else if !m.capabilities.is_empty() {
            return Err(bad(
                "capabilities",
                "package.capabilities describes a Ware; an app may do anything".into(),
            ));
        }

        if let Some(deps) = value.get("dependencies") {
            let deps = deps
                .as_table()
                .ok_or_else(|| bad("dependencies", "[dependencies] must be a table".into()))?;
            for (alias, spec) in deps {
                check_alias(alias).map_err(|msg| bad(alias, msg))?;
                let dep = dependency(alias, spec).map_err(|msg| bad(alias, msg))?;
                m.dependencies.insert(alias.clone(), dep);
            }
            let mut seen = BTreeMap::new();
            for (alias, dep) in &m.dependencies {
                if let Some(other) = seen.insert(dep.name.clone(), alias) {
                    return Err(bad(
                        alias,
                        format!("`{}` is already a dependency, as `{other}`", dep.name),
                    ));
                }
            }
        }
        if let Some(aliases) = value.get("aliases") {
            let table = aliases
                .as_table()
                .ok_or_else(|| bad("aliases", "[aliases] must be a table".into()))?;
            for (name, folder) in table {
                let folder = folder.as_str().ok_or_else(|| {
                    bad(
                        name,
                        format!("aliases.{name} must be a folder, as a string"),
                    )
                })?;
                if name.is_empty()
                    || !name.starts_with(|c: char| c.is_ascii_lowercase())
                    || !name.chars().all(|c| {
                        c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-'
                    })
                {
                    return Err(bad(
                        name,
                        format!("alias `{name}`: use lowercase letters, digits, `-` and `_`, starting with a letter (`@/` is the project root and cannot be redefined)"),
                    ));
                }
                if m.dependencies.contains_key(name) {
                    return Err(bad(
                        name,
                        format!("alias `{name}` has the name of a dependency; rename one"),
                    ));
                }
                let path = Path::new(folder);
                if folder.is_empty()
                    || path.is_absolute()
                    || folder.contains('\\')
                    || path.components().any(|c| {
                        !matches!(
                            c,
                            std::path::Component::Normal(_) | std::path::Component::CurDir
                        )
                    })
                {
                    return Err(bad(
                        name,
                        format!("aliases.{name} must be a folder inside the project, written with `/`, such as \"components\""),
                    ));
                }
                m.aliases.insert(
                    name.clone(),
                    folder
                        .trim_start_matches("./")
                        .trim_end_matches('/')
                        .to_string(),
                );
            }
        }
        if let Some(registry) = value.get("registry") {
            let table = registry
                .as_table()
                .ok_or_else(|| bad("registry", "[registry] must be a table".into()))?;
            for (key, value) in table {
                if key != "url" {
                    return Err(bad(key, format!("unsupported registry key `{key}`")));
                }
                let url = value
                    .as_str()
                    .ok_or_else(|| bad(key, "registry.url must be a string".into()))?;
                super::registry::check_url(url).map_err(|msg| bad("url", msg))?;
                m.registry = Some(url.trim_end_matches('/').to_string());
            }
        }
        if let Some(policy) = value.get("policy") {
            let table = policy
                .as_table()
                .ok_or_else(|| bad("policy", "[policy] must be a table".into()))?;
            let mut p = Policy::default();
            for (key, value) in table {
                let list = || -> Result<BTreeSet<String>, Error> {
                    value
                        .as_array()
                        .ok_or_else(|| bad(key, format!("policy.{key} must be a list of strings")))?
                        .iter()
                        .map(|v| {
                            v.as_str().map(String::from).ok_or_else(|| {
                                bad(key, format!("policy.{key} must be a list of strings"))
                            })
                        })
                        .collect()
                };
                match key.as_str() {
                    "publishers" => p.publishers = Some(list()?),
                    "sources" => {
                        let sources = list()?;
                        if let Some(s) = sources
                            .iter()
                            .find(|s| !["registry", "git", "path"].contains(&s.as_str()))
                        {
                            return Err(bad(
                                key,
                                format!("unknown source `{s}`: expected registry, git or path"),
                            ));
                        }
                        p.sources = Some(sources)
                    }
                    "capabilities" => {
                        p.capabilities = Some(
                            capability_list(value, "policy.capabilities")
                                .map_err(|msg| bad(key, msg))?,
                        )
                    }
                    "trust" => {
                        let tier =
                            value
                                .as_str()
                                .filter(|t| TRUST.contains(t))
                                .ok_or_else(|| {
                                    bad(
                                        key,
                                        format!("policy.trust must be one of {}", TRUST.join(", ")),
                                    )
                                })?;
                        p.trust = Some(tier.into());
                    }
                    _ => return Err(bad(key, format!("unsupported policy key `{key}`"))),
                }
            }
            m.policy = Some(p);
        }
        if let Some(test) = value.get("test") {
            let table = test
                .as_table()
                .ok_or_else(|| bad("test", "[test] must be a table".into()))?;
            for (key, value) in table {
                match key.as_str() {
                    "timeout" => {
                        m.test.timeout = Some(
                            value
                                .as_integer()
                                .filter(|n| (1..=3600).contains(n))
                                .ok_or_else(|| {
                                    bad(key, "test.timeout is seconds, from 1 to 3600".into())
                                })? as u64,
                        )
                    }
                    "browser" => {
                        m.test.browser = Some(
                            value
                                .as_str()
                                .ok_or_else(|| bad(key, "test.browser must be a string".into()))?
                                .into(),
                        )
                    }
                    "env" => {
                        let env = value.as_table().ok_or_else(|| {
                            bad(key, "test.env must be a table of strings".into())
                        })?;
                        for (k, v) in env {
                            let v = v
                                .as_str()
                                .ok_or_else(|| bad(k, "test.env values must be strings".into()))?;
                            m.test.env.insert(k.clone(), v.into());
                        }
                    }
                    _ => return Err(bad(key, format!("unsupported test key `{key}`"))),
                }
            }
        }
        Ok(m)
    }

    /// The Ware identity of a Ware manifest.
    pub fn ware_name(&self) -> Option<WareName> {
        (self.kind == Kind::Ware)
            .then(|| self.name.as_deref().and_then(|n| WareName::parse(n).ok()))
            .flatten()
    }
}

fn capability_list(value: &toml::Value, what: &str) -> Result<BTreeSet<String>, String> {
    let names: Vec<&str> = CAPABILITIES.iter().map(|(n, _)| *n).collect();
    let list = value
        .as_array()
        .ok_or_else(|| format!("{what} must be a list of strings"))?;
    let mut out = BTreeSet::new();
    for v in list {
        let s = v
            .as_str()
            .ok_or_else(|| format!("{what} must be a list of strings"))?;
        if !names.contains(&s) {
            return Err(format!(
                "unknown capability `{s}`: expected one of {}",
                names.join(", ")
            ));
        }
        out.insert(s.to_string());
    }
    Ok(out)
}

/// One `[dependencies]` entry: `"kiln/ui@0.1"` for the registry, or a
/// table naming the Ware and one source.
fn dependency(alias: &str, spec: &toml::Value) -> Result<Dependency, String> {
    if let Some(s) = spec.as_str() {
        let (name, req) = s.split_once('@').unwrap_or((s, "*"));
        return Ok(Dependency {
            name: WareName::parse(name)?,
            source: DepSource::Registry {
                req: parse_req(req)?,
                registry: None,
            },
        });
    }
    let table = spec.as_table().ok_or_else(|| {
        format!("dependency `{alias}` must be \"publisher/name@version\" or a table")
    })?;
    for key in table.keys() {
        if ![
            "ware", "version", "registry", "path", "git", "rev", "branch", "tag",
        ]
        .contains(&key.as_str())
        {
            return Err(format!("unsupported dependency key `{key}`"));
        }
    }
    let get = |key: &str| -> Result<Option<String>, String> {
        match table.get(key) {
            None => Ok(None),
            Some(v) => v
                .as_str()
                .map(|s| Some(s.to_string()))
                .ok_or_else(|| format!("`{key}` must be a string")),
        }
    };
    let name = WareName::parse(
        &get("ware")?
            .ok_or_else(|| format!("dependency `{alias}` needs `ware = \"publisher/name\"`"))?,
    )?;
    let path = get("path")?;
    let git = get("git")?;
    let refs = [get("rev")?, get("branch")?, get("tag")?];
    let source = match (path, git) {
        (Some(_), Some(_)) => return Err("a dependency has one source: `path` or `git`".into()),
        (Some(path), None) => {
            if table.contains_key("version")
                || table.contains_key("registry")
                || refs.iter().any(Option::is_some)
            {
                return Err(
                    "a `path` dependency takes no version, registry or Git revision".into(),
                );
            }
            DepSource::Path(path)
        }
        (None, Some(url)) => {
            if table.contains_key("registry") {
                return Err("a `git` dependency takes no registry".into());
            }
            let reference = match refs {
                [Some(r), None, None] => {
                    if r.len() != 40 || !r.chars().all(|c| c.is_ascii_hexdigit()) {
                        return Err("`rev` must be a full 40-character commit hash".into());
                    }
                    GitRef::Rev(r.to_lowercase())
                }
                [None, Some(b), None] => GitRef::Branch(b),
                [None, None, Some(t)] => GitRef::Tag(t),
                [None, None, None] => GitRef::Default,
                _ => return Err("give at most one of `rev`, `branch` and `tag`".into()),
            };
            DepSource::Git { url, reference }
        }
        (None, None) => {
            if refs.iter().any(Option::is_some) {
                return Err("`rev`, `branch` and `tag` need `git`".into());
            }
            let registry = get("registry")?;
            if let Some(r) = &registry {
                super::registry::check_url(r)?;
            }
            DepSource::Registry {
                req: parse_req(&get("version")?.unwrap_or_else(|| "*".into()))?,
                registry: registry.map(|r| r.trim_end_matches('/').to_string()),
            }
        }
    };
    Ok(Dependency { name, source })
}

pub fn parse_req(req: &str) -> Result<VersionReq, String> {
    VersionReq::parse(req).map_err(|e| format!("version requirement `{req}`: {e}"))
}

/// Where `key` is written as a key, for errors: the first line that starts
/// with it, else its first mention.
pub fn key_span(text: &str, key: &str) -> Span {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        let bare = trimmed.trim_start_matches('[').trim_start_matches('"');
        if bare.starts_with(key)
            && bare[key.len()..]
                .trim_start_matches('"')
                .trim_start()
                .starts_with(['=', ']', '.'])
        {
            let start = offset + indent + (trimmed.len() - bare.len());
            return Span::new(start, start + key.len());
        }
        offset += line.len();
    }
    let start = text.find(key).unwrap_or(0);
    Span::new(start, (start + key.len()).min(text.len()))
}

/// Writes a manifest's dependency table back, keeping the rest of the file
/// as it is. Used by `kiln add` and `kiln remove`.
pub fn with_dependencies(text: &str, deps: &BTreeMap<String, Dependency>) -> String {
    let mut out = String::new();
    let mut skipping = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('[') {
            skipping = t == "[dependencies]";
            if skipping {
                continue;
            }
        }
        if !skipping {
            out.push_str(line);
        }
    }
    let mut out = out.trim_end().to_string();
    if !deps.is_empty() {
        out.push_str("\n\n[dependencies]\n");
        for (alias, dep) in deps {
            out.push_str(&format!("{alias} = {}\n", dependency_toml(dep)));
        }
    } else {
        out.push('\n');
    }
    out
}

pub fn dependency_toml(dep: &Dependency) -> String {
    let q = |s: &str| toml::Value::String(s.to_string()).to_string();
    match &dep.source {
        DepSource::Registry { req, registry } => {
            let mut s = format!(
                "{{ ware = {}, version = {}",
                q(&dep.name.to_string()),
                q(&req.to_string())
            );
            if let Some(r) = registry {
                s.push_str(&format!(", registry = {}", q(r)));
            }
            s + " }"
        }
        DepSource::Path(p) => format!("{{ ware = {}, path = {} }}", q(&dep.name.to_string()), q(p)),
        DepSource::Git { url, reference } => {
            let r = match reference {
                GitRef::Rev(r) => format!(", rev = {}", q(r)),
                GitRef::Branch(b) => format!(", branch = {}", q(b)),
                GitRef::Tag(t) => format!(", tag = {}", q(t)),
                GitRef::Default => String::new(),
            };
            format!(
                "{{ ware = {}, git = {}{r} }}",
                q(&dep.name.to_string()),
                q(url)
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependencies_and_errors() {
        let text = "[package]\nentry = \"main.kiln\"\n\n[dependencies]\nui = \"kiln/ui@0.1\"\nw = { ware = \"local/widgets\", path = \"vendor/w\" }\n";
        let m = Manifest::parse(Path::new("kiln.toml"), text).unwrap();
        assert_eq!(m.dependencies.len(), 2);
        assert!(matches!(m.dependencies["w"].source, DepSource::Path(_)));
        let again = with_dependencies(text, &m.dependencies);
        let m2 = Manifest::parse(Path::new("kiln.toml"), &again).unwrap();
        assert_eq!(m.dependencies, m2.dependencies);
        let err = Manifest::parse(Path::new("k"), "[package]\n[dependencies]\nweb = \"a/b\"\n")
            .unwrap_err();
        assert!(err.1.contains("standard module"), "{}", err.1);
        assert_eq!(
            &"[package]\n[dependencies]\nweb = \"a/b\"\n"[err.0.start as usize..err.0.end as usize],
            "web"
        );
        let err = Manifest::parse(
            Path::new("k"),
            "[package]\nkind = \"ware\"\nname = \"a/b\"\n",
        )
        .unwrap_err();
        assert!(err.1.contains("version"));
    }
}
