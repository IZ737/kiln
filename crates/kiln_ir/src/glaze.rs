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
use std::collections::{BTreeMap, HashMap};

/// A glaze to compile: its file name (`"post.glaze"`), its text, its parsed
/// form, and what its checker found each value to be.
pub struct GlazeInput<'a> {
    pub file: &'a str,
    pub src: &'a str,
    pub glaze: &'a Glaze,
    pub values: &'a HashMap<Span, ValueKind>,
    /// The attribute that marks the glaze's elements, when it has a
    /// scoped `<style>`.
    pub scope: Option<&'a str>,
    /// Its CSS Modules, by the name it imports them as: each class as a
    /// Kiln field name, with the class name it is renamed to.
    pub modules: Option<&'a HashMap<String, BTreeMap<String, String>>>,
    /// Its `{% state %}`s, each with its type as Kiln writes it.
    pub states: &'a [(String, String)],
}

/// What the app's glazes share: the stylesheet built from their CSS, and
/// the files the program serves.
#[derive(Default)]
pub struct App<'a> {
    /// Components reachable from the program's render calls, including layouts
    /// and nested components. None keeps all components for standalone callers.
    pub reachable_components: Option<&'a std::collections::HashSet<String>>,
    /// The path the stylesheet is served at, which every `<head>` links.
    pub stylesheet: Option<&'a str>,
    pub assets: &'a [Asset],
    /// The path of the script that hydrates embers, which pages with an
    /// ember load.
    pub ember_script: Option<&'a str>,
    /// The `server fn`s embers call, which get a route each.
    pub server_fns: &'a [ServerFn],
    /// Changes whenever the embers' code does, so a page loaded before a
    /// deploy cannot call the new server with the old arguments.
    pub rpc_version: &'a str,
}

/// A `server fn`, as its declaration writes it, or another function a
/// handler's code calls through the bridge and waits for: one of
/// `browser`'s, or an npm package's that returns a promise, which have a
/// `module`.
#[derive(Clone, Debug, PartialEq)]
pub struct ServerFn {
    pub module: Option<String>,
    pub name: String,
    /// Each parameter's name, type, and default value if it has one.
    pub params: Vec<(String, String, Option<String>)>,
    /// The type it returns, if any.
    pub ret: Option<String>,
    pub throws: bool,
}

/// A file the program serves, compiled into it.
#[derive(Clone, Debug, PartialEq)]
pub struct Asset {
    /// Where it is served, such as `/_kiln/logo-1a2b3c.svg`.
    pub path: String,
    pub content_type: &'static str,
    pub body: Vec<u8>,
    /// Whether the path changes when the content does, so browsers may
    /// keep it for good.
    pub immutable: bool,
}

/// The function that adds the routes serving the app's files to a
/// `web.App`. `web.app()` calls it when the program has one.
pub const ASSETS_FN: &str = "__kiln_assets";

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
    if file.contains('/') {
        // A path is an identity, not a basename: posts/index and posts_index
        // must never share a renderer or island ID.
        return format!(
            "__module_{}",
            file.trim_end_matches(".glaze")
                .trim_end_matches(".ember")
                .bytes()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        );
    }
    file.trim_end_matches(".glaze")
        .trim_end_matches(".ember")
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

/// The glaze file a component tag renders: `<LikeButton>` is
/// `like_button.ember`.
pub fn component_file(name: &str) -> String {
    if let Some((module, component)) = name.rsplit_once('.') {
        return format!("{}/{}", module.replace('.', "/"), component_file(component));
    }
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

/// Compiles the glazes of an app, and the files it serves, into Kiln
/// source. Returns `None` when there are neither.
pub fn generate(glazes: &[GlazeInput], app: &App) -> Option<GlazeSource> {
    if glazes.is_empty() && app.assets.is_empty() {
        return None;
    }
    Some(generate_for(glazes, app, false))
}

/// Compiles the glazes of an app into the Kiln source of its embers, which
/// runs in the browser: each ember gets `__ember_<name>`, which makes an
/// `ember.Island`, and the module exports the functions the bridge calls
/// (see [`CLIENT_EXPORTS`]).
pub fn generate_client(glazes: &[GlazeInput], app: &App) -> GlazeSource {
    generate_for(glazes, app, true)
}

/// The functions the browser module exports, which `ember.js` calls.
pub const CLIENT_EXPORTS: &[&str] = &[
    "__kiln_mount",
    "__kiln_render",
    "__kiln_fire",
    "__kiln_props",
    "__kiln_drop",
    "__kiln_reply",
];

fn generate_for(glazes: &[GlazeInput], app: &App, client: bool) -> GlazeSource {
    static NO_VALUES: std::sync::OnceLock<HashMap<Span, ValueKind>> = std::sync::OnceLock::new();
    // Scoped `<style>`s are in the app's stylesheet, so they are taken out
    // of the glazes, and their lines out of the HTML.
    let mut style_tags = vec![Vec::new(); glazes.len()];
    let stripped: Vec<Option<Glaze>> = glazes
        .iter()
        .zip(&mut style_tags)
        .map(|(input, tags)| {
            input.scope?;
            let mut glaze = input.glaze.clone();
            strip_styles(&mut glaze.nodes, tags);
            Some(glaze)
        })
        .collect();
    let mut g = Gen {
        text: String::new(),
        map: Vec::new(),
        diagnostics: Vec::new(),
        glaze: 0,
        src: "",
        values: NO_VALUES.get_or_init(HashMap::new),
        scope: None,
        stylesheet: app.stylesheet,
        pending: String::new(),
        indent: 0,
        temps: 0,
        trims: Vec::new(),
        client,
        embers: glazes
            .iter()
            .filter(|g| {
                g.glaze.is_ember()
                    && app
                        .reachable_components
                        .is_none_or(|names| names.contains(g.file))
            })
            .map(|g| g.file.to_string())
            .collect(),
        is_ember: false,
        state_types: HashMap::new(),
        server_fns: app.server_fns.to_vec(),
    };
    if client {
        g.server_fns.extend(browser_waits());
    }
    if client {
        g.text.push_str(
            "// Generated from the app's glazes, for the browser.\n\
             import html\n\
             import json\n\
             import ember\n\
             import browser\n\
             import js\n\
             \n\
             fn __glaze_no_slot(__out: List<String>, __h: List<fn(ember.Event)>) {}\n",
        );
        // The npm packages, which handlers use without imports.
        for f in kiln_check::foreign_modules() {
            g.text.push_str(&format!("import {}\n", f.name));
        }
    } else {
        g.text.push_str(
            "// Generated from the app's glazes.\n\
             import html\n\
             import web\n\
             import base64\n\
             import json\n\
             \n\
             fn __glaze_no_slot(__out: List<String>) {}\n",
        );
    }
    g.text.push_str(
        "fn __glaze_text(text: String) -> String { html.escape(text) }\n\
         fn __glaze_url(url: String) -> String { html.escape(html.safe_url(url)) }\n\
         fn __glaze_ui(style: Ui) -> String { ui.classes(style) }\n",
    );
    if let (Some(script), false) = (app.ember_script, client) {
        g.text.push_str(&format!(
            "\nfn __kiln_hydrate_page(html: String) -> String {{\n\
             \x20   if not html.contains(\"<kiln-island\") {{\n\
             \x20       return html\n\
             \x20   }}\n\
             \x20   let tag = {}\n\
             \x20   if html.contains(\"</head>\") {{\n\
             \x20       return html.replace(\"</head>\", \"${{tag}}</head>\")\n\
             \x20   }}\n\
             \x20   return \"${{html}}${{tag}}\"\n\
             }}\n",
            kiln_string(&format!(
                "<script type=\"module\" src=\"{}\"></script>",
                html_attr(script)
            ))
        ));
    }
    if client {
        g.server_proxies();
        g.dispatcher(glazes);
    } else if !app.assets.is_empty() || !app.server_fns.is_empty() {
        g.assets(app.assets);
        g.rpc_routes(app);
        g.text.push_str("}\n");
        if !app.server_fns.is_empty() {
            g.rpc_check(app.rpc_version);
        }
    }
    let mut names: HashMap<String, &str> = HashMap::new();
    for (i, input) in glazes.iter().enumerate() {
        if client
            && app
                .reachable_components
                .is_some_and(|names| !names.contains(input.file))
        {
            continue;
        }
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
        g.state_types = input.states.iter().cloned().collect();
        g.values = input.values;
        g.scope = input.scope;
        g.temps = 0;
        // A slot's content takes the slot's place in its line.
        let slots = slot_spans(&input.glaze.nodes);
        let mut tags: Vec<Span> = input
            .glaze
            .tags
            .iter()
            .filter(|t| !slots.contains(t))
            .copied()
            .collect();
        tags.append(&mut style_tags[i]);
        g.trims = standalone_tags(input.src, &tags);
        let glaze = stripped[i].as_ref().unwrap_or(input.glaze);
        g.is_ember = input.glaze.is_ember();
        let input = GlazeInput { glaze, ..*input };
        g.glaze_fns(&input);
        if client && g.is_ember {
            g.island_fn(&input);
        }
    }
    GlazeSource {
        text: g.text,
        map: g.map,
        diagnostics: g.diagnostics,
    }
}

struct Gen<'a> {
    text: String,
    map: Vec<Segment>,
    diagnostics: Vec<(usize, Diagnostic)>,
    /// The glaze being compiled, its text and its values.
    glaze: usize,
    src: &'a str,
    values: &'a HashMap<Span, ValueKind>,
    /// The glaze's scope attribute, and the app's stylesheet.
    scope: Option<&'a str>,
    stylesheet: Option<&'a str>,
    /// Static HTML not yet pushed.
    pending: String,
    indent: usize,
    /// Numbers the locals made for values.
    temps: usize,
    /// Ranges of the glaze's text left out of the HTML, in order: the
    /// whitespace and line break around tags alone on their lines.
    trims: Vec<(usize, usize)>,
    /// Whether this is the code for the browser.
    client: bool,
    /// The glazes that are embers, by file name.
    embers: std::collections::HashSet<String>,
    /// Whether the glaze being compiled is an ember.
    is_ember: bool,
    /// The type of each state of the glaze being compiled, as written.
    state_types: HashMap<String, String>,
    /// The functions handlers wait for: the `server fn`s, and in the
    /// browser the others too.
    server_fns: Vec<ServerFn>,
}

impl<'a> Gen<'a> {
    fn glaze_fns(&mut self, input: &GlazeInput<'a>) {
        let glaze = input.glaze;
        if let (true, Some(layout)) = (self.is_ember, &glaze.layout) {
            self.diagnostics.push((
                self.glaze,
                Diagnostic::error(
                    codes::NOT_COMPILED_YET,
                    "an ember cannot use a layout: it is a part of a page, which the page's layout wraps",
                    layout.span,
                )
                .with_help("use the ember as a component in a page that has the layout"),
            ));
        }
        let props = glaze.props.as_ref().map_or(&[][..], |p| &p.params[..]);
        let slots = slot_names(&glaze.nodes);

        // fn __glaze_x(__out: List<String>, props..., __slot...) {
        let start = self.text.len();
        self.text.push_str(&format!(
            "\nfn {}(__out: List<String>",
            glaze_fn(input.file)
        ));
        if self.client {
            self.text.push_str(", __h: List<fn(ember.Event)>");
        }
        for p in props {
            self.text.push_str(", ");
            self.copy_prop(p);
        }
        for slot in &slots {
            self.text.push_str(&format!(
                ", {}: {} = __glaze_no_slot",
                slot_param(slot.as_deref()),
                self.slot_type()
            ));
        }
        let island = self.is_ember && !self.client;
        if island {
            self.text.push_str(", __hydrate: String = \"load\"");
        }
        self.text.push_str(") {\n");
        self.indent = 1;
        self.module_locals(input);
        self.states(glaze, !self.client);
        if island {
            self.island_open(input.file, props, glaze);
            self.line("if __hydrate != \"only\" {");
            self.indent += 1;
        }
        match &glaze.layout {
            Some(layout) if !self.is_ember => self.layout(layout, &glaze.nodes),
            _ => self.nodes(&glaze.nodes),
        }
        self.flush();
        if island {
            self.indent -= 1;
            self.line("}");
            self.line("__out.push(\"</kiln-island>\")");
        }
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
            self.copy_prop(p);
        }
        self.text.push_str(") -> String {\n");
        self.text.push_str("    let __out: List<String> = []\n");
        if self.client {
            self.text
                .push_str("    let __h: List<fn(ember.Event)> = []\n");
            self.text
                .push_str(&format!("    {}(__out, __h", glaze_fn(input.file)));
        } else {
            self.text
                .push_str(&format!("    {}(__out", glaze_fn(input.file)));
        }
        for p in props {
            self.text
                .push_str(&format!(", {}: {}", p.name.name, p.name.name));
        }
        if self.client || self.embers.is_empty() {
            self.text.push_str(")\n    __out.join(\"\")\n}\n");
        } else {
            self.text
                .push_str(")\n    __kiln_hydrate_page(__out.join(\"\"))\n}\n");
        }
        self.segment(start, whole);
    }

    /// A prop as a parameter. A glaze's `request: web.Request` is the
    /// request being handled unless the caller passes one.
    fn copy_prop(&mut self, p: &Param) {
        self.copy(p.span, false);
        let ty =
            p.ty.as_ref()
                .map(|t| &self.src[t.span.start as usize..t.span.end as usize]);
        if !self.is_ember
            && p.name.name == "request"
            && p.default.is_none()
            && ty.map(str::trim) == Some("web.Request")
        {
            self.text.push_str(" = web.__request()");
        }
    }

    /// The type of a slot's parameter.
    fn slot_type(&self) -> &'static str {
        if self.client {
            "fn(List<String>, List<fn(ember.Event)>)"
        } else {
            "fn(List<String>)"
        }
    }

    /// The locals that hold the classes of the glaze's CSS Modules. A
    /// module with no classes has nothing to use, so it needs no local.
    fn module_locals(&mut self, input: &GlazeInput) {
        let mut modules: Vec<_> = input.modules.into_iter().flatten().collect();
        modules.sort();
        for (name, classes) in modules {
            if classes.is_empty() {
                continue;
            }
            let fields: Vec<String> = classes
                .iter()
                .map(|(field, class)| format!("{field}: {}", kiln_string(class)))
                .collect();
            self.line(&format!("let {name} = {{ {} }}", fields.join(", ")));
        }
    }

    /// `{% state count = 0 %}` as a local: one that does not change when
    /// `fixed`, as on the server, which only renders it.
    fn states(&mut self, glaze: &Glaze, fixed: bool) {
        for state in &glaze.states {
            self.flush();
            self.start_line();
            let start = self.text.len();
            let kw = if fixed { "let" } else { "var" };
            self.text.push_str(&format!("{kw} {}", state.name.name));
            if let Some(ty) = &state.ty {
                self.text.push_str(": ");
                self.copy(ty.span, false);
            }
            self.text.push_str(" = ");
            let value = &self.src[state.value.span.start as usize..state.value.span.end as usize];
            let shown = self.state_types.get(&state.name.name).cloned();
            match shown {
                // The server computes a state that calls a `server fn`,
                // and gives the browser its value; the browser cannot.
                Some(ty) if self.client && calls_any(value, &self.server_fns) => {
                    self.text.truncate(self.text.len() - 3);
                    self.text.push_str(&format!(
                        ": {ty} = panic(\"the state `{}` calls a server fn, so only the server can start this ember\")",
                        state.name.name
                    ));
                }
                _ => self.copy(state.value.span, true),
            }
            self.text.push('\n');
            self.segment(start, state.span);
        }
    }

    /// `<kiln-island ...>`, which marks where an ember's HTML is, with
    /// what the browser needs to take it over: its name, when to, and its
    /// props and state as JSON.
    fn island_open(&mut self, file: &str, props: &[Param], glaze: &Glaze) {
        self.pending.push_str(&format!(
            "<kiln-island data-k-ember=\"{}\" data-k-hydrate=\"",
            stem(file)
        ));
        self.flush();
        self.line("__out.push(__glaze_text(__hydrate))");
        self.pending.push_str("\" data-k-props=\"");
        let names: Vec<&str> = props.iter().map(|p| p.name.name.as_str()).collect();
        self.json_attr(&names);
        self.pending.push_str("\" data-k-state=\"");
        let names: Vec<&str> = glaze.states.iter().map(|s| s.name.name.as_str()).collect();
        self.json_attr(&names);
        self.pending.push_str("\">");
    }

    /// Pushes the JSON object of the locals `names`, escaped for an
    /// attribute.
    fn json_attr(&mut self, names: &[&str]) {
        if names.is_empty() {
            self.pending.push_str("{}");
            return;
        }
        let fields: Vec<String> = names.iter().map(|n| format!("{n}: {n}")).collect();
        self.line(&format!(
            "__out.push(__glaze_text(json.encode({{ {} }})))",
            fields.join(", ")
        ));
    }

    /// `fn __ember_x(__props: String, __state: String) -> ember.Island`,
    /// which takes over an ember in the browser: it restores the props and
    /// state the server rendered it with (or, for an island the browser
    /// adds, runs the state's initial values), and renders it again with
    /// each event.
    fn island_fn(&mut self, input: &GlazeInput<'a>) {
        let glaze = input.glaze;
        let props = glaze.props.as_ref().map_or(&[][..], |p| &p.params[..]);
        let name = stem(input.file);
        let start = self.text.len();
        self.text.push_str(&format!(
            "\nfn __ember_{name}(__props: String, __state: String) -> ember.Island {{\n"
        ));
        self.indent = 1;
        if !props.is_empty() {
            self.props_from(
                "__props",
                "__p",
                props,
                &format!(
                    "catch __e {{ panic(\"the props of `{}` do not decode: ${{__e.message}}\") }}",
                    input.file
                ),
                true,
            );
        }
        if !glaze.states.is_empty() {
            // The state is the server's, as JSON, or else (an ember the
            // browser adds) from the initial values.
            let fields: Vec<String> = input
                .states
                .iter()
                .map(|(n, t)| format!("{n}: {t}"))
                .collect();
            let record = format!("{{ {} }}", fields.join(", "));
            self.line(&format!("let __start = fn() -> {record} {{"));
            self.indent += 1;
            self.states(glaze, false);
            let values: Vec<String> = input
                .states
                .iter()
                .map(|(n, _)| format!("{n}: {n}"))
                .collect();
            self.line(&format!("return {{ {} }}", values.join(", ")));
            self.indent -= 1;
            self.line("}");
            self.line(&format!(
                "let __s = if __state != \"\" {{ json.decode<{record}>(__state) catch __e {{ panic(\"the state of `{}` does not decode: ${{__e.message}}\") }} }} else {{ __start() }}",
                input.file
            ));
            for (n, t) in input.states {
                self.line(&format!("var {n}: {t} = __s.{n}"));
            }
        }
        self.line("var __h: List<fn(ember.Event)> = []");
        self.line("return ember.Island(");
        self.line("    render: fn() -> String {");
        self.indent = 3;
        self.line("__h = []");
        self.line("let __out: List<String> = []");
        self.module_locals(input);
        self.nodes(&glaze.nodes);
        self.flush();
        self.line("return __out.join(\"\")");
        self.indent = 1;
        self.line("    },");
        self.line("    fire: fn(__i: Int, __e: ember.Event) {");
        self.line("        if __i >= 0 and __i < __h.len() {");
        self.line("            __h[__i](__e)");
        self.line("        }");
        self.line("    },");
        self.line("    props: fn(__text: String) {");
        if !props.is_empty() {
            self.indent = 3;
            self.props_from("__text", "__q", props, "catch __e { return }", false);
            self.indent = 1;
        }
        self.line("    },");
        self.line(")");
        self.indent = 0;
        self.text.push_str("}\n");
        self.segment(start, Span::new(0, self.src.len()));
    }

    /// Decodes the props from the JSON in `text` into `local`, and sets
    /// them: as new variables when `declare`. Props with defaults may be
    /// left out, as a parent ember leaves them out.
    fn props_from(&mut self, text: &str, local: &str, props: &[Param], fail: &str, declare: bool) {
        self.flush();
        self.start_line();
        let start = self.text.len();
        self.text
            .push_str(&format!("let {local} = json.decode<{{ "));
        for (i, p) in props.iter().enumerate() {
            if i > 0 {
                self.text.push_str(", ");
            }
            self.text.push_str(&format!("{}: ", p.name.name));
            if let Some(ty) = &p.ty {
                self.copy(ty.span, false);
                let written = &self.src[ty.span.start as usize..ty.span.end as usize];
                if p.default.is_some() && !written.trim_end().ends_with('?') {
                    self.text.push('?');
                }
            }
        }
        self.text.push_str(&format!(" }}>({text}) {fail}\n"));
        for p in props {
            self.start_line();
            let n = &p.name.name;
            if declare {
                self.text.push_str("var ");
            }
            self.text.push_str(&format!("{n} = {local}.{n}"));
            if let Some(d) = &p.default {
                let optional = p.ty.as_ref().is_some_and(|t| {
                    self.src[t.span.start as usize..t.span.end as usize]
                        .trim_end()
                        .ends_with('?')
                });
                if !optional {
                    self.text.push_str(" ?? ");
                    self.copy(d.span, true);
                }
            }
            self.text.push('\n');
        }
        self.segment(start, props[0].span);
    }

    /// The registry of the islands on the page, and the functions the
    /// bridge calls.
    fn dispatcher(&mut self, glazes: &[GlazeInput]) {
        self.text.push_str(
            "\nconst __KILN_ISLANDS: Map<Int, ember.Island> = Map()\n\
             const __KILN_NEXT: List<Int> = [0]\n\
             \nfn __kiln_mount(name: String, props: String, state: String) -> Int {\n\
             \x20   let id = __KILN_NEXT[0]\n",
        );
        let mut first = true;
        for g in glazes.iter().filter(|g| self.embers.contains(g.file)) {
            let name = stem(g.file);
            self.text.push_str(&format!(
                "    {}if name == {} {{\n        __KILN_ISLANDS[id] = __ember_{name}(props, state)\n    }}",
                if first { "" } else { " else " },
                kiln_string(&name),
            ));
            first = false;
        }
        if first {
            self.text.push_str("    return -1\n");
        } else {
            self.text.push_str(" else {\n        return -1\n    }\n");
        }
        self.text.push_str(
            "    __KILN_NEXT[0] = id + 1\n\
             \x20   return id\n\
             }\n\
             \nfn __kiln_render(id: Int) -> String {\n\
             \x20   if __KILN_ISLANDS[id] is some(island) {\n\
             \x20       return island.render()\n\
             \x20   }\n\
             \x20   return \"\"\n\
             }\n\
             \nfn __kiln_fire(id: Int, index: Int, kind: String, value: String, checked: Bool, key: String, form: String) {\n\
             \x20   let fields = json.decode<Map<String, String>>(form) catch err { Map() }\n\
             \x20   let event = ember.Event(kind: kind, value: value, checked: checked, key: key, fields: fields)\n\
             \x20   if __KILN_ISLANDS[id] is some(island) {\n\
             \x20       island.fire(index, event)\n\
             \x20   }\n\
             }\n\
             \nfn __kiln_props(id: Int, text: String) {\n\
             \x20   if __KILN_ISLANDS[id] is some(island) {\n\
             \x20       island.props(text)\n\
             \x20   }\n\
             }\n\
             \nfn __kiln_drop(id: Int) {\n\
             \x20   __KILN_ISLANDS.remove(id)\n\
             }\n\
             \nfn __kiln_reply(done: fn(ember.Reply), ok: Bool, text: String) {\n\
             \x20   done(ember.Reply(ok: ok, text: text))\n\
             }\n",
        );
    }

    /// For each `server fn`, `__rpc_<name>`, which reads its result from
    /// the server's reply, failing when the call failed.
    fn server_proxies(&mut self) {
        for f in &self.server_fns {
            let ret = f.ret.as_deref().unwrap_or("()");
            let returns = f.ret.is_some();
            let mut text = format!(
                "\nfn {}(reply: ember.Reply){} ! {{\n\
                 \x20   if not reply.ok {{\n\
                 \x20       throw Error(message: reply.text)\n\
                 \x20   }}\n",
                f.proxy(),
                if returns {
                    format!(" -> {ret}")
                } else {
                    String::new()
                },
            );
            if returns {
                text.push_str(&format!(
                    "    let value = try json.decode<{{ ok: {ret} }}>(reply.text)\n    return value.ok\n"
                ));
            }
            text.push_str("}\n");
            self.text.push_str(&text);
        }
    }

    /// The route of each `server fn`: `POST /_kiln/rpc/<name>`, whose body
    /// is a JSON object of its arguments and whose reply is `{"ok":
    /// result}`, or `{"error": message}` with the status of the failure.
    /// Every call passes `__kiln_rpc_check` first.
    fn rpc_routes(&mut self, app: &App) {
        if app.server_fns.is_empty() {
            return;
        }
        for f in app.server_fns {
            let mut fields = Vec::new();
            let mut args = Vec::new();
            if f.takes_request() {
                args.push("req".to_string());
            }
            for (name, ty, default) in f.sent() {
                match default {
                    Some(d) if !ty.trim_end().ends_with('?') => {
                        fields.push(format!("{name}: {ty}?"));
                        args.push(format!("{name}: __args.{name} ?? ({d})"));
                    }
                    _ => {
                        fields.push(format!("{name}: {ty}"));
                        args.push(format!("{name}: __args.{name}"));
                    }
                }
            }
            let read = if fields.is_empty() {
                String::new()
            } else {
                format!(
                    "        let __args = req.json<{{ {} }}>() catch err {{\n            throw web.error(400, \"the arguments do not decode: ${{err.message}}\")\n        }}\n",
                    fields.join(", ")
                )
            };
            let call = format!(
                "{}{}({})",
                if f.throws { "try " } else { "" },
                f.name,
                args.join(", ")
            );
            let reply = if f.ret.is_some() {
                format!(
                    "        let __result = {call}\n        return web.json({{ ok: __result }})\n"
                )
            } else {
                format!("        {call}\n        return web.json({{ ok: true }})\n")
            };
            self.text.push_str(&format!(
                "    app.post({}, fn(req) {{\n        try __kiln_rpc_check(req)\n{read}{reply}    }})\n",
                kiln_string(&format!("/_kiln/rpc/{}", f.name)),
            ));
        }
    }

    /// `fn __kiln_assets(app: web.App)`, which adds a route for each file.
    /// Files that are not UTF-8 are carried in Base64, decoded once to
    /// `Bytes` when the routes are added.
    fn assets(&mut self, assets: &[Asset]) {
        if assets.iter().any(|a| std::str::from_utf8(&a.body).is_err()) {
            self.text.push_str(
                "\nfn __kiln_asset_bytes(content_type: String, cache: String, body: Bytes) -> web.Response {\n\
                 \x20   return web.bytes(body, content_type: content_type, headers: [\"cache-control\": cache, \"x-content-type-options\": \"nosniff\"])\n\
                 }\n",
            );
        }
        self.text.push_str(&format!(
            "\nfn __kiln_asset(content_type: String, cache: String, body: String) -> web.Response {{\n\
             \x20   let headers: Map<String, String> = Map()\n\
             \x20   headers[\"content-type\"] = content_type\n\
             \x20   headers[\"cache-control\"] = cache\n\
             \x20   headers[\"x-content-type-options\"] = \"nosniff\"\n\
             \x20   return web.Response(status: 200, headers: headers, body: body)\n\
             }}\n\
             \nfn {ASSETS_FN}(app: web.App) {{\n"
        ));
        for (i, asset) in assets.iter().enumerate() {
            let (body, make) = match std::str::from_utf8(&asset.body) {
                Ok(text) => (kiln_string(text), "__kiln_asset"),
                Err(_) => {
                    use base64::Engine;
                    let data = base64::engine::general_purpose::STANDARD.encode(&asset.body);
                    (
                        format!("base64.decode(\"{data}\") catch err {{ \"\".bytes() }}"),
                        "__kiln_asset_bytes",
                    )
                }
            };
            let cache = if asset.immutable {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            self.text.push_str(&format!(
                "    let __body{i} = {body}\n    app.get({}, fn(req) {{\n        return {make}({}, \"{cache}\", __body{i})\n    }})\n",
                kiln_string(&asset.path),
                kiln_string(asset.content_type),
            ));
        }
    }

    /// `__kiln_rpc_check(req)`, which turns away calls that do not come
    /// from this version of the site's own pages: the `x-kiln-rpc` header,
    /// which only a script can set and which makes the browser ask before
    /// sending it from another site, must be this version; the body must
    /// be JSON, at most 1 MiB; and `Origin` and `Sec-Fetch-Site` must say
    /// the call comes from this host.
    fn rpc_check(&mut self, version: &str) {
        self.text.push_str(&format!(
            "\nfn __kiln_rpc_check(req: web.Request) ! {{\n\
             \x20   let version = req.header(\"x-kiln-rpc\") ?? \"\"\n\
             \x20   if version != {} {{\n\
             \x20       throw web.error(409, \"the page is out of date: reload it\")\n\
             \x20   }}\n\
             \x20   let kind = req.header(\"content-type\") ?? \"\"\n\
             \x20   if not kind.starts_with(\"application/json\") {{\n\
             \x20       throw web.error(415, \"a call must send JSON\")\n\
             \x20   }}\n\
             \x20   let host = req.header(\"host\") ?? \"\"\n\
             \x20   let origin = req.header(\"origin\") ?? \"\"\n\
             \x20   if host == \"\" or (origin != \"http://${{host}}\" and origin != \"https://${{host}}\") {{\n\
             \x20       throw web.error(403, \"a call must come from this site\")\n\
             \x20   }}\n\
             \x20   let site = req.header(\"sec-fetch-site\") ?? \"same-origin\"\n\
             \x20   if site != \"same-origin\" {{\n\
             \x20       throw web.error(403, \"a call must come from this site\")\n\
             \x20   }}\n\
             \x20   if req.body.len() > 1048576 {{\n\
             \x20       throw web.error(413, \"the arguments are too large\")\n\
             \x20   }}\n\
             }}\n",
            kiln_string(version)
        ));
    }

    /// The parameters of a function that fills a slot.
    fn slot_args_names(&self) -> &'static str {
        if self.client {
            "__out, __h"
        } else {
            "__out"
        }
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
            // The checker allows fills only where `component` and `layout`
            // take them.
            Node::Fill { .. } => {}
            Node::Slot { name, span } => {
                let name = name.as_ref().map(|n| n.name.as_str());
                // An ember's slots hold its caller's HTML, which the
                // browser keeps as it is when the ember renders again.
                if self.is_ember {
                    self.pending.push_str(&format!(
                        "<kiln-slot data-k-slot=\"{}\">",
                        name.unwrap_or("")
                    ));
                }
                if !(self.is_ember && self.client) {
                    let start = self.text.len();
                    let args = if self.client { "__out, __h" } else { "__out" };
                    self.line(&format!("{}({args})", slot_param(name)));
                    self.segment(start, *span);
                }
                if self.is_ember {
                    self.pending.push_str("</kiln-slot>");
                }
            }
        }
    }

    fn element(&mut self, e: &Element) {
        let name = &e.name.name;
        let lower = name.to_ascii_lowercase();
        self.pending.push('<');
        self.pending.push_str(name);
        // CSS cannot style a script, so it needs no scope.
        if let (Some(scope), false) = (self.scope, lower == "script") {
            self.pending.push(' ');
            self.pending.push_str(scope);
        }
        let style = e.attrs.iter().find_map(|a| match &a.value {
            AttrValue::Expr(x) if a.name.name == "ui" => Some((x, a.span)),
            _ => None,
        });
        for attr in &e.attrs {
            if style.is_none() || !matches!(attr.name.name.as_str(), "ui" | "class") {
                self.attr(attr);
            }
        }
        if let Some((x, span)) = style {
            self.styled_class(e, x, span);
        }
        self.pending.push('>');
        if VOID_ELEMENTS.contains(&lower.as_str()) {
            return;
        }
        self.nodes(&e.children);
        if let (Some(sheet), "head") = (self.stylesheet, lower.as_str()) {
            self.pending.push_str(&format!(
                "<link rel=\"stylesheet\" href=\"{}\">",
                html_attr(sheet)
            ));
        }
        self.pending.push_str("</");
        self.pending.push_str(name);
        self.pending.push('>');
    }

    fn attr(&mut self, attr: &Attr) {
        if self.is_ember && attr.name.name == "key" {
            let renamed = Attr {
                name: Ident {
                    name: "data-k-key".into(),
                    span: attr.name.span,
                },
                ..attr.clone()
            };
            return self.attr_named(&renamed);
        }
        self.attr_named(attr)
    }

    fn attr_named(&mut self, attr: &Attr) {
        let name = &attr.name.name;
        let is_url = URL_ATTRIBUTES.contains(&name.to_ascii_lowercase().as_str());
        match &attr.value {
            AttrValue::Flag => {
                self.pending.push(' ');
                self.pending.push_str(name);
            }
            // The server renders no handlers: the browser adds them when
            // it takes the ember over.
            AttrValue::Handler(block) if self.client => self.handler(attr, block),
            AttrValue::Handler(_) => {}
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
                    // Decode only authored text, before joining it with values.
                    // Validate that URL, then escape it once for HTML. Decoding
                    // after joining would interpret untrusted values as HTML.
                    // References the decoder leaves unchanged are escaped too,
                    // so the browser cannot decode them into a scheme later.
                    let decoded: Vec<_> = parts
                        .iter()
                        .map(|part| match part {
                            AttrPart::Text(t) => {
                                AttrPart::Text(html_escape::decode_html_entities(t).into_owned())
                            }
                            AttrPart::Expr(e) => AttrPart::Expr(e.clone()),
                        })
                        .collect();
                    let value = self.text_value(&decoded);
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

    /// The `class` of an element with `ui={...}`: its own classes, then the
    /// style's.
    fn styled_class(&mut self, e: &Element, style: &Expr, span: Span) {
        self.pending.push_str(" class=\"");
        let class = e.attrs.iter().find(|a| a.name.name == "class");
        if let Some(class) = class {
            match &class.value {
                AttrValue::Text(parts) => {
                    for part in parts {
                        match part {
                            AttrPart::Text(t) => self.pending.push_str(&t.replace('"', "&quot;")),
                            AttrPart::Expr(x) => self.push_value(x, "__glaze_text"),
                        }
                    }
                }
                AttrValue::Expr(x) => self.push_value(x, "__glaze_text"),
                _ => {}
            }
            self.pending.push(' ');
        }
        self.flush();
        self.start_line();
        let start = self.text.len();
        self.text.push_str("__out.push(__glaze_text(__glaze_ui(");
        self.copy(style.span, true);
        self.text.push_str(")))\n");
        self.segment(start, span);
        self.pending.push('"');
    }

    /// `<Card title={t}>children</Card>` calls `__glaze_card`.
    fn component(&mut self, e: &Element) {
        let mut file = component_file(&e.name.name);
        let ember = file.trim_end_matches(".glaze").to_owned() + ".ember";
        if self.embers.contains(&ember) {
            file = ember;
        }
        let hint = e
            .attrs
            .iter()
            .find_map(|a| a.name.name.strip_prefix("client:"))
            .unwrap_or("load");
        if self.client && self.embers.contains(&file) {
            return self.nested_island(e, &file, hint);
        }
        self.flush();
        let start = self.text.len();
        // The values in text props are worked out first, in order.
        let mut texts = HashMap::new();
        for (i, attr) in e.attrs.iter().enumerate() {
            if let AttrValue::Text(parts) = &attr.value {
                texts.insert(i, self.text_value(parts));
            }
        }
        self.start_line();
        let call = self.text.len();
        self.text.push_str(&format!("{}(__out", glaze_fn(&file)));
        if self.client {
            self.text.push_str(", __h");
        }
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
        self.slot_args(&e.children, false);
        if self.embers.contains(&file) {
            self.text
                .push_str(&format!(", __hydrate: {}", kiln_string(hint)));
        }
        self.text.push_str(")\n");
        self.segment(call, e.span);
        self.segment(start, e.span);
    }

    /// An ember inside an ember, in the browser: the outer one renders
    /// only where the inner one is, with its props and the content of its
    /// slots, and the inner one renders itself.
    fn nested_island(&mut self, e: &Element, file: &str, hint: &str) {
        self.flush();
        let start = self.text.len();
        let mut fields = Vec::new();
        for attr in &e.attrs {
            let name = &attr.name.name;
            if name.starts_with("client:") {
                continue;
            }
            match &attr.value {
                AttrValue::Flag => fields.push(format!("{name}: true")),
                AttrValue::Expr(x) => {
                    let v = self.temp_value(x);
                    fields.push(format!("{name}: {v}"));
                }
                AttrValue::Text(parts) => {
                    let v = self.text_value(parts);
                    fields.push(format!("{name}: {v}"));
                }
                AttrValue::Handler(_) => {}
            }
        }
        self.pending.push_str(&format!(
            "<kiln-island data-k-ember=\"{}\" data-k-hydrate=\"{}\" data-k-props=\"",
            stem(file),
            html_attr(hint)
        ));
        if fields.is_empty() {
            self.pending.push_str("{}");
        } else {
            self.line(&format!(
                "__out.push(__glaze_text(json.encode({{ {} }})))",
                fields.join(", ")
            ));
        }
        self.pending.push_str("\">");
        let (fills, rest): (Vec<&Node>, Vec<&Node>) = e
            .children
            .iter()
            .partition(|n| matches!(n, Node::Fill { .. }));
        for fill in fills {
            if let Node::Fill { name, body, .. } = fill {
                self.pending
                    .push_str(&format!("<kiln-slot data-k-slot=\"{}\">", name.name));
                let body: Vec<&Node> = body.iter().collect();
                for n in trim_ends(&body) {
                    self.node(&n);
                }
                self.pending.push_str("</kiln-slot>");
            }
        }
        if rest.iter().any(|n| !is_blank(n)) {
            self.pending.push_str("<kiln-slot data-k-slot=\"\">");
            for n in trim_ends(&rest) {
                self.node(&n);
            }
            self.pending.push_str("</kiln-slot>");
        }
        self.pending.push_str("</kiln-island>");
        self.flush();
        self.segment(start, e.span);
    }

    /// `on:click.prevent={ ... }` in the browser: the element is marked
    /// with the handler's number (and its modifiers), and the handler is
    /// added to `__h`, which the island's `fire` calls.
    fn handler(&mut self, attr: &Attr, block: &Block) {
        let spec = attr.name.name.trim_start_matches("on:");
        let mut parts = spec.split('.');
        let event = parts.next().unwrap_or("").to_ascii_lowercase();
        let mods: Vec<&str> = parts.collect();
        self.pending.push_str(&format!(" data-k-on-{event}=\""));
        self.line("__out.push(\"${__h.len()}\")");
        for m in &mods {
            self.pending.push(':');
            self.pending.push_str(m);
        }
        self.pending.push('"');
        self.flush();
        let start = self.text.len();
        self.line("__h.push(fn(event: ember.Event) {");
        self.indent += 1;
        self.handler_stmts(&block.stmts);
        self.indent -= 1;
        self.line("})");
        self.segment(start, attr.span);
    }

    /// A handler's statements. A statement that calls a `server fn`, such
    /// as `n = add(n, 1) catch err { n }`, sends the call, and it and the
    /// statements after it run when the reply comes.
    fn handler_stmts(&mut self, stmts: &[Stmt]) {
        for (i, stmt) in stmts.iter().enumerate() {
            let span = stmt_span(stmt);
            if let Some((call, f, args)) = self.rpc_call(stmt) {
                let mut fields = Vec::new();
                for (k, arg) in args.iter().enumerate() {
                    let name = match &arg.name {
                        Some(n) => n.name.clone(),
                        None => match f.sent().get(k) {
                            Some((n, _, _)) => n.clone(),
                            None => continue,
                        },
                    };
                    let v = self.temp_value(&arg.value);
                    fields.push(format!("{name}: {v}"));
                }
                // A JavaScript function takes its arguments by position,
                // so each one is sent, the ones left out as `none`.
                if f.module.is_some() {
                    for (n, _, default) in &f.params {
                        if !fields.iter().any(|x| x.starts_with(&format!("{n}: "))) {
                            fields.push(format!("{n}: {}", default.as_deref().unwrap_or("none")));
                        }
                    }
                    let order: Vec<&str> = f.params.iter().map(|(n, _, _)| n.as_str()).collect();
                    fields.sort_by_key(|x| {
                        order
                            .iter()
                            .position(|n| x.starts_with(&format!("{n}: ")))
                            .unwrap_or(usize::MAX)
                    });
                }
                let encoded = if fields.is_empty() {
                    "\"{}\"".to_string()
                } else {
                    format!("json.encode({{ {} }})", fields.join(", "))
                };
                self.start_line();
                let start = self.text.len();
                self.text.push_str(&format!(
                    "ember.call({}, {encoded}, fn(__reply: ember.Reply) {{\n",
                    kiln_string(&f.remote())
                ));
                self.indent += 1;
                self.start_line();
                self.copy(Span::new(span.start as usize, call.start as usize), false);
                self.text.push_str(&format!("{}(__reply)", f.proxy()));
                self.copy(Span::new(call.end as usize, span.end as usize), false);
                self.text.push('\n');
                self.handler_stmts(&stmts[i + 1..]);
                self.indent -= 1;
                self.line("})");
                self.segment(start, span);
                return;
            }
            self.start_line();
            self.copy(span, false);
            self.text.push('\n');
        }
    }

    /// The call of a `server fn` that a statement's value is, if it is
    /// one: `f(...)`, or `f(...) catch err { ... }`.
    fn rpc_call<'s>(&self, stmt: &'s Stmt) -> Option<(Span, ServerFn, &'s [Arg])> {
        let value = match stmt {
            Stmt::Let { value, .. } | Stmt::Assign { value, .. } => value,
            Stmt::Expr(e) => e,
            _ => return None,
        };
        let call = match &value.kind {
            ExprKind::Catch { expr, .. } => expr,
            _ => value,
        };
        let ExprKind::Call { callee, args, .. } = &call.kind else {
            return None;
        };
        let (module, name) = match &callee.kind {
            ExprKind::Ident(name) => (None, name),
            ExprKind::Field { expr, name } => match &expr.kind {
                ExprKind::Ident(m) => (Some(m), &name.name),
                _ => return None,
            },
            _ => return None,
        };
        let f = self
            .server_fns
            .iter()
            .find(|f| &f.name == name && f.module.as_ref() == module)?;
        Some((call.span, f.clone(), args))
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
        if self.client {
            self.text.push_str(", __h");
        }
        for arg in &layout.args {
            if let Some(name) = &arg.name {
                self.text.push_str(&format!(", {}: ", name.name));
                self.copy(arg.value.span, true);
            }
        }
        self.slot_args(nodes, true);
        self.text.push_str(")\n");
        self.segment(start, layout.span);
    }

    /// Writes the arguments that fill a component's or layout's slots:
    /// one function per `{% fill %}`, and one for the other nodes, unless
    /// they are blank (and `always` is false).
    fn slot_args(&mut self, nodes: &[Node], always: bool) {
        let (fills, rest): (Vec<&Node>, Vec<&Node>) =
            nodes.iter().partition(|n| matches!(n, Node::Fill { .. }));
        for fill in fills {
            if let Node::Fill { name, body, .. } = fill {
                self.text.push_str(&format!(
                    ", {}: fn({}) {{\n",
                    slot_param(Some(&name.name)),
                    self.slot_args_names()
                ));
                let body: Vec<&Node> = body.iter().collect();
                self.block(&trim_ends(&body));
                self.start_line();
                self.text.push('}');
            }
        }
        if always || rest.iter().any(|n| !is_blank(n)) {
            let names = self.slot_args_names();
            self.text.push_str(&format!(", __slot: fn({names}) {{\n"));
            self.indent += 1;
            for n in trim_ends(&rest) {
                self.node(&n);
            }
            self.flush();
            self.indent -= 1;
            self.start_line();
            self.text.push('}');
        }
    }
}

impl ServerFn {
    /// Whether its first parameter is the request it was called by, which
    /// the server fills in and the browser does not send.
    pub fn takes_request(&self) -> bool {
        self.params
            .first()
            .is_some_and(|(_, ty, _)| ty.trim() == "web.Request")
    }

    /// The parameters the caller sends, without the request.
    pub fn sent(&self) -> &[(String, String, Option<String>)] {
        if self.takes_request() {
            &self.params[1..]
        } else {
            &self.params
        }
    }

    /// The name the bridge calls it by: `add`, or `browser.fetch`.
    fn remote(&self) -> String {
        match &self.module {
            Some(m) => format!("{m}.{}", self.name),
            None => self.name.clone(),
        }
    }

    /// The function that reads its reply: `__rpc_add`.
    fn proxy(&self) -> String {
        match &self.module {
            Some(m) => format!("__rpc_{m}__{}", self.name),
            None => format!("__rpc_{}", self.name),
        }
    }
}

/// Whether the code `text` calls one of `fns`, by name.
fn calls_any(text: &str, fns: &[ServerFn]) -> bool {
    fns.iter().filter(|f| f.module.is_none()).any(|f| {
        text.match_indices(&f.name).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + f.name.len()..].trim_start().starts_with('(');
            after && !before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == '.')
        })
    })
}

/// The functions of `browser` that wait for the browser.
fn browser_waits() -> Vec<ServerFn> {
    let f = |name: &str, param: &str, ret: Option<&str>| ServerFn {
        module: Some("browser".into()),
        name: name.into(),
        params: vec![(param.into(), "String".into(), None)],
        ret: ret.map(String::from),
        throws: true,
    };
    vec![f("fetch", "url", Some("String")), f("copy", "text", None)]
}

/// The span of a statement.
fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::For { span, .. }
        | Stmt::While { span, .. } => *span,
        Stmt::Break(span) | Stmt::Continue(span) => *span,
        Stmt::Expr(e) => e.span,
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

/// Takes the `<style>` elements out of `nodes`, adding their spans to
/// `out`.
fn strip_styles(nodes: &mut Vec<Node>, out: &mut Vec<Span>) {
    nodes.retain(|n| match n {
        Node::Element(e) if e.name.name.eq_ignore_ascii_case("style") => {
            out.push(e.span);
            false
        }
        _ => true,
    });
    for n in nodes {
        match n {
            Node::Element(e) => strip_styles(&mut e.children, out),
            Node::Fill { body, .. } => strip_styles(body, out),
            Node::If {
                branches,
                otherwise,
                ..
            } => {
                for b in branches {
                    strip_styles(&mut b.body, out);
                }
                if let Some(o) = otherwise {
                    strip_styles(o, out);
                }
            }
            Node::For { body, empty, .. } => {
                strip_styles(body, out);
                if let Some(e) = empty {
                    strip_styles(e, out);
                }
            }
            Node::Match { cases, .. } => {
                for c in cases {
                    strip_styles(&mut c.body, out);
                }
            }
            _ => {}
        }
    }
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
                Node::Fill { body, .. } => walk(body, f),
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

/// The nodes without the whitespace they start and end with, so that slot
/// content sits where its slot is.
fn trim_ends(nodes: &[&Node]) -> Vec<Node> {
    let mut out: Vec<Node> = nodes
        .iter()
        .skip_while(|n| is_blank(n))
        .map(|n| (*n).clone())
        .collect();
    while out.last().is_some_and(is_blank) {
        out.pop();
    }
    if let Some(Node::Text { text, span }) = out.first_mut() {
        let trimmed = text.trim_start().len();
        span.start += (text.len() - trimmed) as u32;
        *text = text.trim_start().to_string();
    }
    if let Some(Node::Text { text, span }) = out.last_mut() {
        span.end -= (text.len() - text.trim_end().len()) as u32;
        *text = text.trim_end().to_string();
    }
    out
}

fn is_blank(node: &Node) -> bool {
    matches!(node, Node::Text { text, .. } if text.trim().is_empty())
}

/// Whether the text before the first value already decides what kind of
/// URL an attribute holds, as `/posts/{id}` or `https://x/{p}` do, so the
/// values cannot make it a `javascript:` URL.
fn settles_scheme(parts: &[AttrPart]) -> bool {
    // Fully static URLs are authored HTML, not an untrusted value.
    if !parts.iter().any(|p| matches!(p, AttrPart::Expr(_))) {
        return true;
    }
    let Some(AttrPart::Text(prefix)) = parts.first() else {
        return false;
    };
    let mut seen = false;
    for c in prefix.trim_start_matches(|c| c <= '\u{20}').chars() {
        match c {
            '\t' | '\n' | '\r' => {}
            // A character reference could supply a scheme character or a
            // control character the browser ignores. Check the decoded URL.
            '&' => return false,
            ':' => return true,
            'a'..='z' | 'A'..='Z' => seen = true,
            '0'..='9' | '+' | '-' | '.' if seen => {}
            _ => return true,
        }
    }
    // A value could still finish the scheme.
    false
}

/// A Kiln string literal with this text.
/// Text for a double-quoted attribute value.
fn html_attr(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
}

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
        assert!(!settles_scheme(&[text("java&#x73;"), value.clone()]));
        assert!(!settles_scheme(&[text("JaVa&Tab;S"), value.clone()]));
        assert!(!settles_scheme(&[text("\u{1}java"), value.clone()]));
        assert!(settles_scheme(&[text("/posts/&amp;"), value.clone()]));
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
