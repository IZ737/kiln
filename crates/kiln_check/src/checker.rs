//! The type checker: name resolution, inference by unification, and the
//! rules for errors (`try`, `catch`, `throw`), patterns and routes.

use crate::ide::{Completion, Ref, COMPLETION_MARKER};
use crate::types::*;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::*;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

#[derive(Clone, Copy)]
pub(crate) enum Scope {
    Global,
    Module(ModId),
}

/// What the surrounding code needs from an expression.
#[derive(Clone)]
pub(crate) enum Expect {
    /// A value, of this type when known.
    Value(Option<Ty>),
    /// Nothing: the expression is a statement and its value is dropped.
    Discard,
}

#[derive(Clone)]
pub(crate) struct Local {
    pub(crate) ty: Ty,
    pub(crate) mutable: bool,
    /// The `let` keyword, so an assignment error can suggest `var`.
    let_span: Option<Span>,
    /// The name where it was bound.
    def: Span,
}

pub(crate) struct FnCtx {
    ret: Ty,
    /// The error type this function may fail with.
    throws: Option<Ty>,
    /// For closures with no expected type: whether `try`/`throw` was used,
    /// which makes the closure fallible.
    infer_throws: Option<bool>,
    /// Where to insert ` !` to make the function fallible.
    bang_at: Option<Span>,
    /// Inference variables from generic calls that must be solved by the end.
    pending: Vec<(u32, Span, String, String)>,
}

pub struct Checker {
    pub(crate) adts: Vec<AdtDef>,
    pub(crate) modules: Vec<ModuleDef>,
    pub(crate) global_fns: HashMap<String, FnSig>,
    pub(crate) global_types: HashMap<String, AdtId>,
    pub(crate) imports: HashMap<String, ModId>,
    pub(crate) consts: HashMap<String, Ty>,
    /// The names of the program's own types, and of its functions and
    /// constants, to report a name defined twice in a folder.
    pub(crate) declared_types: HashSet<String>,
    pub(crate) declared_values: HashSet<String>,
    /// Struct and enum declarations that repeat a name, by the address of
    /// their name in the tree. Spans cannot tell them apart, because each
    /// file's spans start at 0.
    pub(crate) duplicate_types: HashSet<usize>,
    subst: Vec<Option<Ty>>,
    /// The type parameters in scope in the function being checked.
    type_params: Vec<String>,
    scopes: Vec<HashMap<String, Local>>,
    fns: Vec<FnCtx>,
    pub(crate) diags: Vec<Diagnostic>,
    /// The glazes next to the file being checked, by file name. `None` when
    /// a file is checked on its own, so `web.render` cannot be checked.
    pub(crate) glazes: Option<HashMap<String, GlazeSig>>,
    /// Whether the code being checked runs in the browser.
    pub(crate) in_ember: bool,
    /// Whether the code being checked is an `on:` handler, where a call to
    /// a `server fn` goes over the network and so can fail.
    pub(crate) in_handler: bool,
    /// Whether this is the code of a page's embers, all of which runs in
    /// the browser.
    pub(crate) browser_program: bool,
    /// The program's `server fn`s, by name.
    pub(crate) server_fns: HashSet<String>,
    /// Each `{% state %}` of the glaze being checked, with its type as Kiln
    /// writes it, for the code that restores it in the browser.
    pub(crate) glaze_states: Vec<(String, String)>,
    /// The class names of the CSS Modules each glaze imports: by glaze
    /// file, then by the name it imports them as.
    pub(crate) css_modules: crate::CssModules,
    /// The app's design tokens, for styles.
    pub(crate) tokens: std::rc::Rc<crate::style::Tokens>,
    /// Every atomic CSS rule the program's styles use, by class name.
    pub(crate) ui_rules: std::collections::BTreeMap<String, crate::style::UiRule>,
    /// How to build each expression in a style, by its span.
    pub(crate) styles: HashMap<Span, crate::style::StyleExpr>,
    /// What each value a glaze outputs is, by the span of its expression.
    pub(crate) glaze_values: HashMap<Span, crate::glaze::ValueKind>,
    /// The type of every expression, by span, for the code generator.
    pub(crate) expr_types: HashMap<Span, Ty>,
    /// The type of every name a declaration, parameter or pattern binds, by
    /// the span of the name.
    pub(crate) binding_types: HashMap<Span, Ty>,
    /// The type arguments of every use of a generic function or method, by
    /// the span of the call or name: each type parameter (the `impl`
    /// target's and the function's own) and its type.
    pub(crate) instantiations: HashMap<Span, HashMap<String, Ty>>,
    /// Types and functions with ids or names below these come from the
    /// standard library.
    pub(crate) std_adts: usize,
    /// Whether to record what names refer to, for the language server.
    pub(crate) recording: bool,
    pub(crate) references: Vec<(Span, Ref)>,
    /// Whether `COMPLETION_MARKER` marks where the editor wants completions,
    /// and what is wanted there once it is found.
    pub(crate) completing: bool,
    pub(crate) completion: Option<Completion>,
}

/// What other files need to know about a glaze.
#[derive(Clone, Debug)]
pub(crate) struct GlazeSig {
    pub(crate) props: Vec<ParamSig>,
    /// Whether it has a default `{% slot %}`, so it takes children.
    pub(crate) has_slot: bool,
    /// Its named slots, which callers fill with `{% fill name %}`.
    pub(crate) slots: Vec<String>,
    /// The glazes it always renders: its layout, and components outside
    /// blocks. A glaze that reaches itself through these never ends.
    pub(crate) renders: Vec<String>,
}

/// Whether an argument is a closure that takes a parameter type from the
/// call: `fn(x) { ... }` or `.field`.
fn infers_from_call(e: &Expr) -> bool {
    match &e.kind {
        ExprKind::FieldShorthand(_) => true,
        ExprKind::Closure { params, .. } => params.iter().any(|p| p.ty.is_none()),
        _ => false,
    }
}

/// Modules that run only in the browser: only handlers can use them.
pub(crate) const BROWSER_MODULES: &[&str] = &["browser", "js"];

/// Modules that need the server: they cannot be used in embers.
const SERVER_ONLY_MODULES: &[&str] = &["sql", "auth", "fs", "os", "io"];

const STD_MODULES: &[(&str, &str)] = &[
    ("web", include_str!("../std/web.kiln")),
    ("sql", include_str!("../std/sql.kiln")),
    ("auth", include_str!("../std/auth.kiln")),
    ("log", include_str!("../std/log.kiln")),
    ("json", include_str!("../std/json.kiln")),
    ("time", include_str!("../std/time.kiln")),
    ("fs", include_str!("../std/fs.kiln")),
    ("os", include_str!("../std/os.kiln")),
    ("io", include_str!("../std/io.kiln")),
    ("html", include_str!("../std/html.kiln")),
    ("base64", include_str!("../std/base64.kiln")),
    ("markdown", include_str!("../std/markdown.kiln")),
    ("ember", include_str!("../std/ember.kiln")),
    ("js", include_str!("../std/js.kiln")),
    ("browser", include_str!("../std/browser.kiln")),
];
const PRELUDE: &str = include_str!("../std/prelude.kiln");

impl Checker {
    pub fn new() -> Self {
        let mut checker = Checker {
            adts: Vec::new(),
            modules: Vec::new(),
            global_fns: HashMap::new(),
            global_types: HashMap::new(),
            imports: HashMap::new(),
            consts: HashMap::new(),
            declared_types: HashSet::new(),
            declared_values: HashSet::new(),
            duplicate_types: HashSet::new(),
            subst: Vec::new(),
            type_params: Vec::new(),
            instantiations: HashMap::new(),
            scopes: Vec::new(),
            fns: Vec::new(),
            diags: Vec::new(),
            glazes: None,
            in_ember: false,
            in_handler: false,
            browser_program: false,
            server_fns: HashSet::new(),
            glaze_states: Vec::new(),
            glaze_values: HashMap::new(),
            css_modules: HashMap::new(),
            tokens: Default::default(),
            ui_rules: Default::default(),
            styles: HashMap::new(),
            expr_types: HashMap::new(),
            binding_types: HashMap::new(),
            std_adts: 0,
            recording: false,
            references: Vec::new(),
            completing: false,
            completion: None,
        };
        let prelude = parse_std("prelude", PRELUDE);
        checker.declare_types(&prelude, Scope::Global);
        checker.define_items(&prelude, Scope::Global);
        let modules: Vec<(String, Module)> = STD_MODULES
            .iter()
            .map(|(name, src)| (name.to_string(), parse_std(name, src)))
            .collect();
        for (name, module) in &modules {
            let id = checker.modules.len();
            checker.modules.push(ModuleDef {
                name: name.clone(),
                ..Default::default()
            });
            checker.declare_types(module, Scope::Module(id));
        }
        // An npm package's bindings are a module too.
        let foreign = crate::foreign_modules();
        let mut modules = modules;
        for f in &foreign {
            let id = checker.modules.len();
            checker.modules.push(ModuleDef {
                name: f.name.clone(),
                foreign: true,
                unsupported: f.unsupported.iter().cloned().collect(),
                ..Default::default()
            });
            let module = parse_std(&f.name, &f.source);
            checker.declare_types(&module, Scope::Module(id));
            modules.push((f.name.clone(), module));
        }
        for (id, (_, module)) in modules.iter().enumerate() {
            checker.define_items(module, Scope::Module(id));
        }
        assert!(
            checker.diags.is_empty(),
            "standard library signatures have errors: {:#?}",
            checker.diags
        );
        checker.std_adts = checker.adts.len();
        // The program may reuse the prelude's names.
        checker.declared_types.clear();
        checker.declared_values.clear();
        checker
    }

    pub fn check_module(&mut self, module: &Module) {
        self.collect_module(module);
        self.check_bodies(module);
    }

    /// Imports and declarations: everything other files can refer to.
    pub(crate) fn collect_module(&mut self, module: &Module) {
        let imports = self.collect_imports(module);
        self.imports.extend(imports);
        self.declare_types(module, Scope::Global);
        self.define_items(module, Scope::Global);
    }

    /// The standard library modules a file imports, by the name it uses.
    pub(crate) fn collect_imports(&mut self, module: &Module) -> HashMap<String, ModId> {
        let mut imports = HashMap::new();
        for item in &module.items {
            if let Item::Import(import) = item {
                for path in &import.paths {
                    let name = path
                        .iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    let span = path[0].span.to(path[path.len() - 1].span);
                    match self.modules.iter().position(|m| m.name == name) {
                        Some(id) => {
                            self.refer(span, Ref::Module(id));
                            imports.insert(path[path.len() - 1].name.clone(), id);
                        }
                        None => {
                            let mut diag = Diagnostic::error(
                                codes::UNDEFINED_NAME,
                                format!("there is no module named `{name}`"),
                                span,
                            );
                            let names: Vec<&str> = STD_MODULES.iter().map(|(n, _)| *n).collect();
                            if let Some(s) = similar(&name, names) {
                                diag =
                                    diag.with_suggestion(format!("did you mean `{s}`?"), span, s);
                            }
                            self.error(diag);
                        }
                    }
                }
            }
        }
        imports
    }

    /// The types of a file's constants, so other files can use them.
    pub(crate) fn check_consts(&mut self, module: &Module) {
        for item in &module.items {
            if let Item::Const(c) = item {
                self.enter_fn(Ty::Unit, None, None);
                let ty = match &c.ty {
                    Some(t) => {
                        let ty = self.resolve_type(t, &[], Scope::Global);
                        self.check(&c.value, &ty);
                        ty
                    }
                    None => self.infer(&c.value, &Expect::Value(None)),
                };
                self.exit_fn();
                self.consts.insert(c.name.name.clone(), ty);
            }
        }
    }

    /// Constants, function bodies and field defaults.
    pub(crate) fn check_bodies(&mut self, module: &Module) {
        self.refer_declarations(module);
        self.check_consts(module);
        for item in &module.items {
            match item {
                Item::Fn(f) => self.check_fn(f, None, &[]),
                Item::Impl(imp) => {
                    let generics: Vec<String> = match &imp.target.kind {
                        TypeKind::Named { args, .. } => args
                            .iter()
                            .filter_map(|a| match &a.kind {
                                TypeKind::Named { path, .. } if path.len() == 1 => {
                                    Some(path[0].name.clone())
                                }
                                _ => None,
                            })
                            .collect(),
                        _ => Vec::new(),
                    };
                    let self_ty = self.resolve_type(&imp.target, &generics, Scope::Global);
                    for m in &imp.methods {
                        self.check_fn(m, Some(self_ty.clone()), &generics);
                    }
                }
                Item::Struct(s) => {
                    let generics: Vec<String> = s.generics.iter().map(|g| g.name.clone()).collect();
                    for f in &s.fields {
                        if let Some(default) = &f.default {
                            let ty = self.resolve_type(&f.ty, &generics, Scope::Global);
                            self.enter_fn(Ty::Unit, None, None);
                            self.check(default, &ty);
                            self.exit_fn();
                        }
                    }
                }
                _ => {}
            }
        }
    }

    // ----- diagnostics -----

    pub(crate) fn error(&mut self, diag: Diagnostic) {
        if self
            .diags
            .iter()
            .any(|d| d.span == diag.span && d.code == diag.code)
        {
            return;
        }
        self.diags.push(diag);
    }

    pub(crate) fn error_ty(&self) -> Ty {
        Ty::Adt(self.global_types["Error"], Vec::new())
    }

    pub(crate) fn prim(&self, name: &str) -> Ty {
        Ty::Adt(self.global_types[name], Vec::new())
    }

    /// Whether `ty` is `Ui`, the type of styles.
    pub(crate) fn is_ui(&self, ty: &Ty) -> bool {
        matches!(ty, Ty::Adt(id, _) if Some(id) == self.global_types.get("Ui") && *id < self.std_adts)
    }

    pub(crate) fn list_of(&self, elem: Ty) -> Ty {
        Ty::Adt(self.global_types["List"], vec![elem])
    }

    pub(crate) fn is_prim(&self, ty: &Ty, name: &str) -> bool {
        matches!(ty, Ty::Adt(id, _) if *id == self.global_types[name])
    }

    // ----- inference variables -----

    fn fresh(&mut self) -> Ty {
        self.subst.push(None);
        Ty::Var(self.subst.len() as u32 - 1)
    }

    /// Follows solved inference variables at the top level.
    pub(crate) fn resolve(&self, ty: &Ty) -> Ty {
        let mut ty = ty.clone();
        while let Ty::Var(v) = ty {
            match &self.subst[v as usize] {
                Some(t) => ty = t.clone(),
                None => break,
            }
        }
        ty
    }

    /// Resolves inference variables everywhere inside a type.
    pub(crate) fn zonk(&self, ty: &Ty) -> Ty {
        match self.resolve(ty) {
            Ty::Adt(id, args) => Ty::Adt(id, args.iter().map(|a| self.zonk(a)).collect()),
            Ty::Opt(inner) => Ty::Opt(Box::new(self.zonk(&inner))),
            Ty::Fn(f) => Ty::Fn(Rc::new(FnTy {
                params: f.params.iter().map(|p| self.zonk(p)).collect(),
                ret: self.zonk(&f.ret),
                throws: f.throws.as_ref().map(|t| self.zonk(t)),
            })),
            Ty::Record(fields) => Ty::Record(
                fields
                    .iter()
                    .map(|(n, t)| (n.clone(), self.zonk(t)))
                    .collect(),
            ),
            other => other,
        }
    }

    fn occurs(&self, v: u32, ty: &Ty) -> bool {
        match self.resolve(ty) {
            Ty::Var(w) => v == w,
            Ty::Adt(_, args) => args.iter().any(|a| self.occurs(v, a)),
            Ty::Opt(inner) => self.occurs(v, &inner),
            Ty::Fn(f) => f.params.iter().any(|p| self.occurs(v, p)) || self.occurs(v, &f.ret),
            Ty::Record(fields) => fields.iter().any(|(_, t)| self.occurs(v, t)),
            _ => false,
        }
    }

    fn unify(&mut self, a: &Ty, b: &Ty) -> bool {
        let (a, b) = (self.resolve(a), self.resolve(b));
        match (&a, &b) {
            (Ty::Unknown | Ty::Any | Ty::Never, _) | (_, Ty::Unknown | Ty::Any | Ty::Never) => true,
            (Ty::Var(x), Ty::Var(y)) if x == y => true,
            (Ty::Var(v), t) | (t, Ty::Var(v)) => {
                if self.occurs(*v, t) {
                    return false;
                }
                self.subst[*v as usize] = Some(t.clone());
                true
            }
            (Ty::Adt(i, xs), Ty::Adt(j, ys)) => {
                i == j && xs.len() == ys.len() && xs.iter().zip(ys).all(|(x, y)| self.unify(x, y))
            }
            (Ty::Opt(x), Ty::Opt(y)) => self.unify(x, y),
            (Ty::Fn(f), Ty::Fn(g)) => {
                f.params.len() == g.params.len()
                    && f.params
                        .iter()
                        .zip(&g.params)
                        .all(|(x, y)| self.unify(x, y))
                    && self.unify(&f.ret, &g.ret)
            }
            (Ty::Record(xs), Ty::Record(ys)) => {
                xs.len() == ys.len()
                    && xs
                        .iter()
                        .zip(ys)
                        .all(|((n, x), (m, y))| n == m && self.unify(x, y))
            }
            (Ty::Param(x), Ty::Param(y)) => x == y,
            (Ty::Unit, Ty::Unit) => true,
            (Ty::Style(x), Ty::Style(y)) => x == y,
            _ => false,
        }
    }

    /// Checks that a value of type `actual` can be used where `expected` is
    /// required. A `T` is accepted where `T?` is expected.
    pub(crate) fn coerce(&mut self, actual: &Ty, expected: &Ty, span: Span) {
        let (a, e) = (self.resolve(actual), self.resolve(expected));
        if let Ty::Opt(inner) = &e {
            if !matches!(
                a,
                Ty::Opt(_) | Ty::Var(_) | Ty::Never | Ty::Unknown | Ty::Any
            ) && self.unify(&a, inner)
            {
                return;
            }
        }
        if self.unify(&a, &e) {
            return;
        }
        let (shown_a, shown_e) = (self.show(&a), self.show(&e));
        let diag = if a == Ty::Unit {
            Diagnostic::error(
                codes::MISSING_VALUE,
                format!("expected a value of type `{shown_e}`, but this produces no value"),
                span,
            )
        } else if let (Ty::Opt(inner), false) = (&a, matches!(e, Ty::Opt(_))) {
            let inner = self.show(inner);
            Diagnostic::error(
                codes::TYPE_MISMATCH,
                format!("expected `{shown_e}`, found `{shown_a}`, which may be `none`"),
                span,
            )
            .with_help(format!(
                "handle the `none` case first: `match`, `if x is some(v)`, or `x ?? default` to get a `{inner}`"
            ))
        } else {
            let mut diag = Diagnostic::error(
                codes::TYPE_MISMATCH,
                format!("expected `{shown_e}`, found `{shown_a}`"),
                span,
            );
            if self.is_prim(&e, "String") && !matches!(a, Ty::Opt(_)) {
                diag = diag
                    .with_help("convert with `.to_string()` or string interpolation: \"${value}\"");
            }
            diag
        };
        self.error(diag);
    }

    pub(crate) fn show(&self, ty: &Ty) -> String {
        show_type(&self.adts, &self.resolve(ty), &|t| self.resolve(t))
    }

    // ----- scopes -----

    pub(crate) fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// Binds `name`, written at `def`, in the innermost scope.
    pub(crate) fn bind(
        &mut self,
        name: &str,
        def: Span,
        ty: Ty,
        mutable: bool,
        let_span: Option<Span>,
    ) {
        self.record_binding(def, &ty);
        if name.is_empty() || name == "_" {
            return;
        }
        self.scopes.last_mut().unwrap().insert(
            name.to_string(),
            Local {
                ty,
                mutable,
                let_span,
                def,
            },
        );
    }

    // ----- the language server -----

    /// Records that the name at `span` refers to `target`.
    pub(crate) fn refer(&mut self, span: Span, target: Ref) {
        if self.recording {
            self.references.push((span, target));
        }
    }

    /// Whether `name` is where the editor wants completions.
    pub(crate) fn is_marker(&self, name: &str) -> bool {
        self.completing && name == COMPLETION_MARKER
    }

    pub(crate) fn complete(&mut self, completion: Completion) {
        if self.completion.is_none() {
            self.completion = Some(completion);
        }
    }

    /// The locals in scope, innermost first, each name once.
    fn visible_locals(&self) -> Vec<(String, Ty)> {
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for scope in self.scopes.iter().rev() {
            let mut names: Vec<(&String, &Local)> = scope.iter().collect();
            names.sort_by_key(|(_, l)| std::cmp::Reverse(l.def.start));
            for (name, local) in names {
                if seen.insert(name.clone()) {
                    out.push((name.clone(), local.ty.clone()));
                }
            }
        }
        out
    }

    /// Completions after `base.`.
    fn complete_member(&mut self, base: &Expr) {
        if let Some(m) = self.module_named(base) {
            self.refer(base.span, Ref::Module(m));
            self.complete(Completion::Module(m));
        } else if let Some(id) = self.type_named(base) {
            self.refer(base.span, Ref::Type(id));
            self.complete(Completion::TypeMembers(id));
        } else {
            let ty = self.infer(base, &Expect::Value(None));
            self.complete(Completion::Members(ty));
        }
    }

    /// Records what the names in a file's declarations are.
    fn refer_declarations(&mut self, module: &Module) {
        if !self.recording {
            return;
        }
        for item in &module.items {
            match item {
                Item::Fn(f) => self.refer(f.name.span, Ref::Fn(f.name.name.clone())),
                Item::Const(c) => self.refer(c.name.span, Ref::Const(c.name.name.clone())),
                Item::Struct(s) => {
                    let Some(&id) = self.global_types.get(&s.name.name) else {
                        continue;
                    };
                    self.refer(s.name.span, Ref::Type(id));
                    for f in &s.fields {
                        self.refer(f.name.span, Ref::Field(id, f.name.name.clone()));
                    }
                }
                Item::Enum(e) => {
                    let Some(&id) = self.global_types.get(&e.name.name) else {
                        continue;
                    };
                    self.refer(e.name.span, Ref::Type(id));
                    for v in &e.variants {
                        self.refer(v.name.span, Ref::Variant(id, v.name.name.clone()));
                    }
                }
                Item::Impl(imp) => {
                    let TypeKind::Named { path, .. } = &imp.target.kind else {
                        continue;
                    };
                    let Some(&id) = path.last().and_then(|n| self.global_types.get(&n.name)) else {
                        continue;
                    };
                    for m in &imp.methods {
                        self.refer(m.name.span, Ref::Method(id, m.name.name.clone()));
                    }
                }
                Item::Import(_) => {}
            }
        }
    }

    pub(crate) fn local(&self, name: &str) -> Option<&Local> {
        self.scopes.iter().rev().find_map(|s| s.get(name))
    }

    pub(crate) fn enter_fn(&mut self, ret: Ty, throws: Option<Ty>, bang_at: Option<Span>) {
        self.fns.push(FnCtx {
            ret,
            throws,
            infer_throws: None,
            bang_at,
            pending: Vec::new(),
        });
        self.push_scope();
    }

    pub(crate) fn exit_fn(&mut self) -> FnCtx {
        self.pop_scope();
        let ctx = self.fns.pop().unwrap();
        for (v, span, param, func) in &ctx.pending {
            if matches!(self.resolve(&Ty::Var(*v)), Ty::Var(_)) {
                self.error(
                    Diagnostic::error(
                        codes::CANNOT_INFER,
                        format!("cannot infer the type `{param}` for this call to `{func}`"),
                        *span,
                    )
                    .with_help(format!("write it out, as in `{func}<Post>(...)`")),
                );
            }
        }
        ctx
    }

    fn check_fn(&mut self, f: &FnDecl, self_ty: Option<Ty>, outer: &[String]) {
        let generics: Vec<String> = outer
            .iter()
            .cloned()
            .chain(f.generics.iter().map(|g| g.name.clone()))
            .collect();
        let ret = match &f.ret {
            Some(t) => self.resolve_type(t, &generics, Scope::Global),
            None => Ty::Unit,
        };
        let throws = self.resolve_throws(&f.throws, &generics, Scope::Global);
        let bang_after = match &f.ret {
            Some(t) => t.span,
            None => f.params.last().map(|p| p.span).unwrap_or(f.name.span),
        };
        // Insert just before the body's `{`.
        let bang_at = Span::new(f.body.span.start as usize, f.body.span.start as usize);
        let _ = bang_after;
        self.enter_fn(ret.clone(), throws, Some(bang_at));
        for (i, p) in f.params.iter().enumerate() {
            let ty = if i == 0 && p.name.name == "self" && p.ty.is_none() {
                self_ty.clone().unwrap_or(Ty::Unknown)
            } else {
                match &p.ty {
                    Some(t) => self.resolve_type(t, &generics, Scope::Global),
                    None => Ty::Unknown,
                }
            };
            if let Some(default) = &p.default {
                self.check(default, &ty);
            }
            self.bind(&p.name.name, p.name.span, ty, false, None);
        }
        let outer_params = std::mem::replace(&mut self.type_params, generics);
        self.check_body(&f.body, &ret);
        self.type_params = outer_params;
        self.exit_fn();
    }

    fn check_body(&mut self, body: &Block, ret: &Ty) {
        if *ret == Ty::Unit {
            self.check_block(body, &Expect::Discard);
        } else {
            let ty = self.check_block(body, &Expect::Value(Some(ret.clone())));
            let span = match body.stmts.last() {
                Some(Stmt::Expr(e)) => e.span,
                _ => Span::new(body.span.end as usize - 1, body.span.end as usize),
            };
            self.coerce(&ty, ret, span);
        }
    }

    // ----- statements -----

    pub(crate) fn check_block(&mut self, block: &Block, expect: &Expect) -> Ty {
        self.push_scope();
        let mut ty = Ty::Unit;
        let mut diverges = false;
        let last = block.stmts.len().saturating_sub(1);
        for (i, stmt) in block.stmts.iter().enumerate() {
            match stmt {
                Stmt::Expr(e) if i == last => ty = self.infer(e, expect),
                Stmt::Expr(e) => {
                    let t = self.infer(e, &Expect::Discard);
                    diverges |= self.resolve(&t) == Ty::Never;
                }
                Stmt::Return { .. } | Stmt::Break(_) | Stmt::Continue(_) => {
                    self.check_stmt(stmt);
                    diverges = true;
                }
                _ => self.check_stmt(stmt),
            }
        }
        self.pop_scope();
        if diverges {
            Ty::Never
        } else {
            ty
        }
    }

    fn check_stmt(&mut self, stmt: &Stmt) {
        match stmt {
            Stmt::Let {
                mutable,
                name,
                ty,
                value,
                span,
            } => {
                let ty = match ty {
                    Some(t) => {
                        let params = self.type_params.clone();
                        let ty = self.resolve_type(t, &params, Scope::Global);
                        self.check(value, &ty);
                        ty
                    }
                    None => {
                        let ty = self.infer(value, &Expect::Value(None));
                        if self.resolve(&ty) == Ty::Unit {
                            self.error(Diagnostic::error(
                                codes::MISSING_VALUE,
                                format!(
                                    "`{}` would have no value: this expression produces none",
                                    name.name
                                ),
                                value.span,
                            ));
                        }
                        ty
                    }
                };
                let keyword = Span::new(span.start as usize, span.start as usize + 3);
                self.bind(&name.name, name.span, ty, *mutable, Some(keyword));
            }
            Stmt::Assign {
                target, op, value, ..
            } => {
                if let ExprKind::Ident(name) = &target.kind {
                    if let Some(local) = self.local(name).cloned() {
                        if !local.mutable {
                            let mut diag = Diagnostic::error(
                                codes::ASSIGN_TO_IMMUTABLE,
                                format!("cannot assign to `{name}`: it was declared with `let`"),
                                target.span,
                            );
                            diag = match local.let_span {
                                Some(s) => diag.with_suggestion(
                                    format!("declare `{name}` with `var`"),
                                    s,
                                    "var",
                                ),
                                None => diag.with_help(
                                    "parameters cannot be reassigned; copy into a `var` first",
                                ),
                            };
                            self.error(diag);
                        }
                    }
                }
                let mut target_ty = self.infer(target, &Expect::Value(None));
                // `map[key]` reads a `V?`, but `map[key] = value` stores a `V`.
                if let ExprKind::Index { expr, .. } = &target.kind {
                    let base = self.expr_types.get(&expr.span).cloned();
                    if let Some(Ty::Adt(id, args)) = base.map(|b| self.resolve(&b)) {
                        if id == self.global_types["Map"] {
                            target_ty = args[1].clone();
                        }
                    }
                }
                if *op != AssignOp::Set {
                    let t = self.resolve(&target_ty);
                    let numeric = self.is_prim(&t, "Int") || self.is_prim(&t, "Float");
                    let ok = numeric || (*op == AssignOp::Add && self.is_prim(&t, "String"));
                    if !ok && !matches!(t, Ty::Unknown | Ty::Var(_)) {
                        let shown = self.show(&t);
                        self.error(Diagnostic::error(
                            codes::TYPE_MISMATCH,
                            format!("this operator does not work on `{shown}`"),
                            target.span,
                        ));
                    }
                }
                self.check(value, &target_ty);
            }
            Stmt::Return { value, span } => {
                let ret = self.fns.last().unwrap().ret.clone();
                match value {
                    Some(v) => self.check(v, &ret),
                    None => {
                        if self.resolve(&ret) != Ty::Unit && !matches!(ret, Ty::Var(_)) {
                            let shown = self.show(&ret);
                            self.error(Diagnostic::error(
                                codes::MISSING_VALUE,
                                format!(
                                    "this function returns `{shown}`, so `return` needs a value"
                                ),
                                *span,
                            ));
                        }
                    }
                }
            }
            Stmt::Break(_) | Stmt::Continue(_) => {}
            Stmt::For {
                binding,
                iter,
                filter,
                body,
                ..
            } => {
                let elem = self.element_type(iter);
                self.push_scope();
                self.bind(&binding.name, binding.span, elem, false, None);
                if let Some(f) = filter {
                    let bool_ty = self.prim("Bool");
                    self.check(f, &bool_ty);
                }
                self.check_block(body, &Expect::Discard);
                self.pop_scope();
            }
            Stmt::While { cond, body, .. } => {
                let bool_ty = self.prim("Bool");
                self.check(cond, &bool_ty);
                self.check_block(body, &Expect::Discard);
            }
            Stmt::Expr(e) => {
                self.infer(e, &Expect::Discard);
            }
        }
    }

    // ----- expressions -----

    pub(crate) fn check(&mut self, e: &Expr, expected: &Ty) {
        let ty = self.infer(e, &Expect::Value(Some(expected.clone())));
        self.coerce(&ty, expected, e.span);
    }

    fn expected_ty(&self, expect: &Expect) -> Option<Ty> {
        match expect {
            Expect::Value(Some(t)) => Some(self.resolve(t)),
            _ => None,
        }
    }

    pub(crate) fn infer(&mut self, e: &Expr, expect: &Expect) -> Ty {
        let ty = self.infer_kind(e, expect);
        self.expr_types.insert(e.span, ty.clone());
        ty
    }

    pub(crate) fn record_binding(&mut self, span: Span, ty: &Ty) {
        self.binding_types.insert(span, ty.clone());
    }

    /// Resolves every inference variable in the recorded types.
    pub(crate) fn zonk_recorded(&mut self) {
        let exprs: Vec<(Span, Ty)> = self.expr_types.drain().collect();
        for (span, ty) in exprs {
            let ty = self.zonk(&ty);
            self.expr_types.insert(span, ty);
        }
        let bindings: Vec<(Span, Ty)> = self.binding_types.drain().collect();
        for (span, ty) in bindings {
            let ty = self.zonk(&ty);
            self.binding_types.insert(span, ty);
        }
        let uses: Vec<(Span, HashMap<String, Ty>)> = self.instantiations.drain().collect();
        for (span, map) in uses {
            let map = map.into_iter().map(|(k, t)| (k, self.zonk(&t))).collect();
            self.instantiations.insert(span, map);
        }
    }

    fn infer_kind(&mut self, e: &Expr, expect: &Expect) -> Ty {
        match self.expected_ty(expect) {
            Some(Ty::Style(pos)) => {
                if let Some(ty) = self.infer_style(e, &pos) {
                    return ty;
                }
            }
            // A record where a `Ui` is expected is a style.
            Some(ui) if self.is_ui(&ui) => {
                if let ExprKind::Record(fields) = &e.kind {
                    let pos = crate::style::StylePos::Group {
                        state: None,
                        at: None,
                    };
                    self.infer_style_record(fields, &pos, e.span);
                    return ui;
                }
            }
            _ => {}
        }
        match &e.kind {
            ExprKind::Int(_) => self.prim("Int"),
            ExprKind::Float(_) => self.prim("Float"),
            ExprKind::Duration(_) => self.prim("Duration"),
            ExprKind::Bool(_) => self.prim("Bool"),
            ExprKind::Str(pieces) => {
                for piece in pieces {
                    if let StrPiece::Expr(inner) = piece {
                        self.infer(inner, &Expect::Value(None));
                    }
                }
                self.prim("String")
            }
            ExprKind::None => match self.expected_ty(expect) {
                Some(Ty::Opt(inner)) => Ty::Opt(inner),
                _ => {
                    let v = self.fresh();
                    Ty::Opt(Box::new(v))
                }
            },
            ExprKind::Ident(name) => self.infer_ident(name, e.span),
            ExprKind::List(items) => {
                let elem = match self.expected_ty(expect) {
                    Some(Ty::Adt(id, args)) if id == self.global_types["List"] => args[0].clone(),
                    _ if items.is_empty() => self.fresh(),
                    // The items share one type, as the branches of an `if`
                    // do: `[3, none]` is a `List<Int?>`.
                    _ => {
                        let tys = items
                            .iter()
                            .map(|item| (self.infer(item, &Expect::Value(None)), item.span))
                            .collect();
                        let elem = self.join(tys, &Expect::Value(None));
                        return self.list_of(elem);
                    }
                };
                for item in items {
                    self.check(item, &elem);
                }
                self.list_of(elem)
            }
            ExprKind::Record(fields) => {
                let mut tys = Vec::new();
                for f in fields {
                    let ty = match &f.value {
                        Some(v) => self.infer(v, &Expect::Value(None)),
                        None => self.infer_ident(&f.name.name, f.name.span),
                    };
                    tys.push((f.name.name.clone(), ty));
                }
                Ty::Record(tys)
            }
            ExprKind::Unary { op, expr } => {
                let ty = match op {
                    UnaryOp::Neg => self.infer(expr, &Expect::Value(None)),
                    UnaryOp::Not => {
                        let b = self.prim("Bool");
                        self.check(expr, &b);
                        b
                    }
                };
                if *op == UnaryOp::Neg {
                    let t = self.resolve(&ty);
                    if !(self.is_prim(&t, "Int") || self.is_prim(&t, "Float") || t == Ty::Unknown) {
                        let shown = self.show(&t);
                        self.error(Diagnostic::error(
                            codes::TYPE_MISMATCH,
                            format!("cannot negate a `{shown}`"),
                            e.span,
                        ));
                        return Ty::Unknown;
                    }
                }
                ty
            }
            ExprKind::Binary { op, lhs, rhs } => self.infer_binary(*op, lhs, rhs, e.span),
            ExprKind::Call { .. } => {
                let (ty, throws) = self.infer_call(e);
                if throws.is_some() {
                    self.error(
                        Diagnostic::error(
                            codes::MISSING_TRY,
                            "this call can fail, so it needs `try` or `catch`",
                            e.span,
                        )
                        .with_help("`try` passes the error to the caller; `catch err { ... }` handles it here")
                        .with_suggestion(
                            "add `try`",
                            Span::new(e.span.start as usize, e.span.start as usize),
                            "try ",
                        ),
                    );
                }
                ty
            }
            ExprKind::Field { expr, name } => self.infer_field(expr, name, e.span),
            ExprKind::Index { expr, index } => {
                let base = self.infer(expr, &Expect::Value(None));
                match self.resolve(&base) {
                    Ty::Adt(id, args) if id == self.global_types["List"] => {
                        let int = self.prim("Int");
                        self.check(index, &int);
                        args[0].clone()
                    }
                    Ty::Adt(id, args) if id == self.global_types["Map"] => {
                        self.check(index, &args[0]);
                        Ty::Opt(Box::new(args[1].clone()))
                    }
                    Ty::Unknown => Ty::Unknown,
                    other => {
                        let shown = self.show(&other);
                        self.error(Diagnostic::error(
                            codes::TYPE_MISMATCH,
                            format!("cannot index into a `{shown}`"),
                            e.span,
                        ));
                        Ty::Unknown
                    }
                }
            }
            ExprKind::FieldShorthand(name) => {
                let Some(Ty::Fn(f)) = self.expected_ty(expect) else {
                    self.error(
                        Diagnostic::error(
                            codes::CANNOT_INFER,
                            format!("cannot tell which type `.{}` belongs to here", name.name),
                            e.span,
                        )
                        .with_help(format!(
                            "write a closure instead: `fn(x: Type) {{ x.{} }}`",
                            name.name
                        )),
                    );
                    return Ty::Unknown;
                };
                let Some(param) = f.params.first().cloned() else {
                    return Ty::Unknown;
                };
                let ret = self.member_type(&param, name, e.span, true);
                Ty::Fn(Rc::new(FnTy {
                    params: vec![param],
                    ret,
                    throws: None,
                }))
            }
            ExprKind::Closure { params, ret, body } => {
                self.infer_closure(params, ret, body, expect, e.span)
            }
            ExprKind::If {
                cond,
                then,
                otherwise,
            } => {
                self.push_scope();
                let b = self.prim("Bool");
                self.check(cond, &b);
                let then_ty = self.check_block(then, expect);
                self.pop_scope();
                let Some(other) = otherwise else {
                    return Ty::Unit;
                };
                let else_ty = self.infer(other, expect);
                if matches!(expect, Expect::Discard) {
                    return Ty::Unit;
                }
                self.join(vec![(then_ty, then.span), (else_ty, other.span)], expect)
            }
            ExprKind::Block(block) => self.check_block(block, expect),
            ExprKind::Match { scrutinee, arms } => {
                self.infer_match(scrutinee, arms, expect, e.span)
            }
            ExprKind::Try(inner) => {
                if !matches!(inner.kind, ExprKind::Call { .. }) {
                    let ty = self.infer(inner, expect);
                    self.nothing_to_try(e.span, inner.span);
                    return ty;
                }
                let (ty, throws) = self.infer_call(inner);
                match throws {
                    Some(err) => self.propagate(&err, e.span),
                    // An unresolved callee was already reported.
                    None if self.resolve(&ty) == Ty::Unknown => {}
                    None => self.nothing_to_try(e.span, inner.span),
                }
                ty
            }
            ExprKind::Catch {
                expr,
                binding,
                handler,
            } => {
                let (ty, throws) = if matches!(expr.kind, ExprKind::Call { .. }) {
                    self.infer_call(expr)
                } else {
                    (self.infer(expr, &Expect::Value(None)), None)
                };
                let err = match throws {
                    Some(err) => err,
                    None if self.resolve(&ty) == Ty::Unknown => Ty::Unknown,
                    None => {
                        self.error(Diagnostic::error(
                            codes::NOTHING_TO_TRY,
                            "this cannot fail, so there is nothing to `catch`",
                            expr.span,
                        ));
                        Ty::Unknown
                    }
                };
                self.push_scope();
                if let Some(b) = binding {
                    self.bind(&b.name, b.span, err, false, None);
                }
                let handled = self.check_block(handler, &Expect::Value(Some(ty.clone())));
                self.pop_scope();
                self.coerce(&handled, &ty, handler.span);
                ty
            }
            ExprKind::Throw(value) => {
                let err = self.infer(value, &Expect::Value(None));
                self.propagate(&err, e.span);
                Ty::Never
            }
            ExprKind::Is { expr, pattern } => {
                let ty = self.infer(expr, &Expect::Value(None));
                self.check_pattern(pattern, &ty);
                self.prim("Bool")
            }
            ExprKind::Error => Ty::Unknown,
        }
    }

    /// The common type of the branches of an `if` or `match`. It is
    /// optional when the surrounding code expects an optional or when a
    /// branch is optional, so `none` may come after a value.
    fn join(&mut self, branches: Vec<(Ty, Span)>, expect: &Expect) -> Ty {
        let branches: Vec<(Ty, Span)> = branches
            .into_iter()
            .filter(|(ty, _)| self.resolve(ty) != Ty::Never)
            .collect();
        let Some((first, _)) = branches.first() else {
            return Ty::Never;
        };
        let first = self.resolve(first);
        let expected = self.expected_ty(expect).map(|t| self.resolve(&t));
        let result = match expected {
            Some(t @ Ty::Opt(_)) => t,
            _ if !matches!(first, Ty::Opt(_))
                && branches
                    .iter()
                    .any(|(ty, _)| matches!(self.resolve(ty), Ty::Opt(_))) =>
            {
                Ty::Opt(Box::new(first))
            }
            _ => first,
        };
        for (ty, span) in branches {
            self.coerce(&ty, &result, span);
        }
        result
    }

    fn nothing_to_try(&mut self, span: Span, inner: Span) {
        self.error(
            Diagnostic::error(
                codes::NOTHING_TO_TRY,
                "this cannot fail, so `try` is not needed",
                span,
            )
            .with_suggestion(
                "remove `try`",
                Span::new(span.start as usize, inner.start as usize),
                "",
            ),
        );
    }

    /// Passes an error of type `err` to the enclosing function's caller.
    fn propagate(&mut self, err: &Ty, span: Span) {
        let error_ty = self.error_ty();
        let ctx = self.fns.last_mut().unwrap();
        if let Some(used) = &mut ctx.infer_throws {
            *used = true;
            return;
        }
        match ctx.throws.clone() {
            None => {
                let bang_at = ctx.bang_at;
                let mut diag = Diagnostic::error(
                    codes::CANNOT_FAIL,
                    "this passes an error to the caller, but the enclosing function cannot fail",
                    span,
                );
                diag = match bang_at {
                    Some(at) => diag
                        .with_help("add `!` to the signature, or handle the error with `catch`")
                        .with_suggestion("make the function fallible", at, "! "),
                    None => diag.with_help(
                        "this closure is expected not to fail; handle the error with `catch`",
                    ),
                };
                self.error(diag);
            }
            Some(expected) => {
                if self.unify(&expected, &error_ty) {
                    return;
                }
                if !self.unify(err, &expected) {
                    let (shown_err, shown_expected) = (self.show(err), self.show(&expected));
                    self.error(
                        Diagnostic::error(
                            codes::ERROR_TYPE_MISMATCH,
                            format!("this fails with `{shown_err}`, but the function can only fail with `{shown_expected}`"),
                            span,
                        )
                        .with_help("handle it with `catch`, or declare the function with a bare `!` to accept any error"),
                    );
                }
            }
        }
    }

    fn infer_binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr, span: Span) -> Ty {
        let bool_ty = self.prim("Bool");
        match op {
            BinaryOp::And | BinaryOp::Or => {
                self.check(lhs, &bool_ty);
                self.check(rhs, &bool_ty);
                bool_ty
            }
            BinaryOp::Eq | BinaryOp::Ne => {
                let l = self.infer(lhs, &Expect::Value(None));
                let r = self.infer(rhs, &Expect::Value(Some(l.clone())));
                let (lr, rr) = (self.resolve(&l), self.resolve(&r));
                // Comparing a `T?` with a `T` is allowed.
                let ok = match (&lr, &rr) {
                    (Ty::Opt(inner), other) | (other, Ty::Opt(inner))
                        if !matches!(other, Ty::Opt(_)) =>
                    {
                        self.unify(inner, other)
                    }
                    _ => self.unify(&lr, &rr),
                };
                if !ok {
                    let (a, b) = (self.show(&lr), self.show(&rr));
                    self.error(Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        format!("cannot compare `{a}` with `{b}`"),
                        span,
                    ));
                }
                bool_ty
            }
            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                let l = self.infer(lhs, &Expect::Value(None));
                self.check(rhs, &l);
                let lr = self.resolve(&l);
                let ordered = ["Int", "Float", "String", "Duration"]
                    .iter()
                    .any(|p| self.is_prim(&lr, p));
                if !ordered && !matches!(lr, Ty::Unknown | Ty::Var(_)) {
                    let shown = self.show(&lr);
                    self.error(Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        format!("`{shown}` values cannot be ordered with `<` or `>`"),
                        span,
                    ));
                }
                bool_ty
            }
            BinaryOp::Coalesce => {
                let l = self.infer(lhs, &Expect::Value(None));
                match self.resolve(&l) {
                    Ty::Opt(inner) => {
                        self.check(rhs, &inner);
                        *inner
                    }
                    Ty::Unknown => {
                        self.infer(rhs, &Expect::Value(None));
                        Ty::Unknown
                    }
                    other => {
                        let shown = self.show(&other);
                        self.error(
                            Diagnostic::error(
                                codes::TYPE_MISMATCH,
                                format!("`??` needs an optional on the left, found `{shown}`"),
                                lhs.span,
                            )
                            .with_help("a value that cannot be `none` needs no default"),
                        );
                        self.infer(rhs, &Expect::Value(None));
                        other
                    }
                }
            }
            BinaryOp::Range => {
                let int = self.prim("Int");
                self.check(lhs, &int);
                self.check(rhs, &int);
                self.list_of(int)
            }
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div | BinaryOp::Rem => {
                let l = self.infer(lhs, &Expect::Value(None));
                let r = self.infer(rhs, &Expect::Value(Some(l.clone())));
                let (lr, rr) = (self.resolve(&l), self.resolve(&r));
                if matches!(lr, Ty::Unknown) || matches!(rr, Ty::Unknown) {
                    return Ty::Unknown;
                }
                let numeric = |c: &Self, t: &Ty| c.is_prim(t, "Int") || c.is_prim(t, "Float");
                let ok = self.unify(&lr, &rr)
                    && (numeric(self, &lr)
                        || (op == BinaryOp::Add && self.is_prim(&lr, "String"))
                        || (matches!(op, BinaryOp::Add | BinaryOp::Sub)
                            && self.is_prim(&lr, "Duration")));
                if !ok {
                    let (a, b) = (self.show(&lr), self.show(&rr));
                    let verb = match op {
                        BinaryOp::Add => "add",
                        BinaryOp::Sub => "subtract",
                        BinaryOp::Mul => "multiply",
                        BinaryOp::Div => "divide",
                        _ => "take the remainder of",
                    };
                    let mut diag = Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        format!("cannot {verb} `{a}` and `{b}`"),
                        span,
                    );
                    if op == BinaryOp::Add
                        && (self.is_prim(&lr, "String") || self.is_prim(&rr, "String"))
                    {
                        diag = diag.with_help("build strings with interpolation: \"${a}${b}\"");
                    } else if numeric(self, &lr) && numeric(self, &rr) {
                        diag =
                            diag.with_help("convert explicitly with `.to_float()` or `.round()`");
                    }
                    self.error(diag);
                    return Ty::Unknown;
                }
                lr
            }
        }
    }

    // ----- names, fields and calls -----

    pub(crate) fn infer_ident(&mut self, name: &str, span: Span) -> Ty {
        if self.is_marker(name) {
            let locals = self.visible_locals();
            self.complete(Completion::Names(locals));
            return Ty::Unknown;
        }
        if let Some(local) = self.local(name) {
            let (ty, def) = (local.ty.clone(), local.def);
            self.refer(span, Ref::Local(def));
            return ty;
        }
        if let Some(sig) = self.global_fns.get(name).cloned() {
            self.refer(span, Ref::Fn(name.to_string()));
            let (params, ret, throws) = self.instantiate(&sig, &[], &[], span);
            return Ty::Fn(Rc::new(FnTy {
                params: params.into_iter().map(|p| p.ty).collect(),
                ret,
                throws,
            }));
        }
        if let Some(ty) = self.consts.get(name).cloned() {
            self.refer(span, Ref::Const(name.to_string()));
            return ty;
        }
        if let Some(&id) = self.global_types.get(name) {
            self.refer(span, Ref::Type(id));
            self.error(
                Diagnostic::error(
                    codes::NOT_CALLABLE,
                    format!("`{name}` is a type, not a value"),
                    span,
                )
                .with_help(format!("create a value with `{name}(field: value, ...)`")),
            );
            return Ty::Unknown;
        }
        if let Some(&m) = self.imports.get(name) {
            self.refer(span, Ref::Module(m));
            self.error(
                Diagnostic::error(
                    codes::NOT_CALLABLE,
                    format!("`{name}` is a module, not a value"),
                    span,
                )
                .with_help(format!(
                    "use one of its functions, as in `{name}.something(...)`"
                )),
            );
            return Ty::Unknown;
        }
        self.undefined(name, span);
        Ty::Unknown
    }

    fn undefined(&mut self, name: &str, span: Span) {
        let mut diag = Diagnostic::error(
            codes::UNDEFINED_NAME,
            format!("`{name}` is not defined"),
            span,
        );
        if self.modules.iter().any(|m| m.name == name) {
            diag = Diagnostic::error(
                codes::UNDEFINED_NAME,
                format!("the module `{name}` is not imported"),
                span,
            )
            .with_suggestion(
                format!("add `import {name}` at the top of the file"),
                Span::new(0, 0),
                format!("import {name}\n"),
            );
        } else {
            let mut candidates: Vec<String> =
                self.scopes.iter().flat_map(|s| s.keys().cloned()).collect();
            candidates.extend(self.global_fns.keys().cloned());
            candidates.extend(self.consts.keys().cloned());
            candidates.extend(self.imports.keys().cloned());
            if let Some(s) = similar(name, candidates.iter().map(String::as_str)) {
                diag = diag.with_suggestion(format!("did you mean `{s}`?"), span, s);
            }
        }
        self.error(diag);
    }

    /// A module referred to by a bare name, unless a local shadows it.
    pub(crate) fn module_named(&self, e: &Expr) -> Option<ModId> {
        match &e.kind {
            ExprKind::Ident(name) if self.local(name).is_none() => self.imports.get(name).copied(),
            _ => None,
        }
    }

    /// A type referred to by name: `Role` or `web.Response`.
    fn type_named(&self, e: &Expr) -> Option<AdtId> {
        match &e.kind {
            ExprKind::Ident(name)
                if self.local(name).is_none() && !self.global_fns.contains_key(name) =>
            {
                self.global_types.get(name).copied()
            }
            ExprKind::Field { expr, name } => {
                let m = self.module_named(expr)?;
                self.modules[m].types.get(&name.name).copied()
            }
            _ => None,
        }
    }

    /// Records what the type name `Role` or `web.Response` refers to.
    fn refer_type_name(&mut self, e: &Expr, id: AdtId) {
        match &e.kind {
            ExprKind::Field { expr, name } => {
                if let Some(m) = self.module_named(expr) {
                    self.refer(expr.span, Ref::Module(m));
                }
                self.refer(name.span, Ref::Type(id));
            }
            _ => self.refer(e.span, Ref::Type(id)),
        }
    }

    /// Reports a server-only module used in browser code, and a browser
    /// one used outside a handler.
    fn use_module(&mut self, m: ModId, span: Span) {
        let name = &self.modules[m].name;
        if !self.in_handler
            && !self.browser_program
            && (BROWSER_MODULES.contains(&name.as_str()) || self.modules[m].foreign)
        {
            let message = format!(
                "`{name}` runs only in the browser, so only an ember's `on:` handlers can use it"
            );
            self.error(
                Diagnostic::error(codes::BROWSER_ONLY, message, span).with_help(
                    "call it from a handler, and keep what it gives in `{% state %}` to show it",
                ),
            );
            return;
        }
        let name = &self.modules[m].name;
        if self.in_ember && SERVER_ONLY_MODULES.contains(&name.as_str()) {
            let message =
                format!("`{name}` works only on the server, but this ember runs in the browser");
            self.error(
                Diagnostic::error(codes::SERVER_ONLY, message, span)
                    .with_help("move this into a `server fn` in a `.kiln` file and call that"),
            );
        }
    }

    fn infer_field(&mut self, base: &Expr, name: &Ident, span: Span) -> Ty {
        if self.is_marker(&name.name) {
            self.complete_member(base);
            return Ty::Unknown;
        }
        if let Some(m) = self.module_named(base) {
            self.refer(base.span, Ref::Module(m));
            self.use_module(m, base.span);
            let module = &self.modules[m];
            if module.fns.contains_key(&name.name) {
                self.error(Diagnostic::error(
                    codes::NOT_CALLABLE,
                    format!(
                        "`{}.{}` is a function; call it with `(...)`",
                        module.name, name.name
                    ),
                    span,
                ));
            } else {
                self.no_member_in_module(m, name);
            }
            return Ty::Unknown;
        }
        if let Some(id) = self.type_named(base) {
            self.refer_type_name(base, id);
            return self.variant_value(id, name, span);
        }
        let base_ty = self.infer(base, &Expect::Value(None));
        self.member_type(&base_ty, name, span, false)
    }

    fn no_member_in_module(&mut self, m: ModId, name: &Ident) {
        let module = &self.modules[m];
        let candidates: Vec<String> = module
            .fns
            .keys()
            .chain(module.types.keys())
            .cloned()
            .collect();
        let mut diag = Diagnostic::error(
            codes::NO_SUCH_MEMBER,
            format!("the module `{}` has no `{}`", module.name, name.name),
            name.span,
        );
        if let Some(why) = module.unsupported.get(&name.name) {
            diag = Diagnostic::error(
                codes::NO_SUCH_MEMBER,
                format!(
                    "the npm package `{}` exports `{}`, but Kiln cannot call it: {why}",
                    module.name, name.name
                ),
                name.span,
            )
            .with_help(
                "use `js.global(...)` and `js.Value` for it, or a package with simpler types",
            );
        } else if let Some(s) = similar(&name.name, candidates.iter().map(String::as_str)) {
            diag = diag.with_suggestion(format!("did you mean `{s}`?"), name.span, s);
        }
        self.error(diag);
    }

    /// `Role.Admin` as a value.
    fn variant_value(&mut self, id: AdtId, name: &Ident, span: Span) -> Ty {
        let adt = self.adts[id].clone();
        let AdtKind::Enum(variants) = &adt.kind else {
            self.error(Diagnostic::error(
                codes::NO_SUCH_MEMBER,
                format!(
                    "`{}` is a struct; create one with `{}(...)`",
                    adt.name, adt.name
                ),
                span,
            ));
            return Ty::Unknown;
        };
        if variants.iter().any(|v| v.name == name.name) {
            self.refer(name.span, Ref::Variant(id, name.name.clone()));
        }
        match variants.iter().find(|v| v.name == name.name) {
            Some(v) if v.fields.is_empty() => {
                let args = adt.params.iter().map(|_| self.fresh()).collect();
                Ty::Adt(id, args)
            }
            Some(_) => {
                self.error(Diagnostic::error(
                    codes::BAD_ARGUMENTS,
                    format!("the variant `{}.{}` needs arguments", adt.name, name.name),
                    span,
                ));
                Ty::Unknown
            }
            None => {
                self.no_variant(&adt, variants, name);
                Ty::Unknown
            }
        }
    }

    fn no_variant(&mut self, adt: &AdtDef, variants: &[VariantDef], name: &Ident) {
        let mut diag = Diagnostic::error(
            codes::NO_SUCH_MEMBER,
            format!("`{}` has no variant `{}`", adt.name, name.name),
            name.span,
        );
        if let Some(s) = similar(&name.name, variants.iter().map(|v| v.name.as_str())) {
            diag = diag.with_suggestion(format!("did you mean `{s}`?"), name.span, s);
        }
        self.error(diag);
    }

    /// The type of `base.name`. With `call_methods`, a method taking no
    /// arguments counts as its result (for `.upper` shorthands).
    fn member_type(&mut self, base: &Ty, name: &Ident, span: Span, call_methods: bool) -> Ty {
        match self.resolve(base) {
            Ty::Unknown => Ty::Unknown,
            Ty::Var(_) => {
                self.error(
                    Diagnostic::error(
                        codes::CANNOT_INFER,
                        format!(
                            "cannot tell the type of this value, so `.{}` cannot be looked up",
                            name.name
                        ),
                        span,
                    )
                    .with_help(
                        "add a type annotation, such as `fn(n: Int)` or `let x: Type = ...`",
                    ),
                );
                Ty::Unknown
            }
            Ty::Record(fields) => match fields.iter().find(|(n, _)| *n == name.name) {
                Some((_, t)) => t.clone(),
                None => {
                    self.no_field(
                        &self.show(base),
                        fields.iter().map(|(n, _)| n.clone()).collect(),
                        name,
                    );
                    Ty::Unknown
                }
            },
            Ty::Opt(_) => {
                let shown = self.show(base);
                self.error(
                    Diagnostic::error(
                        codes::NO_SUCH_MEMBER,
                        format!(
                            "this value is `{shown}` and may be `none`, so it has no `.{}`",
                            name.name
                        ),
                        span,
                    )
                    .with_help(
                        "handle `none` first: `match`, `if x is some(v)`, or `x ?? default`",
                    ),
                );
                Ty::Unknown
            }
            Ty::Adt(id, args) => {
                let adt = self.adts[id].clone();
                let map: HashMap<String, Ty> = adt
                    .params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect();
                if let AdtKind::Struct(fields) = &adt.kind {
                    if let Some(f) = fields.iter().find(|f| f.name == name.name) {
                        self.refer(name.span, Ref::Field(id, name.name.clone()));
                        return substitute(&f.ty, &map);
                    }
                }
                if let Some(m) = adt.methods.iter().find(|m| m.name == name.name) {
                    self.refer(name.span, Ref::Method(id, name.name.clone()));
                    if call_methods && m.params.is_empty() {
                        let (_, ret, _) = self.instantiate(m, &args, &[], span);
                        return ret;
                    }
                    self.error(
                        Diagnostic::error(
                            codes::NOT_CALLABLE,
                            format!("`{}` is a method; call it with `()`", name.name),
                            span,
                        )
                        .with_suggestion(
                            "add `()`",
                            Span::new(span.end as usize, span.end as usize),
                            "()",
                        ),
                    );
                    return Ty::Unknown;
                }
                let mut candidates: Vec<String> =
                    adt.methods.iter().map(|m| m.name.clone()).collect();
                if let AdtKind::Struct(fields) = &adt.kind {
                    candidates.extend(fields.iter().map(|f| f.name.clone()));
                }
                self.no_field(&self.show(base), candidates, name);
                Ty::Unknown
            }
            other => {
                let shown = self.show(&other);
                self.error(Diagnostic::error(
                    codes::NO_SUCH_MEMBER,
                    format!("`{shown}` has no field `{}`", name.name),
                    name.span,
                ));
                Ty::Unknown
            }
        }
    }

    fn no_field(&mut self, owner: &str, candidates: Vec<String>, name: &Ident) {
        let mut diag = Diagnostic::error(
            codes::NO_SUCH_MEMBER,
            format!("`{owner}` has no field or method `{}`", name.name),
            name.span,
        );
        if let Some(s) = similar(&name.name, candidates.iter().map(String::as_str)) {
            diag = diag.with_suggestion(format!("did you mean `{s}`?"), name.span, s);
        }
        self.error(diag);
    }

    /// Instantiates a signature's type parameters, returning its parameters,
    /// return type and error type.
    fn instantiate(
        &mut self,
        sig: &FnSig,
        receiver_args: &[Ty],
        explicit: &[Ty],
        span: Span,
    ) -> (Vec<ParamSig>, Ty, Option<Ty>) {
        let mut map = HashMap::new();
        for (i, p) in sig.impl_params.iter().enumerate() {
            let ty = receiver_args
                .get(i)
                .cloned()
                .unwrap_or_else(|| self.fresh());
            map.insert(p.clone(), ty);
        }
        if !explicit.is_empty() && explicit.len() != sig.generics.len() {
            self.error(Diagnostic::error(
                codes::BAD_ARGUMENTS,
                format!(
                    "`{}` takes {} type argument(s), but {} were given",
                    sig.name,
                    sig.generics.len(),
                    explicit.len()
                ),
                span,
            ));
        }
        for (i, g) in sig.generics.iter().enumerate() {
            let ty = match explicit.get(i) {
                Some(t) => t.clone(),
                None => {
                    let v = self.fresh();
                    if let (Ty::Var(id), Some(ctx)) = (&v, self.fns.last_mut()) {
                        ctx.pending.push((*id, span, g.clone(), sig.name.clone()));
                    }
                    v
                }
            };
            map.insert(g.clone(), ty);
        }
        let params = sig
            .params
            .iter()
            .map(|p| ParamSig {
                name: p.name.clone(),
                ty: substitute(&p.ty, &map),
                has_default: p.has_default,
            })
            .collect();
        if !map.is_empty() {
            self.instantiations.insert(span, map.clone());
        }
        (
            params,
            substitute(&sig.ret, &map),
            sig.throws.as_ref().map(|t| substitute(t, &map)),
        )
    }

    /// Infers a call, returning its result type and, if the callee can fail,
    /// its error type. The caller decides whether `try` was required.
    pub(crate) fn infer_call(&mut self, e: &Expr) -> (Ty, Option<Ty>) {
        let ExprKind::Call {
            callee,
            type_args,
            args,
        } = &e.kind
        else {
            unreachable!("infer_call on a non-call");
        };
        let params = self.type_params.clone();
        let explicit: Vec<Ty> = type_args
            .iter()
            .map(|t| self.resolve_type(t, &params, Scope::Global))
            .collect();
        let fn_name = match &callee.kind {
            ExprKind::Ident(n) => n.clone(),
            ExprKind::Field { name, .. } => name.name.clone(),
            _ => "function".into(),
        };

        if let Some(result) = self.infer_ui_call(callee, args, e.span) {
            return result;
        }

        // `module.function(...)` or `module.Type(...)`
        if let ExprKind::Field { expr: base, name } = &callee.kind {
            if self.is_marker(&name.name) {
                self.complete_member(base);
                self.infer_args_loosely(args);
                return (Ty::Unknown, None);
            }
            if let Some(m) = self.module_named(base) {
                self.refer(base.span, Ref::Module(m));
                if self.modules[m].fns.contains_key(&name.name) {
                    self.refer(name.span, Ref::ModuleFn(m, name.name.clone()));
                } else if let Some(&id) = self.modules[m].types.get(&name.name) {
                    self.refer(name.span, Ref::Type(id));
                }
                self.use_module(m, base.span);
                if self.modules[m].name == "web" && name.name == "render" && self.glazes.is_some() {
                    let ret = self.modules[m].fns["render"].ret.clone();
                    self.check_render(args, e.span);
                    return (ret, None);
                }
                if let Some(sig) = self.modules[m].fns.get(&name.name).cloned() {
                    let (params, ret, throws) = self.instantiate(&sig, &[], &explicit, e.span);
                    self.check_args(&params, args, e.span, &fn_name, None);
                    return (ret, throws);
                }
                if let Some(id) = self.modules[m].types.get(&name.name).copied() {
                    return (self.construct(id, &explicit, args, e.span), None);
                }
                self.no_member_in_module(m, name);
                self.infer_args_loosely(args);
                return (Ty::Unknown, None);
            }
            if let Some(id) = self.type_named(base) {
                self.refer_type_name(base, id);
                // `Type.function(...)`, for methods without `self`
                let sig = self.adts[id]
                    .methods
                    .iter()
                    .find(|m| m.name == name.name && !m.has_self)
                    .cloned();
                if let Some(sig) = sig {
                    self.refer(name.span, Ref::Method(id, name.name.clone()));
                    let (params, ret, throws) = self.instantiate(&sig, &[], &explicit, e.span);
                    self.check_args(&params, args, e.span, &fn_name, None);
                    return (ret, throws);
                }
                // `Enum.Variant(...)`
                return (self.construct_variant(id, name, args, e.span), None);
            }
        }

        if let ExprKind::Ident(name) = &callee.kind {
            if self.local(name).is_none() {
                if let Some(sig) = self.global_fns.get(name).cloned() {
                    self.refer(callee.span, Ref::Fn(name.clone()));
                    let (params, ret, mut throws) = self.instantiate(&sig, &[], &explicit, e.span);
                    self.check_args(&params, args, e.span, &fn_name, None);
                    if self.in_handler && throws.is_none() && self.server_fns.contains(name) {
                        throws = Some(self.error_ty());
                    }
                    return (ret, throws);
                }
                if let Some(id) = self.global_types.get(name).copied() {
                    self.refer(callee.span, Ref::Type(id));
                    return (self.construct(id, &explicit, args, e.span), None);
                }
            }
        }

        // `value.method(...)`
        if let ExprKind::Field { expr: base, name } = &callee.kind {
            let base_ty = self.infer(base, &Expect::Value(None));
            if let Ty::Adt(id, recv_args) = self.resolve(&base_ty) {
                let adt = self.adts[id].clone();
                if let Some(sig) = adt.methods.iter().find(|m| m.name == name.name) {
                    self.refer(name.span, Ref::Method(id, name.name.clone()));
                    if !sig.has_self {
                        let call = format!("{}.{}", adt.name, sig.name);
                        self.error(
                            Diagnostic::error(
                                codes::NO_SUCH_MEMBER,
                                format!(
                                    "`{}` has no `self` parameter, so it is called on the type",
                                    sig.name
                                ),
                                callee.span,
                            )
                            .with_suggestion(
                                format!("call `{call}(...)`"),
                                callee.span,
                                call,
                            ),
                        );
                    }
                    let (params, ret, throws) =
                        self.instantiate(sig, &recv_args, &explicit, e.span);
                    let route = self.route_handler_type(id, &name.name, args);
                    self.check_args(&params, args, e.span, &fn_name, route);
                    return (ret, throws);
                }
            }
            let field_ty = self.member_type(&base_ty, name, callee.span, false);
            return self.call_value(&field_ty, args, e.span, &fn_name);
        }

        let callee_ty = self.infer(callee, &Expect::Value(None));
        self.call_value(&callee_ty, args, e.span, &fn_name)
    }

    fn infer_args_loosely(&mut self, args: &[Arg]) {
        for a in args {
            self.infer(&a.value, &Expect::Value(None));
        }
    }

    fn call_value(
        &mut self,
        callee: &Ty,
        args: &[Arg],
        span: Span,
        fn_name: &str,
    ) -> (Ty, Option<Ty>) {
        match self.resolve(callee) {
            Ty::Fn(f) => {
                let params: Vec<ParamSig> = f
                    .params
                    .iter()
                    .map(|t| ParamSig {
                        name: None,
                        ty: t.clone(),
                        has_default: false,
                    })
                    .collect();
                self.check_args(&params, args, span, fn_name, None);
                (f.ret.clone(), f.throws.clone())
            }
            Ty::Unknown => {
                self.infer_args_loosely(args);
                (Ty::Unknown, None)
            }
            other => {
                let shown = self.show(&other);
                self.error(Diagnostic::error(
                    codes::NOT_CALLABLE,
                    format!("a `{shown}` cannot be called"),
                    span,
                ));
                self.infer_args_loosely(args);
                (Ty::Unknown, None)
            }
        }
    }

    /// `Post(title: "Hi", body: "...")`
    fn construct(&mut self, id: AdtId, explicit: &[Ty], args: &[Arg], span: Span) -> Ty {
        let adt = self.adts[id].clone();
        let AdtKind::Struct(fields) = &adt.kind else {
            self.error(Diagnostic::error(
                codes::NOT_CALLABLE,
                format!(
                    "`{}` is an enum; create a value with one of its variants, as in `{}.Variant`",
                    adt.name, adt.name
                ),
                span,
            ));
            self.infer_args_loosely(args);
            return Ty::Unknown;
        };
        let type_args: Vec<Ty> = if explicit.is_empty() {
            adt.params.iter().map(|_| self.fresh()).collect()
        } else {
            explicit.to_vec()
        };
        let map: HashMap<String, Ty> = adt
            .params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();
        let params: Vec<ParamSig> = fields
            .iter()
            .map(|f| ParamSig {
                name: Some(f.name.clone()),
                ty: substitute(&f.ty, &map),
                has_default: f.has_default,
            })
            .collect();
        for arg in args {
            if let Some(n) = arg
                .name
                .as_ref()
                .filter(|n| fields.iter().any(|f| f.name == n.name))
            {
                self.refer(n.span, Ref::Field(id, n.name.clone()));
            }
        }
        self.check_args(&params, args, span, &adt.name, None);
        Ty::Adt(id, type_args)
    }

    /// `DbError.Timeout(after: 5s)`
    fn construct_variant(&mut self, id: AdtId, name: &Ident, args: &[Arg], span: Span) -> Ty {
        let adt = self.adts[id].clone();
        let AdtKind::Enum(variants) = &adt.kind else {
            self.error(Diagnostic::error(
                codes::NO_SUCH_MEMBER,
                format!(
                    "`{}` is a struct; create one with `{}(...)`",
                    adt.name, adt.name
                ),
                span,
            ));
            self.infer_args_loosely(args);
            return Ty::Unknown;
        };
        let Some(variant) = variants.iter().find(|v| v.name == name.name) else {
            self.no_variant(&adt, variants, name);
            self.infer_args_loosely(args);
            return Ty::Unknown;
        };
        self.refer(name.span, Ref::Variant(id, name.name.clone()));
        let type_args: Vec<Ty> = adt.params.iter().map(|_| self.fresh()).collect();
        let map: HashMap<String, Ty> = adt
            .params
            .iter()
            .cloned()
            .zip(type_args.iter().cloned())
            .collect();
        let params: Vec<ParamSig> = variant
            .fields
            .iter()
            .map(|(n, t)| ParamSig {
                name: n.clone(),
                ty: substitute(t, &map),
                has_default: false,
            })
            .collect();
        self.check_args(
            &params,
            args,
            span,
            &format!("{}.{}", adt.name, variant.name),
            None,
        );
        Ty::Adt(id, type_args)
    }

    /// For `app.get("/posts/{id: Int}", handler)`, the handler's type with
    /// one parameter per route segment.
    fn route_handler_type(
        &mut self,
        adt: AdtId,
        method: &str,
        args: &[Arg],
    ) -> Option<(Ty, usize, Span)> {
        let web = self.modules.iter().position(|m| m.name == "web")?;
        if self.modules[web].types.get("App") != Some(&adt)
            || !matches!(method, "get" | "post" | "put" | "patch" | "delete")
        {
            return None;
        }
        let first = args.first()?;
        let ExprKind::Str(pieces) = &first.value.kind else {
            return None;
        };
        let [StrPiece::Text(pattern)] = pieces.as_slice() else {
            return None;
        };
        let request = Ty::Adt(self.modules[web].types["Request"], Vec::new());
        let response = Ty::Adt(self.modules[web].types["Response"], Vec::new());
        let mut params = vec![request];
        let mut rest = pattern.as_str();
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            let segment = &rest[open + 1..open + close];
            let ty_name = segment
                .split_once(':')
                .map(|(_, t)| t.trim())
                .unwrap_or("String");
            let ty = match ty_name {
                "Int" | "String" | "Float" | "Bool" => self.prim(ty_name),
                other => {
                    self.error(Diagnostic::error(
                        codes::UNKNOWN_TYPE,
                        format!("route parameters can be `Int`, `String`, `Float` or `Bool`, not `{other}`"),
                        first.value.span,
                    ));
                    Ty::Unknown
                }
            };
            params.push(ty);
            rest = &rest[open + close + 1..];
        }
        let count = params.len() - 1;
        let error = self.error_ty();
        Some((
            Ty::Fn(Rc::new(FnTy {
                params,
                ret: response,
                throws: Some(error),
            })),
            count,
            first.value.span,
        ))
    }

    pub(crate) fn check_args(
        &mut self,
        params: &[ParamSig],
        args: &[Arg],
        span: Span,
        fn_name: &str,
        route: Option<(Ty, usize, Span)>,
    ) {
        let mut filled: Vec<bool> = vec![false; params.len()];
        // After a misspelled argument name, "missing" errors would be noise.
        let mut misnamed = false;
        let mut next = 0;
        // Closures whose parameter types come from the call are checked
        // last, once the other arguments have fixed the type arguments.
        let mut deferred = Vec::new();
        for (i, arg) in args.iter().enumerate() {
            let index = match &arg.name {
                None => {
                    while next < params.len() && filled[next] && params[next].ty != Ty::Rest {
                        next += 1;
                    }
                    if next >= params.len() {
                        self.error(Diagnostic::error(
                            codes::BAD_ARGUMENTS,
                            format!("too many arguments: `{fn_name}` takes {}", params.len()),
                            arg.value.span,
                        ));
                        self.infer(&arg.value, &Expect::Value(None));
                        continue;
                    }
                    next
                }
                Some(name) => match params
                    .iter()
                    .position(|p| p.name.as_deref() == Some(&name.name))
                {
                    Some(index) => {
                        if filled[index] {
                            self.error(Diagnostic::error(
                                codes::BAD_ARGUMENTS,
                                format!("`{}` is given more than once", name.name),
                                name.span,
                            ));
                        }
                        index
                    }
                    None => {
                        let mut diag = Diagnostic::error(
                            codes::BAD_ARGUMENTS,
                            format!("`{fn_name}` has no parameter named `{}`", name.name),
                            name.span,
                        );
                        let names: Vec<&str> = params
                            .iter()
                            .zip(&filled)
                            .filter(|(_, f)| !**f)
                            .filter_map(|(p, _)| p.name.as_deref())
                            .collect();
                        if let Some(s) = similar(&name.name, names) {
                            diag =
                                diag.with_suggestion(format!("did you mean `{s}`?"), name.span, s);
                        }
                        self.error(diag);
                        misnamed = true;
                        self.infer(&arg.value, &Expect::Value(None));
                        continue;
                    }
                },
            };
            filled[index] = true;
            let param_ty = params[index].ty.clone();
            if param_ty == Ty::Rest {
                self.infer(&arg.value, &Expect::Value(None));
                continue;
            }
            match (&route, i) {
                (Some((handler, count, _)), 1) => {
                    if let ExprKind::Closure {
                        params: cparams, ..
                    } = &arg.value.kind
                    {
                        if cparams.len() != count + 1 {
                            self.error(Diagnostic::error(
                                codes::ROUTE_PARAMS,
                                format!(
                                    "the route has {count} parameter(s), so the handler takes the request and {count} more, but it takes {}",
                                    cparams.len()
                                ),
                                arg.value.span,
                            ).with_help("route `{name}` segments become handler parameters, in order"));
                            continue;
                        }
                    }
                    let handler = handler.clone();
                    self.check(&arg.value, &handler);
                }
                _ if infers_from_call(&arg.value) => deferred.push((&arg.value, param_ty)),
                _ => self.check(&arg.value, &param_ty),
            }
        }
        for (value, ty) in deferred {
            self.check(value, &ty);
        }
        let missing: Vec<String> = params
            .iter()
            .zip(&filled)
            .filter(|(p, f)| !**f && !p.has_default && p.ty != Ty::Rest)
            .map(|(p, _)| match &p.name {
                Some(n) => format!("`{n}`"),
                None => "an argument".into(),
            })
            .collect();
        if !missing.is_empty() && !misnamed {
            self.error(Diagnostic::error(
                codes::BAD_ARGUMENTS,
                format!("missing {} in this call to `{fn_name}`", missing.join(", ")),
                span,
            ));
        }
    }

    fn infer_closure(
        &mut self,
        params: &[Param],
        ret: &Option<TypeExpr>,
        body: &Block,
        expect: &Expect,
        span: Span,
    ) -> Ty {
        let expected = match self.expected_ty(expect) {
            Some(Ty::Fn(f)) => Some(f),
            _ => None,
        };
        let mut param_tys = Vec::new();
        for (i, p) in params.iter().enumerate() {
            let from_expected = expected
                .as_ref()
                .and_then(|f| f.params.get(i))
                .map(|t| self.resolve(t));
            let ty = match (&p.ty, from_expected) {
                (Some(t), _) => {
                    let params = self.type_params.clone();
                    self.resolve_type(t, &params, Scope::Global)
                }
                (None, Some(t)) if t != Ty::Rest => t,
                (None, Some(_)) => Ty::Unknown,
                (None, None) => {
                    self.error(
                        Diagnostic::error(
                            codes::CANNOT_INFER,
                            format!("cannot infer the type of parameter `{}`", p.name.name),
                            p.name.span,
                        )
                        .with_suggestion(
                            "add a type",
                            p.name.span,
                            format!("{}: Type", p.name.name),
                        ),
                    );
                    Ty::Unknown
                }
            };
            param_tys.push(ty);
        }
        if let Some(f) = &expected {
            let fixed = f.params.iter().filter(|t| **t != Ty::Rest).count();
            if params.len() != fixed && !f.params.contains(&Ty::Rest) {
                self.error(Diagnostic::error(
                    codes::BAD_ARGUMENTS,
                    format!(
                        "this function should take {fixed} parameter(s), but takes {}",
                        params.len()
                    ),
                    span,
                ));
            }
        }
        let ret_ty = match (ret, &expected) {
            (Some(t), _) => {
                let params = self.type_params.clone();
                self.resolve_type(t, &params, Scope::Global)
            }
            (None, Some(f)) => f.ret.clone(),
            (None, None) => self.fresh(),
        };
        let (throws, infer) = match &expected {
            Some(f) => (f.throws.clone(), None),
            None => (None, Some(false)),
        };
        self.enter_fn(ret_ty.clone(), throws.clone(), None);
        self.fns.last_mut().unwrap().infer_throws = infer;
        for (p, ty) in params.iter().zip(&param_tys) {
            self.bind(&p.name.name, p.name.span, ty.clone(), false, None);
        }
        let resolved_ret = self.resolve(&ret_ty);
        if resolved_ret == Ty::Unit {
            self.check_block(body, &Expect::Discard);
        } else {
            let ty = self.check_block(body, &Expect::Value(Some(ret_ty.clone())));
            let at = match body.stmts.last() {
                Some(Stmt::Expr(e)) => e.span,
                _ => body.span,
            };
            self.coerce(&ty, &ret_ty, at);
        }
        let ctx = self.exit_fn();
        let throws = match ctx.infer_throws {
            Some(true) => Some(self.error_ty()),
            Some(false) => None,
            None => throws,
        };
        Ty::Fn(Rc::new(FnTy {
            params: param_tys,
            ret: self.zonk(&ret_ty),
            throws,
        }))
    }

    // ----- match and patterns -----

    fn infer_match(
        &mut self,
        scrutinee: &Expr,
        arms: &[MatchArm],
        expect: &Expect,
        span: Span,
    ) -> Ty {
        let ty = self.infer(scrutinee, &Expect::Value(None));
        let mut branches = Vec::new();
        for arm in arms {
            self.push_scope();
            self.check_pattern(&arm.pattern, &ty);
            if let Some(guard) = &arm.guard {
                let b = self.prim("Bool");
                self.check(guard, &b);
            }
            let body_ty = self.infer(&arm.body, expect);
            if let Some(expected) = self.expected_ty(expect) {
                self.coerce(&body_ty, &expected, arm.body.span);
            }
            branches.push((body_ty, arm.body.span));
            self.pop_scope();
        }
        let patterns: Vec<(&Pattern, bool)> = arms
            .iter()
            .map(|a| (&a.pattern, a.guard.is_some()))
            .collect();
        self.check_exhaustive(
            &ty,
            &patterns,
            span,
            "add the missing arms, or a final `_ => ...` arm",
        );
        match expect {
            Expect::Discard => Ty::Unit,
            _ => self.join(branches, expect),
        }
    }

    /// The element type of the list or set `iter` loops over.
    pub(crate) fn element_type(&mut self, iter: &Expr) -> Ty {
        let iter_ty = self.infer(iter, &Expect::Value(None));
        match self.resolve(&iter_ty) {
            Ty::Adt(id, args)
                if id == self.global_types["List"] || id == self.global_types["Set"] =>
            {
                args[0].clone()
            }
            Ty::Unknown => Ty::Unknown,
            other => {
                let shown = self.show(&other);
                self.error(Diagnostic::error(
                    codes::TYPE_MISMATCH,
                    format!("`for` loops over a `List` or `Set`, found `{shown}`"),
                    iter.span,
                ));
                Ty::Unknown
            }
        }
    }

    pub(crate) fn check_pattern(&mut self, p: &Pattern, ty: &Ty) {
        match &p.kind {
            PatternKind::Wildcard | PatternKind::Error => {}
            PatternKind::Binding(name) => self.bind(name, p.span, ty.clone(), false, None),
            PatternKind::Literal(e) => self.check(e, ty),
            PatternKind::None => match self.resolve(ty) {
                Ty::Opt(_) | Ty::Unknown | Ty::Var(_) => {}
                other => {
                    let shown = self.show(&other);
                    self.error(Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        format!("`none` cannot match a `{shown}`, which is never missing"),
                        p.span,
                    ));
                }
            },
            PatternKind::Variant { path, args } => {
                let resolved = self.resolve(ty);
                let last = &path[path.len() - 1];
                if last.name == "some" && path.len() == 1 {
                    match resolved {
                        Ty::Opt(inner) if args.len() == 1 => self.check_pattern(&args[0], &inner),
                        Ty::Unknown => {}
                        other => {
                            let shown = self.show(&other);
                            self.error(Diagnostic::error(
                                codes::TYPE_MISMATCH,
                                format!("`some(...)` matches an optional, but this is a `{shown}`"),
                                p.span,
                            ));
                        }
                    }
                    return;
                }
                let Ty::Adt(id, type_args) = resolved else {
                    if resolved != Ty::Unknown {
                        let shown = self.show(&resolved);
                        self.error(Diagnostic::error(
                            codes::TYPE_MISMATCH,
                            format!("`{}` is not a variant of `{shown}`", last.name),
                            p.span,
                        ));
                    }
                    return;
                };
                let adt = self.adts[id].clone();
                let AdtKind::Enum(variants) = &adt.kind else {
                    self.error(Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        format!("`{}` is a struct, so it has no variants to match", adt.name),
                        p.span,
                    ));
                    return;
                };
                if path.len() == 2 && path[0].name != adt.name {
                    self.error(Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        format!(
                            "expected a variant of `{}`, found one of `{}`",
                            adt.name, path[0].name
                        ),
                        p.span,
                    ));
                    return;
                }
                let Some(variant) = variants.iter().find(|v| v.name == last.name) else {
                    self.no_variant(&adt, variants, last);
                    return;
                };
                if path.len() == 2 {
                    self.refer(path[0].span, Ref::Type(id));
                }
                self.refer(last.span, Ref::Variant(id, last.name.clone()));
                if !args.is_empty() && args.len() != variant.fields.len() {
                    self.error(Diagnostic::error(
                        codes::BAD_ARGUMENTS,
                        format!(
                            "`{}` has {} field(s), but the pattern has {}",
                            variant.name,
                            variant.fields.len(),
                            args.len()
                        ),
                        p.span,
                    ));
                    return;
                }
                let map: HashMap<String, Ty> = adt.params.iter().cloned().zip(type_args).collect();
                for (arg, (_, field_ty)) in args.iter().zip(&variant.fields) {
                    let field_ty = substitute(field_ty, &map);
                    self.check_pattern(arg, &field_ty);
                }
            }
        }
    }

    /// Checks that `patterns` (each with whether it has a guard) cover
    /// every value of `ty`.
    pub(crate) fn check_exhaustive(
        &mut self,
        ty: &Ty,
        patterns: &[(&Pattern, bool)],
        span: Span,
        help: &str,
    ) {
        let irrefutable = |p: &Pattern| {
            matches!(
                p.kind,
                PatternKind::Wildcard | PatternKind::Binding(_) | PatternKind::Error
            )
        };
        let unguarded: Vec<&Pattern> = patterns
            .iter()
            .filter(|(_, guarded)| !guarded)
            .map(|(p, _)| *p)
            .collect();
        if unguarded.iter().any(|p| irrefutable(p)) {
            return;
        }
        let covered = |name: &str| {
            unguarded.iter().any(|p| match &p.kind {
                PatternKind::Variant { path, args } => {
                    path[path.len() - 1].name == name && args.iter().all(irrefutable)
                }
                PatternKind::None => name == "none",
                PatternKind::Literal(e) => matches!(
                    (&e.kind, name),
                    (ExprKind::Bool(true), "true") | (ExprKind::Bool(false), "false")
                ),
                _ => false,
            })
        };
        let missing: Vec<String> = match self.resolve(ty) {
            Ty::Opt(_) => ["some(_)", "none"]
                .iter()
                .filter(|n| !covered(n.trim_end_matches("(_)")))
                .map(|n| n.to_string())
                .collect(),
            Ty::Adt(id, _) if self.is_prim(&Ty::Adt(id, vec![]), "Bool") => ["true", "false"]
                .iter()
                .filter(|n| !covered(n))
                .map(|n| n.to_string())
                .collect(),
            Ty::Adt(id, _) => match &self.adts[id].kind {
                AdtKind::Enum(variants) => variants
                    .iter()
                    .filter(|v| !covered(&v.name))
                    .map(|v| {
                        if v.fields.is_empty() {
                            v.name.clone()
                        } else {
                            format!("{}(...)", v.name)
                        }
                    })
                    .collect(),
                AdtKind::Struct(_) => vec!["_".into()],
            },
            Ty::Unknown => Vec::new(),
            _ => vec!["_".into()],
        };
        if !missing.is_empty() {
            self.error(
                Diagnostic::error(
                    codes::NON_EXHAUSTIVE_MATCH,
                    format!("this `match` does not handle {}", missing.join(", ")),
                    span,
                )
                .with_help(help),
            );
        }
    }
}

/// How a type is written, using `resolve` on each part first.
pub(crate) fn show_type(adts: &[AdtDef], ty: &Ty, resolve: &dyn Fn(&Ty) -> Ty) -> String {
    let show = |t: &Ty| show_type(adts, &resolve(t), resolve);
    match ty {
        Ty::Adt(id, args) => {
            let name = &adts[*id].name;
            if args.is_empty() {
                name.clone()
            } else {
                let args: Vec<String> = args.iter().map(show).collect();
                format!("{name}<{}>", args.join(", "))
            }
        }
        Ty::Opt(inner) => format!("{}?", show(inner)),
        Ty::Fn(f) => {
            let params: Vec<String> = f.params.iter().map(show).collect();
            let mut s = format!("fn({})", params.join(", "));
            if f.ret != Ty::Unit {
                s += &format!(" -> {}", show(&f.ret));
            }
            if f.throws.is_some() {
                s += " !";
            }
            s
        }
        Ty::Record(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(n, t)| format!("{n}: {}", show(t)))
                .collect();
            format!("{{ {} }}", fields.join(", "))
        }
        Ty::Param(name) => name.clone(),
        Ty::Var(_) => "_".into(),
        Ty::Unit => "()".into(),
        Ty::Never => "Never".into(),
        Ty::Any => "Any".into(),
        Ty::Rest => "...".into(),
        Ty::Style(_) => "style value".into(),
        Ty::Unknown => "{unknown}".into(),
    }
}

pub(crate) fn std_source(name: &str) -> Option<&'static str> {
    if name == "prelude" {
        return Some(PRELUDE);
    }
    STD_MODULES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, src)| *src)
}

fn parse_std(name: &str, src: &str) -> Module {
    let result = kiln_syntax::parse(src);
    assert!(
        result.diagnostics.is_empty(),
        "std/{name}.kiln has syntax errors: {:#?}",
        result.diagnostics
    );
    result.module
}

impl Default for Checker {
    fn default() -> Self {
        Self::new()
    }
}
