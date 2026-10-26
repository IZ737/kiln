//! Git Wares: a bare mirror of each repository in the cache, from which a
//! commit's files are read without checking anything out. A branch or tag
//! is resolved to a commit once and the lockfile pins the commit.
//! Cloning runs no hooks, and only ordinary transports are allowed.

use super::archive::{self, Files};
use super::manifest::GitRef;
use std::path::PathBuf;
use std::process::Command;

fn mirror(url: &str) -> PathBuf {
    let hash = archive::sha256(url.as_bytes());
    super::cache::dir()
        .join("git")
        .join(&hash["sha256:".len().."sha256:".len() + 24])
}

fn git() -> Command {
    let mut c = Command::new("git");
    c.args([
        "-c",
        "protocol.ext.allow=never",
        "-c",
        "core.hooksPath=/dev/null",
        "-c",
        "core.autocrlf=false",
    ]);
    c.env("GIT_TERMINAL_PROMPT", "0");
    c
}

fn run(cmd: &mut Command, what: &str) -> Result<Vec<u8>, String> {
    let out = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Git Wares need `git`, which is not installed".to_string()
        } else {
            format!("cannot run git: {e}")
        }
    })?;
    if !out.status.success() {
        return Err(format!(
            "{what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn fetch(url: &str, offline: bool) -> Result<PathBuf, String> {
    let dir = mirror(url);
    if offline {
        return Err(format!("{url} is not available offline"));
    }
    if super::cache::read_only() {
        return Err("the Ware cache is read-only (KILN_CACHE_READONLY=1)".into());
    }
    if !dir.join("HEAD").is_file() {
        std::fs::create_dir_all(dir.parent().unwrap()).map_err(|e| e.to_string())?;
        let tmp = dir.with_extension(format!("tmp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        run(
            git()
                .args(["clone", "--bare", "--quiet", "--", url])
                .arg(&tmp),
            &format!("cannot clone {url}"),
        )?;
        if std::fs::rename(&tmp, &dir).is_err() {
            let _ = std::fs::remove_dir_all(&tmp);
        }
    }
    run(
        git().arg("-C").arg(&dir).args([
            "fetch",
            "--quiet",
            "--force",
            "--prune",
            "origin",
            "+refs/heads/*:refs/heads/*",
            "+refs/tags/*:refs/tags/*",
        ]),
        &format!("cannot fetch {url}"),
    )?;
    Ok(dir)
}

fn has_commit(url: &str, rev: &str) -> bool {
    let dir = mirror(url);
    dir.join("HEAD").is_file()
        && git()
            .arg("-C")
            .arg(&dir)
            .args(["cat-file", "-e", &format!("{rev}^{{commit}}")])
            .output()
            .is_ok_and(|o| o.status.success())
}

/// The commit a reference names now.
pub fn resolve(url: &str, reference: &GitRef, offline: bool) -> Result<String, String> {
    if let GitRef::Rev(rev) = reference {
        if !has_commit(url, rev) {
            fetch(url, offline)?;
        }
        if !has_commit(url, rev) {
            return Err(format!("{url} has no commit {rev}"));
        }
        return Ok(rev.clone());
    }
    let dir = fetch(url, offline)?;
    let name = match reference {
        GitRef::Branch(b) => format!("refs/heads/{b}^{{commit}}"),
        GitRef::Tag(t) => format!("refs/tags/{t}^{{commit}}"),
        _ => "HEAD^{commit}".into(),
    };
    let out = run(
        git()
            .arg("-C")
            .arg(&dir)
            .args(["rev-parse", "--verify", "--quiet", &name]),
        &format!("{url} has no {}", name.trim_end_matches("^{commit}")),
    )?;
    Ok(String::from_utf8_lossy(&out).trim().to_string())
}

/// The published files of a commit, as a Ware archive would hold them.
pub fn files(url: &str, rev: &str, offline: bool) -> Result<Files, String> {
    if !has_commit(url, rev) {
        fetch(url, offline)?;
    }
    let dir = mirror(url);
    let listing = run(
        git()
            .arg("-C")
            .arg(&dir)
            .args(["ls-tree", "-r", "-z", "--full-tree", rev]),
        &format!("cannot read {rev} of {url}"),
    )?;
    let mut files = Files::new();
    for entry in listing.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let entry = String::from_utf8_lossy(entry);
        let (meta, path) = entry.split_once('\t').ok_or("unexpected git output")?;
        let mut parts = meta.split(' ');
        let (mode, kind, object) = (parts.next(), parts.next(), parts.next());
        if !archive::published(path) {
            continue;
        }
        if mode == Some("120000") || kind != Some("blob") {
            return Err(format!(
                "{path} in {url} is a symbolic link or submodule, which Wares cannot contain"
            ));
        }
        archive::check_path(path)?;
        let blob = run(
            git()
                .arg("-C")
                .arg(&dir)
                .args(["cat-file", "blob", object.unwrap_or("")]),
            &format!("cannot read {path}"),
        )?;
        let text = String::from_utf8(blob)
            .map_err(|_| format!("{path}: Ware files must be UTF-8 text"))?;
        files.insert(path.to_string(), text);
    }
    Ok(files)
}
