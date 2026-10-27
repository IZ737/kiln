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
pub mod orm;
mod query;
pub mod style;
pub mod types;

pub use checker::internal_name;
pub use glaze::ValueKind;
pub use query::Query;

use kiln_diagnostics::{Diagnostic, Span};
use kiln_syntax::ast::{Glaze, Module};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

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
    /// How to build each expression in a style, by its span. `Ui` and
    /// style values are `String`s in the types above.
    pub styles: HashMap<Span, style::StyleExpr>,
    /// The query each `orm` call runs, by the span of the call, with the
    /// SQL and the values to bind already worked out.
    pub queries: HashMap<Span, Query>,
    /// The models the program uses, in the order it first uses them.
    pub models: Vec<orm::Model>,
    /// The model each `admin.pages<T>` call makes pages for, by the span
    /// of the call.
    pub admins: HashMap<Span, String>,
}

/// Type-checks a whole program for compiling.
pub fn check_program(module: &Module) -> Program {
    check_program_with_glazes(module, &[])
}

/// What an app's stylesheets tell the checker: the classes of CSS Modules
/// and the design tokens.
#[derive(Clone, Debug, Default)]
pub struct Styling {
    pub modules: CssModules,
    pub tokens: style::Tokens,
}

/// Type-checks a whole program for compiling, with the glazes that its
/// `web.render` calls name, so that their data is checked against the
/// glazes' props. The glazes' own code is checked by [`check_app`].
pub fn check_program_with_glazes(module: &Module, glazes: &[(&str, &Glaze)]) -> Program {
    check_program_with_tokens(module, glazes, &style::Tokens::default())
}

/// [`check_program_with_glazes`], with the app's design tokens.
pub fn check_program_with_tokens(
    module: &Module,
    glazes: &[(&str, &Glaze)],
    tokens: &style::Tokens,
) -> Program {
    check_program_for(module, glazes, tokens, false)
}

/// [`check_program_with_tokens`] for the code of a page's embers, which
/// runs in the browser: the handlers the compiler wrote from the glazes
/// may use the browser's modules. The glazes were checked with
/// [`check_app`], which made sure only handlers do.
pub fn check_browser_program(
    module: &Module,
    glazes: &[(&str, &Glaze)],
    tokens: &style::Tokens,
) -> Program {
    check_program_for(module, glazes, tokens, true)
}

fn check_program_for(
    module: &Module,
    glazes: &[(&str, &Glaze)],
    tokens: &style::Tokens,
    browser: bool,
) -> Program {
    let mut checker = checker::Checker::new();
    checker.browser_program = browser;
    checker.tokens = Rc::new(tokens.clone());
    checker.collect_module(module);
    if !glazes.is_empty() {
        checker.glazes = Some(Default::default());
        for (file, glaze) in glazes {
            checker.register_glaze(file, glaze);
        }
    }
    checker.check_bodies(module);
    checker.zonk_recorded();
    erase_styles(&mut checker);
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
        styles: checker.styles,
        queries: std::mem::take(&mut checker.query_runs),
        admins: std::mem::take(&mut checker.admins),
        models: checker
            .model_order
            .iter()
            .filter_map(|adt| checker.models.get(adt).cloned().flatten())
            .collect(),
    }
}

/// Makes `Ui` and style values `String`s, which is what they are when the
/// program runs.
fn erase_styles(c: &mut checker::Checker) {
    use types::{FnSig, FnTy, Ty};
    let ui = c.global_types["Ui"];
    let string = Ty::Adt(c.global_types["String"], Vec::new());
    fn erase(ty: &Ty, ui: usize, string: &Ty) -> Ty {
        let e = |t: &Ty| erase(t, ui, string);
        match ty {
            Ty::Adt(id, _) if *id == ui => string.clone(),
            Ty::Style(_) => string.clone(),
            Ty::Adt(id, args) => Ty::Adt(*id, args.iter().map(e).collect()),
            Ty::Opt(inner) => Ty::Opt(Box::new(e(inner))),
            Ty::Fn(f) => Ty::Fn(Rc::new(FnTy {
                params: f.params.iter().map(e).collect(),
                ret: e(&f.ret),
                throws: f.throws.as_ref().map(e),
            })),
            Ty::Record(fields) => {
                Ty::Record(fields.iter().map(|(n, t)| (n.clone(), e(t))).collect())
            }
            other => other.clone(),
        }
    }
    let e = |t: &Ty| erase(t, ui, &string);
    let sig = |s: &mut FnSig| {
        for p in &mut s.params {
            p.ty = e(&p.ty);
        }
        s.ret = e(&s.ret);
        s.throws = s.throws.as_ref().map(e);
    };
    for t in c.expr_types.values_mut() {
        *t = e(t);
    }
    for t in c.binding_types.values_mut() {
        *t = e(t);
    }
    for map in c.instantiations.values_mut() {
        for t in map.values_mut() {
            *t = e(t);
        }
    }
    for t in c.consts.values_mut() {
        *t = e(t);
    }
    for s in c.global_fns.values_mut() {
        sig(s);
    }
    for m in &mut c.modules {
        for s in m.fns.values_mut() {
            sig(s);
        }
    }
    for adt in &mut c.adts {
        for s in &mut adt.methods {
            sig(s);
        }
        match &mut adt.kind {
            types::AdtKind::Struct(fields) => {
                for f in fields {
                    f.ty = e(&f.ty);
                }
            }
            types::AdtKind::Enum(variants) => {
                for v in variants {
                    for f in &mut v.fields {
                        f.1 = e(&f.1);
                    }
                }
            }
        }
    }
}

/// The diagnostics of each file in an app, in the order the files were given.
pub struct AppDiagnostics {
    pub modules: Vec<Vec<Diagnostic>>,
    pub glazes: Vec<Vec<Diagnostic>>,
    /// For each glaze, what each value it outputs is, by the span of the
    /// value's expression: what the compiler needs to render it.
    pub glaze_values: Vec<HashMap<Span, ValueKind>>,
    /// For each glaze, its `{% state %}`s with their types as Kiln writes
    /// them.
    pub glaze_states: Vec<Vec<(String, String)>>,
    /// The atomic CSS rules the app's styles use, by class name (see
    /// [`style::stylesheet`]).
    pub ui_rules: BTreeMap<String, style::UiRule>,
}

/// Type-checks the `.kiln` files and glazes of one directory together. The
/// `.kiln` files share one namespace: each sees the functions, types and
/// constants of the others, while imports stay per file. Glazes see the
/// types and functions of the `.kiln` files, and `.kiln` files see the
/// props of the glazes. Glazes are given with their file names, such as
/// `"index.glaze"`. Call it only on files without parse errors.
pub fn check_app(modules: &[&Module], glazes: &[(&str, &Glaze)]) -> AppDiagnostics {
    check_app_with_css(modules, glazes, &Styling::default())
}

/// The class names of the CSS Modules glazes import, by glaze file name and
/// then by the name each import gives them (`styles` in
/// `{% css module styles from "card.module.css" %}`). Names are as Kiln
/// fields: `card-title` is `card_title`.
pub type CssModules = HashMap<String, HashMap<String, Vec<String>>>;

/// [`check_app`], with what the app's stylesheets declare: the classes of
/// the CSS Modules the glazes import, so that `styles.card` is checked (a
/// module missing from them has no known classes, and any name in it is
/// accepted), and the design tokens styles use.
pub fn check_app_with_css(
    modules: &[&Module],
    glazes: &[(&str, &Glaze)],
    css: &Styling,
) -> AppDiagnostics {
    let mut out = AppDiagnostics {
        modules: Vec::new(),
        glazes: Vec::new(),
        glaze_values: Vec::new(),
        glaze_states: Vec::new(),
        ui_rules: BTreeMap::new(),
    };
    for i in 0..modules.len() {
        let checker = module_checker(modules, glazes, i, false, &css.tokens);
        out.modules.push(sorted(checker.diags));
        out.ui_rules.extend(checker.ui_rules);
    }
    for i in 0..glazes.len() {
        let checker = glaze_checker(modules, glazes, i, false, css);
        out.glazes.push(sorted(checker.diags));
        out.glaze_values.push(checker.glaze_values);
        out.glaze_states.push(checker.glaze_states);
        out.ui_rules.extend(checker.ui_rules);
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
    tokens: &style::Tokens,
) -> checker::Checker {
    let mut checker = checker::Checker::new();
    checker.tokens = Rc::new(tokens.clone());
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
    css: &Styling,
) -> checker::Checker {
    let (file, glaze) = glazes[i];
    let mut checker = checker::Checker::new();
    checker.tokens = Rc::new(css.tokens.clone());
    if let Some(m) = css.modules.get(file) {
        checker.css_modules.insert(file.to_string(), m.clone());
    }
    checker.completing = analyze;
    for module in modules {
        checker.collect_module(module);
    }
    // A glaze can read the constants and stores of the modules.
    for module in modules {
        checker.check_consts(module);
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

/// An npm package's bindings: Kiln declarations made from its TypeScript
/// ones, such as `fn nanoid(size: Float? = none) -> String ! {}`, and the
/// exports Kiln cannot call, with why. The checker sees each as a module
/// named `name`, which only an ember's handlers can use.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Foreign {
    pub name: String,
    pub source: String,
    pub unsupported: Vec<(String, String)>,
}

thread_local! {
    static FOREIGN: std::cell::RefCell<Vec<Foreign>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Sets the npm packages that checking on this thread sees, for the
/// program being compiled.
pub fn set_foreign_modules(modules: Vec<Foreign>) {
    FOREIGN.with(|f| *f.borrow_mut() = modules);
}

/// The npm packages set by [`set_foreign_modules`].
pub fn foreign_modules() -> Vec<Foreign> {
    FOREIGN.with(|f| f.borrow().clone())
}
