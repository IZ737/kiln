//! Builds definitions (types, functions, methods) from declarations, for
//! the standard library and for the file being checked.

use crate::checker::{Checker, Scope};
use crate::ide::{Completion, Ref};
use crate::types::*;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::{self, Item, Module, Throws, TypeExpr, TypeKind};
use std::collections::HashMap;

impl Checker {
    /// Registers every struct and enum name in `module` before any
    /// signature is resolved, so declarations can refer to each other in any
    /// order.
    pub(crate) fn declare_types(&mut self, module: &Module, scope: Scope) {
        let mut seen: HashMap<String, Span> = HashMap::new();
        for item in &module.items {
            if let Item::Interface(i) = item {
                let again = self.global_interfaces.contains_key(&i.name.name)
                    || !self.declared_types.insert(i.name.name.clone());
                if again {
                    self.duplicate(&i.name.name, i.name.span);
                    continue;
                }
                let id = self.interfaces.len();
                self.interfaces.push(InterfaceDef {
                    name: i.name.name.clone(),
                    methods: Vec::new(),
                });
                self.global_interfaces.insert(i.name.name.clone(), id);
                continue;
            }
            let (name, generics) = match item {
                Item::Struct(s) => (&s.name, &s.generics),
                Item::Enum(e) => (&e.name, &e.generics),
                _ => continue,
            };
            let again = match scope {
                // The files of a folder share one namespace.
                Scope::Global => !self.declared_types.insert(name.name.clone()),
                Scope::Module(_) => seen.insert(name.name.clone(), name.span).is_some(),
            };
            if again {
                self.duplicate(&name.name, name.span);
                self.duplicate_types
                    .insert(std::ptr::from_ref(name) as usize);
                continue;
            }
            let id = self.adts.len();
            self.adts.push(AdtDef {
                name: name.name.clone(),
                params: generics.iter().map(|g| g.name.clone()).collect(),
                kind: AdtKind::Struct(Vec::new()),
                methods: Vec::new(),
            });
            match scope {
                Scope::Global => {
                    self.global_types.insert(name.name.clone(), id);
                }
                Scope::Module(m) => {
                    self.modules[m].types.insert(name.name.clone(), id);
                }
            }
        }
    }

    /// Resolves fields, variants, function signatures and methods.
    pub(crate) fn define_items(&mut self, module: &Module, scope: Scope) {
        let mut seen_fns: HashMap<String, Span> = HashMap::new();
        for item in &module.items {
            self.internal = self.std_adts == 0 || crate::checker::item_name(item).starts_with("__");
            match item {
                Item::Struct(s) => {
                    if self.is_duplicate_type(&s.name) {
                        continue;
                    }
                    let Some(id) = self.type_in_scope(&s.name.name, scope) else {
                        continue;
                    };
                    let generics: Vec<String> = s.generics.iter().map(|g| g.name.clone()).collect();
                    let mut fields = Vec::new();
                    let mut seen: HashMap<String, Span> = HashMap::new();
                    for f in &s.fields {
                        if seen.insert(f.name.name.clone(), f.name.span).is_some() {
                            self.duplicate(&f.name.name, f.name.span);
                        }
                        fields.push(FieldDef {
                            name: f.name.name.clone(),
                            ty: self.resolve_type(&f.ty, &generics, scope),
                            has_default: f.default.is_some(),
                        });
                    }
                    self.adts[id].kind = AdtKind::Struct(fields);
                }
                Item::Enum(e) => {
                    if self.is_duplicate_type(&e.name) {
                        continue;
                    }
                    let Some(id) = self.type_in_scope(&e.name.name, scope) else {
                        continue;
                    };
                    let generics: Vec<String> = e.generics.iter().map(|g| g.name.clone()).collect();
                    let mut variants = Vec::new();
                    let mut seen: HashMap<String, Span> = HashMap::new();
                    for v in &e.variants {
                        if seen.insert(v.name.name.clone(), v.name.span).is_some() {
                            self.duplicate(&v.name.name, v.name.span);
                        }
                        let fields = v
                            .fields
                            .iter()
                            .map(|f| {
                                (
                                    f.name.as_ref().map(|n| n.name.clone()),
                                    self.resolve_type(&f.ty, &generics, scope),
                                )
                            })
                            .collect();
                        variants.push(VariantDef {
                            name: v.name.name.clone(),
                            fields,
                        });
                    }
                    self.adts[id].kind = AdtKind::Enum(variants);
                }
                Item::Fn(f) => {
                    let again = match scope {
                        Scope::Global => !self.declared_values.insert(f.name.name.clone()),
                        Scope::Module(_) => {
                            seen_fns.insert(f.name.name.clone(), f.name.span).is_some()
                        }
                    };
                    if again {
                        self.duplicate(&f.name.name, f.name.span);
                        continue;
                    }
                    let sig = self.signature(f, &[], scope);
                    match scope {
                        Scope::Global => {
                            if f.is_server {
                                self.server_fns.insert(f.name.name.clone());
                            }
                            self.global_fns.insert(f.name.name.clone(), sig);
                        }
                        Scope::Module(m) => {
                            self.modules[m].fns.insert(f.name.name.clone(), sig);
                        }
                    }
                }
                Item::Impl(imp) => self.define_impl(imp, scope),
                Item::Interface(i) => self.define_interface(i, scope),
                Item::Const(c) => {
                    if matches!(scope, Scope::Global)
                        && !self.declared_values.insert(c.name.name.clone())
                    {
                        self.duplicate(&c.name.name, c.name.span);
                    }
                }
                Item::Import(_) => {}
            }
        }
        self.internal = true;
    }

    fn define_interface(&mut self, i: &ast::InterfaceDecl, scope: Scope) {
        let Some(&id) = self.global_interfaces.get(&i.name.name) else {
            return;
        };
        if !self.interfaces[id].methods.is_empty() {
            return;
        }
        let mut methods: Vec<FnSig> = Vec::new();
        for m in &i.methods {
            if methods.iter().any(|s| s.name == m.name.name) {
                self.duplicate(&m.name.name, m.name.span);
                continue;
            }
            if !m.generics.is_empty() {
                self.error(Diagnostic::error(
                    codes::INTERFACE_MISUSE,
                    "an interface's methods have no type parameters",
                    m.name.span,
                ));
            }
            let sig = self.signature(m, &[], scope);
            if !sig.has_self {
                self.error(
                    Diagnostic::error(
                        codes::INTERFACE_MISUSE,
                        format!("the interface method `{}` needs `self` first", m.name.name),
                        m.name.span,
                    )
                    .with_help(
                        "an interface lists methods called on a value: `fn name(self) -> String`",
                    ),
                );
                continue;
            }
            methods.push(sig);
        }
        self.interfaces[id].methods = methods;
    }

    fn define_impl(&mut self, imp: &ast::ImplDecl, scope: Scope) {
        let TypeKind::Named { path, args } = &imp.target.kind else {
            return;
        };
        let target = self.resolve_type_path(path, imp.target.span, scope);
        let Some(id) = target else { return };
        // Type arguments in an impl target introduce type parameters:
        // `impl List<T>` declares `T`.
        let impl_params: Vec<String> = args
            .iter()
            .filter_map(|a| match &a.kind {
                TypeKind::Named { path, .. } if path.len() == 1 => Some(path[0].name.clone()),
                _ => None,
            })
            .collect();
        let mut seen: HashMap<String, Span> = self.adts[id]
            .methods
            .iter()
            .map(|m| (m.name.clone(), Span::default()))
            .collect();
        for method in &imp.methods {
            if seen
                .insert(method.name.name.clone(), method.name.span)
                .is_some()
            {
                self.duplicate(&method.name.name, method.name.span);
                continue;
            }
            let mut sig = self.signature(method, &impl_params, scope);
            sig.impl_params = impl_params.clone();
            self.adts[id].methods.push(sig);
        }
    }

    pub(crate) fn signature(&mut self, f: &ast::FnDecl, outer: &[String], scope: Scope) -> FnSig {
        let generics: Vec<String> = outer
            .iter()
            .cloned()
            .chain(f.generics.iter().map(|g| g.name.clone()))
            .collect();
        let mut params = Vec::new();
        let mut has_self = false;
        let mut seen: HashMap<String, Span> = HashMap::new();
        for (i, p) in f.params.iter().enumerate() {
            if seen.insert(p.name.name.clone(), p.name.span).is_some() {
                self.duplicate(&p.name.name, p.name.span);
            }
            if i == 0 && p.name.name == "self" && p.ty.is_none() {
                has_self = true;
                continue;
            }
            let ty = match &p.ty {
                Some(t) => self.resolve_type(t, &generics, scope),
                None => Ty::Unknown,
            };
            params.push(ParamSig {
                name: Some(p.name.name.clone()),
                ty,
                has_default: p.default.is_some(),
            });
        }
        let ret = match &f.ret {
            Some(t) => self.resolve_type(t, &generics, scope),
            None => Ty::Unit,
        };
        let throws = self.resolve_throws(&f.throws, &generics, scope);
        let bounds = self.resolve_bounds(f, scope);
        FnSig {
            name: f.name.name.clone(),
            generics: f.generics.iter().map(|g| g.name.clone()).collect(),
            impl_params: Vec::new(),
            has_self,
            params,
            ret,
            throws,
            bounds,
        }
    }

    /// The interfaces of each bounded type parameter of `f`.
    pub(crate) fn resolve_bounds(
        &mut self,
        f: &ast::FnDecl,
        scope: Scope,
    ) -> Vec<(String, Vec<usize>)> {
        let mut out: Vec<(String, Vec<usize>)> = Vec::new();
        for b in &f.bounds {
            let mut ids = Vec::new();
            for t in &b.interfaces {
                if let Some(id) = self.resolve_interface(t, scope) {
                    if !ids.contains(&id) {
                        ids.push(id);
                    }
                }
            }
            match out.iter_mut().find(|(p, _)| *p == b.param.name) {
                Some((_, have)) => {
                    for id in ids {
                        if !have.contains(&id) {
                            have.push(id);
                        }
                    }
                }
                None => out.push((b.param.name.clone(), ids)),
            }
        }
        out
    }

    /// The interface a bound names, reporting a name that is not one.
    fn resolve_interface(&mut self, t: &TypeExpr, scope: Scope) -> Option<usize> {
        let TypeKind::Named { path, args } = &t.kind else {
            self.error(Diagnostic::error(
                codes::INTERFACE_MISUSE,
                "a bound names an interface",
                t.span,
            ));
            return None;
        };
        if let [name] = &path[..] {
            if let Some(&id) = self.global_interfaces.get(&name.name) {
                self.refer(name.span, Ref::Interface(id));
                if !args.is_empty() {
                    self.error(Diagnostic::error(
                        codes::INTERFACE_MISUSE,
                        "an interface has no type arguments",
                        t.span,
                    ));
                }
                return Some(id);
            }
        }
        let text: Vec<&str> = path.iter().map(|p| p.name.as_str()).collect();
        let text = text.join(".");
        let found_type = match &path[..] {
            [name] => {
                self.type_in_scope(&name.name, scope).is_some()
                    || self.global_types.contains_key(&name.name)
            }
            _ => false,
        };
        let diag = if found_type {
            Diagnostic::error(
                codes::INTERFACE_MISUSE,
                format!("`{text}` is a type, not an interface"),
                t.span,
            )
            .with_help(
                "a bound lists methods: declare `interface Name { fn ... }` and name it here",
            )
        } else {
            Diagnostic::error(
                codes::UNKNOWN_TYPE,
                format!("unknown interface `{text}`"),
                t.span,
            )
        };
        self.error(diag);
        None
    }

    pub(crate) fn resolve_throws(
        &mut self,
        throws: &Throws,
        generics: &[String],
        scope: Scope,
    ) -> Option<Ty> {
        match throws {
            Throws::Never => None,
            Throws::Default => Some(self.error_ty()),
            Throws::Type(t) => Some(self.resolve_type(t, generics, scope)),
        }
    }

    fn type_in_scope(&self, name: &str, scope: Scope) -> Option<AdtId> {
        match scope {
            Scope::Global => self.global_types.get(name).copied(),
            Scope::Module(m) => self.modules[m].types.get(name).copied(),
        }
    }

    pub(crate) fn resolve_type(&mut self, t: &TypeExpr, generics: &[String], scope: Scope) -> Ty {
        match &t.kind {
            TypeKind::Error => Ty::Unknown,
            TypeKind::Record(fields) => {
                let mut out: Vec<(String, Ty)> = Vec::new();
                for (name, ty) in fields {
                    let ty = self.resolve_type(ty, generics, scope);
                    if out.iter().any(|(n, _)| *n == name.name) {
                        self.error(Diagnostic::error(
                            codes::DUPLICATE_DEFINITION,
                            format!("the field `{}` is listed twice", name.name),
                            name.span,
                        ));
                        continue;
                    }
                    out.push((name.name.clone(), ty));
                }
                Ty::Record(out)
            }
            TypeKind::Optional(inner) => {
                Ty::Opt(Box::new(self.resolve_type(inner, generics, scope)))
            }
            TypeKind::Fn {
                params,
                ret,
                throws,
            } => {
                let params = params
                    .iter()
                    .map(|p| self.resolve_type(p, generics, scope))
                    .collect();
                let ret = match ret {
                    Some(r) => self.resolve_type(r, generics, scope),
                    None => Ty::Unit,
                };
                let throws = self.resolve_throws(throws, generics, scope);
                Ty::Fn(std::rc::Rc::new(FnTy {
                    params,
                    ret,
                    throws,
                }))
            }
            TypeKind::Named { path, args } => {
                if path.len() == 1 {
                    let name = path[0].name.as_str();
                    if generics.iter().any(|g| g == name) {
                        return Ty::Param(name.to_string());
                    }
                    // The standard library describes what Kiln cannot say
                    // yet with `Any` and `Rest`; programs use generics.
                    if matches!(name, "Any" | "Rest") && self.std_adts > 0 {
                        self.error(
                            Diagnostic::error(
                                codes::RESERVED_TYPE,
                                format!("`{name}` is reserved for the standard library"),
                                t.span,
                            )
                            .with_help(
                                "a function that takes any type is generic: `fn show<T>(value: T)`",
                            ),
                        );
                        return Ty::Unknown;
                    }
                    match name {
                        "Any" => return Ty::Any,
                        "Rest" => return Ty::Rest,
                        "Never" => return Ty::Never,
                        _ => {}
                    }
                }
                if path.last().is_some_and(|n| self.is_marker(&n.name)) {
                    let module = match &path[..] {
                        [module, _] => self.imports.get(&module.name).copied(),
                        _ => None,
                    };
                    self.complete(match module {
                        Some(m) => Completion::ModuleTypes(m),
                        None => Completion::Types,
                    });
                    return Ty::Unknown;
                }
                if let [name] = &path[..] {
                    if let Some(&id) = self.global_interfaces.get(&name.name) {
                        self.refer(name.span, Ref::Interface(id));
                        self.error(
                            Diagnostic::error(
                                codes::INTERFACE_MISUSE,
                                format!(
                                    "`{}` is an interface, so it is a bound, not a type",
                                    name.name
                                ),
                                t.span,
                            )
                            .with_help(format!(
                                "take a type parameter that fits it: `fn f<T: {0}>(x: T)`",
                                name.name
                            )),
                        );
                        return Ty::Unknown;
                    }
                }
                let Some(id) = self.resolve_type_path(path, t.span, scope) else {
                    return Ty::Unknown;
                };
                if let (Some(last), Scope::Global) = (path.last(), scope) {
                    if path.len() == 2 {
                        if let Some(&m) = self.imports.get(&path[0].name) {
                            self.refer(path[0].span, Ref::Module(m));
                        }
                    }
                    self.refer(last.span, Ref::Type(id));
                }
                let args: Vec<Ty> = args
                    .iter()
                    .map(|a| self.resolve_type(a, generics, scope))
                    .collect();
                let expected = self.adts[id].params.len();
                if args.len() != expected {
                    let name = self.adts[id].name.clone();
                    self.error(Diagnostic::error(
                        codes::TYPE_MISMATCH,
                        format!(
                            "`{name}` takes {expected} type argument(s), but {} were given",
                            args.len()
                        ),
                        t.span,
                    ));
                    return Ty::Unknown;
                }
                Ty::Adt(id, args)
            }
        }
    }

    fn resolve_type_path(
        &mut self,
        path: &[ast::Ident],
        span: Span,
        scope: Scope,
    ) -> Option<AdtId> {
        let found = match path {
            [name] => {
                let local = match scope {
                    Scope::Module(m) => self.modules[m].types.get(&name.name).copied(),
                    Scope::Global => None,
                };
                local.or_else(|| self.global_types.get(&name.name).copied())
            }
            [module, name] => {
                let m = match scope {
                    Scope::Global => self.imports.get(&module.name).copied(),
                    Scope::Module(_) => self.modules.iter().position(|d| d.name == module.name),
                };
                if let Some(m) = m {
                    if !self.internal
                        && crate::checker::internal_name(&self.modules[m].name, &name.name)
                    {
                        let what = format!("{}.{}", module.name, name.name);
                        self.internal_error(&what, name.span);
                    }
                }
                m.and_then(|m| self.modules[m].types.get(&name.name).copied())
            }
            _ => None,
        };
        if found.is_none() {
            let text: Vec<&str> = path.iter().map(|p| p.name.as_str()).collect();
            let text = text.join(".");
            let mut diag =
                Diagnostic::error(codes::UNKNOWN_TYPE, format!("unknown type `{text}`"), span);
            if path.len() == 1 {
                let candidates: Vec<String> = self.global_types.keys().cloned().collect();
                let guess = foreign_type_name(&text)
                    .map(str::to_string)
                    .or_else(|| similar(&text, candidates.iter().map(String::as_str)));
                if let Some(s) = guess {
                    diag = diag.with_suggestion(format!("did you mean `{s}`?"), path[0].span, s);
                }
            }
            self.error(diag);
        }
        found
    }

    /// Whether this declaration repeats a type's name, so it must not
    /// change the first declaration's definition.
    fn is_duplicate_type(&self, name: &ast::Ident) -> bool {
        self.duplicate_types
            .contains(&(std::ptr::from_ref(name) as usize))
    }

    fn duplicate(&mut self, name: &str, span: Span) {
        self.error(Diagnostic::error(
            codes::DUPLICATE_DEFINITION,
            format!("`{name}` is defined more than once"),
            span,
        ));
    }
}

/// Kiln's name for a type name from another language, such as `Integer` or
/// `HashMap`.
fn foreign_type_name(name: &str) -> Option<&'static str> {
    Some(match name.to_ascii_lowercase().as_str() {
        "int" | "integer" | "i32" | "i64" | "long" | "number" | "usize" => "Int",
        "float" | "double" | "f32" | "f64" | "float64" => "Float",
        "bool" | "boolean" => "Bool",
        "str" | "string" | "text" => "String",
        "array" | "vec" | "vector" | "arraylist" | "slice" => "List",
        "dict" | "dictionary" | "hashmap" | "object" | "record" => "Map",
        "hashset" => "Set",
        "exception" => "Error",
        _ => return None,
    })
}
