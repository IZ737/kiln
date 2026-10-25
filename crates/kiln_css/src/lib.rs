//! CSS for glazes, on [Lightning CSS](https://lightningcss.dev): every
//! stylesheet a program uses is parsed as CSS, transformed here, and
//! printed into the one stylesheet the program serves.
//!
//! - [`scoped`]: a glaze's own `<style>`. Each selector is limited to the
//!   glaze's elements by an attribute the compiler adds to them
//!   (`[data-k-1a2b3c]`), without Shadow DOM, so inheritance and custom
//!   properties still cross into components. `:global(...)` is left as it
//!   is.
//! - [`global`]: a plain `.css` file a glaze imports, applied to the whole
//!   page.
//! - [`module`]: a `.module.css` file, whose class names are renamed so that
//!   they cannot collide, with the new names exported for the glaze.
//!
//! Relative `url(...)`s and `@import`s are passed to a resolver, which
//! gives the URL the program serves the file at.

use lightningcss::css_modules::{Config, Pattern};
use lightningcss::rules::CssRule;
use lightningcss::selector::{Component, Selector};
use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};
use lightningcss::values::ident::Ident;
use lightningcss::values::url::Url;
use lightningcss::visit_types;
use lightningcss::visitor::{Visit, VisitTypes, Visitor};
use std::collections::BTreeMap;
use std::convert::Infallible;

/// A stylesheet ready to serve.
#[derive(Debug, Clone, PartialEq)]
pub struct Css {
    pub code: String,
    /// For a CSS Module, each class name and the name it was renamed to.
    pub exports: BTreeMap<String, String>,
}

/// CSS that could not be read: what was wrong, and where, as a 0-based
/// line and a 1-based column.
#[derive(Debug, Clone, PartialEq)]
pub struct CssError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

/// Where a relative URL appears in a stylesheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UrlKind {
    /// `@import "theme.css"`
    Import,
    /// `url(logo.svg)`
    Url,
}

/// Turns a relative URL in a stylesheet into the URL it is served at, or
/// says why it cannot be.
pub type Resolve<'a> = &'a mut dyn FnMut(&str, UrlKind) -> Result<String, String>;

/// The attribute that marks the elements of the glaze `file`, for its
/// scoped styles: `data-k-` and a hash of the file's name.
pub fn scope_attribute(file: &str) -> String {
    format!("data-k-{}", short_hash(file.as_bytes()))
}

/// A short, stable hash of `bytes`: six base-36 digits of FNV-1a.
pub fn short_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x100000001b3);
    }
    let mut out = String::new();
    for _ in 0..6 {
        out.push(char::from_digit((h % 36) as u32, 36).unwrap());
        h /= 36;
    }
    out
}

/// A glaze's `<style>`, with every selector limited to elements that have
/// the attribute `attr`.
pub fn scoped(css: &str, file: &str, attr: &str, resolve: Resolve) -> Result<Css, CssError> {
    // CSS Modules mode parses `:global(...)`; the `[local]` pattern keeps
    // every name as it is.
    let config = Config {
        pattern: Pattern::parse("[local]").expect("a valid pattern"),
        animation: false,
        grid: false,
        custom_idents: false,
        container: false,
        ..Config::default()
    };
    process(css, file, Some(config), Some(attr), resolve)
}

/// A plain stylesheet, applied to the whole page.
pub fn global(css: &str, file: &str, resolve: Resolve) -> Result<Css, CssError> {
    process(css, file, None, None, resolve)
}

/// A CSS Module: its class names, ids and keyframes are renamed with a
/// hash of `file`, and the class names are exported.
pub fn module(css: &str, file: &str, resolve: Resolve) -> Result<Css, CssError> {
    let config = Config {
        pattern: Pattern::parse("[local]_[hash]").expect("a valid pattern"),
        ..Config::default()
    };
    process(css, file, Some(config), None, resolve)
}

fn process(
    css: &str,
    file: &str,
    modules: Option<Config>,
    scope: Option<&str>,
    resolve: Resolve,
) -> Result<Css, CssError> {
    let options = ParserOptions {
        filename: file.to_string(),
        css_modules: modules,
        ..ParserOptions::default()
    };
    let mut sheet = StyleSheet::parse(css, options).map_err(|e| error(e.to_string(), e.loc))?;
    let mut v = Transform {
        scope: scope.map(|s| Ident::from(s.to_string())),
        resolve,
        failed: None,
    };
    let _ = sheet.visit(&mut v);
    if let Some(e) = v.failed {
        return Err(e);
    }
    let printed = sheet
        .to_css(PrinterOptions {
            minify: true,
            ..PrinterOptions::default()
        })
        .map_err(|e| error(e.to_string(), e.loc))?;
    let exports = printed
        .exports
        .map(|exports| {
            exports
                .into_iter()
                .map(|(name, export)| {
                    let mut renamed = export.name;
                    // `composes: other` adds the other class's name.
                    for c in export.composes {
                        if let lightningcss::css_modules::CssModuleReference::Local { name } = c {
                            renamed.push(' ');
                            renamed.push_str(&name);
                        }
                    }
                    (name, renamed)
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Css {
        code: printed.code,
        exports,
    })
}

fn error(message: String, loc: Option<lightningcss::error::ErrorLocation>) -> CssError {
    let (line, column) = loc.as_ref().map_or((0, 1), |l| (l.line, l.column));
    // Lightning CSS adds the place to its message.
    let message = match message.find(" at ") {
        Some(i) if loc.is_some() => message[..i].to_string(),
        _ => message,
    };
    CssError {
        message,
        line,
        column,
    }
}

struct Transform<'a> {
    scope: Option<Ident<'static>>,
    resolve: Resolve<'a>,
    failed: Option<CssError>,
}

impl<'i> Visitor<'i> for Transform<'_> {
    type Error = Infallible;

    fn visit_types(&self) -> VisitTypes {
        visit_types!(RULES | URLS)
    }

    fn visit_rule(&mut self, rule: &mut CssRule<'i>) -> Result<(), Self::Error> {
        match rule {
            CssRule::Style(style) => {
                if let Some(attr) = &self.scope {
                    for selector in style.selectors.0.iter_mut() {
                        scope_selector(selector, attr);
                    }
                }
            }
            CssRule::Import(import) => {
                let url = import.url.to_string();
                let loc = import.loc;
                if let Some(new) = self.rewrite(&url, UrlKind::Import, loc.line, loc.column) {
                    import.url = new.into();
                }
            }
            _ => {}
        }
        rule.visit_children(self)
    }

    fn visit_url(&mut self, url: &mut Url<'i>) -> Result<(), Self::Error> {
        let old = url.url.to_string();
        if let Some(new) = self.rewrite(&old, UrlKind::Url, url.loc.line, url.loc.column) {
            url.url = new.into();
        }
        Ok(())
    }
}

impl Transform<'_> {
    /// The URL a relative one is served at; `None` leaves it as it is.
    fn rewrite(&mut self, url: &str, kind: UrlKind, line: u32, column: u32) -> Option<String> {
        if !is_relative(url) {
            return None;
        }
        match (self.resolve)(url, kind) {
            Ok(new) => Some(new),
            Err(message) => {
                self.failed.get_or_insert(CssError {
                    message,
                    line,
                    column,
                });
                None
            }
        }
    }
}

/// Whether a URL in CSS names a file next to the stylesheet, as
/// `./logo.svg` or `fonts/a.woff2` do, rather than a page, a data URL or
/// `#fragment`.
pub fn is_relative(url: &str) -> bool {
    let url = url.trim();
    !(url.is_empty()
        || url.starts_with('/')
        || url.starts_with('#')
        || url.contains(':') && url.split(':').next().is_some_and(|s| !s.contains('/')))
}

/// Adds `[attr]` to the last compound selector of `selector`, before any
/// pseudo-element, unless that compound is `:global(...)`.
fn scope_selector<'i>(selector: &mut Selector<'i>, attr: &Ident<'static>) {
    // Raw storage has the compounds last first, each in written order.
    let mut groups: Vec<Vec<Component<'i>>> = vec![Vec::new()];
    for c in selector.iter_raw_match_order() {
        if c.is_combinator() {
            groups.push(vec![c.clone()]);
            groups.push(Vec::new());
        } else {
            groups.last_mut().unwrap().push(c.clone());
        }
    }
    let parts: Vec<Component<'i>> = groups.into_iter().rev().flatten().collect();
    let last_start = parts
        .iter()
        .rposition(|c| c.is_combinator())
        .map_or(0, |i| i + 1);
    let last = &parts[last_start..];
    let global = |c: &Component| {
        matches!(
            c,
            Component::NonTSPseudoClass(lightningcss::selector::PseudoClass::Global { .. })
        )
    };
    if !last.is_empty() && last.iter().all(global) {
        return;
    }
    let at = last
        .iter()
        .position(|c| matches!(c, Component::PseudoElement(_)) || c.is_combinator())
        .map_or(parts.len(), |i| last_start + i);
    let mut parts = parts;
    let name: Ident<'i> = Ident::from(attr.0.to_string());
    parts.insert(
        at,
        Component::AttributeInNoNamespaceExists {
            local_name: name.clone(),
            local_name_lower: name,
        },
    );
    *selector = Selector::from(parts);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_urls(url: &str, _: UrlKind) -> Result<String, String> {
        Err(format!("unexpected url {url}"))
    }

    fn scope(css: &str) -> String {
        scoped(css, "card.glaze", "data-k-x", &mut no_urls)
            .unwrap()
            .code
    }

    #[test]
    fn scoped_selectors() {
        assert_eq!(scope(".a { color: red }"), ".a[data-k-x]{color:red}");
        assert_eq!(
            scope("ul li a { color: red }"),
            "ul li a[data-k-x]{color:red}"
        );
        assert_eq!(
            scope("p::before { content: 'x' }"),
            "p[data-k-x]:before{content:\"x\"}"
        );
        assert_eq!(
            scope(".a:hover, h1 { color: red }"),
            ".a:hover[data-k-x],h1[data-k-x]{color:red}"
        );
        assert_eq!(
            scope(":global(.dark) .a { color: red }"),
            ".dark .a[data-k-x]{color:red}"
        );
        assert_eq!(scope(":global(body) { margin: 0 }"), "body{margin:0}");
        assert_eq!(
            scope("@media (min-width: 40em) { .a { color: red } }"),
            "@media (width>=40em){.a[data-k-x]{color:red}}"
        );
        // Keyframes and custom properties keep their names.
        assert_eq!(
            scope("@keyframes spin { to { rotate: 1turn } } .a { --gap: 1px; animation: spin 1s }"),
            "@keyframes spin{to{rotate:1turn}}.a[data-k-x]{--gap:1px;animation:1s spin}"
        );
    }

    #[test]
    fn modules_export_renamed_classes() {
        let css = module(
            ".card { color: red } .title { composes: card; }",
            "card.module.css",
            &mut no_urls,
        )
        .unwrap();
        let card = &css.exports["card"];
        assert!(card.starts_with("card_"), "{css:?}");
        assert_eq!(
            css.exports["title"],
            format!("{} {card}", css.exports["title"].split(' ').next().unwrap())
        );
        assert!(
            css.code.contains(&format!(".{card}{{color:red}}")),
            "{css:?}"
        );
        // The same file gives the same names.
        assert_eq!(
            css,
            module(
                ".card { color: red } .title { composes: card; }",
                "card.module.css",
                &mut no_urls
            )
            .unwrap()
        );
    }

    #[test]
    fn urls_are_resolved() {
        let mut seen = Vec::new();
        let mut resolve = |u: &str, _: UrlKind| -> Result<String, String> {
            seen.push(u.to_string());
            Ok(format!("/_kiln/{}", u.trim_start_matches("./")))
        };
        let css = global(
            "@import './theme.css'; .a { background: url(./img/logo.png) } .b { background: url(/abs.png) } .c { background: url(data:image/png;base64,AA) }",
            "site.css",
            &mut resolve,
        )
        .unwrap();
        assert_eq!(seen, ["./theme.css", "./img/logo.png"]);
        assert!(
            css.code.contains("url(/_kiln/img/logo.png)"),
            "{}",
            css.code
        );
        assert!(
            css.code.contains("@import \"/_kiln/theme.css\""),
            "{}",
            css.code
        );
    }

    #[test]
    fn errors_have_a_place() {
        let err = global(
            ".a { color: red }\n.b { color: red; ",
            "x.css",
            &mut no_urls,
        );
        assert!(err.is_ok(), "unclosed blocks end at the end of the file");
        let err = global("@import url(./x.css)", "x.css", &mut |_, _| {
            Err("no such file".into())
        })
        .unwrap_err();
        assert_eq!(err.message, "no such file");
    }

    #[test]
    fn relative_urls() {
        for u in ["./a.png", "a.png", "../fonts/x.woff2", "img/a:b.png"] {
            assert!(is_relative(u), "{u}");
        }
        for u in [
            "/a.png",
            "#x",
            "data:image/png;base64,AA",
            "https://x/a.png",
            "",
        ] {
            assert!(!is_relative(u), "{u}");
        }
    }
}
