//! Wares: the packages a project declares in `kiln.toml`, pinned by
//! `kiln.lock` and kept once per user in a cache shared by every project.
//! A Ware is source: its folders are modules, imported by the name the
//! project gives the dependency, and compiled with the program as if they
//! sat under `__ware/<publisher>/<name>/` in the project. Nothing is
//! written into the project but the lockfile.

pub mod archive;
pub mod cache;
pub mod commands;
pub mod git;
pub mod lock;
pub mod manifest;
pub mod name;
pub mod policy;
pub mod registry;
pub mod resolve;

use lock::Lock;
use manifest::Manifest;
use name::WareName;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How commands may treat the lockfile and the network.
#[derive(Clone, Copy, Debug, Default)]
pub struct Mode {
    /// `--locked`: kiln.lock must already match kiln.toml.
    pub locked: bool,
    /// `--offline` (or `KILN_OFFLINE=1`): no network at all.
    pub offline: bool,
    /// Never write kiln.lock, as in the editor.
    pub frozen: bool,
    /// `--allow-capabilities`: accept Wares that gain capabilities.
    pub allow_capabilities: bool,
}

static MODE: Mutex<Mode> = Mutex::new(Mode {
    locked: false,
    offline: false,
    frozen: false,
    allow_capabilities: false,
});

pub fn set_mode(mode: Mode) {
    *MODE.lock().unwrap() = mode;
}

pub fn mode() -> Mode {
    let mut m = *MODE.lock().unwrap();
    if std::env::var("KILN_OFFLINE").is_ok_and(|v| v == "1") {
        m.offline = true;
    }
    m
}

/// A project's Wares, ready to compile.
#[derive(Clone, Debug, Default)]
pub struct Graph {
    /// The project's own dependencies, by import name.
    pub root: BTreeMap<String, WareName>,
    pub wares: BTreeMap<WareName, GraphWare>,
}

#[derive(Clone, Debug)]
pub struct GraphWare {
    pub dir: PathBuf,
    pub deps: BTreeMap<String, WareName>,
    /// Whether its source is a cached tree, which editors must not change.
    pub cached: bool,
}

/// Resolves, locks, fetches and checks a project's Wares. `None` when the
/// project has none.
pub fn prepare(root: &Path, manifest: &Manifest) -> Result<Option<Graph>, String> {
    if manifest.dependencies.is_empty() {
        return Ok(None);
    }
    let mode = mode();
    let previous = Lock::load(root)?;
    let mismatch = "kiln.lock does not match kiln.toml";
    let mut why = "kiln.lock is missing".to_string();
    if let Some(lock) = &previous {
        match resolve::verify(root, manifest, lock, mode.offline) {
            Ok(dirs) => return finish(manifest, lock, dirs).map(Some),
            Err(e) if e.starts_with(mismatch) => why = e,
            Err(e) => return Err(e),
        }
    }
    if mode.locked {
        return Err(format!(
            "{why}, and `--locked` does not allow changing it; run `kiln update` and commit kiln.lock"
        ));
    }
    if mode.frozen {
        return Err(format!("{why}; run `kiln update` to write it"));
    }
    let options = resolve::Options {
        offline: mode.offline,
        update: resolve::Update::Keep,
    };
    let new =
        resolve::Resolver::new(root, manifest, previous.as_ref(), &options).resolve(manifest)?;
    let changes = policy::changes(previous.as_ref(), &new);
    if !changes.escalations.is_empty() && !mode.allow_capabilities {
        return Err(format!(
            "updating kiln.lock would give Wares new capabilities ({}); review and accept them with `kiln update`, or pass --allow-capabilities",
            changes.escalations.join("; ")
        ));
    }
    if let Some(policy) = &manifest.policy {
        policy::check(policy, &new)?;
    }
    for line in &changes.lines {
        eprintln!("kiln.lock: {line}");
    }
    let dirs = resolve::verify(root, manifest, &new, mode.offline)?;
    new.save(root)?;
    finish(manifest, &new, dirs).map(Some)
}

fn finish(
    manifest: &Manifest,
    lock: &Lock,
    dirs: BTreeMap<WareName, PathBuf>,
) -> Result<Graph, String> {
    if let Some(policy) = &manifest.policy {
        policy::check(policy, lock)?;
    }
    let mut graph = Graph {
        root: lock.root.clone(),
        wares: BTreeMap::new(),
    };
    for (name, w) in &lock.wares {
        let dir = dirs[name].clone();
        policy::check_declared(name, &w.capabilities, &dir)?;
        graph.wares.insert(
            name.clone(),
            GraphWare {
                dir,
                deps: w.dependencies.clone(),
                cached: !matches!(w.source, lock::Source::Path(_)),
            },
        );
    }
    Ok(graph)
}

/// The test module, a Ware that comes with kiln and is available to
/// `import test` in tests.
pub fn builtin_test() -> Result<(WareName, PathBuf), String> {
    let mut files = archive::Files::new();
    files.insert(
        "kiln.toml".into(),
        include_str!("../../wares/test/kiln.toml").into(),
    );
    files.insert(
        "test.kiln".into(),
        include_str!("../../wares/test/test.kiln").into(),
    );
    files.insert(
        "browser.kiln".into(),
        include_str!("../../wares/test/browser.kiln").into(),
    );
    let text = archive::encode(&files);
    let checksum = archive::sha256(text.as_bytes());
    let dir = match cache::lookup(&checksum)? {
        Some(dir) => dir,
        None => cache::store(&text, &checksum)?,
    };
    Ok((WareName::parse("kiln/test").unwrap(), dir))
}
