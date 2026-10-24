//! The Kiln type checker: name resolution, type inference, error-handling
//! rules (`try`, `catch`, `!`), match exhaustiveness, typed route
//! parameters, and glazes (templates and components).
//!
//! The standard library is described by stub files in `std/`, written in
//! Kiln. Only their signatures are used; bodies are ignored.

mod checker;
mod collect;
mod glaze;
mod types;

use kiln_diagnostics::Diagnostic;
use kiln_syntax::ast::{Glaze, Module};

/// Type-checks a parsed file on its own. Call it only on a file without
/// parse errors. `web.render` calls are not checked, because the glazes
/// they name are not known.
pub fn check(module: &Module) -> Vec<Diagnostic> {
    let mut checker = checker::Checker::new();
    checker.check_module(module);
    sorted(checker.diags)
}

/// The diagnostics of each file in an app, in the order the files were given.
pub struct AppDiagnostics {
    pub modules: Vec<Vec<Diagnostic>>,
    pub glazes: Vec<Vec<Diagnostic>>,
}

/// Type-checks the `.kiln` files and glazes of one directory together:
/// glazes see the types and functions of the `.kiln` files, and `.kiln`
/// files see the props of the glazes. Glazes are given with their file
/// names, such as `"index.glaze"`. Call it only on files without parse
/// errors.
pub fn check_app(modules: &[&Module], glazes: &[(&str, &Glaze)]) -> AppDiagnostics {
    let module_diags = modules
        .iter()
        .map(|module| {
            let mut checker = checker::Checker::new();
            checker.collect_module(module);
            let own = checker.diags.len();
            checker.glazes = Some(Default::default());
            for (file, glaze) in glazes {
                checker.register_glaze(file, glaze);
            }
            // Errors in glaze props are reported with the glaze.
            checker.diags.truncate(own);
            checker.check_bodies(module);
            sorted(checker.diags)
        })
        .collect();
    let glaze_diags = glazes
        .iter()
        .map(|(file, glaze)| {
            let mut checker = checker::Checker::new();
            for module in modules {
                checker.collect_module(module);
            }
            // Errors in the modules are reported with the modules.
            checker.diags.clear();
            checker.glazes = Some(Default::default());
            for (other, other_glaze) in glazes {
                let before = checker.diags.len();
                checker.register_glaze(other, other_glaze);
                if other != file {
                    checker.diags.truncate(before);
                }
            }
            checker.check_glaze(file, glaze);
            sorted(checker.diags)
        })
        .collect();
    AppDiagnostics {
        modules: module_diags,
        glazes: glaze_diags,
    }
}

fn sorted(mut diags: Vec<Diagnostic>) -> Vec<Diagnostic> {
    diags.sort_by_key(|d| (d.span.start, d.span.end));
    diags
}
