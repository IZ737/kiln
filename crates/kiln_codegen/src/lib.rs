//! The C backend: turns a type-checked Kiln program into one C file, which
//! a C compiler builds against the runtime in `runtime/kiln.c` and the
//! Boehm garbage collector.
//!
//! ```no_run
//! let source = "fn main() {\n    print(\"hi\")\n}\n";
//! let parsed = kiln_syntax::parse(source);
//! let program = kiln_check::check_program(&parsed.module);
//! let c = kiln_codegen::emit_c(&parsed.module, &program).unwrap();
//! kiln_codegen::compile_c(&c, std::path::Path::new("hi")).unwrap();
//! ```

mod emit;

use kiln_check::Program;
use kiln_diagnostics::Diagnostic;
use kiln_syntax::ast::Module;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The runtime, prepended to every generated file.
pub const RUNTIME: &str = include_str!("../runtime/kiln.c");

/// Generates C for a program that type-checked without errors. Returns
/// diagnostics for the parts the compiler does not support yet (E0401).
pub fn emit_c(module: &Module, program: &Program) -> Result<String, Vec<Diagnostic>> {
    emit::emit(module, program)
}

/// Compiles generated C into an executable at `out`, using `$CC` or `cc`.
pub fn compile_c(c_source: &str, out: &Path) -> Result<(), String> {
    let dir = scratch_dir()?;
    let c_path = dir.join("main.c");
    std::fs::write(&c_path, c_source)
        .map_err(|e| format!("cannot write {}: {e}", c_path.display()))?;
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let result = Command::new(&cc)
        .args(["-std=gnu11", "-O2", "-w"])
        .arg(&c_path)
        .arg("-o")
        .arg(out)
        .args(["-lgc", "-lm"])
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
