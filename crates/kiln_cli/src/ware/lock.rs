//! `kiln.lock`: the exact Wares a project builds with. It names each one's
//! version, where it came from (a registry, a Git commit or a local path)
//! and the SHA-256 of its source, with the edges between them. kiln writes
//! it with sorted keys and `\n` line endings, so one resolution always
//! makes the same bytes on every system.

use super::name::WareName;
use semver::Version;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write;
use std::path::Path;

pub const SCHEMA: i64 = 1;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Lock {
    /// The project's own dependencies, by the name it imports them with.
    pub root: BTreeMap<String, WareName>,
    pub wares: BTreeMap<WareName, Locked>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Locked {
    pub version: Version,
    pub source: Source,
    /// The archive's SHA-256; a path Ware has none, its source being live.
    pub checksum: Option<String>,
    pub capabilities: BTreeSet<String>,
    pub dependencies: BTreeMap<String, WareName>,
    /// What the registry said about the release when it was locked.
    pub trust: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Source {
    Registry(String),
    Git {
        url: String,
        rev: String,
    },
    /// Relative to the project root, with `/`.
    Path(String),
}

impl Source {
    pub fn kind(&self) -> &'static str {
        match self {
            Source::Registry(_) => "registry",
            Source::Git { .. } => "git",
            Source::Path(_) => "path",
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::Registry(url) => write!(f, "registry+{url}"),
            Source::Git { url, rev } => write!(f, "git+{url}#{rev}"),
            Source::Path(p) => write!(f, "path+{p}"),
        }
    }
}

impl std::str::FromStr for Source {
    type Err = String;
    fn from_str(s: &str) -> Result<Source, String> {
        if let Some(url) = s.strip_prefix("registry+") {
            return Ok(Source::Registry(url.into()));
        }
        if let Some(rest) = s.strip_prefix("git+") {
            let (url, rev) = rest
                .rsplit_once('#')
                .ok_or_else(|| format!("`{s}`: a Git source needs `#commit`"))?;
            if rev.len() != 40 || !rev.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(format!("`{s}`: a Git source pins a full commit hash"));
            }
            return Ok(Source::Git {
                url: url.into(),
                rev: rev.into(),
            });
        }
        if let Some(p) = s.strip_prefix("path+") {
            return Ok(Source::Path(p.into()));
        }
        Err(format!("unknown source `{s}`"))
    }
}

impl Lock {
    pub fn load(root: &Path) -> Result<Option<Lock>, String> {
        let path = root.join("kiln.lock");
        if !path.is_file() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Lock::parse(&text).map(Some).map_err(|e| {
            format!(
                "{}: {e}; delete it and run `kiln update` to write it again",
                path.display()
            )
        })
    }

    pub fn parse(text: &str) -> Result<Lock, String> {
        let value: toml::Table = toml::from_str(text).map_err(|e| e.message().to_string())?;
        let version = value.get("version").and_then(toml::Value::as_integer);
        if version != Some(SCHEMA) {
            return Err(format!(
                "lockfile version {} is not one this kiln reads (it reads {SCHEMA})",
                version.map_or("missing".into(), |v| v.to_string())
            ));
        }
        let mut lock = Lock::default();
        if let Some(root) = value.get("root").and_then(toml::Value::as_table) {
            for (alias, name) in root {
                let name = name.as_str().ok_or("root entries are Ware names")?;
                lock.root.insert(alias.clone(), WareName::parse(name)?);
            }
        }
        for ware in value
            .get("ware")
            .and_then(toml::Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let t = ware.as_table().ok_or("each [[ware]] is a table")?;
            let s = |k: &str| t.get(k).and_then(toml::Value::as_str);
            let name = WareName::parse(s("name").ok_or("a [[ware]] without a name")?)?;
            let version = Version::parse(s("version").ok_or("a [[ware]] without a version")?)
                .map_err(|e| e.to_string())?;
            let source = s("source").ok_or("a [[ware]] without a source")?.parse()?;
            let checksum = s("checksum").map(String::from);
            if let Some(c) = &checksum {
                super::cache::ware_dir(c)?;
            }
            let strings = |k: &str| -> Result<BTreeSet<String>, String> {
                t.get(k)
                    .and_then(toml::Value::as_array)
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(String::from)
                            .ok_or(format!("{k} lists strings"))
                    })
                    .collect()
            };
            let mut dependencies = BTreeMap::new();
            if let Some(deps) = t.get("dependencies").and_then(toml::Value::as_table) {
                for (alias, n) in deps {
                    dependencies.insert(
                        alias.clone(),
                        WareName::parse(n.as_str().ok_or("dependencies are Ware names")?)?,
                    );
                }
            }
            let locked = Locked {
                version,
                checksum,
                capabilities: strings("capabilities")?,
                dependencies,
                trust: s("trust").map(String::from),
                source,
            };
            if !matches!(locked.source, Source::Path(_)) && locked.checksum.is_none() {
                return Err(format!("{name} has no checksum"));
            }
            if lock.wares.insert(name.clone(), locked).is_some() {
                return Err(format!("{name} is listed twice"));
            }
        }
        for (alias, n) in lock
            .root
            .iter()
            .chain(lock.wares.values().flat_map(|w| w.dependencies.iter()))
        {
            if !lock.wares.contains_key(n) {
                return Err(format!("`{alias}` names {n}, which is not listed"));
            }
        }
        Ok(lock)
    }

    pub fn render(&self) -> String {
        let q = |s: &str| toml::Value::String(s.to_string()).to_string();
        let mut out = String::from(
            "# The exact Wares this project builds with, written by kiln. Commit it.\nversion = 1\n",
        );
        if !self.root.is_empty() {
            out.push_str("\n[root]\n");
            for (alias, name) in &self.root {
                let _ = writeln!(out, "{alias} = {}", q(&name.to_string()));
            }
        }
        for (name, w) in &self.wares {
            let _ = writeln!(out, "\n[[ware]]\nname = {}", q(&name.to_string()));
            let _ = writeln!(out, "version = {}", q(&w.version.to_string()));
            let _ = writeln!(out, "source = {}", q(&w.source.to_string()));
            if let Some(c) = &w.checksum {
                let _ = writeln!(out, "checksum = {}", q(c));
            }
            if let Some(t) = &w.trust {
                let _ = writeln!(out, "trust = {}", q(t));
            }
            let caps: Vec<String> = w.capabilities.iter().map(|c| q(c)).collect();
            let _ = writeln!(out, "capabilities = [{}]", caps.join(", "));
            let deps: Vec<String> = w
                .dependencies
                .iter()
                .map(|(a, n)| format!("{a} = {}", q(&n.to_string())))
                .collect();
            if deps.is_empty() {
                out.push_str("dependencies = {}\n");
            } else {
                let _ = writeln!(out, "dependencies = {{ {} }}", deps.join(", "));
            }
        }
        out
    }

    /// Writes the lockfile through a temporary file and a rename, so a
    /// reader never sees half of it.
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let path = root.join("kiln.lock");
        let text = self.render();
        if std::fs::read_to_string(&path).is_ok_and(|old| old == text) {
            return Ok(());
        }
        write_atomic(&path, &text)
    }
}

pub fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let tmp = path.with_extension(format!("tmp-{}", std::process::id()));
    std::fs::write(&tmp, text).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("cannot write {}: {e}", path.display())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_byte_identical() {
        let ui = WareName::parse("kiln/ui").unwrap();
        let icons = WareName::parse("kiln/icons").unwrap();
        let mut lock = Lock::default();
        lock.root.insert("ui".into(), ui.clone());
        lock.wares.insert(
            ui,
            Locked {
                version: Version::new(0, 1, 0),
                source: Source::Registry("http://127.0.0.1:8081".into()),
                checksum: Some(format!("sha256:{}", "a".repeat(64))),
                capabilities: BTreeSet::new(),
                dependencies: BTreeMap::from([("icons".into(), icons.clone())]),
                trust: Some("unverified".into()),
            },
        );
        lock.wares.insert(
            icons,
            Locked {
                version: Version::new(1, 2, 3),
                source: Source::Path("vendor/icons".into()),
                checksum: None,
                capabilities: BTreeSet::from(["filesystem".into()]),
                dependencies: BTreeMap::new(),
                trust: None,
            },
        );
        let text = lock.render();
        let again = Lock::parse(&text).unwrap();
        assert_eq!(again, lock);
        assert_eq!(again.render(), text);
        assert!(Lock::parse("version = 2\n").is_err());
    }
}
