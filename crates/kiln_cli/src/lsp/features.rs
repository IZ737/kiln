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
        let names: Vec<String> = self.sources.iter().map(|s| file_name(&s.path)).collect();
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
        let styling = crate::styles::load(&self.dir(index), &glaze_files).styling;
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
    if !text.starts_with('/') {
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

pub(super) fn hover(sources: &[Source], index: usize, offset: usize) -> Value {
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
        Ref::Const(name) => match a.consts.get(name) {
            Some(ty) => format!("const {name}: {}", a.show(ty)),
            None => format!("const {name}"),
        },
        Ref::Type(id) => a.show_adt(*id),
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

pub(super) fn definition(sources: &[Source], index: usize, offset: usize) -> Value {
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
            .position(|s| file_name(&s.path) == *file)?;
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
        Ref::Const(_) | Ref::Local(_) | Ref::Component(_) => false,
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
    "or", "not", "is", "true", "false", "none",
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

/// The completions at `offset`: the name being typed there is replaced by
/// `COMPLETION_MARKER`, and the checker says what fits in its place.
pub(super) fn completion(sources: &[Source], index: usize, offset: usize) -> Value {
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
                for m in adt.methods.iter().filter(|m| m.has_self) {
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
            for (name, sig) in &module.fns {
                out.push(item(name, FUNCTION, a.show_fn(sig), 0));
            }
            for (name, &id) in &module.types {
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
            for (name, &id) in &a.modules[*m].types {
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
            Item::Const(c) => out.push(symbol(&c.name.name, SYM_CONSTANT, c.span, c.name.span)),
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
