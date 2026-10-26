//! What the language server needs from the checker: what each name refers
//! to, the type of each expression, and what may be written where the
//! editor asks for completions.
//!
//! To ask for completions, the server writes `COMPLETION_MARKER` where the
//! cursor is, as if it were a name, and analyzes the files with that text.

use crate::checker::show_type;
use crate::types::*;
use crate::{glaze_checker, module_checker, Styling};
use kiln_diagnostics::Span;
use kiln_syntax::ast::{Glaze, Module};
use std::collections::HashMap;

/// A name the language server writes where it wants completions.
pub const COMPLETION_MARKER: &str = "__kiln_complete__";
/// Starts with a letter so the template parser recognizes it as a tag.
pub const COMPONENT_COMPLETION_MARKER: &str = "KilnComponentCompletion";

/// What a name refers to.
#[derive(Clone, Debug, PartialEq)]
pub enum Ref {
    /// A component or layout, by its actual source filename.
    Component(String),
    /// A local variable or parameter, by where it is bound in the same file.
    Local(Span),
    /// A top-level function: the program's, or the prelude's.
    Fn(String),
    Const(String),
    Type(AdtId),
    Field(AdtId, String),
    Method(AdtId, String),
    Variant(AdtId, String),
    Module(ModId),
    /// A function of a standard library module.
    ModuleFn(ModId, String),
    /// A key of a style, with the CSS it sets: `p`, `hover` or `at`.
    StyleKey {
        name: String,
        css: String,
    },
    /// A design token, such as `space.md`, with its custom property
    /// (`--space-md`) and value.
    Token {
        name: String,
        property: String,
        value: String,
    },
    /// A theme, by the name of its class: `theme-dark`.
    Theme(String),
}

/// What may be written where the editor asked for completions.
#[derive(Clone, Debug)]
pub enum Completion {
    /// Component tags and their source filenames.
    Components(Vec<(String, String)>),
    /// A name in an expression. Holds the locals in scope, innermost
    /// first; the program's functions, constants and types also fit.
    Names(Vec<(String, Ty)>),
    /// A field or method after `value.`
    Members(Ty),
    /// A function or type after `module.`
    Module(ModId),
    /// A variant or function after `Type.`
    TypeMembers(AdtId),
    /// A type name.
    Types,
    /// A type after `module.` where a type is written.
    ModuleTypes(ModId),
    /// Keys, tokens or keywords in a style.
    Style(Vec<StyleWord>),
}

/// A word that fits in a style, with what it means in CSS.
#[derive(Clone, Debug, PartialEq)]
pub struct StyleWord {
    pub label: String,
    pub detail: String,
    pub kind: StyleWordKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StyleWordKind {
    Key,
    Token,
    Keyword,
    Theme,
}

/// Which file of an app to analyze.
#[derive(Clone, Copy, Debug)]
pub enum Target {
    Module(usize),
    Glaze(usize),
}

/// One file of an app, analyzed for the language server.
pub struct Analysis {
    pub adts: Vec<AdtDef>,
    /// How many of `adts` come from the standard library.
    pub std_adts: usize,
    pub global_fns: HashMap<String, FnSig>,
    pub global_types: HashMap<String, AdtId>,
    pub consts: HashMap<String, Ty>,
    /// The modules the file imports, by the name it uses.
    pub imports: HashMap<String, ModId>,
    pub modules: Vec<ModuleDef>,
    /// The type of every expression in the file, by its span.
    pub expr_types: HashMap<Span, Ty>,
    /// The type of every name bound in the file, by its span.
    pub binding_types: HashMap<Span, Ty>,
    /// What the names in the file refer to, by their spans.
    pub references: Vec<(Span, Ref)>,
    /// What fits at `COMPLETION_MARKER`, when the file has one.
    pub completion: Option<Completion>,
}

/// Analyzes one file of an app: the `.kiln` files and glazes of a directory,
/// as `check_app_with_css` takes them. Unlike `check_app`, the files may have syntax
/// errors; whatever the parser recovered is analyzed.
pub fn analyze_app(
    modules: &[&Module],
    glazes: &[(&str, &Glaze)],
    target: Target,
    styling: &Styling,
) -> Analysis {
    let mut checker = match target {
        Target::Module(i) => module_checker(modules, glazes, i, true, &styling.tokens),
        Target::Glaze(i) => glaze_checker(modules, glazes, i, true, styling),
    };
    checker.zonk_recorded();
    let completion = checker.completion.take().map(|c| match c {
        Completion::Names(locals) => Completion::Names(
            locals
                .into_iter()
                .map(|(n, t)| (n, checker.zonk(&t)))
                .collect(),
        ),
        Completion::Members(t) => Completion::Members(checker.zonk(&t)),
        other => other,
    });
    Analysis {
        adts: checker.adts,
        std_adts: checker.std_adts,
        global_fns: checker.global_fns,
        global_types: checker.global_types,
        consts: checker.consts,
        imports: checker.imports,
        modules: checker.modules,
        expr_types: checker.expr_types,
        binding_types: checker.binding_types,
        references: checker.references,
        completion,
    }
}

impl Analysis {
    /// How a type is written.
    pub fn show(&self, ty: &Ty) -> String {
        show_type(&self.adts, ty, &|t| t.clone())
    }

    /// A function's signature, as it would be declared.
    pub fn show_fn(&self, sig: &FnSig) -> String {
        let mut params = Vec::new();
        if sig.has_self {
            params.push("self".to_string());
        }
        for p in &sig.params {
            let mut s = match &p.name {
                Some(name) => format!("{name}: {}", self.show(&p.ty)),
                None => self.show(&p.ty),
            };
            if p.has_default {
                s.push_str(" = ...");
            }
            params.push(s);
        }
        let mut s = format!("fn {}", sig.name);
        if !sig.generics.is_empty() {
            s += &format!("<{}>", sig.generics.join(", "));
        }
        s += &format!("({})", params.join(", "));
        if sig.ret != Ty::Unit {
            s += &format!(" -> {}", self.show(&sig.ret));
        }
        match &sig.throws {
            Some(t) if self.show(t) == "Error" => s += " !",
            Some(t) => s += &format!(" ! {}", self.show(t)),
            None => {}
        }
        s
    }

    /// A type's declaration, with its fields or variants.
    pub fn show_adt(&self, id: AdtId) -> String {
        let adt = &self.adts[id];
        let name = if adt.params.is_empty() {
            adt.name.clone()
        } else {
            format!("{}<{}>", adt.name, adt.params.join(", "))
        };
        match &adt.kind {
            AdtKind::Struct(fields) if fields.is_empty() => format!("struct {name}"),
            AdtKind::Struct(fields) => {
                let mut s = format!("struct {name} {{\n");
                for f in fields {
                    s += &format!("    {}: {}\n", f.name, self.show(&f.ty));
                }
                s + "}"
            }
            AdtKind::Enum(variants) => {
                let mut s = format!("enum {name} {{\n");
                for v in variants {
                    s += &format!("    {}\n", self.show_variant(v));
                }
                s + "}"
            }
        }
    }

    pub fn show_variant(&self, v: &VariantDef) -> String {
        if v.fields.is_empty() {
            return v.name.clone();
        }
        let fields: Vec<String> = v
            .fields
            .iter()
            .map(|(n, t)| match n {
                Some(n) => format!("{n}: {}", self.show(t)),
                None => self.show(t),
            })
            .collect();
        format!("{}({})", v.name, fields.join(", "))
    }

    /// The innermost reference whose span holds `offset`.
    pub fn reference_at(&self, offset: usize) -> Option<&(Span, Ref)> {
        self.references
            .iter()
            .filter(|(s, _)| contains(*s, offset))
            .min_by_key(|(s, _)| s.end - s.start)
    }

    /// The innermost expression whose span holds `offset`, and its type.
    pub fn expr_at(&self, offset: usize) -> Option<(Span, &Ty)> {
        self.expr_types
            .iter()
            .filter(|(s, _)| contains(**s, offset))
            .min_by_key(|(s, _)| s.end - s.start)
            .map(|(s, t)| (*s, t))
    }

    /// The name bound at `offset`, and its type.
    pub fn binding_at(&self, offset: usize) -> Option<(Span, &Ty)> {
        self.binding_types
            .iter()
            .find(|(s, t)| contains(**s, offset) && s.end > s.start && **t != Ty::Unknown)
            .map(|(s, t)| (*s, t))
    }

    /// A method of a type, by name.
    pub fn method(&self, id: AdtId, name: &str) -> Option<&FnSig> {
        self.adts[id].methods.iter().find(|m| m.name == name)
    }

    /// A field of a struct, by name, and its type.
    pub fn field(&self, id: AdtId, name: &str) -> Option<&FieldDef> {
        match &self.adts[id].kind {
            AdtKind::Struct(fields) => fields.iter().find(|f| f.name == name),
            AdtKind::Enum(_) => None,
        }
    }

    /// A variant of an enum, by name.
    pub fn variant(&self, id: AdtId, name: &str) -> Option<&VariantDef> {
        match &self.adts[id].kind {
            AdtKind::Enum(variants) => variants.iter().find(|v| v.name == name),
            AdtKind::Struct(_) => None,
        }
    }
}

/// Whether `span` holds `offset`, counting the position just after its end,
/// where the cursor is after typing a name.
fn contains(span: Span, offset: usize) -> bool {
    span.start as usize <= offset && offset <= span.end as usize
}
