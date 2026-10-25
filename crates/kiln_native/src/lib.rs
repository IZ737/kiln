//! The native backend: turns a Kiln program in the IR (see `kiln_ir`) into
//! an executable for Linux or Windows on x86-64, with no outside tools.
//! [Cranelift] generates machine code from the IR, and `kiln` combines it
//! with the runtime it carries for the target: the Kiln runtime, the Boehm
//! garbage collector and a C library.
//!
//! - For Linux the built-in linker ([wild]) links the code with musl's C
//!   library into one static executable that needs no shared libraries.
//! - For Windows the runtime is an executable linked when `kiln` was built,
//!   and `pe.rs` adds the code to it. The result uses only DLLs that every
//!   Windows has (`kernel32`, `msvcrt` and `user32`).
//!
//! The C backend in `kiln_codegen` stays as the reference: both backends
//! share the runtime in `kiln_codegen/runtime/kiln.c`, and the conformance
//! tests run every program with both.
//!
//! ```no_run
//! let source = "fn main() {\n    print(\"hi\")\n}\n";
//! let parsed = kiln_syntax::parse(source);
//! let program = kiln_check::check_program(&parsed.module);
//! let ir = kiln_ir::lower(&parsed.module, &program).unwrap();
//! let target = kiln_native::Target::host();
//! let object = kiln_native::compile(&ir, target).unwrap();
//! kiln_native::link(&object, target, std::path::Path::new("hi")).unwrap();
//! ```
//!
//! [Cranelift]: https://cranelift.dev
//! [wild]: https://github.com/davidlattimore/wild

mod gen;
mod link;
mod pe;
mod runtime;

use kiln_diagnostics::Diagnostic;
use kiln_ir::Program;
use std::path::Path;

/// A system that the native backend builds executables for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    /// Linux on x86-64: a static ELF executable.
    Linux,
    /// Windows on x86-64: a PE executable (`.exe`).
    Windows,
}

impl Target {
    pub const ALL: [Target; 2] = [Target::Linux, Target::Windows];

    /// The system `kiln` itself runs on, or Linux for any other system.
    pub fn host() -> Target {
        if cfg!(windows) {
            Target::Windows
        } else {
            Target::Linux
        }
    }

    /// The target's name, as `--target` takes it.
    pub fn name(self) -> &'static str {
        match self {
            Target::Linux => "linux",
            Target::Windows => "windows",
        }
    }

    pub fn from_name(name: &str) -> Option<Target> {
        Target::ALL.into_iter().find(|t| t.name() == name)
    }

    /// The name of an executable for the target: `name.exe` on Windows.
    pub fn executable(self, name: &str) -> String {
        match self {
            Target::Linux => name.to_string(),
            Target::Windows => format!("{name}.exe"),
        }
    }
}

/// Compiles a program lowered to the IR into an object file for `target`.
/// Returns diagnostics for the parts the backend does not support yet
/// (E0401).
pub fn compile(program: &Program, target: Target) -> Result<Vec<u8>, Vec<Diagnostic>> {
    gen::compile(program, target)
}

/// Makes the executable `out` for `target` from the object that [`compile`]
/// made for it.
pub fn link(object: &[u8], target: Target, out: &Path) -> Result<(), String> {
    if let Some(why) = unavailable(target) {
        return Err(format!(
            "kiln cannot build {} executables: {why}",
            target.name()
        ));
    }
    match target {
        Target::Linux => link::link(object, out),
        Target::Windows => pe::link(object, out),
    }
}

/// Why this `kiln` cannot make executables for `target`, when it cannot:
/// it was built without the tools to build that target's runtime.
pub fn unavailable(target: Target) -> Option<&'static str> {
    runtime::unavailable(target)
}
