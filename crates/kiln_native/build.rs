//! Builds the runtime that native executables link with: the Kiln runtime,
//! the Boehm garbage collector (vendored in `vendor/bdwgc`) and the musl C
//! library, all for Linux x86-64. The `kiln` binary carries the results,
//! so the people who use it need no C compiler or libraries.
//!
//! Building them needs a musl C compiler: `musl-gcc` (the `musl-tools`
//! package on Debian and Ubuntu), or the one `KILN_MUSL_CC` names, and the
//! directory with musl's `libc.a` (found next to the usual install
//! locations, or named by `KILN_MUSL_LIB`). Without them `kiln` still
//! builds, and the native backend says what is missing.

use std::path::{Path, PathBuf};
use std::process::Command;

const MUSL_LIB_DIRS: &[&str] = &[
    "/usr/lib/x86_64-linux-musl",
    "/usr/lib/musl/lib",
    "/usr/local/musl/lib",
    "/usr/x86_64-linux-musl/lib",
    "/usr/lib/musl/x86_64-linux-musl/lib",
];

/// The files the runtime is made of, in the order they are linked.
const FILES: &[&str] = &["crt1.o", "crti.o", "native.o", "gc.o", "libc.a", "crtn.o"];

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let shared = manifest.join("../kiln_codegen/runtime");
    for path in [
        manifest.join("runtime/native.c"),
        shared.join("kiln.c"),
        manifest.join("vendor/bdwgc"),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    println!("cargo:rerun-if-env-changed=KILN_MUSL_CC");
    println!("cargo:rerun-if-env-changed=KILN_MUSL_LIB");
    println!("cargo::rustc-check-cfg=cfg(kiln_runtime)");

    let missing = |why: String| {
        println!("cargo:warning=the native backend is not available: {why}");
        std::fs::write(out.join("missing.txt"), why).unwrap();
    };
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.starts_with("x86_64-") || !target.contains("linux") {
        return missing(format!(
            "it builds Linux x86-64 programs, and kiln was built for {target}"
        ));
    }
    let cc = std::env::var("KILN_MUSL_CC").unwrap_or_else(|_| "musl-gcc".into());
    let lib = match std::env::var("KILN_MUSL_LIB") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => match MUSL_LIB_DIRS
            .iter()
            .map(PathBuf::from)
            .find(|d| d.join("libc.a").is_file())
        {
            Some(dir) => dir,
            None => {
                return missing(
                    "kiln was built without the musl C library; install `musl-tools` (or set KILN_MUSL_LIB) and rebuild kiln".into(),
                )
            }
        },
    };
    let gc = manifest.join("vendor/bdwgc");
    let flags = [
        "-std=gnu11",
        "-O2",
        "-w",
        "-fno-pie",
        "-ffunction-sections",
        "-fdata-sections",
        "-fno-stack-protector",
        "-fno-asynchronous-unwind-tables",
    ];
    let compile = |source: &Path, object: &str, extra: &[&str]| -> Result<(), String> {
        let status = Command::new(&cc)
            .args(flags)
            .args(extra)
            .arg(format!("-I{}", gc.join("include").display()))
            .arg(format!("-I{}", shared.display()))
            .arg("-c")
            .arg(source)
            .arg("-o")
            .arg(out.join(object))
            .status()
            .map_err(|e| format!("cannot run the musl C compiler `{cc}` ({e}); install `musl-tools` (or set KILN_MUSL_CC) and rebuild kiln"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("`{cc}` could not compile {}", source.display()))
        }
    };
    let built = compile(&manifest.join("runtime/native.c"), "native.o", &[]).and_then(|()| {
        compile(
            &gc.join("extra/gc.c"),
            "gc.o",
            &[
                "-DALL_INTERIOR_POINTERS",
                "-DNO_EXECUTE_PERMISSION",
                "-DNO_GETCONTEXT",
                "-DUSE_MMAP",
                "-DUSE_MMAP_ANON",
            ],
        )
    });
    if let Err(why) = built {
        return missing(why);
    }
    for file in ["crt1.o", "crti.o", "crtn.o", "libc.a"] {
        if let Err(e) = std::fs::copy(lib.join(file), out.join(file)) {
            return missing(format!("cannot copy {}: {e}", lib.join(file).display()));
        }
    }
    let _ = std::fs::remove_file(out.join("missing.txt"));
    assert!(FILES.iter().all(|f| out.join(f).is_file()));
    println!("cargo:rustc-cfg=kiln_runtime");
}
