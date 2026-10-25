//! The Kiln type checker: name resolution, type inference, error-handling
//! rules (`try`, `catch`, `!`), match exhaustiveness, typed route
//! parameters, and glazes (templates and components).
//!
//! The standard library is described by stub files in `std/`, written in
//! Kiln. Only their signatures are used; bodies are ignored.

mod checker;
mod collect;
mod glaze;
pub mod ide;
pub mod types;

pub use glaze::ValueKind;

use kiln_diagnostics::{Diagnostic, Span};
use kiln_syntax::ast::{Glaze, Module};
use std::collections::HashMap;

/// Type-checks a parsed file on its own. Call it only on a file without
/// parse errors. `web.render` calls are not checked, because the glazes
/// they name are not known.
/// The source of a standard library module, such as `web`, or of the
/// `prelude`. Its function bodies are placeholders, but the default values
/// of parameters are real.
pub fn std_source(name: &str) -> Option<&'static str> {
    checker::std_source(name)
}

pub fn check(module: &Module) -> Vec<Diagnostic> {
    let mut checker = checker::Checker::new();
    checker.check_module(module);
    sorted(checker.diags)
}

/// A checked program: its diagnostics and, when there are none, the types
/// the code generator needs.
pub struct Program {
    pub diagnostics: Vec<Diagnostic>,
    /// Every struct and enum, the standard library's first.
    pub adts: Vec<types::AdtDef>,
    /// How many of `adts` come from the standard library.
    pub std_adts: usize,
    pub global_types: HashMap<String, types::AdtId>,
    /// Top-level functions, including the prelude's `print`, `env` and `panic`.
    pub global_fns: HashMap<String, types::FnSig>,
    /// Imported standard library modules, by the name used in the file.
    pub imports: HashMap<String, String>,
    /// The standard library modules, by name.
    pub modules: HashMap<String, types::ModuleDef>,
    pub consts: HashMap<String, types::Ty>,
    /// The type of every expression, by its span.
    pub expr_types: HashMap<Span, types::Ty>,
    /// The type of every name bound by `let`, a parameter, a loop or a
    /// pattern, by the span of the name.
    pub binding_types: HashMap<Span, types::Ty>,
    /// The type arguments of every use of a generic function or method, by
    /// the span of the call or name.
    pub instantiations: HashMap<Span, HashMap<String, types::Ty>>,
}

/// Type-checks a whole program for compiling.
pub fn check_program(module: &Module) -> Program {
    check_program_with_glazes(module, &[])
}

/// Type-checks a whole program for compiling, with the glazes that its
/// `web.render` calls name, so that their data is checked against the
/// glazes' props. The glazes' own code is checked by [`check_app`].
pub fn check_program_with_glazes(module: &Module, glazes: &[(&str, &Glaze)]) -> Program {
    let mut checker = checker::Checker::new();
    checker.collect_module(module);
    if !glazes.is_empty() {
        checker.glazes = Some(Default::default());
        for (file, glaze) in glazes {
            checker.register_glaze(file, glaze);
        }
    }
    checker.check_bodies(module);
    checker.zonk_recorded();
    let imports = checker
        .imports
        .iter()
        .map(|(name, id)| (name.clone(), checker.modules[*id].name.clone()))
        .collect();
    Program {
        diagnostics: sorted(std::mem::take(&mut checker.diags)),
        adts: checker.adts,
        std_adts: checker.std_adts,
        global_types: checker.global_types,
        global_fns: checker.global_fns,
        imports,
        modules: checker
            .modules
            .iter()
            .map(|m| (m.name.clone(), m.clone()))
            .collect(),
        consts: checker.consts,
        expr_types: checker.expr_types,
        binding_types: checker.binding_types,
        instantiations: checker.instantiations,
    }
}

/// The diagnostics of each file in an app, in the order the files were given.
pub struct AppDiagnostics {
    pub modules: Vec<Vec<Diagnostic>>,
    pub glazes: Vec<Vec<Diagnostic>>,
    /// For each glaze, what each value it outputs is, by the span of the
    /// value's expression: what the compiler needs to render it.
    pub glaze_values: Vec<HashMap<Span, ValueKind>>,
}

/// Type-checks the `.kiln` files and glazes of one directory together. The
/// `.kiln` files share one namespace: each sees the functions, types and
/// constants of the others, while imports stay per file. Glazes see the
/// types and functions of the `.kiln` files, and `.kiln` files see the
/// props of the glazes. Glazes are given with their file names, such as
/// `"index.glaze"`. Call it only on files without parse errors.
pub fn check_app(modules: &[&Module], glazes: &[(&str, &Glaze)]) -> AppDiagnostics {
    check_app_with_css(modules, glazes, &CssModules::new())
}

/// The class names of the CSS Modules glazes import, by glaze file name and
/// then by the name each import gives them (`styles` in
/// `{% css module styles from "card.module.css" %}`). Names are as Kiln
/// fields: `card-title` is `card_title`.
pub type CssModules = HashMap<String, HashMap<String, Vec<String>>>;

/// [`check_app`], with the classes of the CSS Modules the glazes import,
/// so that `styles.card` is checked. A module missing from `css` has no
/// known classes, and any name in it is accepted.
pub fn check_app_with_css(
    modules: &[&Module],
    glazes: &[(&str, &Glaze)],
    css: &CssModules,
) -> AppDiagnostics {
    AppDiagnostics {
        modules: (0..modules.len())
            .map(|i| sorted(module_checker(modules, glazes, i, false).diags))
            .collect(),
        ..check_glazes(modules, glazes, css)
    }
}

fn check_glazes(
    modules: &[&Module],
    glazes: &[(&str, &Glaze)],
    css: &CssModules,
) -> AppDiagnostics {
    let mut out = AppDiagnostics {
        modules: Vec::new(),
        glazes: Vec::new(),
        glaze_values: Vec::new(),
    };
    for i in 0..glazes.len() {
        let checker = glaze_checker(modules, glazes, i, false, css);
        out.glazes.push(sorted(checker.diags));
        out.glaze_values.push(checker.glaze_values);
    }
    out
}

/// Checks the `i`th module of an app, with only that file's errors left in
/// the checker. With `analyze`, it also records what that file's names refer
/// to, and its types are the only ones recorded.
fn module_checker(
    modules: &[&Module],
    glazes: &[(&str, &Glaze)],
    i: usize,
    analyze: bool,
) -> checker::Checker {
    let mut checker = checker::Checker::new();
    checker.completing = analyze;
    // Each file's errors are reported with that file only.
    let mut own = Vec::new();
    let mut keep = |checker: &mut checker::Checker, j: usize, before: usize| {
        let new = checker.diags.split_off(before);
        if i == j {
            own.extend(new);
        }
    };
    let imports: Vec<_> = (0..modules.len())
        .map(|j| {
            let before = checker.diags.len();
            checker.recording = analyze && i == j;
            let imports = checker.collect_imports(modules[j]);
            keep(&mut checker, j, before);
            imports
        })
        .collect();
    for (j, module) in modules.iter().enumerate() {
        checker.imports = imports[j].clone();
        let before = checker.diags.len();
        checker.declare_types(module, checker::Scope::Global);
        keep(&mut checker, j, before);
    }
    for (j, module) in modules.iter().enumerate() {
        checker.imports = imports[j].clone();
        checker.recording = analyze && i == j;
        let before = checker.diags.len();
        checker.define_items(module, checker::Scope::Global);
        keep(&mut checker, j, before);
    }
    checker.recording = false;
    for (j, module) in modules.iter().enumerate() {
        if j != i {
            checker.imports = imports[j].clone();
            let before = checker.diags.len();
            checker.check_consts(module);
            keep(&mut checker, j, before);
        }
    }
    checker.imports = imports[i].clone();
    checker.glazes = Some(Default::default());
    for (file, glaze) in glazes {
        checker.register_glaze(file, glaze);
    }
    // Errors in glaze props are reported with the glaze.
    checker.diags = own;
    checker.expr_types.clear();
    checker.binding_types.clear();
    checker.recording = analyze;
    checker.check_bodies(modules[i]);
    checker
}

/// Checks the `i`th glaze of an app, as `module_checker` does a module.
fn glaze_checker(
    modules: &[&Module],
    glazes: &[(&str, &Glaze)],
    i: usize,
    analyze: bool,
    css: &CssModules,
) -> checker::Checker {
    let (file, glaze) = glazes[i];
    let mut checker = checker::Checker::new();
    if let Some(m) = css.get(file) {
        checker.css_modules.insert(file.to_string(), m.clone());
    }
    checker.completing = analyze;
    for module in modules {
        checker.collect_module(module);
    }
    // Errors in the modules are reported with the modules.
    checker.diags.clear();
    checker.glazes = Some(Default::default());
    for (other, other_glaze) in glazes {
        let before = checker.diags.len();
        checker.recording = analyze && *other == file;
        checker.register_glaze(other, other_glaze);
        if *other != file {
            checker.diags.truncate(before);
        }
    }
    checker.expr_types.clear();
    checker.binding_types.clear();
    checker.recording = analyze;
    checker.check_glaze(file, glaze);
    checker
}

fn sorted(mut diags: Vec<Diagnostic>) -> Vec<Diagnostic> {
    diags.sort_by_key(|d| (d.span.start, d.span.end));
    diags
}
