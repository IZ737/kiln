//! Builds the runtimes that native executables are made from, one for each
//! target: the Kiln runtime and the Boehm garbage collector (vendored in
//! `vendor/bdwgc`) with a C library. The `kiln` binary carries the results,
//! so the people who use it need no C compiler or libraries.
//!
//! - Linux x86-64 needs a musl C compiler: `musl-gcc` (the `musl-tools`
//!   package on Debian and Ubuntu), or the one `KILN_MUSL_CC` names, and the
//!   directory with musl's `libc.a` (found next to the usual install
//!   locations, or named by `KILN_MUSL_LIB`). The runtime is a set of
//!   objects that the built-in linker links with each program.
//! - Windows x86-64 needs mingw-w64: `x86_64-w64-mingw32-gcc` and its
//!   `windres`, or the compiler `KILN_MINGW_CC` names. The runtime is an
//!   executable, linked here, that `kiln` adds each program to (see
//!   `src/pe.rs`), with a list of where its symbols are.
//!
//! Without the tools for a target, `kiln` still builds, and the native
//! backend says what is missing when asked to build for that target.

use std::path::{Path, PathBuf};
use std::process::Command;

const MUSL_LIB_DIRS: &[&str] = &[
    "/usr/lib/x86_64-linux-musl",
    "/usr/lib/musl/lib",
    "/usr/local/musl/lib",
    "/usr/x86_64-linux-musl/lib",
    "/usr/lib/musl/x86_64-linux-musl/lib",
];

/// The files the Linux runtime is made of, in the order they are linked.
const LINUX_FILES: &[&str] = &["crt1.o", "crti.o", "native.o", "gc.o", "libc.a", "crtn.o"];

struct Paths {
    manifest: PathBuf,
    out: PathBuf,
    shared: PathBuf,
    gc: PathBuf,
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let paths = Paths {
        out: PathBuf::from(std::env::var("OUT_DIR").unwrap()),
        shared: manifest.join("../kiln_codegen/runtime"),
        gc: manifest.join("vendor/bdwgc"),
        manifest,
    };
    for path in [
        paths.manifest.join("runtime"),
        paths.shared.join("kiln.c"),
        paths.gc.clone(),
    ] {
        println!("cargo:rerun-if-changed={}", path.display());
    }
    for var in ["KILN_MUSL_CC", "KILN_MUSL_LIB", "KILN_MINGW_CC"] {
        println!("cargo:rerun-if-env-changed={var}");
    }
    println!("cargo::rustc-check-cfg=cfg(kiln_runtime_linux)");
    println!("cargo::rustc-check-cfg=cfg(kiln_runtime_windows)");

    for (target, built) in [("linux", linux(&paths)), ("windows", windows(&paths))] {
        let missing = paths.out.join(format!("missing-{target}.txt"));
        match built {
            Ok(()) => {
                let _ = std::fs::remove_file(missing);
                println!("cargo:rustc-cfg=kiln_runtime_{target}");
            }
            Err(why) => {
                println!("cargo:warning=kiln cannot build {target} executables: {why}");
                std::fs::write(missing, why).unwrap();
            }
        }
    }
}

/// Runs a C compiler (or another tool) and says what went wrong if it fails.
fn run(cmd: &mut Command, tool: &str, install: &str, what: &str) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("cannot run `{tool}` ({e}); install {install} and rebuild kiln"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{tool}` could not {what}"))
    }
}

fn linux(p: &Paths) -> Result<(), String> {
    let cc = std::env::var("KILN_MUSL_CC").unwrap_or_else(|_| "musl-gcc".into());
    let lib = match std::env::var("KILN_MUSL_LIB") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => MUSL_LIB_DIRS
            .iter()
            .map(PathBuf::from)
            .find(|d| d.join("libc.a").is_file())
            .ok_or("kiln was built without the musl C library; install `musl-tools` (or set KILN_MUSL_LIB) and rebuild kiln")?,
    };
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
    let compile = |source: &Path, object: &str, extra: &[&str]| {
        run(
            Command::new(&cc)
                .args(flags)
                .args(extra)
                .arg(format!("-I{}", p.gc.join("include").display()))
                .arg(format!("-I{}", p.shared.display()))
                .arg("-c")
                .arg(source)
                .arg("-o")
                .arg(p.out.join(object)),
            &cc,
            "`musl-tools` (or set KILN_MUSL_CC)",
            &format!("compile {}", source.display()),
        )
    };
    compile(&p.manifest.join("runtime/native.c"), "native.o", &[])?;
    compile(
        &p.gc.join("extra/gc.c"),
        "gc.o",
        &[
            "-DALL_INTERIOR_POINTERS",
            "-DNO_EXECUTE_PERMISSION",
            "-DNO_GETCONTEXT",
            "-DUSE_MMAP",
            "-DUSE_MMAP_ANON",
        ],
    )?;
    for file in ["crt1.o", "crti.o", "crtn.o", "libc.a"] {
        std::fs::copy(lib.join(file), p.out.join(file))
            .map_err(|e| format!("cannot copy {}: {e}", lib.join(file).display()))?;
    }
    assert!(LINUX_FILES.iter().all(|f| p.out.join(f).is_file()));
    Ok(())
}

fn windows(p: &Paths) -> Result<(), String> {
    let cc = std::env::var("KILN_MINGW_CC").unwrap_or_else(|_| "x86_64-w64-mingw32-gcc".into());
    let windres = match cc.strip_suffix("gcc") {
        Some(prefix) => format!("{prefix}windres"),
        None => "windres".into(),
    };
    let install = "mingw-w64 (or set KILN_MINGW_CC)";
    let dir = p.out.join("windows");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let compile = |source: &Path, object: &str, extra: &[&str]| {
        run(
            Command::new(&cc)
                .args(["-std=gnu11", "-O2", "-w"])
                .args(extra)
                .arg(format!("-I{}", p.gc.join("include").display()))
                .arg(format!("-I{}", p.shared.display()))
                .arg("-c")
                .arg(source)
                .arg("-o")
                .arg(dir.join(object)),
            &cc,
            install,
            &format!("compile {}", source.display()),
        )
    };
    compile(&p.manifest.join("runtime/native.c"), "native.o", &[])?;
    compile(
        &p.gc.join("extra/gc.c"),
        "gc.o",
        &["-DALL_INTERIOR_POINTERS", "-DNO_EXECUTE_PERMISSION"],
    )?;
    run(
        Command::new(&windres)
            .current_dir(p.manifest.join("runtime"))
            .arg("windows.rc")
            .arg("-O")
            .arg("coff")
            .arg("-o")
            .arg(dir.join("manifest.o")),
        &windres,
        install,
        "compile runtime/windows.rc",
    )?;
    // The program's code goes after the runtime's at addresses fixed when it
    // is added, so the image is never moved: no base relocations, no ASLR.
    let linked = dir.join("linked.exe");
    run(
        Command::new(&cc)
            .arg("-static")
            .args([
                "-Wl,--strip-debug",
                "-Wl,--disable-dynamicbase",
                "-Wl,--disable-high-entropy-va",
                "-Wl,--disable-reloc-section",
            ])
            .arg(dir.join("native.o"))
            .arg(dir.join("gc.o"))
            .arg(dir.join("manifest.o"))
            .arg("-lws2_32")
            .arg("-o")
            .arg(&linked),
        &cc,
        install,
        "link the Windows runtime",
    )?;
    let image = std::fs::read(&linked).map_err(|e| e.to_string())?;
    let (stripped, symbols) = strip_symbols(&image)?;
    std::fs::write(p.out.join("runtime.exe"), stripped).map_err(|e| e.to_string())?;
    std::fs::write(p.out.join("runtime.sym"), symbols).map_err(|e| e.to_string())?;
    Ok(())
}

/// Takes the COFF symbol table off the end of a linked executable. Returns
/// the executable without it and the external symbols, one `name rva` line
/// each, with the address relative to the image base in hex.
fn strip_symbols(image: &[u8]) -> Result<(Vec<u8>, String), String> {
    let u16_at = |o: usize| u16::from_le_bytes([image[o], image[o + 1]]) as usize;
    let u32_at = |o: usize| u32::from_le_bytes(image[o..o + 4].try_into().unwrap()) as usize;
    let pe = u32_at(0x3c);
    let coff = pe + 4;
    let sections = u16_at(coff + 2);
    let table = u32_at(coff + 8);
    let count = u32_at(coff + 12);
    if table == 0 {
        return Err("the Windows runtime has no symbol table".into());
    }
    let headers = coff + 20 + u16_at(coff + 16);
    let section_rva = |n: usize| u32_at(headers + 40 * (n - 1) + 12);
    let strings = table + 18 * count;
    let mut symbols = String::new();
    let mut i = 0;
    while i < count {
        let s = table + 18 * i;
        let name = if u32_at(s) == 0 {
            let start = strings + u32_at(s + 4);
            let end = start + image[start..].iter().position(|&b| b == 0).unwrap();
            &image[start..end]
        } else {
            let raw = &image[s..s + 8];
            &raw[..raw.iter().position(|&b| b == 0).unwrap_or(8)]
        };
        let section = u16_at(s + 12);
        let class = image[s + 16];
        // External symbols defined in a section (not absolute or debug).
        if class == 2 && section >= 1 && section <= sections {
            let rva = section_rva(section) + u32_at(s + 8);
            symbols.push_str(&format!("{} {rva:x}\n", String::from_utf8_lossy(name)));
        }
        i += 1 + image[s + 17] as usize;
    }
    let mut stripped = image[..table].to_vec();
    stripped[coff + 8..coff + 16].fill(0);
    Ok((stripped, symbols))
}
