//! Choosing one release of each Ware a project needs, then checking that a
//! lockfile still describes the manifests.
//!
//! Resolution is deterministic: each Ware gets the highest release that
//! every requirement on it allows (or the locked one, while it still fits),
//! taken in manifest order, breadth first. A requirement that the chosen
//! release does not meet is carried into another pass; when no release
//! meets them all, the error lists each requirement and who made it. A
//! graph holds one release per Ware.

use super::lock::{Lock, Locked, Source};
use super::manifest::{DepSource, Dependency, Kind, Manifest};
use super::name::WareName;
use super::registry::{self, Release};
use super::{archive, cache, git};
use semver::{Version, VersionReq};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub enum Update {
    /// Keep locked releases that still fit.
    #[default]
    Keep,
    All,
    Only(BTreeSet<WareName>),
}

#[derive(Clone, Debug, Default)]
pub struct Options {
    pub offline: bool,
    pub update: Update,
}

impl Options {
    fn keeps(&self, name: &WareName) -> bool {
        match &self.update {
            Update::Keep => true,
            Update::All => false,
            Update::Only(names) => !names.contains(name),
        }
    }
}

/// A dependency still to resolve: the Ware that asks for it (none for the
/// project), who that is in words, the folder its paths are relative to,
/// its default registry, the alias it uses, and the dependency.
type Pending = (
    Option<WareName>,
    String,
    PathBuf,
    Option<String>,
    String,
    Dependency,
);

/// Where a requirement says a Ware comes from, to compare two of them.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Origin {
    Registry(String),
    Git(String),
    Path(PathBuf),
}

impl std::fmt::Display for Origin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Origin::Registry(u) => write!(f, "the registry {u}"),
            Origin::Git(u) => write!(f, "Git {u}"),
            Origin::Path(p) => write!(f, "the folder {}", p.display()),
        }
    }
}

struct Candidate {
    version: Version,
    source: Source,
    checksum: Option<String>,
    deps: BTreeMap<String, Dependency>,
    /// The folder `path` dependencies of this Ware are relative to.
    base: Option<PathBuf>,
    /// The registry its own registry dependencies default to.
    registry: Option<String>,
    capabilities: BTreeSet<String>,
    trust: Option<String>,
}

pub struct Resolver<'a> {
    root: &'a Path,
    default_registry: Option<String>,
    previous: Option<&'a Lock>,
    options: &'a Options,
    indexes: BTreeMap<(String, WareName), Vec<Release>>,
}

pub fn default_registry(manifest: &Manifest) -> Option<String> {
    manifest.registry.clone().or_else(|| {
        std::env::var("KILN_REGISTRY")
            .ok()
            .filter(|r| !r.is_empty())
            .map(|r| r.trim_end_matches('/').to_string())
    })
}

/// A path as the lockfile writes it: relative to the root where it can be,
/// with `/` on every system.
pub fn lock_path(root: &Path, path: &Path) -> String {
    let path = normalize(path);
    let root = normalize(root);
    let mut common = 0;
    let a: Vec<_> = root.components().collect();
    let b: Vec<_> = path.components().collect();
    while common < a.len() && common < b.len() && a[common] == b[common] {
        common += 1;
    }
    if common == 0 {
        return path.to_string_lossy().replace('\\', "/");
    }
    let mut parts: Vec<String> = vec!["..".into(); a.len() - common];
    parts.extend(
        b[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        ".".into()
    } else {
        parts.join("/")
    }
}

/// `a/./b/../c` as `a/c`, without touching the disk.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn origin_of(dep: &Dependency, base: &Path, registry: Option<&str>) -> Result<Origin, String> {
    Ok(match &dep.source {
        DepSource::Registry { registry: r, .. } => Origin::Registry(
            r.clone()
                .or_else(|| registry.map(String::from))
                .ok_or_else(|| {
                    format!(
                        "no registry is set for {}: add `[registry]` with `url = \"...\"` to kiln.toml, or set KILN_REGISTRY",
                        dep.name
                    )
                })?,
        ),
        DepSource::Git { url, .. } => Origin::Git(url.clone()),
        DepSource::Path(p) => Origin::Path(normalize(&base.join(p))),
    })
}

impl<'a> Resolver<'a> {
    pub fn new(
        root: &'a Path,
        manifest: &Manifest,
        previous: Option<&'a Lock>,
        options: &'a Options,
    ) -> Self {
        Resolver {
            root,
            default_registry: default_registry(manifest),
            previous,
            options,
            indexes: BTreeMap::new(),
        }
    }

    pub fn resolve(&mut self, manifest: &Manifest) -> Result<Lock, String> {
        let mut constraints: BTreeMap<WareName, Vec<(VersionReq, String)>> = BTreeMap::new();
        for _ in 0..32 {
            match self.pass(manifest, &mut constraints)? {
                Some(lock) => return Ok(lock),
                None => continue,
            }
        }
        Err("the Wares' requirements did not settle; run `kiln tree` and loosen a version".into())
    }

    /// One pass. `None` means a requirement was not met by the release
    /// chosen before it was seen, and another pass is needed.
    fn pass(
        &mut self,
        manifest: &Manifest,
        constraints: &mut BTreeMap<WareName, Vec<(VersionReq, String)>>,
    ) -> Result<Option<Lock>, String> {
        let mut selected: BTreeMap<WareName, Candidate> = BTreeMap::new();
        let mut origins: BTreeMap<WareName, (Origin, String)> = BTreeMap::new();
        let mut edges: BTreeMap<WareName, BTreeMap<String, WareName>> = BTreeMap::new();
        let mut lock = Lock::default();
        let mut queue: VecDeque<Pending> = VecDeque::new();
        for (alias, dep) in &manifest.dependencies {
            queue.push_back((
                None,
                "kiln.toml".into(),
                self.root.to_path_buf(),
                self.default_registry.clone(),
                alias.clone(),
                dep.clone(),
            ));
        }
        while let Some((parent, label, base, registry, alias, dep)) = queue.pop_front() {
            let name = dep.name.clone();
            match &parent {
                None => {
                    lock.root.insert(alias, name.clone());
                }
                Some(p) => {
                    edges
                        .entry(p.clone())
                        .or_default()
                        .insert(alias, name.clone());
                }
            }
            if parent.is_some() {
                if let (DepSource::Path(_), Some(c)) =
                    (&dep.source, parent.as_ref().and_then(|p| selected.get(p)))
                {
                    if !matches!(c.source, Source::Path(_)) {
                        return Err(format!(
                            "{label} depends on {name} by a local path, which only a local Ware may do"
                        ));
                    }
                }
            }
            let origin = origin_of(&dep, &base, registry.as_deref())?;
            if let Some((first, first_from)) = origins.get(&name) {
                if *first != origin {
                    return Err(format!(
                        "{name} comes from two places:\n  {first} (required by {first_from})\n  {origin} (required by {label})\nmake them the same source"
                    ));
                }
                if let DepSource::Registry { req, .. } = &dep.source {
                    let chosen = &selected[&name].version;
                    if !req.matches(chosen) {
                        let list = constraints.entry(name.clone()).or_default();
                        if !list.iter().any(|(r, l)| r == req && *l == label) {
                            list.push((req.clone(), label));
                        }
                        return Ok(None);
                    }
                }
                continue;
            }
            if let DepSource::Registry { req, .. } = &dep.source {
                let list = constraints.entry(name.clone()).or_default();
                if !list.iter().any(|(r, l)| r == req && *l == label) {
                    list.push((req.clone(), label.clone()));
                }
            }
            let candidate = self.select(
                &name,
                &dep,
                &origin,
                constraints
                    .get(&name)
                    .map(Vec::as_slice)
                    .unwrap_or_default(),
            )?;
            let here = format!("{name} {}", candidate.version);
            for (a, d) in &candidate.deps {
                queue.push_back((
                    Some(name.clone()),
                    here.clone(),
                    candidate
                        .base
                        .clone()
                        .unwrap_or_else(|| self.root.to_path_buf()),
                    candidate
                        .registry
                        .clone()
                        .or_else(|| self.default_registry.clone()),
                    a.clone(),
                    d.clone(),
                ));
            }
            origins.insert(name.clone(), (origin, label));
            selected.insert(name, candidate);
        }
        for (name, c) in selected {
            lock.wares.insert(
                name.clone(),
                Locked {
                    version: c.version,
                    source: c.source,
                    checksum: c.checksum,
                    capabilities: c.capabilities,
                    dependencies: edges.remove(&name).unwrap_or_default(),
                    trust: c.trust,
                },
            );
        }
        check_cycles(&lock)?;
        Ok(Some(lock))
    }

    fn locked(&self, name: &WareName) -> Option<&Locked> {
        self.previous
            .filter(|_| self.options.keeps(name))
            .and_then(|l| l.wares.get(name))
    }

    fn select(
        &mut self,
        name: &WareName,
        dep: &Dependency,
        origin: &Origin,
        reqs: &[(VersionReq, String)],
    ) -> Result<Candidate, String> {
        match (&dep.source, origin) {
            (DepSource::Path(_), Origin::Path(dir)) => {
                let (manifest, _) = ware_manifest_at(dir, name)?;
                Ok(Candidate {
                    version: manifest.version.clone().unwrap(),
                    source: Source::Path(lock_path(self.root, dir)),
                    checksum: None,
                    deps: manifest.dependencies.clone(),
                    base: Some(dir.clone()),
                    registry: manifest.registry.clone(),
                    capabilities: manifest.capabilities.clone(),
                    trust: None,
                })
            }
            (DepSource::Git { url, reference }, _) => {
                let locked_rev = self.locked(name).and_then(|l| match &l.source {
                    Source::Git { url: u, rev } if u == url => Some(rev.clone()),
                    _ => None,
                });
                let rev = match (reference, locked_rev) {
                    (super::manifest::GitRef::Rev(r), _) => r.clone(),
                    (_, Some(rev)) => rev,
                    (r, None) => git::resolve(url, r, self.options.offline)?,
                };
                let files = git::files(url, &rev, self.options.offline)?;
                let text = archive::encode(&files);
                let checksum = archive::sha256(text.as_bytes());
                cache::store(&text, &checksum)?;
                let manifest =
                    ware_manifest(files.get("kiln.toml"), name, &format!("{url}#{rev}"))?;
                if let Some((alias, _)) = manifest
                    .dependencies
                    .iter()
                    .find(|(_, d)| matches!(d.source, DepSource::Path(_)))
                {
                    return Err(format!("{name} from Git depends on `{alias}` by a local path, which only a local Ware may do"));
                }
                Ok(Candidate {
                    version: manifest.version.clone().unwrap(),
                    source: Source::Git {
                        url: url.clone(),
                        rev,
                    },
                    checksum: Some(checksum),
                    deps: manifest.dependencies.clone(),
                    base: None,
                    registry: manifest.registry.clone(),
                    capabilities: manifest.capabilities.clone(),
                    trust: None,
                })
            }
            (DepSource::Registry { .. }, Origin::Registry(url)) => {
                self.select_release(name, url, reqs)
            }
            _ => unreachable!("an origin follows its source"),
        }
    }

    fn select_release(
        &mut self,
        name: &WareName,
        url: &str,
        reqs: &[(VersionReq, String)],
    ) -> Result<Candidate, String> {
        let fits = |v: &Version| reqs.iter().all(|(r, _)| r.matches(v));
        // A locked release that still fits and is cached needs no network.
        if let Some(l) = self.locked(name) {
            if l.source == Source::Registry(url.to_string()) && fits(&l.version) {
                if let Some(dir) = l
                    .checksum
                    .as_deref()
                    .map(cache::lookup)
                    .transpose()?
                    .flatten()
                {
                    let (manifest, _) = ware_manifest_at(&dir, name)?;
                    return Ok(Candidate {
                        version: l.version.clone(),
                        source: l.source.clone(),
                        checksum: l.checksum.clone(),
                        deps: manifest.dependencies.clone(),
                        base: None,
                        registry: Some(url.to_string()),
                        capabilities: manifest.capabilities.clone(),
                        trust: l.trust.clone(),
                    });
                }
            }
        }
        if self.options.offline {
            return Err(format!(
                "{name} is needed, and kiln.lock has no cached release of it that fits; run `kiln update` while online"
            ));
        }
        let key = (url.to_string(), name.clone());
        if !self.indexes.contains_key(&key) {
            let releases = registry::index(url, name)?;
            self.indexes.insert(key.clone(), releases);
        }
        let releases = &self.indexes[&key];
        let locked_version = self
            .locked(name)
            .filter(|l| l.source == Source::Registry(url.to_string()))
            .map(|l| l.version.clone());
        let eligible: Vec<&Release> = releases
            .iter()
            .filter(|r| fits(&r.version))
            .filter(|r| !r.yanked || Some(&r.version) == locked_version.as_ref())
            .collect();
        let chosen = locked_version
            .as_ref()
            .and_then(|v| eligible.iter().find(|r| r.version == *v))
            .or_else(|| eligible.iter().max_by(|a, b| a.version.cmp(&b.version)))
            .copied();
        let Some(r) = chosen else {
            let mut msg = format!("no release of {name} fits every requirement:");
            for (req, from) in reqs {
                msg.push_str(&format!("\n  {req} (from {from})"));
            }
            let mut available: Vec<String> = releases
                .iter()
                .map(|r| {
                    if r.yanked {
                        format!("{} (yanked)", r.version)
                    } else {
                        r.version.to_string()
                    }
                })
                .collect();
            if available.is_empty() {
                available.push("none".into());
            }
            msg.push_str(&format!("\n{url} has: {}", available.join(", ")));
            return Err(msg);
        };
        Ok(Candidate {
            version: r.version.clone(),
            source: Source::Registry(url.to_string()),
            checksum: Some(r.checksum.clone()),
            deps: r.dependencies.clone(),
            base: None,
            registry: Some(url.to_string()),
            capabilities: r.capabilities.clone(),
            trust: Some(r.trust.clone()),
        })
    }
}

/// A Ware's manifest from its text, which must name the Ware expected.
pub fn ware_manifest(text: Option<&String>, name: &WareName, at: &str) -> Result<Manifest, String> {
    let text = text.ok_or_else(|| format!("{at} has no kiln.toml, so it is not a Ware"))?;
    let manifest = Manifest::parse(Path::new("kiln.toml"), text)
        .map_err(|(span, msg)| format!("{at}/kiln.toml (byte {}): {msg}", span.start))?;
    if manifest.kind != Kind::Ware {
        return Err(format!(
            "{at} is not a Ware: its kiln.toml needs `kind = \"ware\"`"
        ));
    }
    if manifest.ware_name().as_ref() != Some(name) {
        return Err(format!(
            "{at} is the Ware {}, not {name}",
            manifest.name.as_deref().unwrap_or("?")
        ));
    }
    Ok(manifest)
}

pub fn ware_manifest_at(dir: &Path, name: &WareName) -> Result<(Manifest, String), String> {
    let text = std::fs::read_to_string(dir.join("kiln.toml"))
        .map_err(|e| format!("cannot read {}: {e}", dir.join("kiln.toml").display()))?;
    let mut m = ware_manifest(Some(&text), name, &dir.display().to_string())?;
    m.path = dir.join("kiln.toml");
    Ok((m, text))
}

pub fn check_cycles(lock: &Lock) -> Result<(), String> {
    fn visit(
        lock: &Lock,
        name: &WareName,
        stack: &mut Vec<WareName>,
        done: &mut BTreeSet<WareName>,
    ) -> Result<(), String> {
        if let Some(i) = stack.iter().position(|n| n == name) {
            let chain: Vec<String> = stack[i..]
                .iter()
                .chain([name])
                .map(|n| n.to_string())
                .collect();
            return Err(format!(
                "the Wares depend on each other in a cycle: {}",
                chain.join(" -> ")
            ));
        }
        if done.contains(name) {
            return Ok(());
        }
        stack.push(name.clone());
        if let Some(w) = lock.wares.get(name) {
            for dep in w.dependencies.values() {
                visit(lock, dep, stack, done)?;
            }
        }
        stack.pop();
        done.insert(name.clone());
        Ok(())
    }
    let mut done = BTreeSet::new();
    for name in lock.wares.keys() {
        visit(lock, name, &mut Vec::new(), &mut done)?;
    }
    Ok(())
}

/// Whether a locked release still meets a manifest's requirement.
fn satisfies(
    root: &Path,
    base: &Path,
    registry: Option<&str>,
    dep: &Dependency,
    locked: &Locked,
) -> Result<(), String> {
    let origin = origin_of(dep, base, registry)?;
    let ok = match (&dep.source, &locked.source, origin) {
        (DepSource::Registry { req, .. }, Source::Registry(url), Origin::Registry(want)) => {
            *url == want && req.matches(&locked.version)
        }
        (DepSource::Git { url, reference }, Source::Git { url: u, rev }, _) => {
            url == u
                && match reference {
                    super::manifest::GitRef::Rev(r) => r == rev,
                    _ => true,
                }
        }
        (DepSource::Path(_), Source::Path(p), Origin::Path(dir)) => lock_path(root, &dir) == *p,
        _ => false,
    };
    if ok {
        Ok(())
    } else {
        Err(format!(
            "{} {} from {} no longer matches what kiln.toml asks for",
            dep.name, locked.version, locked.source
        ))
    }
}

/// Where a locked Ware's source is: its folder for a path Ware, else its
/// tree in the cache, fetched first unless `offline`.
pub fn locate(root: &Path, name: &WareName, w: &Locked, offline: bool) -> Result<PathBuf, String> {
    match &w.source {
        Source::Path(p) => {
            let dir = normalize(&root.join(p));
            if !dir.join("kiln.toml").is_file() {
                return Err(format!(
                    "{name} is locked at {}, which has no kiln.toml",
                    dir.display()
                ));
            }
            Ok(dir)
        }
        Source::Registry(url) => {
            let checksum = w.checksum.as_deref().unwrap();
            if let Some(dir) = cache::lookup(checksum)? {
                return Ok(dir);
            }
            if offline {
                return Err(format!(
                    "{name} {} is not in the Ware cache ({}); run `kiln fetch` while online",
                    w.version,
                    cache::dir().display()
                ));
            }
            let text = registry::archive(url, name, &w.version)?;
            cache::store(&text, checksum)
                .map_err(|e| format!("{name} {} from {url}: {e}", w.version))
        }
        Source::Git { url, rev } => {
            let checksum = w.checksum.as_deref().unwrap();
            if let Some(dir) = cache::lookup(checksum)? {
                return Ok(dir);
            }
            let files = git::files(url, rev, offline).map_err(|e| {
                if offline {
                    format!("{name} is not in the Ware cache; run `kiln fetch` while online ({e})")
                } else {
                    e
                }
            })?;
            cache::store(&archive::encode(&files), checksum)
                .map_err(|e| format!("{name} from {url}#{rev}: {e}"))
        }
    }
}

/// Checks a lockfile against the project's manifest and every locked
/// Ware's own manifest, fetching what is missing unless `offline`.
/// Returns each Ware's folder.
pub fn verify(
    root: &Path,
    manifest: &Manifest,
    lock: &Lock,
    offline: bool,
) -> Result<BTreeMap<WareName, PathBuf>, String> {
    let mismatch = |why: String| format!("kiln.lock does not match kiln.toml: {why}");
    let wanted: BTreeSet<&String> = manifest.dependencies.keys().collect();
    let locked: BTreeSet<&String> = lock.root.keys().collect();
    if wanted != locked {
        let added: Vec<_> = wanted.difference(&locked).map(|s| s.as_str()).collect();
        let removed: Vec<_> = locked.difference(&wanted).map(|s| s.as_str()).collect();
        return Err(mismatch(format!(
            "dependencies added: [{}], removed: [{}]",
            added.join(", "),
            removed.join(", ")
        )));
    }
    let registry = default_registry(manifest);
    for (alias, dep) in &manifest.dependencies {
        let name = &lock.root[alias];
        if *name != dep.name {
            return Err(mismatch(format!("`{alias}` is now {}", dep.name)));
        }
        satisfies(root, root, registry.as_deref(), dep, &lock.wares[name]).map_err(mismatch)?;
    }
    let mut dirs = BTreeMap::new();
    for (name, w) in &lock.wares {
        let dir = locate(root, name, w, offline)?;
        let (m, _) = ware_manifest_at(&dir, name)?;
        if m.version.as_ref() != Some(&w.version) {
            return Err(mismatch(format!(
                "{name} at {} is version {}, locked as {}",
                dir.display(),
                m.version.as_ref().unwrap(),
                w.version
            )));
        }
        if m.capabilities != w.capabilities {
            return Err(mismatch(format!(
                "{name} {} declares other capabilities than locked",
                w.version
            )));
        }
        let wanted: BTreeSet<&String> = m.dependencies.keys().collect();
        let have: BTreeSet<&String> = w.dependencies.keys().collect();
        if wanted != have {
            return Err(mismatch(format!(
                "{name} {} has other dependencies than locked",
                w.version
            )));
        }
        let ware_registry = match &w.source {
            Source::Registry(u) => Some(u.clone()),
            _ => m.registry.clone().or_else(|| registry.clone()),
        };
        for (alias, dep) in &m.dependencies {
            let target = &w.dependencies[alias];
            if *target != dep.name {
                return Err(mismatch(format!("{name}'s `{alias}` is now {}", dep.name)));
            }
            satisfies(
                root,
                &dir,
                ware_registry.as_deref(),
                dep,
                &lock.wares[target],
            )
            .map_err(mismatch)?;
        }
        dirs.insert(name.clone(), dir);
    }
    check_cycles(lock)?;
    Ok(dirs)
}
