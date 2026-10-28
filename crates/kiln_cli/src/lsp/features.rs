//! Hover, go to definition, completion and symbols. Each request analyzes
//! the program the file belongs to (the `.kiln` and `.glaze` files of its
//! directory) with `kiln_check::ide`, using the editor's text for open
//! files. Files with syntax errors are analyzed as far as they parsed, and
//! a request that finds nothing gets an empty answer.

use super::range;
use kiln_check::ide::{
    analyze_app, Analysis, Completion, Ref, StyleWordKind, Target, COMPLETION_MARKER,
};
use kiln_check::types::{AdtKind, Ty};
use kiln_diagnostics::Span;
use kiln_syntax::ast::{Glaze, Item, Module, TypeKind};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// A file of a program, with the editor's text when it is open.
pub(super) struct Source {
    pub path: PathBuf,
    /// The editor's URI for the file, when it is open.
    pub uri: Option<String>,
    pub text: String,
}

enum Parsed {
    Kiln(Module),
    Glaze(Box<Glaze>),
}

/// A program's files, parsed.
struct Program<'a> {
    sources: &'a [Source],
    parsed: Vec<Parsed>,
}

impl<'a> Program<'a> {
    fn parse(sources: &'a [Source]) -> Program<'a> {
        let parsed = sources
            .iter()
            .map(|s| {
                if crate::is_glaze(&s.path) {
                    Parsed::Glaze(Box::new(
                        kiln_syntax::glaze::parse_component(&s.text, &s.path.to_string_lossy())
                            .glaze,
                    ))
                } else {
                    Parsed::Kiln(kiln_syntax::parse(&s.text).module)
                }
            })
            .collect();
        Program { sources, parsed }
    }

    fn modules(&self) -> impl Iterator<Item = (usize, &Module)> {
        self.parsed.iter().enumerate().filter_map(|(i, p)| match p {
            Parsed::Kiln(m) => Some((i, m)),
            Parsed::Glaze(_) => None,
        })
    }

    /// Analyzes the `index`th file. `None` if the checker cannot make sense
    /// of what the parser recovered from broken code.
    fn analyze(&self, index: usize) -> Option<Analysis> {
        let modules: Vec<(usize, &Module)> = self.modules().collect();
        let root = crate::project::common_root(self.sources.iter().map(|s| s.path.as_path()));
        let names: Vec<String> = self
            .sources
            .iter()
            .map(|s| {
                s.path
                    .strip_prefix(&root)
                    .unwrap_or(&s.path)
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        let glazes: Vec<(usize, &str, &Glaze)> = self
            .parsed
            .iter()
            .enumerate()
            .filter_map(|(i, p)| match p {
                Parsed::Glaze(g) => Some((i, names[i].as_str(), &**g)),
                Parsed::Kiln(_) => None,
            })
            .collect();
        let target = match modules.iter().position(|(i, _)| *i == index) {
            Some(m) => Target::Module(m),
            None => Target::Glaze(glazes.iter().position(|(i, _, _)| *i == index)?),
        };
        let module_refs: Vec<&Module> = modules.iter().map(|(_, m)| *m).collect();
        let glaze_refs: Vec<(&str, &Glaze)> = glazes.iter().map(|(_, n, g)| (*n, *g)).collect();
        // The stylesheets give the design tokens and CSS Modules.
        let glaze_files: Vec<crate::styles::GlazeFile> = glaze_refs
            .iter()
            .map(|(name, glaze)| crate::styles::GlazeFile { name, glaze })
            .collect();
        let styling = crate::styles::load(&root, &glaze_files).styling;
        // The checker expects code that parsed; recovered trees can break
        // its assumptions, and a failed request must not stop the server.
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            analyze_app(&module_refs, &glaze_refs, target, &styling)
        }))
        .ok()
    }

    /// The directory of the `index`th file, which is the program's.
    fn dir(&self, index: usize) -> PathBuf {
        match self.sources[index].path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
            _ => PathBuf::from("."),
        }
    }

    /// The editor's URI for the `index`th file.
    fn uri(&self, index: usize) -> String {
        let source = &self.sources[index];
        source
            .uri
            .clone()
            .unwrap_or_else(|| path_to_uri(&source.path))
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

/// A `file://` URI for a path.
fn path_to_uri(path: &Path) -> String {
    let mut text = path.to_string_lossy().replace('\\', "/");
    // Windows canonical paths use a verbatim prefix, which is not part
    // of a file URI. UNC paths put the server in the URI authority.
    if let Some(unc) = text.strip_prefix("//?/UNC/") {
        text = format!("//{unc}");
    } else if let Some(local) = text.strip_prefix("//?/") {
        text = local.to_owned();
    }
    if text.starts_with("//") {
        text.drain(..2);
    } else if !text.starts_with('/') {
        text.insert(0, '/');
    }
    let mut uri = String::from("file://");
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"/-_.~:".contains(&b) {
            uri.push(b as char);
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

// ----- hover -----

fn hover_bound(sources: &[Source], index: usize, offset: usize) -> Value {
    let program = Program::parse(sources);
    let Some(analysis) = program.analyze(index) else {
        return Value::Null;
    };
    let text = &sources[index].text;
    if let Some((span, target)) = analysis.reference_at(offset) {
        if let Some(shown) = describe_style(target) {
            return json!({
                "contents": {"kind": "markdown", "value": shown},
                "range": range(text, span.start as usize, span.end as usize),
            });
        }
    }
    let (span, mut shown) = if let Some((span, target)) = analysis.reference_at(offset) {
        (*span, describe(&analysis, target, text))
    } else if let Some((span, ty)) = analysis.binding_at(offset) {
        let name = &text[span.start as usize..span.end as usize];
        (span, format!("{name}: {}", analysis.show(ty)))
    } else if let Some((span, ty)) = analysis.expr_at(offset) {
        // Only short expressions: the type of a whole closure or block
        // says little about the spot under the cursor.
        let shown = &text[span.start as usize..span.end as usize];
        if matches!(ty, Ty::Unknown | Ty::Unit) || shown.contains('\n') {
            return Value::Null;
        }
        (span, analysis.show(ty))
    } else {
        return Value::Null;
    };
    shown = format!("```kiln\n{shown}\n```");
    let target = analysis_ref(&analysis, offset).filter(|r| !matches!(r, Ref::Local(_)));
    if let Some((source, decl)) = find_decl(&program, index, &analysis, &target) {
        let decl_text = match source {
            DeclIn::File(i) => sources[i].text.as_str(),
            DeclIn::Std(name) => kiln_check::std_source(name).unwrap_or_default(),
        };
        if let Some(doc) = doc_comment(decl_text, decl.item.start as usize) {
            shown = format!("{shown}\n\n{doc}");
        }
    }
    json!({
        "contents": {"kind": "markdown", "value": shown},
        "range": range(text, span.start as usize, span.end as usize),
    })
}

fn analysis_ref(analysis: &Analysis, offset: usize) -> Option<Ref> {
    analysis.reference_at(offset).map(|(_, r)| r.clone())
}

/// What a style's key, token or theme means in CSS, for hover.
fn describe_style(target: &Ref) -> Option<String> {
    Some(match target {
        Ref::StyleKey { name, css } => format!("```kiln\n{name}\n```\n\nSets {css}."),
        Ref::Token {
            name,
            property,
            value,
        } => format!("```kiln\n{name}\n```\n\n```css\n{property}: {value};\n```"),
        Ref::Theme(class) => {
            format!("```css\n.{class}\n```\n\nThe theme's class, which sets its tokens.")
        }
        _ => return None,
    })
}

/// How a name's declaration reads.
fn describe(a: &Analysis, target: &Ref, text: &str) -> String {
    match target {
        Ref::Component(file) => format!("component {file}"),
        Ref::Local(def) => {
            let name = &text[def.start as usize..def.end as usize];
            match a.binding_types.get(def) {
                Some(ty) if *ty != Ty::Unknown => format!("{name}: {}", a.show(ty)),
                _ => name.to_string(),
            }
        }
        Ref::Fn(name) => match a.global_fns.get(name) {
            Some(sig) => a.show_fn(sig),
            None => format!("fn {name}"),
        },
        Ref::Const(name) => {
            let kw = if a.stores.contains(name) {
                "store"
            } else {
                "const"
            };
            match a.consts.get(name) {
                Some(ty) => format!("{kw} {name}: {}", a.show(ty)),
                None => format!("{kw} {name}"),
            }
        }
        Ref::Type(id) => a.show_adt(*id),
        Ref::Interface(id) => a.show_interface(*id),
        Ref::Field(id, name) => match a.field(*id, name) {
            Some(f) => format!("{}.{name}: {}", a.adts[*id].name, a.show(&f.ty)),
            None => name.clone(),
        },
        Ref::Method(id, name) => match a.method(*id, name) {
            Some(sig) => qualify(&a.show_fn(sig), &a.adts[*id].name),
            None => name.clone(),
        },
        Ref::Variant(id, name) => match a.variant(*id, name) {
            Some(v) => format!("{}.{}", a.adts[*id].name, a.show_variant(v)),
            None => name.clone(),
        },
        Ref::Module(m) => format!("import {}", a.modules[*m].name),
        Ref::ModuleFn(m, name) => {
            let module = &a.modules[*m];
            match module.fns.get(name) {
                Some(sig) => qualify(&a.show_fn(sig), &module.name),
                None => name.clone(),
            }
        }
        Ref::StyleKey { name, .. } | Ref::Token { name, .. } | Ref::Theme(name) => name.clone(),
    }
}

/// `fn json(...)` as `fn web.json(...)`.
fn qualify(signature: &str, owner: &str) -> String {
    signature.replacen("fn ", &format!("fn {owner}."), 1)
}

/// The `//` comment lines just above the line at `start`, without the
/// slashes.
fn doc_comment(text: &str, start: usize) -> Option<String> {
    let before = &text[..text[..start.min(text.len())]
        .rfind('\n')
        .map_or(0, |i| i + 1)];
    let mut lines: Vec<&str> = before
        .lines()
        .rev()
        .map(str::trim)
        .take_while(|l| l.starts_with("//"))
        .map(|l| {
            l.trim_start_matches('/')
                .strip_prefix(' ')
                .unwrap_or(l.trim_start_matches('/'))
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    lines.reverse();
    Some(lines.join("\n"))
}

// ----- go to definition -----

/// Where a declaration is: in a file of the program, or in the standard
/// library's source.
enum DeclIn {
    File(usize),
    Std(&'static str),
}

struct Decl {
    /// The declaration's name.
    name: Span,
    /// The whole declaration, where its doc comment ends.
    item: Span,
}

fn definition_bound(sources: &[Source], index: usize, offset: usize) -> Value {
    let program = Program::parse(sources);
    let Some(analysis) = program.analyze(index) else {
        return Value::Null;
    };
    let target = analysis_ref(&analysis, offset);
    if let Some(Ref::Token { property, .. } | Ref::Theme(property)) = &target {
        let needle = match target {
            Some(Ref::Theme(_)) => format!(".{property}"),
            _ => property.clone(),
        };
        return find_in_css(&program.dir(index), &needle).unwrap_or(Value::Null);
    }
    match find_decl(&program, index, &analysis, &target) {
        Some((DeclIn::File(i), decl)) => json!({
            "uri": program.uri(i),
            "range": range(&sources[i].text, decl.name.start as usize, decl.name.end as usize),
        }),
        _ => Value::Null,
    }
}

/// Every place the name at `offset` is used, and where it is declared.
/// Names from the standard library, modules and styles have none.
fn references_bound(sources: &[Source], index: usize, offset: usize) -> Value {
    let program = Program::parse(sources);
    let Some(analysis) = program.analyze(index) else {
        return json!([]);
    };
    let Some(target) = analysis_ref(&analysis, offset) else {
        return json!([]);
    };
    let mut found: Vec<(usize, Span)> = Vec::new();
    match &target {
        Ref::Local(def) => {
            found.push((index, *def));
            for (span, r) in &analysis.references {
                if *r == target {
                    found.push((index, *span));
                }
            }
        }
        Ref::Module(_)
        | Ref::ModuleFn(..)
        | Ref::StyleKey { .. }
        | Ref::Token { .. }
        | Ref::Theme(_)
        | Ref::Component(_) => {
            return json!([]);
        }
        _ => {
            match find_decl(&program, index, &analysis, &Some(target.clone())) {
                Some((DeclIn::File(i), decl)) => found.push((i, decl.name)),
                // The standard library's names are not the program's to rename.
                _ => return json!([]),
            }
            for i in 0..sources.len() {
                let other;
                let a = if i == index {
                    &analysis
                } else {
                    match program.analyze(i) {
                        Some(a) => {
                            other = a;
                            &other
                        }
                        None => continue,
                    }
                };
                for (span, r) in &a.references {
                    if *r == target {
                        found.push((i, *span));
                    }
                }
            }
        }
    }
    found.sort_by_key(|(i, s)| (*i, s.start));
    found.dedup();
    Value::Array(
        found
            .into_iter()
            .map(|(i, span)| {
                json!({
                    "uri": program.uri(i),
                    "range": range(&sources[i].text, span.start as usize, span.end as usize),
                })
            })
            .collect(),
    )
}

/// Where the program's stylesheets first declare `needle`, a custom
/// property (`--space-md`) or a class (`.theme-dark`).
fn find_in_css(dir: &Path, needle: &str) -> Option<Value> {
    let mut files = Vec::new();
    css_files(dir, &mut files, 0);
    files.sort();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let mut from = 0;
        while let Some(i) = text[from..].find(needle) {
            let start = from + i;
            let end = start + needle.len();
            let rest = text[end..].trim_start();
            let ends_name =
                !text[end..].starts_with(|c: char| c.is_alphanumeric() || c == '-' || c == '_');
            let declares = if needle.starts_with("--") {
                rest.starts_with(':')
            } else {
                rest.starts_with(['{', ','])
            };
            if ends_name && declares {
                return Some(json!({
                    "uri": path_to_uri(&file),
                    "range": range(&text, start, end),
                }));
            }
            from = end;
        }
    }
    None
}

fn css_files(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') || name == "target" {
            continue;
        }
        if path.is_dir() && depth < 4 {
            css_files(&path, out, depth + 1);
        } else if path.extension().is_some_and(|e| e == "css") {
            out.push(path);
        }
    }
}

const STD_FILES: &[&str] = &[
    "prelude", "web", "sql", "auth", "log", "json", "time", "fs", "os", "io",
];

fn find_decl(
    program: &Program,
    index: usize,
    a: &Analysis,
    target: &Option<Ref>,
) -> Option<(DeclIn, Decl)> {
    let target = target.as_ref()?;
    if let Ref::Local(def) = target {
        // A local is bound in the file being analyzed.
        let decl = Decl {
            name: *def,
            item: *def,
        };
        return Some((DeclIn::File(index), decl));
    }
    if let Ref::Component(file) = target {
        let i = program
            .sources
            .iter()
            .position(|s| s.path.ends_with(file))?;
        let name = match &program.parsed[i] {
            Parsed::Glaze(g) => g.props.as_ref().map(|p| p.span).unwrap_or(Span::new(0, 0)),
            _ => return None,
        };
        return Some((DeclIn::File(i), Decl { name, item: name }));
    }
    let is_std = match target {
        Ref::Type(id) | Ref::Field(id, _) | Ref::Method(id, _) | Ref::Variant(id, _) => {
            *id < a.std_adts
        }
        Ref::Fn(name) => !program.modules().any(|(_, m)| declares_fn(m, name)),
        Ref::Module(_) | Ref::ModuleFn(..) => true,
        Ref::Const(_) | Ref::Local(_) | Ref::Component(_) | Ref::Interface(_) => false,
        Ref::StyleKey { .. } | Ref::Token { .. } | Ref::Theme(_) => return None,
    };
    if !is_std {
        return program
            .modules()
            .find_map(|(i, m)| find_in(m, a, target).map(|d| (DeclIn::File(i), d)));
    }
    let files: Vec<&'static str> = match target {
        Ref::ModuleFn(m, _) | Ref::Module(m) => STD_FILES
            .iter()
            .copied()
            .filter(|f| *f == a.modules[*m].name)
            .collect(),
        _ => STD_FILES.to_vec(),
    };
    if let Ref::Module(_) = target {
        return None;
    }
    for file in files {
        let module = kiln_syntax::parse(kiln_check::std_source(file)?).module;
        if let Some(decl) = find_in(&module, a, target) {
            return Some((DeclIn::Std(file), decl));
        }
    }
    None
}

fn declares_fn(module: &Module, name: &str) -> bool {
    module
        .items
        .iter()
        .any(|item| matches!(item, Item::Fn(f) if f.name.name == name))
}

/// The declaration `target` refers to, if `module` has it.
fn find_in(module: &Module, a: &Analysis, target: &Ref) -> Option<Decl> {
    let adt_name = |id: &usize| a.adts[*id].name.as_str();
    module.items.iter().find_map(|item| match (item, target) {
        (Item::Fn(f), Ref::Fn(name) | Ref::ModuleFn(_, name)) if f.name.name == *name => {
            Some(Decl {
                name: f.name.span,
                item: f.span,
            })
        }
        (Item::Const(c), Ref::Const(name)) if c.name.name == *name => Some(Decl {
            name: c.name.span,
            item: c.span,
        }),
        (Item::Interface(i), Ref::Interface(id)) if i.name.name == a.interfaces[*id].name => {
            Some(Decl {
                name: i.name.span,
                item: i.span,
            })
        }
        (Item::Struct(s), Ref::Type(id)) if s.name.name == adt_name(id) => Some(Decl {
            name: s.name.span,
            item: s.span,
        }),
        (Item::Enum(e), Ref::Type(id)) if e.name.name == adt_name(id) => Some(Decl {
            name: e.name.span,
            item: e.span,
        }),
        (Item::Struct(s), Ref::Field(id, field)) if s.name.name == adt_name(id) => s
            .fields
            .iter()
            .find(|f| f.name.name == *field)
            .map(|f| Decl {
                name: f.name.span,
                item: f.span,
            }),
        (Item::Enum(e), Ref::Variant(id, variant)) if e.name.name == adt_name(id) => e
            .variants
            .iter()
            .find(|v| v.name.name == *variant)
            .map(|v| Decl {
                name: v.name.span,
                item: v.span,
            }),
        (Item::Impl(imp), Ref::Method(id, method)) if impl_name(imp) == Some(adt_name(id)) => imp
            .methods
            .iter()
            .find(|m| m.name.name == *method)
            .map(|m| Decl {
                name: m.name.span,
                item: m.span,
            }),
        _ => None,
    })
}

fn impl_name(imp: &kiln_syntax::ast::ImplDecl) -> Option<&str> {
    match &imp.target.kind {
        TypeKind::Named { path, .. } => path.last().map(|n| n.name.as_str()),
        _ => None,
    }
}

// ----- completion -----

const KEYWORDS: &[&str] = &[
    "fn", "let", "var", "const", "struct", "enum", "impl", "pub", "import", "if", "else", "for",
    "in", "where", "while", "break", "continue", "return", "match", "try", "catch", "throw", "and",
    "or", "not", "is", "true", "false", "none", "store",
];

// LSP completion item kinds.
const METHOD: u32 = 2;
const FUNCTION: u32 = 3;
const FIELD: u32 = 5;
const VARIABLE: u32 = 6;
const MODULE: u32 = 9;
const ENUM: u32 = 13;
const KEYWORD: u32 = 14;
const ENUM_MEMBER: u32 = 20;
const CONSTANT: u32 = 21;
const PROPERTY: u32 = 10;
const VALUE: u32 = 12;
const CLASS: u32 = 7;
const STRUCT: u32 = 22;
const FILE: u32 = 17;

/// The completions at `offset`: the name being typed there is replaced by
/// `COMPLETION_MARKER`, and the checker says what fits in its place.
fn completion_bound(sources: &[Source], index: usize, offset: usize) -> Value {
    let text = &sources[index].text;
    let offset = offset.min(text.len());
    let is_name = |c: char| c.is_alphanumeric() || c == '_';
    let start = text[..offset]
        .char_indices()
        .rev()
        .take_while(|(_, c)| is_name(*c))
        .last()
        .map_or(offset, |(i, _)| i);
    let end = offset
        + text[offset..]
            .char_indices()
            .find(|(_, c)| !is_name(*c))
            .map_or(text.len() - offset, |(i, _)| i);
    let marker = if crate::is_glaze(&sources[index].path)
        && (text[..start].ends_with('<') || text[..start].ends_with("</"))
    {
        kiln_check::ide::COMPONENT_COMPLETION_MARKER
    } else {
        COMPLETION_MARKER
    };
    let marked = format!("{}{marker}{}", &text[..start], &text[end..]);
    let mut marked_sources: Vec<Source> = sources
        .iter()
        .map(|s| Source {
            path: s.path.clone(),
            uri: s.uri.clone(),
            text: s.text.clone(),
        })
        .collect();
    marked_sources[index].text = marked;
    let program = Program::parse(&marked_sources);
    let glaze = crate::is_glaze(&sources[index].path);
    let completion = program.analyze(index).and_then(|a| {
        let c = a.completion.clone();
        c.map(|c| (a, c))
    });
    let items = match completion {
        Some((a, c)) => items(&a, &c, glaze),
        // No name was expected there. Between declarations of a `.kiln`
        // file, keywords still fit; in a comment, a string or a glaze's
        // HTML, nothing does.
        None if !glaze && is_code(&marked_sources[index].text) => keyword_items(),
        None => Vec::new(),
    };
    json!({"isIncomplete": false, "items": items})
}

/// Whether the marker is a name in code, not in a comment or string.
fn is_code(text: &str) -> bool {
    let (tokens, _) = kiln_syntax::lexer::lex(text);
    tokens.iter().any(
        |t| matches!(&t.kind, kiln_syntax::token::TokenKind::Ident(n) if n == COMPLETION_MARKER),
    )
}

fn item(label: &str, kind: u32, detail: String, rank: u32) -> Value {
    json!({
        "label": label,
        "kind": kind,
        "detail": detail,
        "sortText": format!("{rank}{label}"),
    })
}

fn keyword_items() -> Vec<Value> {
    KEYWORDS
        .iter()
        .map(|k| item(k, KEYWORD, String::new(), 5))
        .collect()
}

fn adt_kind(a: &Analysis, id: usize) -> u32 {
    match a.adts[id].kind {
        AdtKind::Struct(_) => STRUCT,
        AdtKind::Enum(_) => ENUM,
    }
}

fn items(a: &Analysis, completion: &Completion, glaze: bool) -> Vec<Value> {
    let mut out = Vec::new();
    match completion {
        Completion::Components(components) => {
            for (tag, file) in components {
                out.push(item(tag, STRUCT, file.clone(), 0));
            }
        }
        Completion::Names(locals) => {
            for (name, ty) in locals {
                out.push(item(name, VARIABLE, a.show(ty), 0));
            }
            for (name, sig) in &a.global_fns {
                out.push(item(name, FUNCTION, a.show_fn(sig), 1));
            }
            for (name, ty) in &a.consts {
                out.push(item(name, CONSTANT, a.show(ty), 1));
            }
            for (name, &id) in &a.global_types {
                out.push(item(name, adt_kind(a, id), a.show_adt(id), 2));
            }
            for (name, &m) in &a.imports {
                out.push(item(
                    name,
                    MODULE,
                    format!("import {}", a.modules[m].name),
                    3,
                ));
            }
            if !glaze {
                // Modules not imported yet, with the import that brings them in.
                for m in &a.modules {
                    if a.imports.values().any(|&i| a.modules[i].name == m.name) {
                        continue;
                    }
                    let mut entry = item(&m.name, MODULE, format!("import {}", m.name), 4);
                    entry["additionalTextEdits"] = json!([{
                        "range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                        "newText": format!("import {}\n", m.name),
                    }]);
                    out.push(entry);
                }
                out.extend(keyword_items());
            } else {
                for k in ["true", "false", "none", "and", "or", "not", "is"] {
                    out.push(item(k, KEYWORD, String::new(), 5));
                }
            }
        }
        Completion::Members(ty) => match ty {
            Ty::Adt(id, args) => {
                let adt = &a.adts[*id];
                let map = adt
                    .params
                    .iter()
                    .cloned()
                    .zip(args.iter().cloned())
                    .collect();
                if let AdtKind::Struct(fields) = &adt.kind {
                    for f in fields {
                        let ty = kiln_check::types::substitute(&f.ty, &map);
                        out.push(item(&f.name, FIELD, a.show(&ty), 0));
                    }
                }
                for m in adt
                    .methods
                    .iter()
                    .filter(|m| m.has_self && !m.name.starts_with("__"))
                {
                    out.push(item(&m.name, METHOD, a.show_fn(m), 1));
                }
            }
            Ty::Record(fields) => {
                for (name, ty) in fields {
                    out.push(item(name, FIELD, a.show(ty), 0));
                }
            }
            _ => {}
        },
        Completion::Module(m) => {
            let module = &a.modules[*m];
            let shown = |name: &&String| !kiln_check::internal_name(&module.name, name);
            for (name, sig) in module.fns.iter().filter(|(n, _)| shown(n)) {
                out.push(item(name, FUNCTION, a.show_fn(sig), 0));
            }
            for (name, &id) in module.types.iter().filter(|(n, _)| shown(n)) {
                out.push(item(name, adt_kind(a, id), a.show_adt(id), 1));
            }
        }
        Completion::TypeMembers(id) => {
            let adt = &a.adts[*id];
            if let AdtKind::Enum(variants) = &adt.kind {
                for v in variants {
                    out.push(item(&v.name, ENUM_MEMBER, a.show_variant(v), 0));
                }
            }
            for m in adt.methods.iter().filter(|m| !m.has_self) {
                out.push(item(&m.name, FUNCTION, a.show_fn(m), 1));
            }
        }
        Completion::Types => {
            for (name, &id) in &a.global_types {
                out.push(item(name, adt_kind(a, id), a.show_adt(id), 0));
            }
            for (name, &m) in &a.imports {
                out.push(item(
                    name,
                    MODULE,
                    format!("import {}", a.modules[m].name),
                    1,
                ));
            }
        }
        Completion::ModuleTypes(m) => {
            let module = &a.modules[*m];
            for (name, &id) in module
                .types
                .iter()
                .filter(|(n, _)| !kiln_check::internal_name(&module.name, n))
            {
                out.push(item(name, adt_kind(a, id), a.show_adt(id), 0));
            }
        }
        Completion::Style(words) => {
            for w in words {
                let kind = match w.kind {
                    StyleWordKind::Key => PROPERTY,
                    StyleWordKind::Token => CONSTANT,
                    StyleWordKind::Keyword => VALUE,
                    StyleWordKind::Theme => CLASS,
                };
                out.push(item(&w.label, kind, w.detail.clone(), 0));
            }
        }
    }
    out
}

// ----- symbols -----

// LSP symbol kinds.
const SYM_CLASS: u32 = 5;
const SYM_METHOD: u32 = 6;
const SYM_FIELD: u32 = 8;
const SYM_ENUM: u32 = 10;
const SYM_INTERFACE: u32 = 11;
const SYM_FUNCTION: u32 = 12;
const SYM_VARIABLE: u32 = 13;
const SYM_CONSTANT: u32 = 14;
const SYM_ENUM_MEMBER: u32 = 22;
const SYM_STRUCT: u32 = 23;

/// A declaration, as the outline shows it.
struct Symbol {
    name: String,
    kind: u32,
    span: Span,
    name_span: Span,
    children: Vec<Symbol>,
}

fn symbol(name: &str, kind: u32, span: Span, name_span: Span) -> Symbol {
    Symbol {
        name: name.to_string(),
        kind,
        span,
        name_span,
        children: Vec::new(),
    }
}

/// The declarations of a file, from what parsed.
fn symbols(path: &Path, text: &str) -> Vec<Symbol> {
    if crate::is_glaze(path) {
        let glaze = kiln_syntax::glaze::parse_component(text, &path.to_string_lossy()).glaze;
        let props = glaze.props.iter().flat_map(|p| &p.params);
        let props = props.map(|p| symbol(&p.name.name, SYM_FIELD, p.span, p.name.span));
        let states = glaze.states.iter();
        let states = states.map(|s| symbol(&s.name.name, SYM_VARIABLE, s.span, s.name.span));
        return named(props.chain(states).collect());
    }
    let module = kiln_syntax::parse(text).module;
    let mut out = Vec::new();
    for item in &module.items {
        match item {
            Item::Fn(f) => out.push(symbol(&f.name.name, SYM_FUNCTION, f.span, f.name.span)),
            Item::Const(c) => {
                let kind = if c.store.is_some() {
                    SYM_VARIABLE
                } else {
                    SYM_CONSTANT
                };
                out.push(symbol(&c.name.name, kind, c.span, c.name.span))
            }
            Item::Struct(s) => {
                let mut sym = symbol(&s.name.name, SYM_STRUCT, s.span, s.name.span);
                sym.children = s
                    .fields
                    .iter()
                    .map(|f| symbol(&f.name.name, SYM_FIELD, f.span, f.name.span))
                    .collect();
                out.push(sym);
            }
            Item::Enum(e) => {
                let mut sym = symbol(&e.name.name, SYM_ENUM, e.span, e.name.span);
                sym.children = e
                    .variants
                    .iter()
                    .map(|v| symbol(&v.name.name, SYM_ENUM_MEMBER, v.span, v.name.span))
                    .collect();
                out.push(sym);
            }
            Item::Impl(imp) => {
                let target = &imp.target.span;
                let name = format!("impl {}", &text[target.start as usize..target.end as usize]);
                let mut sym = symbol(&name, SYM_CLASS, imp.span, imp.target.span);
                sym.children = imp
                    .methods
                    .iter()
                    .map(|m| symbol(&m.name.name, SYM_METHOD, m.span, m.name.span))
                    .collect();
                out.push(sym);
            }
            Item::Interface(i) => {
                let mut sym = symbol(&i.name.name, SYM_INTERFACE, i.span, i.name.span);
                sym.children = i
                    .methods
                    .iter()
                    .map(|m| symbol(&m.name.name, SYM_METHOD, m.span, m.name.span))
                    .collect();
                out.push(sym);
            }
            Item::Import(_) => {}
        }
    }
    named(out)
}

/// Leaves out declarations whose name is not written yet, which editors
/// reject, and stretches each one to hold its name, as they also require.
fn named(symbols: Vec<Symbol>) -> Vec<Symbol> {
    symbols
        .into_iter()
        .filter(|s| !s.name.trim().is_empty() && s.name.trim() != "impl")
        .map(|mut s| {
            s.span.start = s.span.start.min(s.name_span.start);
            s.span.end = s.span.end.max(s.name_span.end);
            s.children = named(s.children);
            s
        })
        .collect()
}

fn document_symbol(text: &str, s: &Symbol) -> Value {
    json!({
        "name": s.name,
        "kind": s.kind,
        "range": range(text, s.span.start as usize, s.span.end as usize),
        "selectionRange": range(text, s.name_span.start as usize, s.name_span.end as usize),
        "children": s.children.iter().map(|c| document_symbol(text, c)).collect::<Vec<_>>(),
    })
}

pub(super) fn document_symbols(path: &Path, text: &str) -> Value {
    let symbols = symbols(path, text);
    Value::Array(symbols.iter().map(|s| document_symbol(text, s)).collect())
}

/// Whether the letters of `query` appear in `name` in order, ignoring case.
fn matches_query(name: &str, query: &str) -> bool {
    let mut chars = name.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .all(|q| chars.any(|c| c == q))
}

/// The declarations matching `query` in the `.kiln` files under `roots`,
/// using the editor's text for open files.
pub(super) fn workspace_symbols(roots: &[PathBuf], open: &[Source], query: &str) -> Value {
    let mut files: Vec<PathBuf> = Vec::new();
    for root in roots {
        collect_sources(root, &mut files, 0);
    }
    for s in open {
        if !files.contains(&s.path) {
            files.push(s.path.clone());
        }
    }
    let mut out = Vec::new();
    for path in files {
        if crate::is_glaze(&path) {
            continue;
        }
        let open_source = open.iter().find(|s| s.path == path);
        let text = match open_source {
            Some(s) => s.text.clone(),
            None => match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(_) => continue,
            },
        };
        let uri = open_source
            .and_then(|s| s.uri.clone())
            .unwrap_or_else(|| path_to_uri(&path));
        let mut push = |s: &Symbol, container: Option<&str>| {
            if matches_query(&s.name, query) && out.len() < 500 {
                out.push(json!({
                    "name": s.name,
                    "kind": s.kind,
                    "containerName": container,
                    "location": {
                        "uri": uri,
                        "range": range(&text, s.name_span.start as usize, s.name_span.end as usize),
                    },
                }));
            }
        };
        for s in symbols(&path, &text) {
            push(&s, None);
            let container = s.name.trim_start_matches("impl ").to_string();
            for c in &s.children {
                push(c, Some(&container));
            }
        }
    }
    Value::Array(out)
}

/// The `.kiln` and `.glaze` files under `dir`, skipping hidden folders,
/// build output and dependencies.
fn collect_sources(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 12 || out.len() > 5000 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = file_name(&path);
        if path.is_dir() {
            if !name.starts_with('.') && name != "target" && name != "node_modules" {
                collect_sources(&path, out, depth + 1);
            }
        } else if path.extension().is_some_and(|e| e == "kiln") || crate::is_glaze(&path) {
            out.push(path);
        }
    }
}

// Folder binding is shared with the compiler. Translate request offsets into
// resolved source and response ranges back to the user's original documents.
pub(super) fn hover(sources: &[Source], index: usize, offset: usize) -> Value {
    if let Some((span, routes)) = mount_at(sources, index, offset) {
        let mut shown = format!(
            "**`app.file_routes`** adds {} route(s) here, tried in this order:\n",
            routes.len()
        );
        let root = crate::project::common_root(sources.iter().map(|s| s.path.as_path()));
        for r in &routes {
            let file = sources[r.file]
                .path
                .strip_prefix(&root)
                .unwrap_or(&sources[r.file].path)
                .to_string_lossy()
                .replace('\\', "/");
            shown.push_str(&format!(
                "\n- `{} {}` from `{file}`{}",
                r.method.to_uppercase(),
                r.pattern,
                if r.dynamic { " (dynamic)" } else { "" }
            ));
        }
        return json!({
            "contents": {"kind": "markdown", "value": shown},
            "range": range(&sources[index].text, span.start as usize, span.end as usize),
        });
    }
    bound_request(sources, index, offset, hover_bound)
}

/// The `app.file_routes(...)` call at `offset`, and the routes it adds.
fn mount_at(
    sources: &[Source],
    index: usize,
    offset: usize,
) -> Option<(Span, Vec<crate::project::routes::RouteInfo>)> {
    if crate::is_glaze(&sources[index].path) {
        return None;
    }
    let module = kiln_syntax::parse(&sources[index].text).module;
    let call = crate::project::routes::mount_calls(&module)
        .into_iter()
        .find(|c| c.span.start as usize <= offset && offset <= c.span.end as usize)?;
    let root = crate::project::common_root(sources.iter().map(|s| s.path.as_path()));
    let texts: Vec<_> = sources
        .iter()
        .map(|s| (s.path.clone(), s.text.clone()))
        .collect();
    let bound = crate::project::bind::bind(&root, &texts);
    let routes = bound
        .routes
        .into_iter()
        .filter(|r| r.mount == (index, call.span))
        .collect();
    Some((call.span, routes))
}
pub(super) fn references(sources: &[Source], index: usize, offset: usize) -> Value {
    bound_request(sources, index, offset, references_bound)
}
pub(super) fn definition(sources: &[Source], index: usize, offset: usize) -> Value {
    // From a mount, the files its routes come from.
    if let Some((_, routes)) = mount_at(sources, index, offset) {
        let mut seen = Vec::new();
        let locations: Vec<Value> = routes
            .iter()
            .filter(|r| {
                let new = !seen.contains(&r.file);
                seen.push(r.file);
                new
            })
            .map(|r| {
                let s = &sources[r.file];
                json!({
                    "uri": s.uri.clone().unwrap_or_else(|| path_to_uri(&s.path)),
                    "range": range(&s.text, r.span.start as usize, r.span.end as usize),
                })
            })
            .collect();
        return Value::Array(locations);
    }
    if let Some(location) = import_definition(sources, index, offset)
        .or_else(|| component_path_definition(sources, index, offset))
    {
        return location;
    }
    bound_request(sources, index, offset, definition_bound)
}

/// The start of a file, as a location.
fn file_location(sources: &[Source], path: &Path) -> Value {
    let uri = sources
        .iter()
        .find(|s| s.path == path)
        .and_then(|s| s.uri.clone())
        .unwrap_or_else(|| path_to_uri(path));
    json!({"uri": uri, "range": range("", 0, 0)})
}

/// From a module in an `import`, its folder's main file: the one named
/// after the folder, or else its first `.kiln` file.
fn import_definition(sources: &[Source], index: usize, offset: usize) -> Option<Value> {
    let source = &sources[index];
    if crate::is_glaze(&source.path) {
        return None;
    }
    let module = kiln_syntax::parse(&source.text).module;
    let root = crate::project::common_root(sources.iter().map(|s| s.path.as_path()));
    for item in &module.items {
        let Item::Import(import) = item else { continue };
        for path in &import.paths {
            let Some(at) = path
                .iter()
                .position(|i| (i.span.start as usize..=i.span.end as usize).contains(&offset))
            else {
                continue;
            };
            let folder = path[..=at]
                .iter()
                .fold(root.clone(), |dir, part| dir.join(&part.name));
            let mut files: Vec<PathBuf> = std::fs::read_dir(crate::project::real(&folder))
                .ok()?
                .filter_map(Result::ok)
                .map(|e| folder.join(e.file_name()))
                .filter(|p| p.extension().is_some_and(|e| e == "kiln"))
                .collect();
            files.sort();
            let main = folder.join(format!("{}.kiln", path[at].name));
            let file = if files.contains(&main) {
                main
            } else {
                files.into_iter().next()?
            };
            return Some(file_location(sources, &file));
        }
    }
    None
}

/// From a component path in `web.render(...)` or `{% layout %}`, the
/// component's file.
fn component_path_definition(sources: &[Source], index: usize, offset: usize) -> Option<Value> {
    let source = &sources[index];
    let slot = path_slot(&source.text, offset, crate::is_glaze(&source.path))?;
    let rest = &source.text[slot.start..];
    let path = &rest[..rest.find(['"', '\n']).unwrap_or(rest.len())];
    let root = crate::project::common_root(sources.iter().map(|s| s.path.as_path()));
    let file = if path.contains('/') {
        root.join(path)
    } else {
        source.path.parent()?.join(path)
    };
    (sources.iter().any(|s| s.path == file) || file.is_file())
        .then(|| file_location(sources, &file))
}
pub(super) fn completion(sources: &[Source], index: usize, offset: usize) -> Value {
    let text = &sources[index].text;
    let offset = offset.min(text.len());
    if let Some(result) = component_path_completion(sources, index, offset) {
        return result;
    }
    if let Some(result) = qualified_tag_completion(sources, index, offset) {
        return result;
    }
    // `"` and `/` ask for component paths; anywhere else nothing fits
    // right after them.
    if text[..offset].ends_with(['"', '/']) {
        return json!({"isIncomplete": false, "items": []});
    }
    if let Some(result) = module_completion(sources, index, offset) {
        return result;
    }
    bound_request(sources, index, offset, completion_bound)
}

/// After `<posts.` in a glaze or ember: the components in `posts/` and
/// the folders inside it, named as a qualified tag names them.
fn qualified_tag_completion(sources: &[Source], index: usize, offset: usize) -> Option<Value> {
    let source = &sources[index];
    if !crate::is_glaze(&source.path) {
        return None;
    }
    let before = &source.text[..offset];
    let start = before.rfind(|c: char| !(c.is_alphanumeric() || "_.[]".contains(c)))?;
    if !before[..=start].ends_with('<') {
        return None;
    }
    let (folder, _) = before[start + 1..].rsplit_once('.')?;
    if folder.is_empty() || folder.split('.').any(str::is_empty) {
        return None;
    }
    let root = crate::project::common_root(sources.iter().map(|s| s.path.as_path()));
    let dir = folder.split('.').fold(root, |dir, part| dir.join(part));
    let mut items = std::collections::BTreeMap::new();
    let mut files: Vec<PathBuf> = sources
        .iter()
        .map(|s| s.path.clone())
        .filter(|p| p.parent() == Some(dir.as_path()))
        .collect();
    for entry in std::fs::read_dir(crate::project::real(&dir))
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = dir.join(entry.file_name());
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.path().is_dir() && !name.starts_with(['.', '_']) {
            let mut inner = Vec::new();
            let _ = crate::collect_sources(&path, &mut inner);
            if !inner.is_empty() {
                items.insert(name.clone(), json!({"label": name, "kind": MODULE}));
            }
        } else {
            files.push(path);
        }
    }
    for file in files.iter().filter(|f| crate::is_glaze(f)) {
        let stem = file.file_stem()?.to_string_lossy().into_owned();
        let tag: String = stem
            .split('_')
            .map(|w| {
                let mut c = w.chars();
                c.next()
                    .map(|f| f.to_ascii_uppercase().to_string() + c.as_str())
                    .unwrap_or_default()
            })
            .collect();
        let kind = if file.extension().is_some_and(|e| e == "ember") {
            "Ember"
        } else {
            "Glaze"
        };
        let detail = file
            .strip_prefix(crate::project::common_root(
                sources.iter().map(|s| s.path.as_path()),
            ))
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        items.insert(
            tag.clone(),
            json!({"label": tag, "kind": CLASS, "detail": format!("{kind} {detail}")}),
        );
    }
    Some(json!({"isIncomplete": false, "items": items.into_values().collect::<Vec<_>>()}))
}

/// Where a component path is being typed: in the first string of
/// `web.render(...)` or of a glaze's `{% layout ... %}`.
#[derive(Debug, PartialEq)]
struct PathSlot {
    /// The string's contents, as byte offsets.
    start: usize,
    end: usize,
    /// Whether embers fit: a layout is a glaze.
    embers: bool,
}

fn path_slot(text: &str, offset: usize, glaze: bool) -> Option<PathSlot> {
    let line_start = text[..offset].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[line_start..offset];
    // Inside a string: an odd number of quotes before the cursor.
    if line.matches('"').count().is_multiple_of(2) {
        return None;
    }
    let quote = line_start + line.rfind('"')?;
    let before = text[..quote].trim_end();
    let embers = if before.ends_with("render(") || before.ends_with("render (") {
        true
    } else if glaze && before.ends_with("{% layout") {
        false
    } else {
        return None;
    };
    let rest = &text[offset..];
    let line_end = rest.find('\n').unwrap_or(rest.len());
    let end = offset + rest[..line_end].find('"').unwrap_or(0);
    Some(PathSlot {
        start: quote + 1,
        end,
        embers,
    })
}

/// Glazes and embers to complete in a `web.render` or layout path, named
/// as the compiler finds them: by file name in the file's own folder, and
/// from the project root elsewhere.
fn component_path_completion(sources: &[Source], index: usize, offset: usize) -> Option<Value> {
    let source = &sources[index];
    let slot = path_slot(&source.text, offset, crate::is_glaze(&source.path))?;
    let root = crate::project::common_root(sources.iter().map(|s| s.path.as_path()));
    let own = source
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let mut files: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
    let _ = crate::collect_sources(&root, &mut files);
    let mut targets = std::collections::BTreeSet::new();
    for file in &files {
        let ember = file.extension().is_some_and(|e| e == "ember");
        if !crate::is_glaze(file) || (ember && !slot.embers) || *file == source.path {
            continue;
        }
        let Ok(relative) = file.strip_prefix(&root) else {
            continue;
        };
        let parts: Vec<String> = relative
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        // Wares and kiln's own folders are not the project's.
        if parts
            .iter()
            .any(|p| p.starts_with("__") || p.starts_with('.'))
        {
            continue;
        }
        let name = if file.parent() == Some(own.as_path()) {
            parts.last()?.clone()
        } else if parts.len() > 1 {
            parts.join("/")
        } else {
            // A root file is named from the root only from the root.
            continue;
        };
        targets.insert((file.parent() != Some(own.as_path()), name));
    }
    let range = range(&source.text, slot.start, slot.end);
    let items: Vec<Value> = targets
        .into_iter()
        .map(|(elsewhere, name)| {
            json!({
                "label": name,
                "kind": FILE,
                "detail": if name.ends_with(".ember") { "Ember" } else { "Glaze" },
                "sortText": format!("{}{name}", u8::from(elsewhere)),
                "filterText": name,
                "textEdit": {"range": range, "newText": name},
            })
        })
        .collect();
    Some(json!({"isIncomplete": false, "items": items}))
}
fn bound_request(
    sources: &[Source],
    index: usize,
    offset: usize,
    request: fn(&[Source], usize, usize) -> Value,
) -> Value {
    let root = crate::project::common_root(sources.iter().map(|s| s.path.as_path()));
    let texts = sources
        .iter()
        .map(|s| (s.path.clone(), s.text.clone()))
        .collect::<Vec<_>>();
    let bound = crate::project::bind::bind(&root, &texts);
    let transformed = bound
        .texts
        .iter()
        .zip(sources)
        .map(|((p, t), s)| Source {
            path: p.clone(),
            text: t.clone(),
            uri: s.uri.clone(),
        })
        .collect::<Vec<_>>();
    let mut result = request(&transformed, index, bound.maps[index].generated(offset));
    fn remap(
        value: &mut Value,
        index: usize,
        original: &[Source],
        resolved: &[Source],
        maps: &[crate::project::bind::SourceMap],
    ) {
        match value {
            Value::Object(object) => {
                if let Some(uri) = object.get("uri").and_then(Value::as_str) {
                    if !original
                        .iter()
                        .any(|s| s.uri.as_deref() == Some(uri) || path_to_uri(&s.path) == uri)
                    {
                        return;
                    }
                }
                let target = object
                    .get("uri")
                    .and_then(Value::as_str)
                    .and_then(|uri| {
                        original.iter().position(|s| {
                            s.uri.as_deref() == Some(uri) || path_to_uri(&s.path) == uri
                        })
                    })
                    .unwrap_or(index);
                for (key, child) in object {
                    if key == "range" && child.get("start").is_some() {
                        let start = super::offset_of(&resolved[target].text, &child["start"]);
                        let end = super::offset_of(&resolved[target].text, &child["end"]);
                        let span = maps[target].original(Span::new(start, end));
                        *child = range(
                            &original[target].text,
                            span.start as usize,
                            span.end as usize,
                        );
                    } else {
                        remap(child, target, original, resolved, maps);
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    remap(item, index, original, resolved, maps);
                }
            }
            Value::String(text) => {
                for (source, map) in original.iter().zip(maps) {
                    for edit in &map.edits {
                        if edit.text.starts_with("__kiln_module_")
                            || edit.text.starts_with("KilnModule_")
                        {
                            *text = text.replace(
                                &edit.text,
                                &source.text[edit.span.start as usize..edit.span.end as usize],
                            );
                        }
                    }
                }
            }
            _ => {}
        }
    }
    // Do not offer private implementation names from other modules.
    if let Some(items) = result.get_mut("items").and_then(Value::as_array_mut) {
        let own = sources[index].path.parent();
        items.retain(|item| {
            let label = item["label"].as_str().unwrap_or_default();
            if !(label.starts_with("__kiln_module_") || label.starts_with("KilnModule_")) {
                return true;
            }
            sources
                .iter()
                .zip(&bound.maps)
                .any(|(s, m)| s.path.parent() == own && m.edits.iter().any(|e| e.text == label))
        });
    }
    remap(&mut result, index, sources, &transformed, &bound.maps);
    result
}
fn module_completion(sources: &[Source], index: usize, offset: usize) -> Option<Value> {
    let text = &sources[index].text;
    let before = &text[..offset];
    let line = before.rsplit('\n').next().unwrap_or("").trim_start();
    let root = crate::project::common_root(sources.iter().map(|s| s.path.as_path()));
    if let Some(paths_typed) = line.strip_prefix("import ") {
        let mut paths: Vec<PathBuf> = sources.iter().map(|s| s.path.clone()).collect();
        let _ = crate::collect_sources(&root, &mut paths);
        let own = sources[index].path.parent();
        let modules: std::collections::BTreeSet<String> = paths
            .iter()
            .filter_map(|p| p.parent())
            .filter(|dir| Some(*dir) != own)
            .filter_map(|dir| dir.strip_prefix(&root).ok())
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().replace(['/', '\\'], "."))
            .filter(|m| !m.split('.').any(|part| part.starts_with("__")))
            .collect();
        // The path being typed: after `import ` or the last `,`.
        let typed = paths_typed
            .rsplit(',')
            .next()
            .unwrap_or("")
            .trim_start();
        let items: Vec<Value> = match typed.rsplit_once('.') {
            // After `posts.`, the folders inside `posts/`.
            Some((parent, _)) => modules
                .iter()
                .filter_map(|m| m.strip_prefix(parent)?.strip_prefix('.'))
                .filter(|child| !child.contains('.'))
                .map(|child| json!({"label": child, "kind": MODULE}))
                .collect(),
            None => modules
                .iter()
                .map(|m| json!({"label": m, "kind": MODULE}))
                .collect(),
        };
        return Some(json!({"isIncomplete": false, "items": items}));
    }
    let word = before
        .rsplit(|c: char| !c.is_alphanumeric() && c != '_' && c != '.')
        .next()?;
    let (alias, _) = word.rsplit_once('.')?;
    let parsed = kiln_syntax::parse(text);
    let module = parsed.module.items.iter().find_map(|i| {
        if let Item::Import(i) = i {
            i.paths
                .iter()
                .find(|p| p.last().is_some_and(|p| p.name == alias))
                .map(|p| {
                    p.iter()
                        .map(|p| p.name.as_str())
                        .collect::<Vec<_>>()
                        .join("/")
                })
        } else {
            None
        }
    })?;
    let dir = root.join(module);
    if !sources.iter().any(|s| s.path.parent() == Some(&dir)) {
        return None;
    }
    let mut items = Vec::new();
    for source in sources.iter().filter(|s| s.path.parent() == Some(&dir)) {
        for item in kiln_syntax::parse(&source.text).module.items {
            let (name, public, kind) = match item {
                Item::Fn(f) => (f.name, f.is_pub, FUNCTION),
                Item::Struct(s) => (s.name, s.is_pub, STRUCT),
                Item::Enum(e) => (e.name, e.is_pub, ENUM),
                Item::Const(c) => (c.name, c.is_pub, CONSTANT),
                _ => continue,
            };
            if public {
                items.push(json!({"label":name.name,"kind":kind}));
            }
        }
    }
    Some(json!({"isIncomplete":false,"items":items}))
}

#[cfg(test)]
mod slot_tests {
    use super::*;

    #[test]
    fn component_paths_are_completed_in_render_and_layout_strings() {
        let text = "return web.render(\"pa\", { posts })";
        let at = text.find("pa").unwrap() + 2;
        assert_eq!(
            path_slot(text, at, false),
            Some(PathSlot {
                start: at - 2,
                end: at,
                embers: true
            })
        );
        let text = "{% layout \"bas\" %}";
        let slot = path_slot(text, text.find("bas").unwrap() + 1, true).unwrap();
        assert!(!slot.embers);
        assert_eq!(&text[slot.start..slot.end], "bas");
        // Not the second argument, other calls or code.
        assert!(path_slot("web.render(\"a.glaze\", \"x", 24, false).is_none());
        assert!(path_slot("print(\"pa", 8, false).is_none());
        assert!(path_slot("web.render(pa", 13, false).is_none());
        assert!(path_slot("{% layout \"x", 12, false).is_none());
    }
}

#[cfg(test)]
mod path_tests {
    use super::*;

    #[test]
    fn file_uris_exclude_windows_verbatim_prefixes() {
        assert_eq!(
            path_to_uri(Path::new(r"\\?\C:\my app\main.kiln")),
            "file:///C:/my%20app/main.kiln"
        );
        assert_eq!(
            path_to_uri(Path::new(r"\\?\UNC\server\share\main.kiln")),
            "file://server/share/main.kiln"
        );
    }
}
