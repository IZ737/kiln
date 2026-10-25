//! Typed styles: `Ui` values, written as records such as
//! `ui({ p: space.md, bg: color.surface, hover: { shadow: shadow.lg } })`.
//!
//! Every value in a style is written in place: a design token such as
//! `space.md`, a keyword such as `.bold`, a number, `true`, or a CSS string
//! such as `"1rem 2px"`, which is checked as CSS. Code may choose between
//! such values with `if` and `match`, but cannot compute them. So the
//! checker knows every declaration a program can use, and each becomes an
//! atomic CSS class (`.k1a2b3c4d5e6f{padding-top:var(--space-md)}`) in the
//! app's stylesheet. At run time a `Ui` is the string of its classes, and
//! [`crate::Program::styles`] tells the compiler how to build it.
//!
//! A class's name starts with `k`, then six characters that name what it
//! sets (the property, state and breakpoint), then six for the value, so
//! that `ui.merge` can drop a class whose property a later style sets.
//!
//! Tokens are the CSS custom properties that global stylesheets declare,
//! named `--<category>-<name>`: `--space-md` is `space.md`, and
//! `--color-on-accent` is `color.on_accent`.

use crate::checker::Checker;
use crate::ide::{Completion, Ref, StyleWord, StyleWordKind};
use crate::types::{similar, Ty};
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::{Expr, ExprKind, RecordField, StrPiece, UnaryOp};
use std::collections::BTreeMap;
use std::rc::Rc;

/// The token categories, in the order they are listed.
pub const CATEGORIES: &[&str] = &[
    "color",
    "space",
    "size",
    "radius",
    "shadow",
    "font",
    "text",
    "weight",
    "leading",
    "tracking",
    "motion",
    "breakpoint",
];

/// A design token: `--space-md: 1rem` is `space.md`.
#[derive(Clone, Debug, PartialEq)]
pub struct Token {
    pub category: String,
    /// As a Kiln field: `on_accent`.
    pub name: String,
    /// The custom property, with its `--`.
    pub property: String,
    /// Its value in the stylesheets, the last one declared.
    pub value: String,
}

/// The tokens an app's global stylesheets declare, and its themes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tokens {
    pub list: Vec<Token>,
    /// The themes: rules whose selector is `.theme-<name>`, each setting
    /// the same tokens to its own values. `ui({ theme: .name })` applies
    /// one.
    pub themes: Vec<Theme>,
}

/// A theme's token values, by the custom property, with its `--`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Theme {
    pub name: String,
    pub values: Vec<(String, String)>,
}

/// The CSS property whose values a token category holds, to check them.
fn category_property(category: &str) -> &'static str {
    match category {
        "color" => "color",
        "space" => "margin-top",
        "size" | "breakpoint" => "width",
        "radius" => "border-top-left-radius",
        "shadow" => "box-shadow",
        "font" => "font-family",
        "text" => "font-size",
        "weight" => "font-weight",
        "leading" => "line-height",
        "tracking" => "letter-spacing",
        _ => "transition",
    }
}

impl Tokens {
    /// The tokens among custom properties, given without their `--`, in
    /// the order they are declared. A later value replaces an earlier one.
    pub fn from_properties<'a>(properties: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        Self::from_rules(properties.into_iter().map(|(n, v)| (":root", n, v)))
    }

    /// The tokens and themes among custom properties, given with the
    /// selector of their rule and without their `--`. Rules for
    /// `.theme-<name>` are themes; others give tokens their values.
    pub fn from_rules<'a>(
        properties: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    ) -> Self {
        let mut tokens = Tokens::default();
        for (selector, property, value) in properties {
            let Some((category, rest)) = property.split_once('-') else {
                continue;
            };
            if !CATEGORIES.contains(&category) || rest.is_empty() {
                continue;
            }
            let name = rest.replace('-', "_");
            let ok = name.starts_with(|c: char| c.is_ascii_alphanumeric())
                && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
            if !ok {
                continue;
            }
            let theme = selector.strip_prefix(".theme-").filter(|t| {
                !t.is_empty() && t.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
            });
            if let Some(theme) = theme {
                let full = format!("--{property}");
                let entry = match tokens.themes.iter_mut().position(|t| t.name == theme) {
                    Some(i) => &mut tokens.themes[i],
                    None => {
                        tokens.themes.push(Theme {
                            name: theme.to_string(),
                            values: Vec::new(),
                        });
                        tokens.themes.last_mut().unwrap()
                    }
                };
                match entry.values.iter_mut().find(|v| v.0 == full) {
                    Some(v) => v.1 = value.to_string(),
                    None => entry.values.push((full, value.to_string())),
                }
                if tokens.get(category, &name).is_some() {
                    continue;
                }
            }
            match tokens
                .list
                .iter_mut()
                .find(|t| t.category == category && t.name == name)
            {
                Some(t) => t.value = value.to_string(),
                None => tokens.list.push(Token {
                    category: category.to_string(),
                    name,
                    property: format!("--{property}"),
                    value: value.to_string(),
                }),
            }
        }
        tokens
    }

    /// What is wrong with the tokens: values that do not suit their
    /// category, and themes that do not set the tokens other themes set.
    /// Each problem comes with the custom property it is about.
    pub fn problems(&self) -> Vec<(String, String)> {
        let mut out = Vec::new();
        let category = |property: &str| {
            let name = property.trim_start_matches("--");
            name.split_once('-').map_or("", |(c, _)| c).to_string()
        };
        let mut check = |property: &str, value: &str, theme: Option<&str>| {
            let cat = category(property);
            let ok = if cat == "breakpoint" {
                theme.is_none()
                    && !value.contains("var(")
                    && kiln_css::declaration("width", value).is_ok()
            } else {
                kiln_css::declaration(category_property(&cat), value).is_ok()
            };
            if !ok {
                let what = match (cat.as_str(), theme) {
                    ("breakpoint", Some(t)) => {
                        format!("theme `{t}` sets `{property}`, but breakpoints are the same in every theme")
                    }
                    ("breakpoint", None) => format!(
                        "`{property}: {value}` is not a width without `var()`, so it cannot be a breakpoint"
                    ),
                    _ => format!("`{property}: {value}` is not a valid `{cat}` value"),
                };
                out.push((property.to_string(), what));
            }
        };
        for t in &self.list {
            check(&t.property, &t.value, None);
        }
        for theme in &self.themes {
            for (p, v) in &theme.values {
                check(p, v, Some(&theme.name));
            }
        }
        for theme in &self.themes {
            for other in &self.themes {
                for (p, _) in &other.values {
                    if !theme.values.iter().any(|v| &v.0 == p) {
                        out.push((
                            p.clone(),
                            format!(
                                "theme `{}` does not set `{p}`, which theme `{}` sets",
                                theme.name, other.name
                            ),
                        ));
                    }
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        out.retain(|p| seen.insert(p.clone()));
        out
    }

    pub fn get(&self, category: &str, name: &str) -> Option<&Token> {
        self.list
            .iter()
            .find(|t| t.category == category && t.name == name)
    }

    pub fn names<'a>(&'a self, category: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.list
            .iter()
            .filter(move |t| t.category == category)
            .map(|t| t.name.as_str())
    }
}

/// Where a value in a style is, which says what it may be. A
/// `Ty::Style(pos)` is expected there while checking.
#[derive(Clone, Debug, PartialEq)]
pub enum StylePos {
    /// A record of style keys, in a state and at a breakpoint.
    Group {
        state: Option<&'static str>,
        at: Option<String>,
    },
    /// `at: { tablet: { ... } }`: a record of breakpoints.
    At,
    /// The value of a key.
    Key {
        key: &'static str,
        state: Option<&'static str>,
        at: Option<String>,
    },
}

/// How the compiler builds an expression in a style, by its span.
#[derive(Clone, Debug, PartialEq)]
pub enum StyleExpr {
    /// A value written in place: the classes it adds, separated by spaces.
    Classes(String),
    /// A record of style keys: its values, merged in order.
    Record,
    /// `ui(style)` or `style.classes()`: the style itself.
    Same,
    /// `ui.merge(a, b, ...)`.
    Merge,
}

/// One atomic CSS rule.
#[derive(Clone, Debug, PartialEq)]
pub struct UiRule {
    /// Such as `:hover`, or empty.
    pub state: &'static str,
    /// The breakpoint, for a rule in its media query.
    pub at: Option<String>,
    /// `padding-top:var(--space-md)`
    pub declaration: String,
}

const STATES: &[(&str, &str)] = &[
    ("hover", ":hover"),
    ("focus", ":focus"),
    ("focusVisible", ":focus-visible"),
    ("disabled", ":disabled"),
];

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Color,
    Space,
    Margin,
    Size,
    Radius,
    Border,
    Shadow,
    Display(&'static str),
    Direction,
    Tracks,
    Items,
    Justify,
    Wrap,
    Font,
    TextSize,
    Weight,
    LineHeight,
    Align,
    Tracking,
    Opacity,
    Overflow,
    Transition,
    Cursor,
    Transform,
    Underline,
    Theme,
}

/// The keys a style may set: each key's CSS properties and its kind.
const KEYS: &[(&str, &[&str], Kind)] = &[
    ("bg", &["background-color"], Kind::Color),
    ("fg", &["color"], Kind::Color),
    ("borderColor", &["border-color"], Kind::Color),
    (
        "p",
        &[
            "padding-top",
            "padding-right",
            "padding-bottom",
            "padding-left",
        ],
        Kind::Space,
    ),
    ("px", &["padding-left", "padding-right"], Kind::Space),
    ("py", &["padding-top", "padding-bottom"], Kind::Space),
    ("pt", &["padding-top"], Kind::Space),
    ("pr", &["padding-right"], Kind::Space),
    ("pb", &["padding-bottom"], Kind::Space),
    ("pl", &["padding-left"], Kind::Space),
    (
        "m",
        &["margin-top", "margin-right", "margin-bottom", "margin-left"],
        Kind::Margin,
    ),
    ("mx", &["margin-left", "margin-right"], Kind::Margin),
    ("my", &["margin-top", "margin-bottom"], Kind::Margin),
    ("mt", &["margin-top"], Kind::Margin),
    ("mr", &["margin-right"], Kind::Margin),
    ("mb", &["margin-bottom"], Kind::Margin),
    ("ml", &["margin-left"], Kind::Margin),
    ("gap", &["row-gap", "column-gap"], Kind::Space),
    ("w", &["width"], Kind::Size),
    ("h", &["height"], Kind::Size),
    ("minW", &["min-width"], Kind::Size),
    ("maxW", &["max-width"], Kind::Size),
    ("minH", &["min-height"], Kind::Size),
    ("maxH", &["max-height"], Kind::Size),
    ("rounded", &["border-radius"], Kind::Radius),
    ("border", &["border-style", "border-width"], Kind::Border),
    ("shadow", &["box-shadow"], Kind::Shadow),
    ("flex", &["display"], Kind::Display("flex")),
    ("grid", &["display"], Kind::Display("grid")),
    ("direction", &["flex-direction"], Kind::Direction),
    ("cols", &["grid-template-columns"], Kind::Tracks),
    ("rows", &["grid-template-rows"], Kind::Tracks),
    ("items", &["align-items"], Kind::Items),
    ("justify", &["justify-content"], Kind::Justify),
    ("wrap", &["flex-wrap"], Kind::Wrap),
    ("font", &["font-family"], Kind::Font),
    ("textSize", &["font-size"], Kind::TextSize),
    ("weight", &["font-weight"], Kind::Weight),
    ("lineHeight", &["line-height"], Kind::LineHeight),
    ("align", &["text-align"], Kind::Align),
    ("tracking", &["letter-spacing"], Kind::Tracking),
    ("opacity", &["opacity"], Kind::Opacity),
    ("overflow", &["overflow"], Kind::Overflow),
    ("transition", &["transition"], Kind::Transition),
    ("cursor", &["cursor"], Kind::Cursor),
    ("transform", &["transform"], Kind::Transform),
    ("underline", &["text-decoration-line"], Kind::Underline),
    ("theme", &[], Kind::Theme),
];

/// The names of the keys a style may set, for messages and completion.
pub fn key_names() -> impl Iterator<Item = &'static str> {
    KEYS.iter()
        .map(|k| k.0)
        .chain(STATES.iter().map(|s| s.0))
        .chain(["at"])
}

/// What a key sets, in words.
fn key_css(k: &(&str, &[&str], Kind)) -> String {
    match k.2 {
        Kind::Theme => "the class of a theme, which sets its tokens".into(),
        Kind::Display(d) => format!("display: {d}"),
        Kind::Border => "border-width (and border-style: solid)".into(),
        _ => k.1.join(", "),
    }
}

fn key(name: &str) -> Option<&'static (&'static str, &'static [&'static str], Kind)> {
    KEYS.iter().find(|k| k.0 == name)
}

impl Kind {
    /// What a value of this kind is, in a message.
    fn what(self) -> &'static str {
        match self {
            Kind::Color => "a colour, such as `color.accent`",
            Kind::Space => "a spacing, such as `space.md` or `0`",
            Kind::Margin => "a spacing, such as `space.md`, `0` or `.auto`",
            Kind::Size => "a size, such as `size.page`, `.full` or `\"20rem\"`",
            Kind::Radius => "a radius, such as `radius.card` or `.full`",
            Kind::Border => "`true` or a border width, such as `\"2px\"`",
            Kind::Shadow => "a shadow, such as `shadow.lg` or `.none`",
            Kind::Display(_) | Kind::Wrap | Kind::Underline => "`true` or `false`",
            Kind::Direction => "`.row`, `.column`, `.rowReverse` or `.columnReverse`",
            Kind::Tracks => "a number of columns or rows, from 1 to 12",
            Kind::Items => "`.start`, `.center`, `.end`, `.stretch` or `.baseline`",
            Kind::Justify => "`.start`, `.center`, `.end`, `.between`, `.around` or `.evenly`",
            Kind::Font => "a font, such as `font.body`",
            Kind::TextSize => "a text size, such as `text.lg` or `.lg`",
            Kind::Weight => "a weight, such as `.bold` or `600`",
            Kind::LineHeight => "a line height, such as `1.5` or `leading.tight`",
            Kind::Align => "`.left`, `.center`, `.right`, `.justify`, `.start` or `.end`",
            Kind::Tracking => "a letter spacing, such as `tracking.wide`",
            Kind::Opacity => "a number from 0 to 1",
            Kind::Overflow => "`.hidden`, `.auto`, `.scroll`, `.visible` or `.clip`",
            Kind::Transition => "a transition, such as `motion.fast` or `.none`",
            Kind::Cursor => "a cursor, such as `.pointer`",
            Kind::Transform => "a CSS transform, such as `\"rotate(2deg)\"`",
            Kind::Theme => "a theme, such as `.dark` for the rule `.theme-dark`",
        }
    }

    /// The token categories it takes.
    fn categories(self) -> &'static [&'static str] {
        match self {
            Kind::Color => &["color"],
            Kind::Space | Kind::Margin => &["space"],
            Kind::Size => &["size", "space"],
            Kind::Radius => &["radius"],
            Kind::Shadow => &["shadow"],
            Kind::Font => &["font"],
            Kind::TextSize => &["text"],
            Kind::Weight => &["weight"],
            Kind::LineHeight => &["leading"],
            Kind::Tracking => &["tracking"],
            Kind::Transition => &["motion"],
            _ => &[],
        }
    }

    /// The CSS value of a keyword, such as `.bold`.
    fn keyword(self, name: &str) -> Option<&'static str> {
        self.keywords().iter().find(|w| w.0 == name).map(|w| w.1)
    }

    /// The keywords it takes, with their CSS values.
    fn keywords(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Kind::Color => &[
                ("transparent", "transparent"),
                ("current", "currentColor"),
                ("inherit", "inherit"),
            ],
            Kind::Margin => &[("auto", "auto")],
            Kind::Size => &[
                ("auto", "auto"),
                ("full", "100%"),
                ("min", "min-content"),
                ("max", "max-content"),
                ("fit", "fit-content"),
            ],
            Kind::Radius => &[("none", "0"), ("full", "9999px")],
            Kind::Shadow | Kind::Transition | Kind::Transform => &[("none", "none")],
            Kind::Direction => &[
                ("row", "row"),
                ("column", "column"),
                ("rowReverse", "row-reverse"),
                ("columnReverse", "column-reverse"),
            ],
            Kind::Tracks => &[("none", "none")],
            Kind::Items => &[
                ("start", "flex-start"),
                ("center", "center"),
                ("end", "flex-end"),
                ("stretch", "stretch"),
                ("baseline", "baseline"),
            ],
            Kind::Justify => &[
                ("start", "flex-start"),
                ("center", "center"),
                ("end", "flex-end"),
                ("between", "space-between"),
                ("around", "space-around"),
                ("evenly", "space-evenly"),
                ("stretch", "stretch"),
            ],
            Kind::Weight => &[
                ("thin", "100"),
                ("light", "300"),
                ("normal", "400"),
                ("medium", "500"),
                ("semibold", "600"),
                ("bold", "700"),
                ("black", "900"),
            ],
            Kind::Align => &[
                ("left", "left"),
                ("center", "center"),
                ("right", "right"),
                ("justify", "justify"),
                ("start", "start"),
                ("end", "end"),
            ],
            Kind::Overflow => &[
                ("hidden", "hidden"),
                ("auto", "auto"),
                ("scroll", "scroll"),
                ("visible", "visible"),
                ("clip", "clip"),
            ],
            Kind::Cursor => &[
                ("pointer", "pointer"),
                ("default", "default"),
                ("text", "text"),
                ("notAllowed", "not-allowed"),
                ("grab", "grab"),
                ("move", "move"),
                ("wait", "wait"),
                ("help", "help"),
                ("none", "none"),
            ],
            _ => &[],
        }
    }
}

/// The name of a class: `k`, then a hash of what it sets, then a hash of
/// the value.
fn class_name(slot: &str, declaration: &str) -> String {
    let what = kiln_css::short_hash(slot.as_bytes());
    let value = kiln_css::short_hash(format!("{slot}{declaration}").as_bytes());
    format!("k{what}{value}")
}

/// The CSS of a program's styles, for the `kiln.ui` layer: rules without a
/// breakpoint first, then each breakpoint's, in the order the tokens
/// declare them.
pub fn stylesheet(rules: &BTreeMap<String, UiRule>, tokens: &Tokens) -> String {
    let mut css = String::new();
    let write = |css: &mut String, at: Option<&str>| {
        for (class, rule) in rules {
            if rule.at.as_deref() == at {
                css.push_str(&format!(".{class}{}{{{}}}", rule.state, rule.declaration));
            }
        }
    };
    write(&mut css, None);
    for t in tokens.list.iter().filter(|t| t.category == "breakpoint") {
        if rules
            .values()
            .any(|r| r.at.as_deref() == Some(t.name.as_str()))
        {
            css.push_str(&format!("@media (min-width:{}){{", t.value));
            write(&mut css, Some(&t.name));
            css.push('}');
        }
    }
    css
}

impl Checker {
    /// Checks a record of style keys, such as the argument of `ui(...)`.
    pub(crate) fn infer_style_record(
        &mut self,
        fields: &[RecordField],
        pos: &StylePos,
        span: Span,
    ) {
        self.styles.insert(span, StyleExpr::Record);
        let mut display: Option<&str> = None;
        for f in fields {
            let name = f.name.name.as_str();
            if self.is_marker(name) {
                let words = self.style_keys(pos);
                self.complete(Completion::Style(words));
                continue;
            }
            let Some(value) = &f.value else {
                self.error(
                    Diagnostic::error(
                        codes::BAD_STYLE,
                        format!("give `{name}` its value: `{name}: ...`"),
                        f.name.span,
                    )
                    .with_help(
                        "a style's values are written in place, so a variable cannot stand for one",
                    ),
                );
                continue;
            };
            let inner = match pos {
                StylePos::At => {
                    if self.tokens.get("breakpoint", name).is_none() {
                        let names: Vec<&str> = self.tokens.names("breakpoint").collect();
                        let mut d = Diagnostic::error(
                            codes::UNKNOWN_TOKEN,
                            format!("there is no breakpoint `{name}`"),
                            f.name.span,
                        );
                        d = match similar(name, names.iter().copied()) {
                            Some(s) => d.with_suggestion(
                                format!("use `{s}`"),
                                f.name.span,
                                s.clone(),
                            ),
                            None if names.is_empty() => d.with_help(
                                "declare breakpoints in a global stylesheet: `:root { --breakpoint-tablet: 768px }`",
                            ),
                            None => d.with_help(format!("the breakpoints are {}", list(&names))),
                        };
                        self.error(d);
                        continue;
                    }
                    self.refer_token(f.name.span, "breakpoint", name);
                    StylePos::Group {
                        state: None,
                        at: Some(name.to_string()),
                    }
                }
                StylePos::Group { state, at } => {
                    if let Some(s) = STATES.iter().find(|s| s.0 == name) {
                        if state.is_some() {
                            self.style_error(
                                f.name.span,
                                format!("`{name}` cannot go inside another state"),
                            );
                            continue;
                        }
                        self.refer(
                            f.name.span,
                            Ref::StyleKey {
                                name: name.to_string(),
                                css: format!("the `{}` state", s.1),
                            },
                        );
                        StylePos::Group {
                            state: Some(s.1),
                            at: at.clone(),
                        }
                    } else if name == "at" {
                        if at.is_some() || state.is_some() {
                            self.style_error(
                                f.name.span,
                                "`at` goes at the top of a style, outside states and other breakpoints".to_string(),
                            );
                            continue;
                        }
                        self.refer(
                            f.name.span,
                            Ref::StyleKey {
                                name: "at".into(),
                                css: "`@media (min-width: ...)`, for each breakpoint".into(),
                            },
                        );
                        StylePos::At
                    } else if let Some(k) = key(name) {
                        self.refer(
                            f.name.span,
                            Ref::StyleKey {
                                name: name.to_string(),
                                css: key_css(k),
                            },
                        );
                        if let Kind::Display(d) = k.2 {
                            if let Some(other) = display.replace(d) {
                                self.style_error(
                                    f.name.span,
                                    format!("a style cannot set both `{other}` and `{d}`: choose one layout"),
                                );
                            }
                        }
                        StylePos::Key {
                            key: k.0,
                            state: *state,
                            at: at.clone(),
                        }
                    } else {
                        let mut d = Diagnostic::error(
                            codes::BAD_STYLE,
                            format!("a style has no key `{name}`"),
                            f.name.span,
                        );
                        d = match similar(name, key_names()) {
                            Some(s) => {
                                d.with_suggestion(format!("use `{s}`"), f.name.span, s.clone())
                            }
                            None => d.with_help(
                                "rarer CSS properties go in a `<style>` or a stylesheet, with a class",
                            ),
                        };
                        self.error(d);
                        continue;
                    }
                }
                StylePos::Key { .. } => unreachable!("a key's value is not a record of keys"),
            };
            let style = Ty::Style(Rc::new(inner));
            self.check(value, &style);
        }
    }

    /// The type of `e` where a style value at `pos` is expected, or `None`
    /// to check it as usual (an `if`, `match` or block, whose branches are
    /// style values in turn).
    pub(crate) fn infer_style(&mut self, e: &Expr, pos: &Rc<StylePos>) -> Option<Ty> {
        let ty = Ty::Style(pos.clone());
        match (&e.kind, pos.as_ref()) {
            (ExprKind::If { .. } | ExprKind::Match { .. } | ExprKind::Block(_), _) => None,
            (ExprKind::Record(fields), StylePos::Group { .. } | StylePos::At) => {
                self.infer_style_record(fields, pos, e.span);
                Some(ty)
            }
            (_, StylePos::Group { .. } | StylePos::At) => {
                let what = if matches!(pos.as_ref(), StylePos::At) {
                    "a record of breakpoints, such as `{ tablet: { cols: 2 } }`"
                } else {
                    "a record of style keys, such as `{ shadow: shadow.lg }`"
                };
                self.style_error(e.span, format!("expected {what}"));
                Some(Ty::Unknown)
            }
            (
                _,
                StylePos::Key {
                    key: name,
                    state,
                    at,
                },
            ) => {
                let (_, props, kind) = *key(name).expect("a known key");
                if kind == Kind::Theme {
                    let ok = self.theme_value(e, state.is_some() || at.is_some());
                    return Some(if ok { ty } else { Ty::Unknown });
                }
                match self.style_value(e, kind, props) {
                    Ok(declarations) => {
                        let slot_end =
                            format!("|{}|{}", state.unwrap_or(""), at.as_deref().unwrap_or(""));
                        let mut classes = Vec::new();
                        for (prop, value) in declarations {
                            let declaration = format!("{prop}:{value}");
                            let class = class_name(&format!("{prop}{slot_end}"), &declaration);
                            self.ui_rules.insert(
                                class.clone(),
                                UiRule {
                                    state: state.unwrap_or(""),
                                    at: at.clone(),
                                    declaration,
                                },
                            );
                            classes.push(class);
                        }
                        self.styles
                            .insert(e.span, StyleExpr::Classes(classes.join(" ")));
                        Some(ty)
                    }
                    Err(d) => {
                        self.error(d);
                        Some(Ty::Unknown)
                    }
                }
            }
        }
    }

    /// `theme: .dark`, which adds the class `theme-dark`, whose rule in a
    /// global stylesheet sets the tokens to the theme's values.
    fn theme_value(&mut self, e: &Expr, nested: bool) -> bool {
        if nested {
            self.style_error(
                e.span,
                "`theme` goes at the top of a style, outside states and breakpoints".into(),
            );
            return false;
        }
        let ExprKind::FieldShorthand(name) = &e.kind else {
            self.style_error(e.span, format!("expected {}", Kind::Theme.what()));
            return false;
        };
        if self.is_marker(&name.name) {
            let words = self.style_values(Kind::Theme, false);
            self.complete(Completion::Style(words));
            return false;
        }
        let css_name = name.name.replace('_', "-");
        if !self.tokens.themes.iter().any(|t| t.name == css_name) {
            let names: Vec<String> = self
                .tokens
                .themes
                .iter()
                .map(|t| t.name.replace('-', "_"))
                .collect();
            let d = Diagnostic::error(
                codes::UNKNOWN_TOKEN,
                format!("there is no theme `{}`", name.name),
                name.span,
            );
            let d = match similar(&name.name, names.iter().map(String::as_str)) {
                Some(s) => d.with_suggestion(format!("use `{s}`"), name.span, s.clone()),
                None => d.with_help(format!(
                    "a theme is a rule in a global stylesheet that sets tokens: `.theme-{css_name} {{ --color-accent: ...; }}`"
                )),
            };
            self.error(d);
            return false;
        }
        self.styles
            .insert(e.span, StyleExpr::Classes(format!("theme-{css_name}")));
        self.refer(name.span, Ref::Theme(format!("theme-{css_name}")));
        true
    }

    fn refer_token(&mut self, span: Span, category: &str, name: &str) {
        if let Some(t) = self.tokens.get(category, name) {
            let target = Ref::Token {
                name: format!("{category}.{name}"),
                property: t.property.clone(),
                value: t.value.clone(),
            };
            self.refer(span, target);
        }
    }

    /// The keys that fit in a style at `pos`, for completion.
    fn style_keys(&self, pos: &StylePos) -> Vec<StyleWord> {
        let word = |label: &str, detail: String, kind| StyleWord {
            label: label.to_string(),
            detail,
            kind,
        };
        let (state, at) = match pos {
            StylePos::At => return self.token_words("breakpoint", false),
            StylePos::Group { state, at } => (state.is_some(), at.is_some()),
            StylePos::Key { .. } => return Vec::new(),
        };
        let mut out: Vec<StyleWord> = KEYS
            .iter()
            .filter(|k| k.2 != Kind::Theme || !(state || at))
            .map(|k| word(k.0, key_css(k), StyleWordKind::Key))
            .collect();
        if !state {
            for s in STATES {
                out.push(word(s.0, s.1.to_string(), StyleWordKind::Key));
            }
        }
        if !state && !at {
            out.push(word(
                "at",
                "@media, by breakpoint".into(),
                StyleWordKind::Key,
            ));
        }
        out
    }

    /// The tokens of a category: `md`, or `space.md` with `qualified`.
    fn token_words(&self, category: &str, qualified: bool) -> Vec<StyleWord> {
        self.tokens
            .list
            .iter()
            .filter(|t| t.category == category)
            .map(|t| StyleWord {
                label: if qualified {
                    format!("{category}.{}", t.name)
                } else {
                    t.name.clone()
                },
                detail: format!("{}: {}", t.property, t.value),
                kind: StyleWordKind::Token,
            })
            .collect()
    }

    /// The values that fit a key: its tokens and keywords. Keywords are
    /// written with their `.` unless the editor's text has it already.
    fn style_values(&self, kind: Kind, whole: bool) -> Vec<StyleWord> {
        let dot = if whole { "." } else { "" };
        let mut out = Vec::new();
        if kind == Kind::Theme {
            for t in &self.tokens.themes {
                out.push(StyleWord {
                    label: format!("{dot}{}", t.name.replace('-', "_")),
                    detail: format!(".theme-{}", t.name),
                    kind: StyleWordKind::Theme,
                });
            }
            return out;
        }
        if whole {
            for category in kind.categories() {
                out.extend(self.token_words(category, true));
            }
        } else if kind == Kind::TextSize {
            out.extend(self.token_words("text", false));
        }
        for (word, css) in kind.keywords() {
            out.push(StyleWord {
                label: format!("{dot}{word}"),
                detail: css.to_string(),
                kind: StyleWordKind::Keyword,
            });
        }
        if whole
            && matches!(
                kind,
                Kind::Display(_) | Kind::Wrap | Kind::Underline | Kind::Border
            )
        {
            for b in ["true", "false"] {
                out.push(StyleWord {
                    label: b.to_string(),
                    detail: String::new(),
                    kind: StyleWordKind::Keyword,
                });
            }
        }
        out
    }

    fn style_error(&mut self, span: Span, message: String) {
        self.error(Diagnostic::error(codes::BAD_STYLE, message, span));
    }

    /// The declarations a value written in place sets.
    #[allow(clippy::result_large_err)]
    fn style_value(
        &mut self,
        e: &Expr,
        kind: Kind,
        props: &[&str],
    ) -> Result<Vec<(String, String)>, Diagnostic> {
        let wrong = |what: String| {
            Diagnostic::error(
                codes::BAD_STYLE,
                format!("expected {}, found {what}", kind.what()),
                e.span,
            )
        };
        let each = |value: &str| -> Vec<(String, String)> {
            props
                .iter()
                .map(|p| (p.to_string(), value.to_string()))
                .collect()
        };
        if let ExprKind::Ident(name) = &e.kind {
            if self.is_marker(name) {
                let words = self.style_values(kind, true);
                self.complete(Completion::Style(words));
                return Ok(Vec::new());
            }
        }
        let value: String = match &e.kind {
            // `space.md`: a token.
            ExprKind::Field { expr, name }
                if matches!(&expr.kind, ExprKind::Ident(c) if CATEGORIES.contains(&c.as_str()) && self.local(c).is_none()) =>
            {
                let ExprKind::Ident(category) = &expr.kind else {
                    unreachable!()
                };
                if self.is_marker(&name.name) {
                    let words = self.token_words(category, false);
                    self.complete(Completion::Style(words));
                    return Ok(Vec::new());
                }
                return self.token(category, &name.name, name.span, e.span, kind, props);
            }
            ExprKind::FieldShorthand(name) => {
                if self.is_marker(&name.name) {
                    let words = self.style_values(kind, false);
                    self.complete(Completion::Style(words));
                    return Ok(Vec::new());
                }
                if kind == Kind::TextSize {
                    return self.token("text", &name.name, name.span, e.span, kind, props);
                }
                match kind.keyword(&name.name) {
                    Some(v) => v.to_string(),
                    None => return Err(wrong(format!("`.{}`", name.name))),
                }
            }
            ExprKind::Bool(b) => match kind {
                Kind::Display(d) => return Ok(if *b { each(d) } else { Vec::new() }),
                Kind::Wrap => (if *b { "wrap" } else { "nowrap" }).to_string(),
                Kind::Underline => (if *b { "underline" } else { "none" }).to_string(),
                Kind::Border if *b => {
                    return Ok(vec![
                        ("border-style".into(), "solid".into()),
                        ("border-width".into(), "1px".into()),
                    ])
                }
                _ => return Err(wrong(format!("`{b}`"))),
            },
            ExprKind::Int(_) | ExprKind::Float(_) | ExprKind::Unary { op: UnaryOp::Neg, .. } => {
                let Some(n) = number(e) else {
                    return Err(wrong("a computed number".into()));
                };
                match kind {
                    Kind::Space | Kind::Margin | Kind::Size | Kind::Radius | Kind::Border
                        if n == 0.0 =>
                    {
                        if kind == Kind::Border {
                            return Ok(vec![("border-width".into(), "0".into())]);
                        }
                        "0".to_string()
                    }
                    Kind::Tracks if n.fract() == 0.0 && (1.0..=12.0).contains(&n) => {
                        format!("repeat({n},minmax(0,1fr))")
                    }
                    Kind::Weight if n.fract() == 0.0 && (1.0..=1000.0).contains(&n) => {
                        format!("{n}")
                    }
                    Kind::LineHeight if n > 0.0 => format!("{n}"),
                    Kind::Opacity if (0.0..=1.0).contains(&n) => format!("{n}"),
                    _ => return Err(wrong(format!("`{}`", self.text_of(e)))),
                }
            }
            ExprKind::Str(pieces) => {
                let [StrPiece::Text(text)] = pieces.as_slice() else {
                    if pieces.is_empty() {
                        return Err(wrong("an empty string".into()));
                    }
                    return Err(Diagnostic::error(
                        codes::BAD_STYLE,
                        "a CSS value in a style is written out, without `${...}`",
                        e.span,
                    )
                    .with_help("choose between whole values with `if` or `match` instead"));
                };
                if matches!(kind, Kind::Display(_) | Kind::Wrap | Kind::Underline) {
                    return Err(wrong("a string".into()));
                }
                let mut out = Vec::new();
                let props: &[&str] = if kind == Kind::Border {
                    out.push(("border-style".to_string(), "solid".to_string()));
                    &["border-width"]
                } else {
                    props
                };
                for p in props {
                    let v = kiln_css::declaration(p, text).map_err(|m| {
                        Diagnostic::error(codes::BAD_STYLE, m, e.span)
                    })?;
                    out.push((p.to_string(), v));
                }
                return Ok(out);
            }
            _ => {
                return Err(Diagnostic::error(
                    codes::BAD_STYLE,
                    format!("a style value must be written in place: {}", kind.what()),
                    e.span,
                )
                .with_help(
                    "styles are compiled to CSS ahead of time, so code chooses between values with `if` or `match` but cannot compute them",
                ))
            }
        };
        Ok(each(&value))
    }

    #[allow(clippy::result_large_err)]
    fn token(
        &mut self,
        category: &str,
        name: &str,
        name_span: Span,
        span: Span,
        kind: Kind,
        props: &[&str],
    ) -> Result<Vec<(String, String)>, Diagnostic> {
        if !kind.categories().contains(&category) {
            return Err(Diagnostic::error(
                codes::BAD_STYLE,
                format!("expected {}, found a `{category}` token", kind.what()),
                span,
            ));
        }
        let Some(token) = self.tokens.get(category, name) else {
            let names: Vec<&str> = self.tokens.names(category).collect();
            let d = Diagnostic::error(
                codes::UNKNOWN_TOKEN,
                format!("there is no token `{category}.{name}`"),
                name_span,
            );
            return Err(match similar(name, names.iter().copied()) {
                Some(s) => d.with_suggestion(format!("use `{s}`"), name_span, s.clone()),
                None if names.is_empty() => d.with_help(format!(
                    "declare it in a global stylesheet: `:root {{ --{category}-{}: ...; }}`",
                    name.replace('_', "-")
                )),
                None => d.with_help(format!("the `{category}` tokens are {}", list(&names))),
            });
        };
        let value = format!("var({})", token.property);
        self.refer_token(name_span, category, name);
        Ok(props
            .iter()
            .map(|p| (p.to_string(), value.clone()))
            .collect())
    }

    fn text_of(&self, e: &Expr) -> String {
        number(e).map(|n| n.to_string()).unwrap_or_default()
    }
}

/// A number written in place, such as `2`, `1.5` or `-1`.
fn number(e: &Expr) -> Option<f64> {
    match &e.kind {
        ExprKind::Int(s) | ExprKind::Float(s) => s.replace('_', "").parse().ok(),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            expr,
        } => number(expr).map(|n| -n),
        _ => None,
    }
}

fn list(names: &[&str]) -> String {
    let shown: Vec<String> = names.iter().take(12).map(|n| format!("`{n}`")).collect();
    shown.join(", ")
}

impl Checker {
    /// `ui(style)`, `ui.merge(a, b, ...)` and `ui.classes(style)`, unless
    /// the program has its own `ui`.
    pub(crate) fn infer_ui_call(
        &mut self,
        callee: &Expr,
        args: &[kiln_syntax::ast::Arg],
        span: Span,
    ) -> Option<(Ty, Option<Ty>)> {
        let is_ui = |e: &Expr| matches!(&e.kind, ExprKind::Ident(n) if n == "ui");
        let method = match &callee.kind {
            ExprKind::Ident(_) if is_ui(callee) => "",
            ExprKind::Field { expr, name } if is_ui(expr) => name.name.as_str(),
            _ => return None,
        };
        if self.local("ui").is_some() || self.declared_values.contains("ui") {
            return None;
        }
        let ui = self.prim("Ui");
        let (ret, expr) = match method {
            "" | "classes" => {
                if args.len() != 1 || args[0].name.is_some() {
                    let call = if method.is_empty() {
                        "ui"
                    } else {
                        "ui.classes"
                    };
                    self.error(Diagnostic::error(
                        codes::BAD_ARGUMENTS,
                        format!("`{call}` takes one style"),
                        span,
                    ));
                }
                let ret = if method.is_empty() {
                    ui.clone()
                } else {
                    self.prim("String")
                };
                (ret, StyleExpr::Same)
            }
            "merge" => {
                if args.is_empty() {
                    self.error(Diagnostic::error(
                        codes::BAD_ARGUMENTS,
                        "`ui.merge` takes the styles to merge, later ones winning",
                        span,
                    ));
                }
                (ui.clone(), StyleExpr::Merge)
            }
            other => {
                let d = Diagnostic::error(
                    codes::NO_SUCH_MEMBER,
                    format!("`ui` has no function `{other}`"),
                    callee.span,
                )
                .with_help("`ui` has `ui(style)`, `ui.merge(a, b, ...)` and `ui.classes(style)`");
                self.error(d);
                for a in args {
                    self.infer(&a.value, &crate::checker::Expect::Value(None));
                }
                return Some((Ty::Unknown, None));
            }
        };
        for a in args {
            if let Some(n) = &a.name {
                self.error(Diagnostic::error(
                    codes::BAD_ARGUMENTS,
                    format!("`{}` takes styles without names", n.name),
                    n.span,
                ));
            }
            self.check(&a.value, &ui);
        }
        self.styles.insert(span, expr);
        Some((ret, None))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_come_from_custom_properties() {
        let t = Tokens::from_properties([
            ("color-on-accent", "white"),
            ("space-md", "1rem"),
            ("unrelated", "1"),
            ("space-md", "2rem"),
        ]);
        assert_eq!(t.list.len(), 2);
        assert_eq!(
            t.get("color", "on_accent").unwrap().property,
            "--color-on-accent"
        );
        assert_eq!(t.get("space", "md").unwrap().value, "2rem");
    }

    #[test]
    fn classes_name_what_they_set() {
        let a = class_name("padding-top||", "padding-top:0");
        let b = class_name("padding-top||", "padding-top:1px");
        assert_eq!(a.len(), 13);
        assert_eq!(a[..7], b[..7]);
        assert_ne!(a, b);
    }
}
