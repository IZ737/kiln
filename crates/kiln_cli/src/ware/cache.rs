//! The per-user Ware cache: each Ware's source once, in a folder named by
//! its content hash, shared by every project on the machine. Nothing is
//! copied into projects. A tree is written to a staging folder, checked
//! against its hash, then moved into place in one rename, so another
//! process sees all of it or none of it.

use super::archive;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

/// Trees this process has already checked.
static VERIFIED: Mutex<BTreeSet<PathBuf>> = Mutex::new(BTreeSet::new());

/// `KILN_CACHE_DIR`, or the system's cache folder for the user: on Linux
/// `$XDG_CACHE_HOME/kiln` or `~/.cache/kiln`, on Windows
/// `%LOCALAPPDATA%\kiln\cache`, on macOS `~/Library/Caches/kiln`.
pub fn dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KILN_CACHE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if cfg!(windows) {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return PathBuf::from(local).join("kiln").join("cache");
        }
    }
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join("Library/Caches/kiln");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CACHE_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(xdg).join("kiln");
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".cache").join("kiln")
}

/// Whether the cache may only be read, as on a CI machine whose cache was
/// filled earlier (`KILN_CACHE_READONLY=1`).
pub fn read_only() -> bool {
    std::env::var("KILN_CACHE_READONLY").is_ok_and(|v| v == "1")
}

fn hex_of(checksum: &str) -> Result<&str, String> {
    let hex = checksum
        .strip_prefix("sha256:")
        .filter(|h| {
            h.len() == 64
                && h.chars()
                    .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        })
        .ok_or_else(|| format!("`{checksum}` is not a sha256 checksum"))?;
    Ok(hex)
}

pub fn ware_dir(checksum: &str) -> Result<PathBuf, String> {
    Ok(dir().join("wares").join(hex_of(checksum)?))
}

/// The cached tree with this checksum, checked once per process. A tree
/// that no longer matches is moved aside so it is fetched again.
pub fn lookup(checksum: &str) -> Result<Option<PathBuf>, String> {
    let path = ware_dir(checksum)?;
    if !path.is_dir() {
        return Ok(None);
    }
    if VERIFIED.lock().unwrap().contains(&path) {
        touch(&path);
        return Ok(Some(path));
    }
    let actual = archive::read_tree(&path, true)
        .map(|files| archive::sha256(archive::encode(&files).as_bytes()));
    if actual.as_deref() == Ok(checksum) {
        VERIFIED.lock().unwrap().insert(path.clone());
        touch(&path);
        return Ok(Some(path));
    }
    if read_only() {
        return Err(format!(
            "the cached Ware {} is damaged, and the cache is read-only",
            path.display()
        ));
    }
    let aside = staging("damaged")?;
    std::fs::rename(&path, &aside)
        .map_err(|e| format!("cannot move aside {}: {e}", path.display()))?;
    eprintln!(
        "warning: the cached Ware {} did not match its checksum; it will be fetched again",
        path.display()
    );
    let _ = std::fs::remove_dir_all(&aside);
    Ok(None)
}

/// Records that a tree was used, for `kiln cache prune`.
fn touch(path: &Path) {
    if read_only() {
        return;
    }
    let mut marker = path.as_os_str().to_owned();
    marker.push(".used");
    let _ = std::fs::write(PathBuf::from(marker), b"");
}

fn staging(what: &str) -> Result<PathBuf, String> {
    let tmp = dir().join("tmp");
    std::fs::create_dir_all(&tmp).map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Ok(tmp.join(format!("{what}-{}-{nanos}", std::process::id())))
}

/// Stores an archive's tree under its checksum, after checking that the
/// archive is what `expected` says. Returns the tree's folder.
pub fn store(archive_text: &str, expected: &str) -> Result<PathBuf, String> {
    let actual = archive::sha256(archive_text.as_bytes());
    if actual != expected {
        return Err(format!(
            "checksum mismatch: expected {expected}, the download is {actual}"
        ));
    }
    if let Some(path) = lookup(expected)? {
        return Ok(path);
    }
    if read_only() {
        return Err("the Ware cache is read-only (KILN_CACHE_READONLY=1)".into());
    }
    let files = archive::decode(archive_text)?;
    let stage = staging("stage")?;
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    let result = (|| {
        archive::write_tree(&stage, &files)?;
        let written =
            archive::sha256(archive::encode(&archive::read_tree(&stage, true)?).as_bytes());
        if written != expected {
            return Err(format!("the unpacked Ware does not match {expected}"));
        }
        let target = ware_dir(expected)?;
        std::fs::create_dir_all(target.parent().unwrap()).map_err(|e| e.to_string())?;
        match std::fs::rename(&stage, &target) {
            Ok(()) => Ok(target),
            // Another process stored it first: theirs is the same tree.
            Err(_) if target.is_dir() => Ok(target),
            Err(e) => Err(format!("cannot store {}: {e}", target.display())),
        }
    })();
    let _ = std::fs::remove_dir_all(&stage);
    let target = result?;
    VERIFIED.lock().unwrap().remove(&target);
    lookup(expected)?.ok_or_else(|| format!("cannot store the Ware {expected}"))
}

pub struct Entry {
    pub path: PathBuf,
    pub bytes: u64,
    pub last_used: Option<SystemTime>,
}

pub fn entries() -> Vec<Entry> {
    let wares = dir().join("wares");
    let mut out = Vec::new();
    let Ok(read) = std::fs::read_dir(&wares) else {
        return out;
    };
    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let mut marker = path.as_os_str().to_owned();
        marker.push(".used");
        let last_used = std::fs::metadata(PathBuf::from(marker))
            .and_then(|m| m.modified())
            .ok()
            .or_else(|| std::fs::metadata(&path).and_then(|m| m.modified()).ok());
        out.push(Entry {
            bytes: size(&path),
            path,
            last_used,
        });
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

pub fn size(path: &Path) -> u64 {
    let Ok(meta) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if meta.is_dir() {
        std::fs::read_dir(path)
            .map(|r| r.flatten().map(|e| size(&e.path())).sum())
            .unwrap_or(0)
    } else {
        meta.len()
    }
}

/// Checks every cached tree against the hash it is stored under, and
/// returns the damaged ones.
pub fn verify_all() -> Vec<(PathBuf, String)> {
    let mut bad = Vec::new();
    for entry in entries() {
        let name = entry
            .path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let expected = format!("sha256:{name}");
        match archive::read_tree(&entry.path, true) {
            Ok(files) => {
                let actual = archive::sha256(archive::encode(&files).as_bytes());
                if actual != expected {
                    bad.push((entry.path, format!("contents hash to {actual}")));
                }
            }
            Err(e) => bad.push((entry.path, e)),
        }
    }
    bad
}

/// Trees not used for `older_than`, and staging folders an interrupted
/// process left behind over an hour ago. A tree used within the last hour
/// is never removed, so a build in progress keeps what it reads.
pub fn prune(older_than: Duration, dry_run: bool) -> Result<Vec<(PathBuf, u64)>, String> {
    let now = SystemTime::now();
    let older_than = older_than.max(Duration::from_secs(3600));
    let mut removed = Vec::new();
    for entry in entries() {
        let age = entry
            .last_used
            .and_then(|t| now.duration_since(t).ok())
            .unwrap_or_default();
        if age >= older_than {
            removed.push((entry.path, entry.bytes));
        }
    }
    if let Ok(read) = std::fs::read_dir(dir().join("tmp")) {
        for e in read.flatten() {
            let old = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| now.duration_since(t).ok())
                .is_some_and(|age| age > Duration::from_secs(3600));
            if old {
                removed.push((e.path(), size(&e.path())));
            }
        }
    }
    if !dry_run {
        if read_only() {
            return Err("the Ware cache is read-only (KILN_CACHE_READONLY=1)".into());
        }
        for (path, _) in &removed {
            if path.is_dir() {
                std::fs::remove_dir_all(path)
                    .map_err(|e| format!("cannot remove {}: {e}", path.display()))?;
            } else {
                let _ = std::fs::remove_file(path);
            }
            let mut marker = path.as_os_str().to_owned();
            marker.push(".used");
            let _ = std::fs::remove_file(PathBuf::from(marker));
        }
    }
    Ok(removed)
}
