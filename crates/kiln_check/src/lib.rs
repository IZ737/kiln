//! The Kiln type checker: name resolution, type inference, error-handling
//! rules (`try`, `catch`, `!`), match exhaustiveness and typed route
//! parameters.
//!
//! The standard library is described by stub files in `std/`, written in
//! Kiln. Only their signatures are used; bodies are ignored.

mod checker;
mod collect;
mod types;

use kiln_diagnostics::Diagnostic;
use kiln_syntax::ast::Module;

/// Type-checks a parsed file. Call it only on a file without parse errors.
pub fn check(module: &Module) -> Vec<Diagnostic> {
    let mut checker = checker::Checker::new();
    checker.check_module(module);
    let mut diags = checker.diags;
    diags.sort_by_key(|d| (d.span.start, d.span.end));
    diags
}
