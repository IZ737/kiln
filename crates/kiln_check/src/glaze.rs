//! Type-checks glazes: their code against props, state and the app's types,
//! components and layouts against the props they declare, and `web.render`
//! data against the page's props.

use crate::checker::{Checker, Expect, GlazeSig, Scope};
use crate::types::*;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::*;
use std::collections::HashMap;

/// How a prop is given: an attribute value or a named argument.
enum PropValue<'a> {
    Expr(&'a Expr),
    /// `{ posts }` in render data, meaning `{ posts: posts }`.
    Shorthand(&'a Ident),
    Text(&'a [AttrPart]),
    Flag,
    Handler,
}

impl Checker {
    /// Records the props of `glaze`, so other files can check their uses.
    pub(crate) fn register_glaze(&mut self, file: &str, glaze: &Glaze) {
        let props = glaze
            .props
            .iter()
            .flat_map(|p| &p.params)
            .map(|p| ParamSig {
                name: Some(p.name.name.clone()),
                ty: match &p.ty {
                    Some(t) => self.resolve_type(t, &[], Scope::Global),
                    None => Ty::Unknown,
                },
                has_default: p.default.is_some(),
            })
            .collect();
        let sig = GlazeSig {
            props,
            has_slot: has_slot(&glaze.nodes),
        };
        self.glazes
            .get_or_insert_with(HashMap::new)
            .insert(file.to_string(), sig);
    }

    pub(crate) fn check_glaze(&mut self, file: &str, glaze: &Glaze) {
        self.in_ember = glaze.is_ember();
        self.enter_fn(Ty::Unit, None, None);
        self.push_scope();
        if let Some(props) = &glaze.props {
            let sig = self.glazes.as_ref().and_then(|g| g.get(file)).cloned();
            for (i, p) in props.params.iter().enumerate() {
                let ty = sig.as_ref().map_or(Ty::Unknown, |s| s.props[i].ty.clone());
                if let Some(default) = &p.default {
                    self.check(default, &ty);
                }
                self.bind(&p.name.name, p.name.span, ty, false, None);
            }
        }
        for state in &glaze.states {
            let ty = self.local_value(state);
            self.bind(&state.name.name, state.name.span, ty, true, None);
        }
        if let Some(layout) = &glaze.layout {
            self.check_layout(layout);
        }
        self.check_nodes(&glaze.nodes);
        self.pop_scope();
        self.exit_fn();
        self.in_ember = false;
    }

    fn local_value(&mut self, decl: &LocalDecl) -> Ty {
        match &decl.ty {
            Some(t) => {
                let ty = self.resolve_type(t, &[], Scope::Global);
                self.check(&decl.value, &ty);
                ty
            }
            None => self.infer(&decl.value, &Expect::Value(None)),
        }
    }

    fn check_nodes(&mut self, nodes: &[Node]) {
        for node in nodes {
            match node {
                Node::Text { .. } | Node::Slot { .. } => {}
                Node::Expr { expr, raw, .. } => {
                    let ty = self.infer(expr, &Expect::Value(None));
                    if *raw {
                        let string = self.prim("String");
                        self.coerce(&ty, &string, expr.span);
                    } else {
                        self.displayable(&ty, expr.span);
                    }
                }
                Node::Let(decl) => {
                    let ty = self.local_value(decl);
                    self.bind(&decl.name.name, decl.name.span, ty, false, None);
                }
                Node::Element(e) if e.is_component() => self.check_component(e),
                Node::Element(e) => {
                    for attr in &e.attrs {
                        self.check_attr(attr);
                    }
                    self.nested(&e.children);
                }
                Node::If {
                    branches,
                    otherwise,
                    ..
                } => {
                    let bool_ty = self.prim("Bool");
                    for b in branches {
                        // `is` patterns in the condition bind names for the body.
                        self.push_scope();
                        self.check(&b.cond, &bool_ty);
                        self.check_nodes(&b.body);
                        self.pop_scope();
                    }
                    if let Some(nodes) = otherwise {
                        self.nested(nodes);
                    }
                }
                Node::For {
                    binding,
                    iter,
                    filter,
                    body,
                    empty,
                    ..
                } => {
                    let elem = self.element_type(iter);
                    self.push_scope();
                    self.bind(&binding.name, binding.span, elem, false, None);
                    if let Some(f) = filter {
                        let bool_ty = self.prim("Bool");
                        self.check(f, &bool_ty);
                    }
                    self.check_nodes(body);
                    self.pop_scope();
                    if let Some(nodes) = empty {
                        self.nested(nodes);
                    }
                }
                Node::Match { expr, cases, span } => {
                    let ty = self.infer(expr, &Expect::Value(None));
                    for c in cases {
                        self.push_scope();
                        self.check_pattern(&c.pattern, &ty);
                        if let Some(guard) = &c.guard {
                            let bool_ty = self.prim("Bool");
                            self.check(guard, &bool_ty);
                        }
                        self.check_nodes(&c.body);
                        self.pop_scope();
                    }
                    let patterns: Vec<(&Pattern, bool)> = cases
                        .iter()
                        .map(|c| (&c.pattern, c.guard.is_some()))
                        .collect();
                    self.check_exhaustive(
                        &ty,
                        &patterns,
                        *span,
                        "add the missing `{% case %}` tags, or a final `{% case _ %}`",
                    );
                }
            }
        }
    }

    fn nested(&mut self, nodes: &[Node]) {
        self.push_scope();
        self.check_nodes(nodes);
        self.pop_scope();
    }

    fn check_attr(&mut self, attr: &Attr) {
        match &attr.value {
            AttrValue::Flag => {}
            AttrValue::Text(parts) => self.check_text_parts(parts),
            AttrValue::Expr(e) => {
                let ty = self.infer(e, &Expect::Value(None));
                // `disabled={busy}`: a boolean adds or leaves out the attribute.
                if !self.is_prim(&self.resolve(&ty), "Bool") {
                    self.displayable(&ty, e.span);
                }
            }
            AttrValue::Handler(block) => self.check_handler(block),
        }
    }

    fn check_text_parts(&mut self, parts: &[AttrPart]) {
        for part in parts {
            if let AttrPart::Expr(e) = part {
                let ty = self.infer(e, &Expect::Value(None));
                self.displayable(&ty, e.span);
            }
        }
    }

    /// `on:click={ ... }`: statements that run in the browser and cannot
    /// fail, so errors must be handled with `catch`.
    fn check_handler(&mut self, block: &Block) {
        self.enter_fn(Ty::Unit, None, None);
        self.check_block(block, &Expect::Discard);
        self.exit_fn();
    }

    /// Reports a value that `{value}` cannot show as text.
    fn displayable(&mut self, ty: &Ty, span: Span) {
        let ty = self.resolve(ty);
        const SHOWN: &[&str] = &[
            "String", "Int", "Float", "Bool", "Decimal", "Duration", "Date",
        ];
        let help = match &ty {
            Ty::Unknown | Ty::Var(_) | Ty::Never | Ty::Any => return,
            t if SHOWN.iter().any(|n| self.is_prim(t, n)) => return,
            Ty::Opt(_) => "it may be `none`: give a default with `??`, or unwrap it with `{% if value is some(v) %}`".to_string(),
            Ty::Adt(id, _) if ["List", "Set", "Map"].iter().any(|n| self.global_types.get(*n) == Some(id)) => {
                "loop over it with `{% for item in items %}`".to_string()
            }
            Ty::Adt(id, _) => match &self.adts[*id].kind {
                AdtKind::Struct(fields) => {
                    match fields.iter().find(|f| self.is_prim(&f.ty, "String")) {
                        Some(f) => format!("show one of its fields, as in `.{}`", f.name),
                        None => "show one of its fields".to_string(),
                    }
                }
                AdtKind::Enum(_) => {
                    "use `{% match %}` to choose the text for each case".to_string()
                }
            },
            Ty::Unit => "it produces no value".to_string(),
            Ty::Fn(_) => "call it with `(...)`".to_string(),
            _ => "show its fields instead".to_string(),
        };
        let shown = self.show(&ty);
        self.error(
            Diagnostic::error(
                codes::NOT_DISPLAYABLE,
                format!("a `{shown}` has no text form"),
                span,
            )
            .with_help(help),
        );
    }

    // ----- components, layouts and render -----

    /// The glaze for a component tag or file name, reporting it if missing.
    fn glaze_sig(&mut self, file: &str, what: &str, span: Span) -> Option<GlazeSig> {
        let glazes = self.glazes.clone().unwrap_or_default();
        if let Some(sig) = glazes.get(file) {
            return Some(sig.clone());
        }
        let mut diag = Diagnostic::error(
            codes::UNKNOWN_GLAZE,
            format!("there is no {what}: no file named `{file}` in this directory"),
            span,
        );
        // Compare names without `.glaze`, which every candidate shares.
        let stems: Vec<&str> = glazes
            .keys()
            .map(|k| k.trim_end_matches(".glaze"))
            .collect();
        if let Some(s) = similar(file.trim_end_matches(".glaze"), stems) {
            diag = diag.with_help(format!("did you mean `{s}.glaze`?"));
        }
        self.error(diag);
        None
    }

    fn check_component(&mut self, e: &Element) {
        let name = &e.name.name;
        let file = format!("{}.glaze", snake_case(name));
        let sig = self.glaze_sig(&file, &format!("component `<{name}>`"), e.name.span);
        let mut given = Vec::new();
        for attr in &e.attrs {
            let value = match &attr.value {
                AttrValue::Expr(x) => PropValue::Expr(x),
                AttrValue::Text(parts) => PropValue::Text(parts),
                AttrValue::Flag => PropValue::Flag,
                AttrValue::Handler(_) => PropValue::Handler,
            };
            if let Some(hint) = attr.name.name.strip_prefix("client:") {
                const HINTS: &[&str] = &["load", "visible", "idle", "only"];
                if !HINTS.contains(&hint) {
                    let mut diag = Diagnostic::error(
                        codes::BAD_ARGUMENTS,
                        format!("unknown hydration hint `{}`", attr.name.name),
                        attr.name.span,
                    );
                    diag = match similar(hint, HINTS.iter().copied()) {
                        Some(s) => diag.with_suggestion(
                            format!("did you mean `client:{s}`?"),
                            attr.name.span,
                            format!("client:{s}"),
                        ),
                        None => diag.with_help(
                            "the hints are `client:load`, `client:visible`, `client:idle` and `client:only`",
                        ),
                    };
                    self.error(diag);
                }
                continue;
            }
            given.push((&attr.name, value, attr.span));
        }
        if let Some(sig) = &sig {
            self.check_props(sig, &given, &format!("<{name}>"), e.name.span);
            let has_children = e
                .children
                .iter()
                .any(|n| !matches!(n, Node::Text { text, .. } if text.trim().is_empty()));
            if has_children && !sig.has_slot {
                self.error(Diagnostic::error(
                    codes::BAD_ARGUMENTS,
                    format!("`<{name}>` has no `{{% slot %}}`, so it takes no children"),
                    e.name.span,
                ));
            }
        } else {
            for (_, value, _) in &given {
                if let PropValue::Expr(x) = value {
                    self.infer(x, &Expect::Value(None));
                }
            }
        }
        self.nested(&e.children);
    }

    fn check_layout(&mut self, layout: &Layout) {
        let Some(file) = string_literal(&layout.path) else {
            self.error(Diagnostic::error(
                codes::TYPE_MISMATCH,
                "the layout must be a file name in quotes, such as `\"base.glaze\"`",
                layout.path.span,
            ));
            return;
        };
        let Some(sig) = self.glaze_sig(&file, "layout", layout.path.span) else {
            return;
        };
        if !sig.has_slot {
            self.error(Diagnostic::error(
                codes::UNKNOWN_GLAZE,
                format!("`{file}` has no `{{% slot %}}`, so it cannot be a layout"),
                layout.path.span,
            ));
        }
        let given: Vec<_> = layout
            .args
            .iter()
            .filter_map(|a| {
                a.name
                    .as_ref()
                    .map(|n| (n, PropValue::Expr(&a.value), n.span.to(a.value.span)))
            })
            .collect();
        self.check_props(&sig, &given, &file, layout.path.span);
    }

    /// `web.render("post.glaze", { post })`: the data must match the page's
    /// props.
    pub(crate) fn check_render(&mut self, args: &[Arg], span: Span) {
        let Some(template) = args.first() else {
            self.error(Diagnostic::error(
                codes::BAD_ARGUMENTS,
                "`render` needs the glaze to render, as in `web.render(\"index.glaze\", { posts })`",
                span,
            ));
            return;
        };
        let string = self.prim("String");
        let Some(file) = string_literal(&template.value) else {
            // A computed name cannot be checked until run time.
            self.check(&template.value, &string);
            for a in &args[1..] {
                self.infer(&a.value, &Expect::Value(None));
            }
            return;
        };
        let sig = self.glaze_sig(&file, "glaze", template.value.span);
        let data = args.get(1);
        if let Some(extra) = args.get(2) {
            self.error(Diagnostic::error(
                codes::BAD_ARGUMENTS,
                "too many arguments: `render` takes the glaze and its data",
                extra.value.span,
            ));
        }
        let Some(sig) = sig else {
            if let Some(d) = data {
                self.infer(&d.value, &Expect::Value(None));
            }
            return;
        };
        let mut given = Vec::new();
        if let Some(d) = data {
            match &d.value.kind {
                ExprKind::Record(fields) => {
                    for f in fields {
                        let value = match &f.value {
                            Some(v) => PropValue::Expr(v),
                            None => PropValue::Shorthand(&f.name),
                        };
                        given.push((&f.name, value, f.name.span));
                    }
                }
                _ => {
                    let ty = self.infer(&d.value, &Expect::Value(None));
                    if !matches!(self.resolve(&ty), Ty::Unknown) {
                        self.error(Diagnostic::error(
                            codes::TYPE_MISMATCH,
                            format!(
                                "the data for `{file}` must be a record of its props, as in `{{ name: value }}`"
                            ),
                            d.value.span,
                        ));
                    }
                    return;
                }
            }
        }
        self.check_props(&sig, &given, &file, span);
    }

    /// Checks props given by name against a glaze's declared props.
    fn check_props(
        &mut self,
        sig: &GlazeSig,
        given: &[(&Ident, PropValue, Span)],
        what: &str,
        span: Span,
    ) {
        let mut seen: Vec<&str> = Vec::new();
        // After a misspelled prop, "needs the prop" errors would be noise.
        let mut misnamed = false;
        for (name, value, value_span) in given {
            let Some(prop) = sig
                .props
                .iter()
                .find(|p| p.name.as_deref() == Some(&name.name))
            else {
                let mut diag = Diagnostic::error(
                    codes::BAD_ARGUMENTS,
                    format!("`{what}` has no prop named `{}`", name.name),
                    name.span,
                );
                let given_names: Vec<&str> =
                    given.iter().map(|(n, _, _)| n.name.as_str()).collect();
                let names = sig
                    .props
                    .iter()
                    .filter_map(|p| p.name.as_deref())
                    .filter(|n| !given_names.contains(n));
                if let Some(s) = similar(&name.name, names) {
                    diag = diag.with_suggestion(format!("did you mean `{s}`?"), name.span, s);
                }
                self.error(diag);
                misnamed = true;
                match value {
                    PropValue::Expr(x) => {
                        self.infer(x, &Expect::Value(None));
                    }
                    PropValue::Shorthand(id) => {
                        self.infer_ident(&id.name, id.span);
                    }
                    _ => {}
                }
                continue;
            };
            if seen.contains(&name.name.as_str()) {
                self.error(Diagnostic::error(
                    codes::BAD_ARGUMENTS,
                    format!("`{}` is given more than once", name.name),
                    name.span,
                ));
            }
            seen.push(&name.name);
            match value {
                PropValue::Expr(x) => self.check(x, &prop.ty),
                PropValue::Shorthand(id) => {
                    let ty = self.infer_ident(&id.name, id.span);
                    self.coerce(&ty, &prop.ty, id.span);
                }
                PropValue::Text(parts) => {
                    self.check_text_parts(parts);
                    let string = self.prim("String");
                    self.coerce(&string, &prop.ty, *value_span);
                }
                PropValue::Flag => {
                    let bool_ty = self.prim("Bool");
                    self.coerce(&bool_ty, &prop.ty, *value_span);
                }
                PropValue::Handler => {
                    self.error(Diagnostic::error(
                        codes::BAD_ARGUMENTS,
                        format!("`{what}` takes props, not event handlers"),
                        name.span,
                    ));
                }
            }
        }
        let missing: Vec<String> = sig
            .props
            .iter()
            .filter(|p| !p.has_default)
            .filter_map(|p| p.name.clone())
            .filter(|n| !seen.contains(&n.as_str()))
            .map(|n| format!("`{n}`"))
            .collect();
        if !missing.is_empty() && !misnamed {
            self.error(Diagnostic::error(
                codes::BAD_ARGUMENTS,
                format!("`{what}` needs the prop {}", missing.join(", ")),
                span,
            ));
        }
    }
}

fn string_literal(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Str(pieces) => match pieces.as_slice() {
            [StrPiece::Text(t)] => Some(t.clone()),
            [] => Some(String::new()),
            _ => None,
        },
        _ => None,
    }
}

/// `LikeButton` is `like_button`.
pub(crate) fn snake_case(name: &str) -> String {
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
    out
}

fn has_slot(nodes: &[Node]) -> bool {
    nodes.iter().any(|n| match n {
        Node::Slot { .. } => true,
        Node::Element(e) => has_slot(&e.children),
        Node::If {
            branches,
            otherwise,
            ..
        } => {
            branches.iter().any(|b| has_slot(&b.body)) || otherwise.as_deref().is_some_and(has_slot)
        }
        Node::For { body, empty, .. } => has_slot(body) || empty.as_deref().is_some_and(has_slot),
        Node::Match { cases, .. } => cases.iter().any(|c| has_slot(&c.body)),
        _ => false,
    })
}
