//! Builds `kiln_rt` as a static library for one target. The build scripts
//! of `kiln_native` and `kiln_codegen` include this file with `#[path]`.
//!
//! It runs a second cargo, with a target directory of its own so that it
//! does not wait for the lock the outer build holds, and the `runtime`
//! profile from the workspace's Cargo.toml.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Builds the library for `target` (a Rust target triple) under `out`, and
/// returns the path of `libkiln_rt.a`. The error says what went wrong.
pub fn build(target: &str, out: &Path) -> Result<PathBuf, String> {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace = manifest.join("../..");
    let rt = workspace.join("crates/kiln_rt");
    println!("cargo:rerun-if-changed={}", rt.display());
    println!(
        "cargo:rerun-if-changed={}",
        workspace.join("Cargo.lock").display()
    );
    let dir = out.join("kiln_rt-build");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(&cargo);
    cmd.args(["build", "--locked", "-p", "kiln_rt", "--profile", "runtime"])
        .arg("--manifest-path")
        .arg(workspace.join("Cargo.toml"))
        .args(["--target", target])
        .arg("--target-dir")
        .arg(&dir);
    // The outer build's settings are for `kiln`, not for the runtime: its
    // flags, its target and its wrappers (clippy's, for one).
    for var in [
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTFLAGS",
        "CARGO_BUILD_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET_DIR",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
    ] {
        cmd.env_remove(var);
    }
    let output = cmd
        .output()
        .map_err(|e| format!("cannot run `{cargo}` to build the runtime: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("target may not be installed")
            || stderr.contains("can't find crate for `std`")
        {
            return Err(format!(
                "Rust's standard library for {target} is not installed; run `rustup target add {target}` and rebuild kiln"
            ));
        }
        let tail: Vec<&str> = stderr.lines().rev().take(20).collect();
        let tail: Vec<&str> = tail.into_iter().rev().collect();
        return Err(format!(
            "cannot build the runtime library for {target}:\n{}",
            tail.join("\n")
        ));
    }
    let lib = dir.join(target).join("runtime/libkiln_rt.a");
    if lib.is_file() {
        Ok(lib)
    } else {
        Err(format!("building the runtime made no {}", lib.display()))
    }
}

/// A file in the library directory of Rust's standard library for
/// `target`, such as the `libunwind.a` that Rust ships for musl.
pub fn sysroot_lib(target: &str, file: &str) -> Result<PathBuf, String> {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let output = Command::new(&rustc)
        .args(["--print", "sysroot"])
        .output()
        .map_err(|e| format!("cannot run `{rustc}`: {e}"))?;
    let sysroot = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let path = Path::new(&sysroot)
        .join("lib/rustlib")
        .join(target)
        .join("lib")
        .join(file);
    if path.is_file() {
        Ok(path)
    } else {
        Err(format!(
            "Rust's standard library for {target} is not installed; run `rustup target add {target}` and rebuild kiln"
        ))
    }
}

/// Links `libs` (the library from `build` first, then any it needs) into
/// one relocatable object at `out`, with the C compiler `cc` and the `nm`
/// of the same binutils: the members
/// the runtime's `k_*` functions need, without debug information. The
/// library holds all of Rust's compiler builtins and is several megabytes;
/// the object is a fraction of that, which keeps `kiln`, which carries it,
/// small. (`--gc-sections` would drop more, but GNU ld's relocatable output
/// with it has local undefined symbols, which linkers reject.)
pub fn prelink(cc: &str, nm: &str, libs: &[PathBuf], out: &Path) -> Result<(), String> {
    let listed = Command::new(nm)
        .args(["--defined-only", "-g", "--format=posix"])
        .arg(&libs[0])
        .output()
        .map_err(|e| format!("cannot run `{nm}` (from binutils): {e}"))?;
    let listing = String::from_utf8_lossy(&listed.stdout);
    let exports: Vec<&str> = listing
        .lines()
        .filter_map(|line| line.split(' ').next())
        .filter(|name| name.starts_with("k_"))
        .collect();
    if !listed.status.success() || exports.is_empty() {
        return Err(format!(
            "`{nm}` found no k_ functions in {}",
            libs[0].display()
        ));
    }
    let status = Command::new(cc)
        .args(["-r", "-nostdlib", "-Wl,--strip-debug"])
        .args(exports.iter().map(|name| format!("-Wl,-u,{name}")))
        .args(libs)
        .arg("-o")
        .arg(out)
        .status()
        .map_err(|e| format!("cannot run `{cc}`: {e}"))?;
    if status.success() && out.is_file() {
        Ok(())
    } else {
        Err(format!(
            "`{cc}` could not link the runtime library into one object"
        ))
    }
}

/// Copies `lib` to `out` without debug information, with binutils'
/// `strip`. For where `prelink` cannot work: mingw's linker cannot make a
/// relocatable object of a library that uses thread-local storage.
pub fn strip_debug(strip: &str, lib: &Path, out: &Path) -> Result<(), String> {
    std::fs::copy(lib, out).map_err(|e| format!("cannot copy {}: {e}", lib.display()))?;
    let status = Command::new(strip)
        .arg("--strip-debug")
        .arg(out)
        .status()
        .map_err(|e| format!("cannot run `{strip}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{strip}` could not strip {}", out.display()))
    }
}
