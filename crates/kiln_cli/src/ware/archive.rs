//! A Ware's source as one canonical text: its files in byte order of their
//! paths, each with its length. The same tree always makes the same bytes,
//! so the archive's SHA-256 is the Ware's content hash, and a cached tree
//! is checked by packing it again.
//!
//! ```text
//! kiln-ware-archive 1
//! button.glaze
//! 120
//! <120 bytes>
//! ```
//!
//! Files are UTF-8 text: Kiln, Glazes, Embers, CSS, SVG, Markdown, TOML.

use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

pub const HEADER: &str = "kiln-ware-archive 1\n";
pub const MAX_ARCHIVE: usize = 16 * 1024 * 1024;
pub const MAX_FILES: usize = 4096;
pub const MAX_PATH: usize = 240;

/// Files a Ware never publishes: secrets, databases, lockfiles and build
/// output. Hidden files (`.env`, `.git`) are left out too.
const EXCLUDED_NAMES: &[&str] = &["kiln.lock", "target", "node_modules"];
const EXCLUDED_EXTENSIONS: &[&str] =
    &["db", "sqlite", "sqlite3", "pem", "key", "p12", "pfx", "exe"];

pub type Files = BTreeMap<String, String>;

/// Whether a file at this relative path is part of a published Ware.
pub fn published(path: &str) -> bool {
    path.split('/').all(|part| {
        let ext = part.rsplit_once('.').map_or("", |(_, e)| e);
        !part.starts_with('.')
            && !EXCLUDED_NAMES.contains(&part)
            && !EXCLUDED_EXTENSIONS.contains(&ext)
    })
}

/// `sha256:<hex>` of some bytes.
pub fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!("sha256:{}", hex(&digest))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn encode(files: &Files) -> String {
    let mut out = String::from(HEADER);
    for (path, content) in files {
        out.push_str(path);
        out.push('\n');
        out.push_str(&content.len().to_string());
        out.push('\n');
        out.push_str(content);
        out.push('\n');
    }
    out
}

/// Reads an archive, refusing anything but the canonical form: paths in
/// order, each once, each safe to write under a folder on any system.
pub fn decode(archive: &str) -> Result<Files, String> {
    if archive.len() > MAX_ARCHIVE {
        return Err(format!("the archive is larger than {MAX_ARCHIVE} bytes"));
    }
    let mut rest = archive
        .strip_prefix(HEADER)
        .ok_or("not a Kiln Ware archive")?;
    let mut files = Files::new();
    let mut folded = BTreeSet::new();
    let mut last: Option<String> = None;
    while !rest.is_empty() {
        let (path, after) = rest.split_once('\n').ok_or("truncated archive")?;
        let (len, after) = after.split_once('\n').ok_or("truncated archive")?;
        let len: usize = len.parse().map_err(|_| format!("bad length for {path}"))?;
        if after.len() < len + 1 || !after.is_char_boundary(len) || &after[len..len + 1] != "\n" {
            return Err(format!("truncated archive at {path}"));
        }
        check_path(path)?;
        if last.as_deref().is_some_and(|l| l >= path) {
            return Err(format!("archive paths are out of order at {path}"));
        }
        if !folded.insert(path.to_lowercase()) {
            return Err(format!("{path} differs from another path only in case"));
        }
        if files.len() >= MAX_FILES {
            return Err(format!("more than {MAX_FILES} files"));
        }
        files.insert(path.to_string(), after[..len].to_string());
        last = Some(path.to_string());
        rest = &after[len + 1..];
    }
    Ok(files)
}

/// A path inside a Ware: relative, `/`-separated, with no part that could
/// leave the folder, hide, or mean something special on Windows.
pub fn check_path(path: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("unsafe path `{path}` in a Ware: {why}"));
    if path.is_empty() || path.len() > MAX_PATH {
        return bad("empty or too long");
    }
    if path.contains(['\\', ':', '\0', '\r', '\n', '*', '?', '"', '<', '>', '|']) {
        return bad("it contains a character paths cannot use everywhere");
    }
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() > 16 {
        return bad("it is nested too deeply");
    }
    for part in parts {
        if part.is_empty() || part == "." || part == ".." {
            return bad("empty, `.` or `..` part");
        }
        if part.starts_with('.') {
            return bad("hidden files are not published");
        }
        if part.ends_with(' ') || part.ends_with('.') {
            return bad("it ends with a space or a dot");
        }
        let stem = part.split('.').next().unwrap_or("").to_ascii_uppercase();
        if [
            "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "LPT1", "LPT2",
        ]
        .contains(&stem.as_str())
        {
            return bad("it is a reserved name on Windows");
        }
    }
    Ok(())
}

/// The files of a Ware's folder that are published. `strict` packs every
/// file, as a cached tree must hold exactly its archive; otherwise secrets,
/// databases and hidden files are left out.
pub fn read_tree(dir: &Path, strict: bool) -> Result<Files, String> {
    fn walk(
        dir: &Path,
        prefix: &str,
        strict: bool,
        out: &mut Files,
        total: &mut usize,
    ) -> Result<(), String> {
        let mut entries = std::fs::read_dir(dir)
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let name = entry.file_name().to_string_lossy().into_owned();
            let path = entry.path();
            let file_type = entry.file_type().map_err(|e| e.to_string())?;
            if !strict && !published(&name) {
                continue;
            }
            if file_type.is_symlink() {
                return Err(format!(
                    "{}: Wares cannot contain symbolic links",
                    path.display()
                ));
            }
            let rel = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if file_type.is_dir() {
                walk(&path, &rel, strict, out, total)?;
            } else if file_type.is_file() {
                check_path(&rel)?;
                let bytes = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                *total += bytes.len();
                if *total > MAX_ARCHIVE {
                    return Err(format!("the Ware is larger than {MAX_ARCHIVE} bytes"));
                }
                let text = String::from_utf8(bytes)
                    .map_err(|_| format!("{}: Ware files must be UTF-8 text", path.display()))?;
                out.insert(rel, text);
                if out.len() > MAX_FILES {
                    return Err(format!("a Ware has at most {MAX_FILES} files"));
                }
            }
        }
        Ok(())
    }
    let mut files = Files::new();
    let mut total = 0;
    walk(dir, "", strict, &mut files, &mut total)?;
    let mut folded = BTreeSet::new();
    for path in files.keys() {
        if !folded.insert(path.to_lowercase()) {
            return Err(format!("{path} differs from another path only in case"));
        }
    }
    Ok(files)
}

/// Writes the files under `dir`, which must be empty. Only new files are
/// created, so nothing outside `dir` can be reached or overwritten.
pub fn write_tree(dir: &Path, files: &Files) -> Result<(), String> {
    use std::io::Write;
    for (path, content) in files {
        check_path(path)?;
        let target = dir.join(path);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("cannot write {}: {e}", parent.display()))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        file.write_all(content.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_rejections() {
        let mut files = Files::new();
        files.insert("a.kiln".into(), "pub fn a() {}\n".into());
        files.insert("forms/b.glaze".into(), "<p>é</p>".into());
        let text = encode(&files);
        assert_eq!(decode(&text).unwrap(), files);
        assert_eq!(
            sha256(text.as_bytes()),
            sha256(encode(&decode(&text).unwrap()).as_bytes())
        );
        for bad in [
            "../x", "/x", "a//b", ".env", "a\\b", "C:x", "nul.txt", "a/./b",
        ] {
            assert!(check_path(bad).is_err(), "{bad}");
        }
        let swapped = format!("{HEADER}b\n1\nx\na\n1\ny\n");
        assert!(decode(&swapped).is_err());
        let traversal = format!("{HEADER}../evil\n1\nx\n");
        assert!(decode(&traversal).is_err());
        let short = format!("{HEADER}a\n10\nx\n");
        assert!(decode(&short).is_err());
        let dup = format!("{HEADER}A\n1\nx\na\n1\ny\n");
        assert!(decode(&dup).is_err());
    }
}
