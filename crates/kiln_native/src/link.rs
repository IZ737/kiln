//! Links a generated object with the Linux runtime into a static
//! executable, using the wild linker inside `kiln` rather than a system
//! linker.

use std::path::Path;

/// Links the object that [`crate::compile`] made into an executable at
/// `out`.
pub(crate) fn link(object: &[u8], out: &Path) -> Result<(), String> {
    // The linker reads files, so the runtime is written out next to the
    // object for the length of the link.
    let dir = scratch_dir()?;
    let result = link_in(&dir, object, out);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn link_in(dir: &Path, object: &[u8], out: &Path) -> Result<(), String> {
    let write = |name: &str, bytes: &[u8]| {
        let path = dir.join(name);
        std::fs::write(&path, bytes)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok::<_, String>(path.display().to_string())
    };
    let mut args: Vec<String> = [
        "wild",
        "-static",
        "--gc-sections",
        "-z",
        "noexecstack",
        "--strip-debug",
        "-o",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.push(out.display().to_string());
    for &(name, bytes) in crate::runtime::LINUX_FILES {
        args.push(write(name, bytes)?);
        if name == "crti.o" {
            args.push(write("program.o", object)?);
        }
    }
    let failed = |e: libwild::error::Error| format!("the linker failed: {}", e.to_string());
    let mut parsed = libwild::Args::new(|| args.iter()).map_err(failed)?;
    parsed.parse(|| args.iter()).map_err(failed)?;
    libwild::run(parsed).map_err(failed)
}

/// A fresh directory for build files.
fn scratch_dir() -> Result<std::path::PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("kiln-link-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}
