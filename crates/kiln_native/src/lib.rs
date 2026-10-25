//! The native backend: turns a Kiln program in the IR (see `kiln_ir`) into
//! a Linux x86-64 executable with no outside tools. [Cranelift] generates
//! machine code from the IR, and the built-in linker ([wild]) links it with
//! the runtime that `kiln` carries: the Kiln runtime, the Boehm garbage
//! collector and the musl C library. The result is one static executable
//! that needs no shared libraries.
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
//! let object = kiln_native::compile(&ir).unwrap();
//! kiln_native::link(&object, std::path::Path::new("hi")).unwrap();
//! ```
//!
//! [Cranelift]: https://cranelift.dev
//! [wild]: https://github.com/davidlattimore/wild

mod gen;
mod link;
mod runtime;

use kiln_diagnostics::Diagnostic;
use kiln_ir::Program;

pub use link::link;

/// Compiles a program lowered to the IR into an object file. Returns
/// diagnostics for the parts the backend does not support yet (E0401).
pub fn compile(program: &Program) -> Result<Vec<u8>, Vec<Diagnostic>> {
    gen::compile(program)
}

/// Why this `kiln` cannot link native executables, when it cannot: it was
/// built for another platform, or without the musl C library.
pub fn unavailable() -> Option<&'static str> {
    runtime::unavailable()
}
