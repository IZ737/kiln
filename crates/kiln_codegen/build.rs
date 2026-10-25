//! Builds `kiln_rt`, the runtime's Rust part, for the C backend: for the
//! system `kiln` runs on, with its usual C library, since that is what the
//! system's C compiler builds programs for. A `kiln` built against musl
//! still makes programs with the system's C compiler, so it carries the
//! library built for glibc.
//!
//! Without it, `kiln` still builds, and the C backend says why it cannot
//! build programs.

use std::path::PathBuf;

#[path = "../kiln_rt/staticlib.rs"]
#[allow(dead_code)]
mod kiln_rt;

fn main() {
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let target = std::env::var("TARGET")
        .unwrap()
        .replace("-linux-musl", "-linux-gnu");
    println!("cargo::rustc-check-cfg=cfg(kiln_rt)");
    let missing = out.join("missing-kiln_rt.txt");
    match kiln_rt::build(&target, &out) {
        Ok(lib) => {
            // One object with only what the runtime uses where the C
            // compiler can make it, otherwise the library without debug
            // information, otherwise the whole library. mingw's linker
            // cannot make one object of a library with thread-local storage.
            let windows = target.contains("windows");
            let strip = if windows {
                "x86_64-w64-mingw32-strip"
            } else {
                "strip"
            };
            let dest = out.join("kiln_rt");
            let _ = std::fs::remove_file(&dest);
            if windows || kiln_rt::prelink("cc", "nm", std::slice::from_ref(&lib), &dest).is_err() {
                if let Err(why) = kiln_rt::strip_debug(strip, &lib, &dest) {
                    println!(
                        "cargo:warning=kiln carries all of the C backend's runtime library: {why}"
                    );
                    std::fs::copy(&lib, &dest).unwrap();
                }
            }
            assert!(
                dest.is_file(),
                "no {} after building kiln_rt",
                dest.display()
            );
            let _ = std::fs::remove_file(missing);
            println!("cargo:rustc-cfg=kiln_rt");
        }
        Err(why) => {
            println!("cargo:warning=kiln's C backend cannot build programs: {why}");
            std::fs::write(missing, why).unwrap();
        }
    }
}
