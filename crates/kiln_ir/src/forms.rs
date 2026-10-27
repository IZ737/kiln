//! `event.form<T>()` in an ember's handler, written as Kiln source.
//!
//! A submitted form reaches a handler as `event.fields`, its text by
//! name. `event.form<T>()` reads those fields as a struct the program
//! declares, the way `req.form<T>()` reads a request on the server: text
//! as it is, numbers parsed, a checkbox as `true` when it is there, and an
//! optional as `none` when it is empty. The compiler writes a function
//! `__kiln_form_T` for each struct the handlers read, and the call becomes
//! a call of it (see `lower.rs`).

use kiln_syntax::ast;

use crate::admin::{field_kind, quote, Kind};

/// The name of the function that reads a form as `ty`.
pub fn reader(ty: &str) -> String {
    format!("__kiln_form_{ty}")
}

/// The Kiln source of a reader for each struct an `event.form<T>()` in
/// `module` names, or None when there are none.
pub fn generate(module: &ast::Module) -> Option<String> {
    let mut wanted: Vec<String> = Vec::new();
    let mut found = |e: &ast::Expr| {
        let ast::ExprKind::Call {
            callee, type_args, ..
        } = &e.kind
        else {
            return;
        };
        let ast::ExprKind::Field { name, .. } = &callee.kind else {
            return;
        };
        if name.name != "form" {
            return;
        }
        if let Some(ast::TypeKind::Named { path, args }) = type_args.first().map(|t| &t.kind) {
            if let (Some(last), true) = (path.last(), args.is_empty()) {
                if !wanted.contains(&last.name) {
                    wanted.push(last.name.clone());
                }
            }
        }
    };
    for item in &module.items {
        match item {
            ast::Item::Fn(f) => kiln_check::orm::walk_block(&f.body, &mut found),
            ast::Item::Impl(imp) => {
                for m in &imp.methods {
                    kiln_check::orm::walk_block(&m.body, &mut found);
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    for name in &wanted {
        let decl = module.items.iter().find_map(|item| match item {
            ast::Item::Struct(s) if s.name.name == *name && s.generics.is_empty() => Some(s),
            _ => None,
        });
        if let Some(s) = decl {
            out.push_str(&reader_source(s));
        }
    }
    if !out.is_empty() {
        out.push_str(BOOL_READER);
    }
    (!out.is_empty()).then_some(out)
}

fn reader_source(s: &ast::StructDecl) -> String {
    let name = &s.name.name;
    let mut out = format!(
        "\nfn {}(fields: Map<String, String>) -> {name} ! {{\n",
        reader(name)
    );
    let mut args = Vec::new();
    for f in &s.fields {
        let field = &f.name.name;
        let key = quote(field);
        let local = format!("__{field}");
        let (kind, optional) = field_kind(&f.ty);
        let missing = quote(&format!("the form has no field `{field}`"));
        let parse = |text: &str| match kind {
            Kind::Str => text.to_string(),
            Kind::Int => format!(
                "{text}.trim().to_int() ?? throw Error(message: {})",
                quote(&format!("the form field `{field}` is not a whole number"))
            ),
            Kind::Float => format!(
                "{text}.trim().to_float() ?? throw Error(message: {})",
                quote(&format!("the form field `{field}` is not a number"))
            ),
            Kind::Bool => format!("try __kiln_form_bool({text}, {key})"),
            Kind::Other => format!(
                "throw Error(message: {})",
                quote(&format!(
                    "the form field `{field}` has a type a form cannot fill"
                ))
            ),
        };
        let value = match (kind, optional) {
            (Kind::Bool, false) => format!(
                "    let {local} = if fields[{key}] is some(__text) {{ {} }} else {{ false }}\n",
                parse("__text")
            ),
            (_, true) => format!(
                "    let {local} = if fields[{key}] is some(__text) and __text != \"\" {{ {} }} else {{ none }}\n",
                parse("__text")
            ),
            _ => format!(
                "    let {local} = {}\n",
                parse(&format!("(fields[{key}] ?? throw Error(message: {missing}))"))
            ),
        };
        out.push_str(&value);
        args.push(format!("{field}: {local}"));
    }
    out.push_str(&format!("    return {name}({})\n}}\n", args.join(", ")));
    out
}

const BOOL_READER: &str = "
fn __kiln_form_bool(text: String, name: String) -> Bool ! {
    let t = text.trim().lower()
    if t == \"on\" or t == \"true\" or t == \"1\" or t == \"yes\" {
        return true
    }
    if t == \"off\" or t == \"false\" or t == \"0\" or t == \"no\" or t == \"\" {
        return false
    }
    throw Error(message: \"the form field `${name}` is not true or false\")
}
";
