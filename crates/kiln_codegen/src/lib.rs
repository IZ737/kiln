//! The C backend: turns a Kiln program in the IR (see `kiln_ir`) into one C
//! file, which a C compiler builds against the runtime (`runtime/kiln.c`,
//! and `kiln_rt`, its Rust part, which `kiln` carries) and the Boehm garbage
//! collector.
//!
//! ```no_run
//! let source = "fn main() {\n    print(\"hi\")\n}\n";
//! let parsed = kiln_syntax::parse(source);
//! let program = kiln_check::check_program(&parsed.module);
//! let ir = kiln_ir::lower(&parsed.module, &program).unwrap();
//! let c = kiln_codegen::emit_c(&ir).unwrap();
//! kiln_codegen::compile_c(&c, std::path::Path::new("hi")).unwrap();
//! ```

mod emit;

use kiln_diagnostics::Diagnostic;
use kiln_ir::Program;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The runtime, prepended to every generated file.
pub const RUNTIME: &str = include_str!("../runtime/kiln.c");

/// The runtime's Rust part, built by `build.rs` for this system: an object,
/// or a library where it could not be made into one. Either way the C
/// compiler hands it to the linker, which tells them apart.
#[cfg(kiln_rt)]
const KILN_RT: Result<&[u8], &str> = Ok(include_bytes!(concat!(env!("OUT_DIR"), "/kiln_rt")));
#[cfg(not(kiln_rt))]
const KILN_RT: Result<&[u8], &str> = Err(include_str!(concat!(
    env!("OUT_DIR"),
    "/missing-kiln_rt.txt"
)));

/// The system libraries Rust's standard library needs, as
/// `--print native-static-libs` lists them.
const KILN_RT_LIBS: &[&str] = if cfg!(windows) {
    &[
        "-lkernel32",
        "-lntdll",
        "-luserenv",
        "-lws2_32",
        "-ldbghelp",
    ]
} else if cfg!(target_os = "macos") {
    &["-liconv", "-lSystem", "-lc", "-lm"]
} else {
    &[
        "-lgcc_s",
        "-lutil",
        "-lrt",
        "-lpthread",
        "-lm",
        "-ldl",
        "-lc",
    ]
};

/// Generates C for a program lowered to the IR. Returns diagnostics for the
/// parts the backend does not support yet (E0401).
pub fn emit_c(program: &Program) -> Result<String, Vec<Diagnostic>> {
    emit::emit(program)
}

/// Compiles generated C into an executable at `out`, using `$CC` or `cc`.
pub fn compile_c(c_source: &str, out: &Path) -> Result<(), String> {
    let rt = KILN_RT.map_err(|why| {
        format!("this kiln was built without its runtime library for the C backend: {why}")
    })?;
    let dir = scratch_dir()?;
    let c_path = dir.join("main.c");
    let rt_path = dir.join("kiln_rt");
    for (path, bytes) in [(&c_path, c_source.as_bytes()), (&rt_path, rt)] {
        std::fs::write(path, bytes).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    }
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let result = Command::new(&cc)
        .args(["-std=gnu11", "-O2", "-w"])
        .arg(&c_path)
        .arg("-o")
        .arg(out)
        .arg(&rt_path)
        .args(["-lgc", "-lm"])
        .args(KILN_RT_LIBS)
        .output();
    let _ = std::fs::remove_dir_all(&dir);
    let output = match result {
        Ok(output) => output,
        Err(e) => {
            return Err(format!(
                "cannot run the C compiler `{cc}`: {e}. Install a C compiler, or set CC to one"
            ))
        }
    };
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("gc.h") || stderr.contains("-lgc") {
        return Err("the Boehm garbage collector is not installed. Install `libgc-dev` (Debian and Ubuntu), `gc-devel` (Fedora) or `bdw-gc` (Homebrew)".into());
    }
    Err(format!(
        "internal compiler error: the C compiler rejected the generated code. Please report this.\n{stderr}"
    ))
}

/// A fresh directory for build files.
pub fn scratch_dir() -> Result<PathBuf, String> {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!("kiln-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    Ok(dir)
}
