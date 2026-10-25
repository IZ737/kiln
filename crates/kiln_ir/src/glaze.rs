//! Compiles glazes into Kiln functions, which the rest of the compiler then
//! checks, lowers and builds like any other code.
//!
//! Each glaze `post_card.glaze` becomes two functions:
//!
//! - `__glaze_post_card(__out, <props>, __slot, ...)` pushes the glaze's
//!   HTML onto `__out`, a `List<String>`. Components and layouts call it.
//!   Each `{% slot %}` is a parameter holding a function that pushes the
//!   caller's content; it pushes nothing when the caller gives none.
//! - `__render_post_card(<props>) -> String` joins that HTML into one
//!   string, for `web.render("post_card.glaze", { ... })`.
//!
//! Code inside braces is copied from the glaze as it is written, in
//! parentheses so that its line breaks mean what they mean in a glaze, and
//! [`GlazeSource::locate`] maps errors in the generated text back to it.
//! Every value is escaped for HTML by the runtime's `html.escape`, and
//! values in attributes that browsers follow as URLs, such as `href`, by
//! `html.safe_url` first. Text written in the glaze is copied as written.

use crate::ValueKind;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::*;
use std::collections::HashMap;

/// A glaze to compile: its file name (`"post.glaze"`), its text, its parsed
/// form, and what its checker found each value to be.
pub struct GlazeInput<'a> {
    pub file: &'a str,
    pub src: &'a str,
    pub glaze: &'a Glaze,
    pub values: &'a HashMap<Span, ValueKind>,
}

/// The Kiln source made from an app's glazes.
pub struct GlazeSource {
    pub text: String,
    /// Where parts of `text` come from, for [`GlazeSource::locate`].
    map: Vec<Segment>,
    /// Parts of glazes the compiler cannot build yet, by glaze index.
    pub diagnostics: Vec<(usize, Diagnostic)>,
}

/// A part of the generated text and the glaze source it comes from.
struct Segment {
    start: usize,
    end: usize,
    glaze: usize,
    span: Span,
    /// Copied character for character, so offsets map one to one.
    copied: bool,
}

impl GlazeSource {
    /// The glaze (by index) and span that a span of the generated text comes
    /// from.
    pub fn locate(&self, span: Span) -> Option<(usize, Span)> {
        let (start, end) = (span.start as usize, span.end as usize);
        let seg = self
            .map
            .iter()
            .filter(|s| s.start <= start && end <= s.end.max(s.start + 1))
            .min_by_key(|s| s.end - s.start)?;
        if seg.copied {
            let shift = |o: usize| (seg.span.start as usize + o - seg.start) as u32;
            Some((
                seg.glaze,
                Span::new(shift(start) as usize, shift(end) as usize),
            ))
        } else {
            Some((seg.glaze, seg.span))
        }
    }
}

/// The function that renders a glaze to a `String`.
pub fn render_fn(file: &str) -> String {
    format!("__render_{}", stem(file))
}

fn glaze_fn(file: &str) -> String {
    format!("__glaze_{}", stem(file))
}

/// `post_card.glaze` is `post_card`. Other characters than letters,
/// digits and `_` become `_`.
fn stem(file: &str) -> String {
    file.trim_end_matches(".glaze")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// The glaze file a component tag renders: `<LikeButton>` is
/// `like_button.glaze`.
pub fn component_file(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out + ".glaze"
}

/// Attributes whose values browsers follow as URLs.
const URL_ATTRIBUTES: &[&str] = &[
    "href",
    "src",
    "action",
    "formaction",
    "poster",
    "cite",
    "background",
    "data",
    "codebase",
    "longdesc",
    "manifest",
    "icon",
    "xlink:href",
];

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Compiles the glazes of an app into Kiln source. Returns `None` when
/// there are none.
pub fn generate(glazes: &[GlazeInput]) -> Option<GlazeSource> {
    if glazes.is_empty() {
        return None;
    }
    let mut g = Gen {
        text: String::new(),
        map: Vec::new(),
        diagnostics: Vec::new(),
        glaze: 0,
        src: "",
        values: glazes[0].values,
        pending: String::new(),
        indent: 0,
        temps: 0,
        ember_reported: false,
        trims: Vec::new(),
    };
    g.text.push_str(
        "// Generated from the app's glazes.\n\
         import html\n\
         \n\
         fn __glaze_no_slot(__out: List<String>) {}\n\
         fn __glaze_text(text: String) -> String { html.escape(text) }\n\
         fn __glaze_url(url: String) -> String { html.escape(html.safe_url(url)) }\n",
    );
    let mut names: HashMap<String, &str> = HashMap::new();
    for (i, input) in glazes.iter().enumerate() {
        if let Some(other) = names.insert(stem(input.file), input.file) {
            g.diagnostics.push((
                i,
                Diagnostic::error(
                    codes::NOT_COMPILED_YET,
                    format!(
                        "`{}` and `{other}` cannot both be compiled: rename one so that their names differ in letters or digits",
                        input.file
                    ),
                    Span::new(0, 0),
                ),
            ));
            continue;
        }
        g.glaze = i;
        g.src = input.src;
        g.values = input.values;
        g.temps = 0;
        g.ember_reported = false;
        // A slot's content takes the slot's place in its line.
        let slots = slot_spans(&input.glaze.nodes);
        let tags: Vec<Span> = input
            .glaze
            .tags
            .iter()
            .filter(|t| !slots.contains(t))
            .copied()
            .collect();
        g.trims = standalone_tags(input.src, &tags);
        g.glaze_fns(input);
    }
    Some(GlazeSource {
        text: g.text,
        map: g.map,
        diagnostics: g.diagnostics,
    })
}

struct Gen<'a> {
    text: String,
    map: Vec<Segment>,
    diagnostics: Vec<(usize, Diagnostic)>,
    /// The glaze being compiled, its text and its values.
    glaze: usize,
    src: &'a str,
    values: &'a HashMap<Span, ValueKind>,
    /// Static HTML not yet pushed.
    pending: String,
    indent: usize,
    /// Numbers the locals made for values.
    temps: usize,
    ember_reported: bool,
    /// Ranges of the glaze's text left out of the HTML, in order: the
    /// whitespace and line break around tags alone on their lines.
    trims: Vec<(usize, usize)>,
}

impl<'a> Gen<'a> {
    fn glaze_fns(&mut self, input: &GlazeInput<'a>) {
        let glaze = input.glaze;
        if let Some(state) = glaze.states.first() {
            self.ember(state.span);
        }
        let props = glaze.props.as_ref().map_or(&[][..], |p| &p.params[..]);
        let slots = slot_names(&glaze.nodes);

        // fn __glaze_x(__out: List<String>, props..., __slot...) {
        let start = self.text.len();
        self.text.push_str(&format!(
            "\nfn {}(__out: List<String>",
            glaze_fn(input.file)
        ));
        for p in props {
            self.text.push_str(", ");
            self.copy(p.span, false);
        }
        for slot in &slots {
            self.text.push_str(&format!(
                ", {}: fn(List<String>) = __glaze_no_slot",
                slot_param(slot.as_deref())
            ));
        }
        self.text.push_str(") {\n");
        self.indent = 1;
        match &glaze.layout {
            Some(layout) => self.layout(layout, &glaze.nodes),
            None => self.nodes(&glaze.nodes),
        }
        self.flush();
        self.text.push_str("}\n");
        let whole = Span::new(0, self.src.len());
        self.segment(start, whole);

        // fn __render_x(props...) -> String { ... }
        let start = self.text.len();
        self.text
            .push_str(&format!("\nfn {}(", render_fn(input.file)));
        for (i, p) in props.iter().enumerate() {
            if i > 0 {
                self.text.push_str(", ");
            }
            self.copy(p.span, false);
        }
        self.text.push_str(") -> String {\n");
        self.text.push_str("    let __out: List<String> = []\n");
        self.text
            .push_str(&format!("    {}(__out", glaze_fn(input.file)));
        for p in props {
            self.text
                .push_str(&format!(", {}: {}", p.name.name, p.name.name));
        }
        self.text.push_str(")\n    __out.join(\"\")\n}\n");
        self.segment(start, whole);
    }

    /// Reports a part of an ember, once per glaze.
    fn ember(&mut self, span: Span) {
        if self.ember_reported {
            return;
        }
        self.ember_reported = true;
        self.diagnostics.push((
            self.glaze,
            Diagnostic::error(
                codes::NOT_COMPILED_YET,
                "the compiler cannot build embers yet: `{% state %}` and `on:` handlers run in the browser, which comes in phase 6",
                span,
            )
            .with_help("render the glaze without its state and handlers for now"),
        ));
    }

    // ----- writing -----

    fn line(&mut self, code: &str) {
        self.flush();
        self.start_line();
        self.text.push_str(code);
        self.text.push('\n');
    }

    fn start_line(&mut self) {
        for _ in 0..self.indent {
            self.text.push_str("    ");
        }
    }

    /// Pushes the static HTML collected so far.
    fn flush(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let html = std::mem::take(&mut self.pending);
        self.start_line();
        self.text
            .push_str(&format!("__out.push({})\n", kiln_string(&html)));
    }

    /// Copies the glaze's text at `span`, in parentheses when `paren`.
    fn copy(&mut self, span: Span, paren: bool) {
        if paren {
            self.text.push('(');
        }
        let start = self.text.len();
        self.text
            .push_str(&self.src[span.start as usize..span.end as usize]);
        self.map.push(Segment {
            start,
            end: self.text.len(),
            glaze: self.glaze,
            span,
            copied: true,
        });
        if paren {
            self.text.push(')');
        }
    }

    /// Marks the text written since `start` as coming from `span`.
    fn segment(&mut self, start: usize, span: Span) {
        self.map.push(Segment {
            start,
            end: self.text.len(),
            glaze: self.glaze,
            span,
            copied: false,
        });
    }

    fn temp(&mut self, prefix: &str) -> String {
        self.temps += 1;
        format!("__{prefix}{}", self.temps)
    }

    /// Stores a value in a new local, returning its name, for use inside
    /// `"${...}"`, which cannot span lines as copied code can.
    fn temp_value(&mut self, e: &Expr) -> String {
        let v = self.temp("v");
        self.flush();
        self.start_line();
        let start = self.text.len();
        self.text.push_str(&format!("let {v} = "));
        self.copy(e.span, true);
        self.text.push('\n');
        self.segment(start, e.span);
        v
    }

    /// Writes a line that pushes `wrap(value)` (the value itself when
    /// `wrap` is empty), with the value as text.
    fn push_value(&mut self, e: &Expr, wrap: &str) {
        self.flush();
        if self.values.get(&e.span) == Some(&ValueKind::String) || wrap.is_empty() {
            self.start_line();
            let start = self.text.len();
            self.text.push_str("__out.push(");
            if !wrap.is_empty() {
                self.text.push_str(wrap);
                self.text.push('(');
            }
            self.copy(e.span, true);
            if !wrap.is_empty() {
                self.text.push(')');
            }
            self.text.push_str(")\n");
            self.segment(start, e.span);
            return;
        }
        let v = self.temp_value(e);
        self.line(&format!("__out.push({wrap}(\"${{{v}}}\"))"));
    }

    /// A Kiln string expression for a text attribute or prop: its text, with
    /// its values' text forms in place.
    fn text_value(&mut self, parts: &[AttrPart]) -> String {
        let mut value = String::from("\"");
        for part in parts {
            match part {
                AttrPart::Text(t) => value.push_str(&kiln_string_body(t)),
                AttrPart::Expr(e) => {
                    let v = self.temp_value(e);
                    value.push_str(&format!("${{{v}}}"));
                }
            }
        }
        value + "\""
    }

    // ----- nodes -----

    fn nodes(&mut self, nodes: &[Node]) {
        for node in nodes {
            self.node(node);
        }
    }

    /// Writes code for nodes inside a block of their own.
    fn block(&mut self, nodes: &[Node]) {
        self.indent += 1;
        self.nodes(nodes);
        self.flush();
        self.indent -= 1;
    }

    fn node(&mut self, node: &Node) {
        match node {
            Node::Text { text, span } => {
                let (start, end) = (span.start as usize, span.end as usize);
                if self.src.get(start..end) != Some(text.as_str()) {
                    self.pending.push_str(text);
                    return;
                }
                // Leave out the lines that hold only tags.
                let mut at = start;
                for &(a, b) in &self.trims {
                    if b <= at || a >= end {
                        continue;
                    }
                    self.pending.push_str(&self.src[at..a.max(at)]);
                    at = b.min(end);
                }
                self.pending.push_str(&self.src[at..end]);
            }
            Node::Expr { expr, raw, .. } => {
                let wrap = if *raw { "" } else { "__glaze_text" };
                self.push_value(expr, wrap);
            }
            Node::Element(e) if e.is_component() => self.component(e),
            Node::Element(e) => self.element(e),
            Node::Let(decl) => {
                self.flush();
                self.start_line();
                let start = self.text.len();
                self.text.push_str(&format!("let {}", decl.name.name));
                if let Some(ty) = &decl.ty {
                    self.text.push_str(": ");
                    self.copy(ty.span, false);
                }
                self.text.push_str(" = ");
                self.copy(decl.value.span, true);
                self.text.push('\n');
                self.segment(start, decl.span);
            }
            Node::If {
                branches,
                otherwise,
                span,
            } => {
                self.flush();
                let start = self.text.len();
                for (i, b) in branches.iter().enumerate() {
                    if i == 0 {
                        self.start_line();
                        self.text.push_str("if ");
                    } else {
                        self.text.push_str(" else if ");
                    }
                    self.copy(b.cond.span, true);
                    self.text.push_str(" {\n");
                    self.block(&b.body);
                    self.start_line();
                    self.text.push('}');
                }
                if let Some(nodes) = otherwise {
                    self.text.push_str(" else {\n");
                    self.block(nodes);
                    self.start_line();
                    self.text.push('}');
                }
                self.text.push('\n');
                self.segment(start, *span);
            }
            Node::For {
                binding,
                iter,
                filter,
                body,
                empty,
                span,
            } => {
                self.flush();
                let start = self.text.len();
                let any = empty.as_ref().map(|_| self.temp("any"));
                if let Some(any) = &any {
                    self.line(&format!("var {any} = false"));
                }
                self.start_line();
                self.text.push_str(&format!("for {} in ", binding.name));
                self.copy(iter.span, true);
                if let Some(f) = filter {
                    self.text.push_str(" where ");
                    self.copy(f.span, true);
                }
                self.text.push_str(" {\n");
                if let Some(any) = &any {
                    self.indent += 1;
                    self.line(&format!("{any} = true"));
                    self.indent -= 1;
                }
                self.block(body);
                self.line("}");
                if let (Some(any), Some(nodes)) = (&any, empty) {
                    self.line(&format!("if not {any} {{"));
                    self.block(nodes);
                    self.line("}");
                }
                self.segment(start, *span);
            }
            Node::Match { expr, cases, span } => {
                self.flush();
                let start = self.text.len();
                self.start_line();
                self.text.push_str("match ");
                self.copy(expr.span, true);
                self.text.push_str(" {\n");
                self.indent += 1;
                for case in cases {
                    self.start_line();
                    self.copy(case.pattern.span, false);
                    if let Some(guard) = &case.guard {
                        self.text.push_str(" if ");
                        self.copy(guard.span, true);
                    }
                    self.text.push_str(" => {\n");
                    self.block(&case.body);
                    self.line("}");
                }
                self.indent -= 1;
                self.line("}");
                self.segment(start, *span);
            }
            Node::Slot { name, span } => {
                let start = self.text.len();
                let name = name.as_ref().map(|n| n.name.as_str());
                self.line(&format!("{}(__out)", slot_param(name)));
                self.segment(start, *span);
            }
        }
    }

    fn element(&mut self, e: &Element) {
        let name = &e.name.name;
        self.pending.push('<');
        self.pending.push_str(name);
        for attr in &e.attrs {
            self.attr(attr);
        }
        self.pending.push('>');
        if VOID_ELEMENTS.contains(&name.to_ascii_lowercase().as_str()) {
            return;
        }
        self.nodes(&e.children);
        self.pending.push_str("</");
        self.pending.push_str(name);
        self.pending.push('>');
    }

    fn attr(&mut self, attr: &Attr) {
        let name = &attr.name.name;
        let is_url = URL_ATTRIBUTES.contains(&name.to_ascii_lowercase().as_str());
        match &attr.value {
            AttrValue::Flag => {
                self.pending.push(' ');
                self.pending.push_str(name);
            }
            AttrValue::Handler(_) => self.ember(attr.span),
            AttrValue::Expr(e) => {
                if self.values.get(&e.span) == Some(&ValueKind::Bool) {
                    // `disabled={busy}` adds the attribute when `busy`.
                    self.flush();
                    self.start_line();
                    let start = self.text.len();
                    self.text.push_str("if ");
                    self.copy(e.span, true);
                    self.text
                        .push_str(&format!(" {{ __out.push(\" {name}\") }}\n"));
                    self.segment(start, attr.span);
                    return;
                }
                self.pending.push_str(&format!(" {name}=\""));
                let wrap = if is_url {
                    "__glaze_url"
                } else {
                    "__glaze_text"
                };
                self.push_value(e, wrap);
                self.pending.push('"');
            }
            AttrValue::Text(parts) => {
                self.pending.push_str(&format!(" {name}=\""));
                if is_url && !settles_scheme(parts) {
                    // The values decide what kind of URL it is, so the whole
                    // URL is checked.
                    let value = self.text_value(parts);
                    self.line(&format!("__out.push(__glaze_url({value}))"));
                } else {
                    for part in parts {
                        match part {
                            AttrPart::Text(t) => self.pending.push_str(&t.replace('"', "&quot;")),
                            AttrPart::Expr(e) => self.push_value(e, "__glaze_text"),
                        }
                    }
                }
                self.pending.push('"');
            }
        }
    }

    /// `<Card title={t}>children</Card>` calls `__glaze_card`.
    fn component(&mut self, e: &Element) {
        let file = component_file(&e.name.name);
        self.flush();
        let start = self.text.len();
        // The values in text props are worked out first, in order.
        let mut texts = HashMap::new();
        for (i, attr) in e.attrs.iter().enumerate() {
            match &attr.value {
                AttrValue::Text(parts) => {
                    texts.insert(i, self.text_value(parts));
                }
                AttrValue::Handler(_) => self.ember(attr.span),
                _ if attr.name.name.starts_with("client:") => self.ember(attr.span),
                _ => {}
            }
        }
        self.start_line();
        let call = self.text.len();
        self.text.push_str(&format!("{}(__out", glaze_fn(&file)));
        for (i, attr) in e.attrs.iter().enumerate() {
            let name = &attr.name.name;
            if name.starts_with("client:") {
                continue;
            }
            match &attr.value {
                AttrValue::Flag => self.text.push_str(&format!(", {name}: true")),
                AttrValue::Expr(x) => {
                    self.text.push_str(&format!(", {name}: "));
                    self.copy(x.span, true);
                }
                AttrValue::Text(_) => {
                    self.text.push_str(&format!(", {name}: {}", texts[&i]));
                }
                AttrValue::Handler(_) => {}
            }
        }
        if e.children.iter().any(|n| !is_blank(n)) {
            self.text.push_str(", __slot: fn(__out) {\n");
            self.block(&e.children);
            self.start_line();
            self.text.push('}');
        }
        self.text.push_str(")\n");
        self.segment(call, e.span);
        self.segment(start, e.span);
    }

    /// `{% layout "base.glaze" title: t %}` wraps the glaze's nodes in a call
    /// to the layout, with the nodes as its slot.
    fn layout(&mut self, layout: &Layout, nodes: &[Node]) {
        let file = match &layout.path.kind {
            ExprKind::Str(pieces) => match pieces.as_slice() {
                [StrPiece::Text(t)] => t.clone(),
                _ => String::new(),
            },
            _ => String::new(),
        };
        self.start_line();
        let start = self.text.len();
        self.text.push_str(&format!("{}(__out", glaze_fn(&file)));
        for arg in &layout.args {
            if let Some(name) = &arg.name {
                self.text.push_str(&format!(", {}: ", name.name));
                self.copy(arg.value.span, true);
            }
        }
        self.text.push_str(", __slot: fn(__out) {\n");
        self.block(nodes);
        self.start_line();
        self.text.push_str("})\n");
        self.segment(start, layout.span);
    }
}

/// The ranges to leave out so that lines holding only tags and comments
/// render as nothing: the whitespace before the tags, and the whitespace and
/// line break after them.
fn standalone_tags(src: &str, tags: &[Span]) -> Vec<(usize, usize)> {
    let blank = |t: &str| t.chars().all(|c| c == ' ' || c == '\t');
    let mut tags: Vec<(usize, usize)> = tags
        .iter()
        .map(|s| (s.start as usize, s.end as usize))
        .collect();
    tags.sort();
    let mut out = Vec::new();
    let mut i = 0;
    while i < tags.len() {
        // Tags on one line, with only spaces between them.
        let mut j = i;
        while j + 1 < tags.len() && blank(&src[tags[j].1..tags[j + 1].0]) {
            j += 1;
        }
        let (first, last) = (tags[i].0, tags[j].1);
        i = j + 1;
        let line_start = src[..first].rfind('\n').map_or(0, |n| n + 1);
        if !blank(&src[line_start..first]) {
            continue;
        }
        let rest = &src[last..];
        let spaces = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        let after = &rest[spaces..];
        let line_end = if after.starts_with("\r\n") {
            last + spaces + 2
        } else if after.starts_with('\n') {
            last + spaces + 1
        } else if after.is_empty() {
            src.len()
        } else {
            continue;
        };
        out.push((line_start, first));
        out.push((last, line_end));
    }
    // The line break that ends the file, so that a component's HTML can
    // sit inside a line.
    let end = src.trim_end_matches('\n').trim_end_matches('\r').len();
    if src.len() - end <= 2 && out.last().is_none_or(|&(_, b)| b <= end) {
        out.push((end, src.len()));
    }
    out
}

/// The names of a glaze's slots, `None` for the default one, in order.
fn slot_names(nodes: &[Node]) -> Vec<Option<String>> {
    let mut out = Vec::new();
    for_each_slot(nodes, &mut |name, _| {
        let name = name.map(|n| n.name.clone());
        if !out.contains(&name) {
            out.push(name);
        }
    });
    out
}

fn slot_spans(nodes: &[Node]) -> Vec<Span> {
    let mut out = Vec::new();
    for_each_slot(nodes, &mut |_, span| out.push(span));
    out
}

fn for_each_slot(nodes: &[Node], f: &mut impl FnMut(Option<&Ident>, Span)) {
    fn walk(nodes: &[Node], f: &mut impl FnMut(Option<&Ident>, Span)) {
        for n in nodes {
            match n {
                Node::Slot { name, span } => f(name.as_ref(), *span),
                Node::Element(e) => walk(&e.children, f),
                Node::If {
                    branches,
                    otherwise,
                    ..
                } => {
                    for b in branches {
                        walk(&b.body, f);
                    }
                    if let Some(o) = otherwise {
                        walk(o, f);
                    }
                }
                Node::For { body, empty, .. } => {
                    walk(body, f);
                    if let Some(e) = empty {
                        walk(e, f);
                    }
                }
                Node::Match { cases, .. } => {
                    for c in cases {
                        walk(&c.body, f);
                    }
                }
                _ => {}
            }
        }
    }
    walk(nodes, f);
}

fn slot_param(name: Option<&str>) -> String {
    match name {
        None => "__slot".into(),
        Some(n) => format!("__slot_{n}"),
    }
}

fn is_blank(node: &Node) -> bool {
    matches!(node, Node::Text { text, .. } if text.trim().is_empty())
}

/// Whether the text before the first value already decides what kind of
/// URL an attribute holds, as `/posts/{id}` or `https://x/{p}` do, so the
/// values cannot make it a `javascript:` URL.
fn settles_scheme(parts: &[AttrPart]) -> bool {
    let Some(AttrPart::Text(prefix)) = parts.first() else {
        return false;
    };
    let mut seen = false;
    for c in prefix.trim_start().chars() {
        match c {
            '\t' | '\n' | '\r' => {}
            ':' => return true,
            'a'..='z' | 'A'..='Z' => seen = true,
            '0'..='9' | '+' | '-' | '.' if seen => {}
            _ => return true,
        }
    }
    // All text, or a value could still finish the scheme.
    parts.len() == 1
}

/// A Kiln string literal with this text.
fn kiln_string(text: &str) -> String {
    format!("\"{}\"", kiln_string_body(text))
}

fn kiln_string_body(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes() {
        let text = |t: &str| AttrPart::Text(t.into());
        let value = AttrPart::Expr(Expr {
            kind: ExprKind::Ident("x".into()),
            span: Span::new(0, 1),
        });
        assert!(settles_scheme(&[text("/posts/"), value.clone()]));
        assert!(settles_scheme(&[text("https://x/"), value.clone()]));
        assert!(settles_scheme(&[text("?q="), value.clone()]));
        assert!(!settles_scheme(&[value.clone(), text("/x")]));
        assert!(!settles_scheme(&[text("java"), value.clone()]));
        assert!(settles_scheme(&[text("javascript:x")]));
    }

    #[test]
    fn strings() {
        assert_eq!(kiln_string("a\"b$c\\\n"), r#""a\"b\$c\\\n""#);
    }

    #[test]
    fn names() {
        assert_eq!(component_file("LikeButton"), "like_button.glaze");
        assert_eq!(render_fn("post-card.glaze"), "__render_post_card");
    }
}
