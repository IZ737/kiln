//! The `kiln` commands for Wares: `add`, `remove`, `update`, `tree`,
//! `fetch`, `publish`, `login`, `yank`, `cache` and `ware verify`.

use super::lock::{Lock, Source};
use super::manifest::{self, DepSource, Dependency, GitRef, Kind, Manifest};
use super::name::{check_alias, WareName};
use super::{archive, cache, policy, registry, resolve};
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

pub fn project_root(dir: &Path) -> Result<PathBuf, String> {
    crate::project::root(dir).map_err(|e| format!("cannot read {}: {e}", dir.display()))
}

fn load_manifest(root: &Path) -> Result<Option<Manifest>, String> {
    Manifest::load(root).map_err(|(path, (span, msg))| {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        let line = text[..(span.start as usize).min(text.len())]
            .lines()
            .count()
            .max(1);
        format!("{}:{line}: {msg}", path.display())
    })
}

pub struct AddOptions {
    pub version: Option<String>,
    pub path: Option<PathBuf>,
    pub git: Option<String>,
    pub rev: Option<String>,
    pub branch: Option<String>,
    pub tag: Option<String>,
    pub registry: Option<String>,
    pub alias: Option<String>,
}

/// Asks before Wares gain capabilities: on a terminal, a question; anywhere
/// else, `--allow-capabilities` must have been given.
fn approve(changes: &policy::Changes, allow: bool) -> Result<(), String> {
    for line in &changes.lines {
        println!("{line}");
    }
    if changes.escalations.is_empty() || allow {
        return Ok(());
    }
    let list = changes.escalations.join("; ");
    if std::io::stdin().is_terminal() && std::io::stdout().is_terminal() {
        print!("Wares would gain capabilities: {list}. A Ware's code runs with your program's rights; capabilities are declared, not enforced. Continue? [y/N] ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        let _ = std::io::stdin().read_line(&mut answer);
        if matches!(answer.trim(), "y" | "Y" | "yes") {
            return Ok(());
        }
        return Err("nothing was changed".into());
    }
    Err(format!(
        "Wares would gain capabilities: {list}; nothing was changed. Run again with --allow-capabilities to accept them"
    ))
}

/// Resolves a manifest and writes it and its lockfile together: both or
/// neither.
fn apply(
    root: &Path,
    manifest_text: Option<&str>,
    manifest: &Manifest,
    update: resolve::Update,
    allow: bool,
) -> Result<Lock, String> {
    let mode = super::mode();
    let previous = Lock::load(root)?;
    let options = resolve::Options {
        offline: mode.offline,
        update,
    };
    let new =
        resolve::Resolver::new(root, manifest, previous.as_ref(), &options).resolve(manifest)?;
    if let Some(policy) = &manifest.policy {
        policy::check(policy, &new)?;
    }
    let dirs = resolve::verify(root, manifest, &new, mode.offline)?;
    for (name, w) in &new.wares {
        policy::check_declared(name, &w.capabilities, &dirs[name])?;
    }
    approve(
        &policy::changes(previous.as_ref(), &new),
        mode.allow_capabilities || allow,
    )?;
    let manifest_path = root.join("kiln.toml");
    let old_manifest = std::fs::read_to_string(&manifest_path).ok();
    if let Some(text) = manifest_text {
        super::lock::write_atomic(&manifest_path, text)?;
    }
    if new.wares.is_empty() && new.root.is_empty() {
        let _ = std::fs::remove_file(root.join("kiln.lock"));
        return Ok(new);
    }
    if let Err(e) = new.save(root) {
        if let (Some(_), Some(old)) = (manifest_text, old_manifest) {
            let _ = super::lock::write_atomic(&manifest_path, &old);
        }
        return Err(e);
    }
    Ok(new)
}

pub fn add(spec: &str, dir: &Path, o: AddOptions, allow: bool) -> Result<String, String> {
    let root = project_root(dir)?;
    let (name, inline_version) = match spec.split_once('@') {
        Some((n, v)) => (n, Some(v.to_string())),
        None => (spec, None),
    };
    if !name.contains('/') {
        return Err(format!(
            "`{name}` is not a Ware name: Ware names are `publisher/name`, such as `kiln/ui`; for an npm package, write `npm:<package>@<version>`"
        ));
    }
    let name = WareName::parse(name)?;
    let version = o.version.clone().or(inline_version);
    let alias = o.alias.clone().unwrap_or_else(|| name.name.clone());
    check_alias(&alias)?;
    if root.join(&alias).is_dir() {
        return Err(format!(
            "the folder `{alias}/` already uses the name `{alias}`; choose another with `--as`"
        ));
    }
    let source = match (&o.path, &o.git) {
        (Some(_), Some(_)) => return Err("give `--path` or `--git`, not both".into()),
        (Some(path), None) => {
            if version.is_some() || o.registry.is_some() {
                return Err("a `--path` Ware takes no version or registry".into());
            }
            let abs = if path.is_absolute() {
                path.clone()
            } else {
                std::env::current_dir()
                    .map_err(|e| e.to_string())?
                    .join(path)
            };
            let abs = abs
                .canonicalize()
                .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
            DepSource::Path(resolve::lock_path(&root, &abs))
        }
        (None, Some(url)) => {
            if version.is_some() || o.registry.is_some() {
                return Err(
                    "a `--git` Ware takes `--rev`, `--branch` or `--tag`, not a version".into(),
                );
            }
            let reference = match (&o.rev, &o.branch, &o.tag) {
                (Some(r), None, None) => GitRef::Rev(r.to_lowercase()),
                (None, Some(b), None) => GitRef::Branch(b.clone()),
                (None, None, Some(t)) => GitRef::Tag(t.clone()),
                (None, None, None) => GitRef::Default,
                _ => return Err("give one of `--rev`, `--branch` and `--tag`".into()),
            };
            if !matches!(reference, GitRef::Rev(_)) {
                eprintln!("note: a branch or tag can move; kiln.lock pins the commit it names now");
            }
            DepSource::Git {
                url: url.clone(),
                reference,
            }
        }
        (None, None) => {
            if let Some(r) = &o.registry {
                registry::check_url(r)?;
            }
            DepSource::Registry {
                req: manifest::parse_req(version.as_deref().unwrap_or("*"))?,
                registry: o
                    .registry
                    .as_ref()
                    .map(|r| r.trim_end_matches('/').to_string()),
            }
        }
    };
    let text = std::fs::read_to_string(root.join("kiln.toml")).unwrap_or_else(|_| {
        let name = root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "app".into());
        format!("[package]\nname = {}\n", toml::Value::String(name))
    });
    let mut manifest = Manifest::parse(&root.join("kiln.toml"), &text)
        .map_err(|(_, m)| format!("kiln.toml: {m}"))?;
    if let Some((other, _)) = manifest
        .dependencies
        .iter()
        .find(|(a, d)| d.name == name && **a != alias)
    {
        return Err(format!("{name} is already a dependency, as `{other}`"));
    }
    let explicit = version.is_some();
    manifest.dependencies.insert(
        alias.clone(),
        Dependency {
            name: name.clone(),
            source,
        },
    );
    // A registry Ware added without a version gets `^` the release chosen.
    if !explicit {
        if let Some(Dependency {
            source: DepSource::Registry { .. },
            ..
        }) = manifest.dependencies.get(&alias)
        {
            let mode = super::mode();
            let previous = Lock::load(&root)?;
            let options = resolve::Options {
                offline: mode.offline,
                update: resolve::Update::Keep,
            };
            let lock = resolve::Resolver::new(&root, &manifest, previous.as_ref(), &options)
                .resolve(&manifest)?;
            let v = &lock.wares[&name].version;
            if let Some(Dependency {
                source: DepSource::Registry { req, .. },
                ..
            }) = manifest.dependencies.get_mut(&alias)
            {
                *req = manifest::parse_req(&format!("^{v}"))?;
            }
        }
    }
    let new_text = manifest::with_dependencies(&text, &manifest.dependencies);
    let manifest = Manifest::parse(&root.join("kiln.toml"), &new_text)
        .map_err(|(_, m)| format!("kiln.toml: {m}"))?;
    let lock = apply(
        &root,
        Some(&new_text),
        &manifest,
        resolve::Update::Keep,
        allow,
    )?;
    let w = &lock.wares[&name];
    Ok(format!(
        "added {name} {} as `{alias}`: `import {alias}` in your code",
        w.version
    ))
}

pub fn remove(which: &str, dir: &Path) -> Result<String, String> {
    let root = project_root(dir)?;
    let text = std::fs::read_to_string(root.join("kiln.toml"))
        .map_err(|_| "this project has no kiln.toml".to_string())?;
    let mut manifest = Manifest::parse(&root.join("kiln.toml"), &text)
        .map_err(|(_, m)| format!("kiln.toml: {m}"))?;
    let alias = manifest
        .dependencies
        .iter()
        .find(|(a, d)| *a == which || d.name.to_string() == which)
        .map(|(a, _)| a.clone())
        .ok_or_else(|| format!("`{which}` is not a dependency"))?;
    let removed = manifest.dependencies.remove(&alias).unwrap();
    let new_text = manifest::with_dependencies(&text, &manifest.dependencies);
    let manifest = Manifest::parse(&root.join("kiln.toml"), &new_text)
        .map_err(|(_, m)| format!("kiln.toml: {m}"))?;
    apply(
        &root,
        Some(&new_text),
        &manifest,
        resolve::Update::Keep,
        false,
    )?;
    Ok(format!("removed {} (`{alias}`)", removed.name))
}

pub fn update(names: &[String], dir: &Path, allow: bool) -> Result<String, String> {
    let root = project_root(dir)?;
    let manifest = load_manifest(&root)?.ok_or("this project has no kiln.toml")?;
    let update = if names.is_empty() {
        resolve::Update::All
    } else {
        let mut set = BTreeSet::new();
        for n in names {
            let name = manifest
                .dependencies
                .get(n)
                .map(|d| d.name.clone())
                .map(Ok)
                .unwrap_or_else(|| WareName::parse(n))?;
            set.insert(name);
        }
        resolve::Update::Only(set)
    };
    let lock = apply(&root, None, &manifest, update, allow)?;
    Ok(format!("kiln.lock lists {} Ware(s)", lock.wares.len()))
}

fn locked(root: &Path) -> Result<Lock, String> {
    Lock::load(root)?.ok_or_else(|| "this project has no kiln.lock; run `kiln update`".to_string())
}

pub fn tree(dir: &Path, why: Option<&str>) -> Result<String, String> {
    let root = project_root(dir)?;
    let manifest = load_manifest(&root)?;
    if manifest.as_ref().is_none_or(|m| m.dependencies.is_empty()) {
        return Ok("this project has no Wares".into());
    }
    let lock = locked(&root)?;
    let label = |name: &WareName| {
        let w = &lock.wares[name];
        let mut extra = vec![w.source.to_string()];
        if let Some(t) = &w.trust {
            extra.push(t.clone());
        }
        let caps = if w.capabilities.is_empty() {
            String::new()
        } else {
            format!(
                " [{}]",
                w.capabilities
                    .iter()
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        format!("{name} {} ({}){caps}", w.version, extra.join(", "))
    };
    if let Some(target) = why {
        let target = lock
            .root
            .get(target)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| WareName::parse(target))?;
        if !lock.wares.contains_key(&target) {
            return Err(format!("{target} is not in kiln.lock"));
        }
        let mut paths = Vec::new();
        fn walk(
            lock: &Lock,
            at: &WareName,
            target: &WareName,
            trail: &mut Vec<String>,
            out: &mut Vec<String>,
        ) {
            if at == target {
                out.push(trail.join(" -> "));
                return;
            }
            for (alias, n) in &lock.wares[at].dependencies {
                if trail.len() > 64 {
                    return;
                }
                trail.push(format!("{n} (as `{alias}`)"));
                walk(lock, n, target, trail, out);
                trail.pop();
            }
        }
        for (alias, n) in &lock.root {
            let mut trail = vec!["kiln.toml".to_string(), format!("{n} (as `{alias}`)")];
            walk(&lock, n, &target, &mut trail, &mut paths);
        }
        return Ok(format!(
            "{} is here because:\n  {}",
            label(&target),
            paths.join("\n  ")
        ));
    }
    let mut out = String::new();
    fn print(
        lock: &Lock,
        label: &dyn Fn(&WareName) -> String,
        deps: &BTreeMap<String, WareName>,
        prefix: &str,
        seen: &mut Vec<WareName>,
        out: &mut String,
    ) {
        let n = deps.len();
        for (i, (alias, name)) in deps.iter().enumerate() {
            let last = i + 1 == n;
            out.push_str(&format!(
                "{prefix}{}{alias}: {}\n",
                if last { "└── " } else { "├── " },
                label(name)
            ));
            if seen.contains(name) {
                continue;
            }
            seen.push(name.clone());
            let child = format!("{prefix}{}", if last { "    " } else { "│   " });
            print(
                lock,
                label,
                &lock.wares[name].dependencies,
                &child,
                seen,
                out,
            );
            seen.pop();
        }
    }
    out.push_str(&format!(
        "{}\n",
        manifest.and_then(|m| m.name).unwrap_or_else(|| root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned())
    ));
    print(&lock, &label, &lock.root, "", &mut Vec::new(), &mut out);
    Ok(out.trim_end().to_string())
}

/// Puts every locked Ware in the cache, so builds can be offline.
pub fn fetch(dir: &Path) -> Result<String, String> {
    let root = project_root(dir)?;
    let Some(manifest) = load_manifest(&root)? else {
        return Ok("this project has no kiln.toml, so no Wares".into());
    };
    let graph = super::prepare(&root, &manifest)?;
    let count = graph.map_or(0, |g| g.wares.len());
    Ok(format!(
        "{count} Ware(s) ready in {}",
        cache::dir().display()
    ))
}

pub fn verify_ware(which: &str, dir: &Path) -> Result<String, String> {
    let root = project_root(dir)?;
    let lock = locked(&root)?;
    let name = lock
        .root
        .get(which)
        .cloned()
        .map(Ok)
        .unwrap_or_else(|| WareName::parse(which))?;
    let w = lock
        .wares
        .get(&name)
        .ok_or_else(|| format!("{name} is not in kiln.lock"))?;
    let dir = resolve::locate(&root, &name, w, super::mode().offline)?;
    let mut report = vec![format!("{name} {} from {}", w.version, w.source)];
    if let Some(checksum) = &w.checksum {
        let files = archive::read_tree(&dir, true)?;
        let actual = archive::sha256(archive::encode(&files).as_bytes());
        if &actual != checksum {
            return Err(format!(
                "{name}: the cached source hashes to {actual}, but kiln.lock says {checksum}"
            ));
        }
        report.push(format!(
            "checksum {checksum} matches ({} files)",
            files.len()
        ));
    } else {
        report.push("a local folder: its source is live, so it has no checksum".into());
    }
    policy::check_declared(&name, &w.capabilities, &dir)?;
    let used = policy::used(&dir)?;
    report.push(if w.capabilities.is_empty() {
        "declares no capabilities, and its imports need none".into()
    } else {
        format!(
            "declares {}; its imports use {}",
            w.capabilities
                .iter()
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
            if used.is_empty() {
                "none".to_string()
            } else {
                used.keys().cloned().collect::<Vec<_>>().join(", ")
            }
        )
    });
    report.push(format!(
        "trust: {}",
        w.trust.as_deref().unwrap_or(match w.source {
            Source::Registry(_) => "unverified",
            _ => "not reviewed by a registry",
        })
    ));
    report.push(
        "A checksum shows the source is what was locked; it does not show the code is safe.".into(),
    );
    Ok(report.join("\n"))
}

pub fn publish(dir: &Path, registry_url: Option<&str>, dry_run: bool) -> Result<String, String> {
    let root = project_root(dir)?;
    let manifest =
        load_manifest(&root)?.ok_or("publishing needs a kiln.toml with `kind = \"ware\"`")?;
    if manifest.kind != Kind::Ware {
        return Err("only a Ware can be published: set `kind = \"ware\"`, `name` and `version` in kiln.toml".into());
    }
    let name = manifest.ware_name().unwrap();
    let version = manifest.version.clone().unwrap();
    let mut deps = serde_json::Map::new();
    for (alias, d) in &manifest.dependencies {
        match &d.source {
            DepSource::Registry { req, registry } => {
                let mut v = serde_json::json!({ "ware": d.name.to_string(), "version": req.to_string() });
                if let Some(r) = registry {
                    v["registry"] = serde_json::Value::String(r.clone());
                }
                deps.insert(alias.clone(), v);
            }
            _ => {
                return Err(format!(
                    "`{alias}` is a path or Git dependency; a published Ware depends only on registry Wares"
                ))
            }
        }
    }
    policy::check_declared(&name, &manifest.capabilities, &root)?;
    let files = archive::read_tree(&root, false)?;
    let text = archive::encode(&files);
    let checksum = archive::sha256(text.as_bytes());
    archive::decode(&text)?;
    let mut summary = format!(
        "{name} {version}: {} files, {} bytes, {checksum}\n",
        files.len(),
        text.len()
    );
    for path in files.keys() {
        summary.push_str(&format!("  {path}\n"));
    }
    if dry_run {
        summary.push_str("dry run: nothing was sent");
        return Ok(summary);
    }
    let url = registry_url
        .map(|r| r.trim_end_matches('/').to_string())
        .or_else(|| resolve::default_registry(&manifest))
        .ok_or("say where to publish with --registry, or set [registry] url in kiln.toml")?;
    registry::check_url(&url)?;
    let token = registry::token(&url)
        .ok_or_else(|| format!("no token for {url}; run `kiln login --registry {url}`"))?;
    let body = serde_json::json!({
        "name": name.to_string(),
        "version": version.to_string(),
        "checksum": checksum,
        "description": manifest.description.clone().unwrap_or_default(),
        "license": manifest.license.clone().unwrap_or_default(),
        "dependencies": deps,
        "capabilities": manifest.capabilities.iter().cloned().collect::<Vec<_>>(),
        "archive": text,
    });
    let done = registry::publish(&url, &token, &name, &version, &body)?;
    Ok(format!("{summary}{done}"))
}

pub fn login(registry_url: &str) -> Result<String, String> {
    let url = registry_url.trim_end_matches('/');
    registry::check_url(url)?;
    if std::io::stdin().is_terminal() {
        eprintln!("Paste a publish token from {url}/tokens, then press Enter:");
    }
    let mut token = String::new();
    std::io::stdin()
        .read_line(&mut token)
        .map_err(|e| e.to_string())?;
    let token = token.trim();
    if token.is_empty() || token.contains(char::is_whitespace) {
        return Err("expected one token on standard input".into());
    }
    let path = registry::save_token(url, token)?;
    Ok(format!("saved the token for {url} in {}", path.display()))
}

pub fn yank(
    spec: &str,
    registry_url: Option<&str>,
    dir: &Path,
    undo: bool,
) -> Result<String, String> {
    let (name, version) = spec
        .split_once('@')
        .ok_or("say which release: `publisher/name@1.2.3`")?;
    let name = WareName::parse(name)?;
    let version = semver::Version::parse(version).map_err(|e| e.to_string())?;
    let root = project_root(dir)?;
    let url = registry_url
        .map(|r| r.trim_end_matches('/').to_string())
        .or_else(|| {
            load_manifest(&root)
                .ok()
                .flatten()
                .and_then(|m| resolve::default_registry(&m))
        })
        .ok_or("say which registry with --registry")?;
    registry::check_url(&url)?;
    let token = registry::token(&url)
        .ok_or_else(|| format!("no token for {url}; run `kiln login --registry {url}`"))?;
    registry::yank(&url, &token, &name, &version, !undo)?;
    Ok(if undo {
        format!("{name} {version} can be chosen again")
    } else {
        format!(
            "yanked {name} {version}: new resolutions skip it, and existing lockfiles keep working"
        )
    })
}

fn bytes(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MiB", n as f64 / (1 << 20) as f64)
    } else if n >= 1 << 10 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}

pub enum CacheAction {
    Dir,
    Stats,
    Verify,
    Prune { dry_run: bool, days: u64 },
}

pub fn cache_command(action: CacheAction) -> Result<String, String> {
    match action {
        CacheAction::Dir => Ok(cache::dir().display().to_string()),
        CacheAction::Stats => {
            let entries = cache::entries();
            let total: u64 = entries.iter().map(|e| e.bytes).sum();
            let git = cache::size(&cache::dir().join("git"));
            Ok(format!(
                "{}\n{} Ware source tree(s), {}\nGit mirrors: {}{}",
                cache::dir().display(),
                entries.len(),
                bytes(total),
                bytes(git),
                if cache::read_only() {
                    "\nread-only (KILN_CACHE_READONLY=1)"
                } else {
                    ""
                }
            ))
        }
        CacheAction::Verify => {
            let bad = cache::verify_all();
            let count = cache::entries().len();
            if bad.is_empty() {
                Ok(format!("{count} cached Ware(s) match their checksums"))
            } else {
                Err(format!(
                    "damaged: {}\nrun `kiln cache prune` or delete them; they are fetched again when needed",
                    bad.iter().map(|(p, why)| format!("{} ({why})", p.display())).collect::<Vec<_>>().join("\n  ")
                ))
            }
        }
        CacheAction::Prune { dry_run, days } => {
            let removed = cache::prune(std::time::Duration::from_secs(days * 24 * 3600), dry_run)?;
            let total: u64 = removed.iter().map(|(_, b)| b).sum();
            let mut out = String::new();
            for (p, b) in &removed {
                out.push_str(&format!(
                    "{} {} ({})\n",
                    if dry_run { "would remove" } else { "removed" },
                    p.display(),
                    bytes(*b)
                ));
            }
            out.push_str(&format!(
                "{} {} in {} item(s)",
                if dry_run { "would free" } else { "freed" },
                bytes(total),
                removed.len()
            ));
            Ok(out)
        }
    }
}
