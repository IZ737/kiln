//! The CSS of an app: the stylesheets its glazes import, their CSS
//! Modules and `<style>`s, the files those refer to, and the `static/`
//! folder. `kiln check` and the language server use it to check CSS and
//! CSS Module class names; `kiln build` also bundles it into the program.

use kiln_css::UrlKind;
use kiln_diagnostics::{codes, Diagnostic, Span};
pub use kiln_ir::glaze::Asset;
use kiln_syntax::ast::{CssImport, ExprKind, Glaze, Node, StrPiece};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// A glaze, as `load` needs it: its file name and parsed form.
pub struct GlazeFile<'a> {
    pub name: &'a str,
    pub glaze: &'a Glaze,
}

#[derive(Default)]
pub struct AppStyles {
    /// The CSS Module class names each glaze can use, and the design
    /// tokens, for the checker.
    pub styling: kiln_check::Styling,
    /// Errors in each glaze's CSS, by glaze file name.
    pub diagnostics: HashMap<String, Vec<Diagnostic>>,
    /// For the compiler: each glaze's CSS Modules, by import name, with
    /// each class (as a Kiln field name) and its renamed class names.
    pub exports: HashMap<String, HashMap<String, BTreeMap<String, String>>>,
    /// The attribute that marks the elements of each glaze with a `<style>`.
    pub scopes: HashMap<String, String>,
    /// The app's stylesheet, the first asset once [`AppStyles::finish`]
    /// makes it, and the files it refers to.
    pub stylesheet: Option<String>,
    pub assets: Vec<Asset>,
    /// The parts of the stylesheet: global stylesheets' URLs, CSS Modules
    /// and scoped styles.
    globals: Vec<String>,
    modules_css: Vec<String>,
    scoped_css: Vec<String>,
}

/// Reads and processes the CSS of the glazes in `dir`.
pub fn load(dir: &Path, glazes: &[GlazeFile]) -> AppStyles {
    let mut l = Loader {
        root: dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()),
        assets: Vec::new(),
        by_file: HashMap::new(),
        globals: Vec::new(),
        properties: Vec::new(),
    };
    let mut out = AppStyles::default();
    let mut modules_css = Vec::new();
    let mut scoped_css = Vec::new();
    for g in glazes {
        let mut diags = Vec::new();
        for import in &g.glaze.css {
            let Some(path) = string_literal(import) else {
                continue; // the checker reports it
            };
            match &import.module {
                None => {
                    if let Err(e) = l.global_file(&path) {
                        diags.push(import_error(import, &path, e));
                    }
                }
                Some(name) => match l.module_file(&path) {
                    Ok(css) => {
                        let fields: BTreeMap<String, String> = css
                            .exports
                            .iter()
                            .filter_map(|(class, renamed)| {
                                field_name(class).map(|f| (f, renamed.clone()))
                            })
                            .collect();
                        out.styling
                            .modules
                            .entry(g.name.to_string())
                            .or_default()
                            .insert(name.name.clone(), fields.keys().cloned().collect());
                        out.exports
                            .entry(g.name.to_string())
                            .or_default()
                            .insert(name.name.clone(), fields);
                        if !modules_css.contains(&css.code) {
                            modules_css.push(css.code);
                        }
                    }
                    Err(e) => {
                        // Unknown classes would only repeat the error.
                        out.styling
                            .modules
                            .entry(g.name.to_string())
                            .or_default()
                            .remove(&name.name);
                        diags.push(import_error(import, &path, e));
                    }
                },
            }
        }
        let styles = style_elements(&g.glaze.nodes);
        if !styles.is_empty() {
            let attr = kiln_css::scope_attribute(g.name);
            for (text, span) in styles {
                let mut resolve =
                    |url: &str, kind: UrlKind| l.resolve(&l.root.clone(), url, kind, false);
                match kiln_css::scoped(text, g.name, &attr, &mut resolve) {
                    Ok(css) => scoped_css.push(css.code),
                    Err(e) => {
                        let at = offset(text, e.line, e.column) + span.start as usize;
                        diags.push(Diagnostic::error(
                            codes::BAD_CSS,
                            format!("this CSS cannot be read: {}", e.message),
                            Span::new(at, at + 1),
                        ));
                    }
                }
            }
            out.scopes.insert(g.name.to_string(), attr);
        }
        if !diags.is_empty() {
            out.diagnostics.insert(g.name.to_string(), diags);
        }
    }

    out.styling.tokens = kiln_check::style::Tokens::from_properties(
        l.properties.iter().map(|(n, v)| (n.as_str(), v.as_str())),
    );
    out.assets = l.assets;
    out.globals = l.globals;
    out.modules_css = modules_css;
    out.scoped_css = scoped_css;
    out
}

impl AppStyles {
    /// Makes the app's stylesheet, with `ui_css`, the CSS of its typed
    /// styles. Kiln's styles are in layers so that their order is fixed:
    /// global stylesheets, then CSS Modules, then scoped styles, then
    /// typed styles. Unlayered CSS from elsewhere comes after all of them.
    pub fn finish(&mut self, ui_css: &str) {
        if self.globals.is_empty()
            && self.modules_css.is_empty()
            && self.scoped_css.is_empty()
            && ui_css.is_empty()
        {
            return;
        }
        let mut sheet = String::from("@layer kiln.global,kiln.modules,kiln.scoped,kiln.ui;");
        for url in &self.globals {
            sheet.push_str(&format!("@import \"{url}\" layer(kiln.global);"));
        }
        let ui = [ui_css.to_string()];
        for (layer, parts) in [
            ("modules", &self.modules_css[..]),
            ("scoped", &self.scoped_css[..]),
            ("ui", &ui[..]),
        ] {
            let css = parts.concat();
            if !css.is_empty() {
                sheet.push_str(&format!("@layer kiln.{layer}{{{css}}}"));
            }
        }
        let path = format!("/_kiln/app-{}.css", kiln_css::short_hash(sheet.as_bytes()));
        self.assets.insert(
            0,
            Asset {
                path: path.clone(),
                content_type: "text/css; charset=utf-8",
                body: sheet.into_bytes(),
                immutable: true,
            },
        );
        self.stylesheet = Some(path);
    }
}

/// The files in the app's `static/` folder, served at `/static/...`.
/// Hidden files and links are left out.
pub fn static_files(dir: &Path) -> Result<Vec<Asset>, String> {
    fn walk(dir: &Path, url: &str, out: &mut Vec<Asset>) -> Result<(), String> {
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
            .flatten()
            .map(|e| e.path())
            .collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if name.starts_with('.') || meta.file_type().is_symlink() {
                continue;
            }
            let child = format!("{url}/{}", encode_segment(&name));
            if meta.is_dir() {
                walk(&path, &child, out)?;
            } else if meta.is_file() {
                let body = std::fs::read(&path)
                    .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
                out.push(Asset {
                    path: child,
                    content_type: content_type(&path),
                    body,
                    immutable: false,
                });
            }
        }
        Ok(())
    }
    let dir = dir.join("static");
    let mut out = Vec::new();
    if dir.is_dir() {
        walk(&dir, "/static", &mut out)?;
    }
    Ok(out)
}

struct Loader {
    root: PathBuf,
    assets: Vec<Asset>,
    /// The URL each file is served at, by its path.
    by_file: HashMap<PathBuf, String>,
    /// The URLs of the global stylesheets glazes import, in order.
    globals: Vec<String>,
    /// The custom properties they declare, in order: the design tokens.
    properties: Vec<(String, String)>,
}

impl Loader {
    /// A file named relative to `base`, which must be inside the app.
    fn file(&self, base: &Path, name: &str) -> Result<PathBuf, String> {
        let name = name.split(['?', '#']).next().unwrap_or(name);
        if Path::new(name).is_absolute() {
            return Err(format!("`{name}` must be a path relative to the glaze"));
        }
        let path = base.join(name);
        let full = path
            .canonicalize()
            .map_err(|_| format!("there is no file `{name}`"))?;
        if !full.starts_with(&self.root) {
            return Err(format!("`{name}` is outside the program's folder"));
        }
        if !full.is_file() {
            return Err(format!("`{name}` is not a file"));
        }
        Ok(full)
    }

    fn global_file(&mut self, name: &str) -> Result<(), String> {
        let path = self.file(&self.root.clone(), name)?;
        let url = self.css_asset(&path)?;
        if !self.globals.contains(&url) {
            self.globals.push(url);
        }
        Ok(())
    }

    fn module_file(&mut self, name: &str) -> Result<kiln_css::Css, String> {
        let path = self.file(&self.root.clone(), name)?;
        let text = read_text(&path)?;
        let base = path.parent().unwrap_or(&self.root).to_path_buf();
        let label = self.label(&path);
        let mut resolve = |url: &str, kind: UrlKind| self.resolve(&base, url, kind, false);
        kiln_css::module(&text, &label, &mut resolve).map_err(|e| css_error(&label, e))
    }

    /// A plain stylesheet as an asset, with the files it refers to.
    fn css_asset(&mut self, path: &Path) -> Result<String, String> {
        if let Some(url) = self.by_file.get(path) {
            return Ok(url.clone());
        }
        // Marks the file while it is processed, so an `@import` cycle ends.
        self.by_file.insert(path.to_path_buf(), String::new());
        let text = read_text(path)?;
        let base = path.parent().unwrap_or(&self.root).to_path_buf();
        let label = self.label(path);
        let mut resolve = |url: &str, kind: UrlKind| self.resolve(&base, url, kind, true);
        let css =
            kiln_css::global(&text, &label, &mut resolve).map_err(|e| css_error(&label, e))?;
        self.properties.extend(css.properties);
        let url = self.add(path, "text/css; charset=utf-8", css.code.into_bytes());
        Ok(url)
    }

    /// The URL a file that CSS refers to is served at.
    fn resolve(
        &mut self,
        base: &Path,
        url: &str,
        kind: UrlKind,
        imports: bool,
    ) -> Result<String, String> {
        let path = self.file(base, url)?;
        let suffix = &url[url.find(['?', '#']).unwrap_or(url.len())..];
        if kind == UrlKind::Import {
            if !imports {
                return Err(format!(
                    "`@import` works in stylesheets imported with `{{% css %}}`, not here; import `{url}` that way"
                ));
            }
            return match self.by_file.get(&path) {
                Some(u) if u.is_empty() => Err(format!("`{url}` imports itself")),
                _ => self.css_asset(&path),
            };
        }
        if let Some(u) = self.by_file.get(&path) {
            return Ok(format!("{u}{suffix}"));
        }
        let body = std::fs::read(&path).map_err(|e| format!("cannot read `{url}`: {e}"))?;
        let served = self.add(&path, content_type(&path), body);
        Ok(format!("{served}{suffix}"))
    }

    fn add(&mut self, path: &Path, content_type: &'static str, body: Vec<u8>) -> String {
        let stem = path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let ext = path
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()))
            .unwrap_or_default();
        let url = format!("/_kiln/{stem}-{}{ext}", kiln_css::short_hash(&body));
        self.by_file.insert(path.to_path_buf(), url.clone());
        if !self.assets.iter().any(|a| a.path == url) {
            self.assets.push(Asset {
                path: url.clone(),
                content_type,
                body,
                immutable: true,
            });
        }
        url
    }

    /// A file's name relative to the app, for messages and hashes.
    fn label(&self, path: &Path) -> String {
        path.strip_prefix(&self.root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/")
    }
}

fn read_text(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

fn css_error(file: &str, e: kiln_css::CssError) -> String {
    format!("{file}:{}:{}: {}", e.line + 1, e.column, e.message)
}

fn import_error(import: &CssImport, path: &str, message: String) -> Diagnostic {
    Diagnostic::error(
        codes::BAD_CSS,
        format!("cannot use `{path}`: {message}"),
        import.path.span,
    )
}

fn string_literal(import: &CssImport) -> Option<String> {
    match &import.path.kind {
        ExprKind::Str(pieces) => match pieces.as_slice() {
            [StrPiece::Text(t)] => Some(t.clone()),
            _ => None,
        },
        _ => None,
    }
}

/// A class name as a Kiln field: `card-title` is `card_title`. Names that
/// do not make a field are left out.
fn field_name(class: &str) -> Option<String> {
    let name = class.replace('-', "_");
    let ok = name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    ok.then_some(name)
}

/// The text of each `<style>` element in `nodes`, with its span.
fn style_elements(nodes: &[Node]) -> Vec<(&str, Span)> {
    fn walk<'a>(nodes: &'a [Node], out: &mut Vec<(&'a str, Span)>) {
        for n in nodes {
            match n {
                Node::Element(e) if e.name.name.eq_ignore_ascii_case("style") => {
                    if let Some(Node::Text { text, span }) = e.children.first() {
                        out.push((text, *span));
                    }
                }
                Node::Element(e) => walk(&e.children, out),
                Node::Fill { body, .. } => walk(body, out),
                Node::If {
                    branches,
                    otherwise,
                    ..
                } => {
                    for b in branches {
                        walk(&b.body, out);
                    }
                    if let Some(o) = otherwise {
                        walk(o, out);
                    }
                }
                Node::For { body, empty, .. } => {
                    walk(body, out);
                    if let Some(e) = empty {
                        walk(e, out);
                    }
                }
                Node::Match { cases, .. } => {
                    for c in cases {
                        walk(&c.body, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(nodes, &mut out);
    out
}

/// The byte offset of a 0-based line and 1-based column in `text`.
fn offset(text: &str, line: u32, column: u32) -> usize {
    let start: usize = text
        .split_inclusive('\n')
        .take(line as usize)
        .map(str::len)
        .sum();
    (start + column.saturating_sub(1) as usize).min(text.len())
}

/// A path segment for a URL: characters other than letters, digits and
/// `-._~` as `%XX`.
fn encode_segment(name: &str) -> String {
    let mut out = String::new();
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The `content-type` for a file, by its extension.
pub fn content_type(path: &Path) -> &'static str {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "html" | "htm" => "text/html; charset=utf-8",
        "txt" | "md" => "text/plain; charset=utf-8",
        "json" | "map" => "application/json",
        "xml" => "application/xml",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "pdf" => "application/pdf",
        "wasm" => "application/wasm",
        "webmanifest" => "application/manifest+json",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offsets() {
        assert_eq!(offset("ab\ncd", 1, 2), 4);
        assert_eq!(offset("ab", 0, 1), 0);
    }

    #[test]
    fn field_names() {
        assert_eq!(field_name("card-title").as_deref(), Some("card_title"));
        assert_eq!(field_name("2col"), None);
    }

    #[test]
    fn segments() {
        assert_eq!(encode_segment("a b.png"), "a%20b.png");
    }
}
