//! The admin's pages, written as Kiln source.
//!
//! `admin.pages<Post>(app, prefix, site)` asks for the pages of a model.
//! This reads the model's fields from the program's own source and writes
//! the handlers for them in Kiln, which the compiler then checks and
//! builds with the rest of the program: the list with its search, order
//! and pages, the form that creates a row, the form that changes one, and
//! the page that asks before deleting one.
//!
//! The pages are ordinary Kiln, so they reach the database through `orm`,
//! which means a model's mapping, relations and defaults apply as they do
//! anywhere else, and nothing a visitor types is ever part of a query.
//! Every page asks `site.authorize` after `sessions.require`, and every
//! form carries the session's token.

use std::collections::BTreeMap;

use kiln_syntax::ast;

/// What a field can be in a form.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Str,
    Int,
    Float,
    Bool,
    /// A type a form cannot carry: a relation, a list, a time, a record.
    Other,
}

#[derive(Clone, Debug)]
struct Field {
    name: String,
    kind: Kind,
    optional: bool,
    has_default: bool,
}

impl Field {
    /// Whether a form can carry it.
    fn in_form(&self) -> bool {
        self.kind != Kind::Other
    }

    /// The label a page gives it: `author_name` is "Author name".
    fn label(&self) -> String {
        let words = self.name.replace('_', " ");
        let mut chars = words.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => words,
        }
    }

    /// The kind of HTML input it gets.
    fn input(&self) -> &'static str {
        match self.kind {
            Kind::Int => "number",
            Kind::Float => "decimal",
            _ => "text",
        }
    }
}

/// A model the program asks for pages for.
#[derive(Clone, Debug)]
struct Model {
    name: String,
    fields: Vec<Field>,
}

impl Model {
    /// The name the program writes, which pages show: a model of a folder
    /// module has been renamed by the time the generator reads it.
    fn shown(&self) -> &str {
        kiln_check::orm::declared_name(&self.name)
    }

    fn key(&self) -> Option<&Field> {
        self.fields
            .iter()
            .find(|f| f.name == "id" && f.kind == Kind::Int && !f.optional)
    }

    /// The fields a form carries, which is every one a form can hold but
    /// the key.
    fn editable(&self) -> Vec<&Field> {
        self.fields
            .iter()
            .filter(|f| f.in_form() && f.name != "id" && !secret(&f.name))
            .collect()
    }

    /// Whether a row can be created here: every field a form cannot
    /// carry, and every secret, must have a default to fall back on.
    fn creatable(&self) -> bool {
        self.fields
            .iter()
            .all(|f| f.name == "id" || f.has_default || (f.in_form() && !secret(&f.name)))
    }

    /// The columns the list shows, and that it can be ordered by.
    fn columns(&self) -> Vec<&Field> {
        let mut out = Vec::new();
        if let Some(key) = self.key() {
            out.push(key);
        }
        for f in self.editable() {
            if out.len() == 5 {
                break;
            }
            out.push(f);
        }
        out
    }

    /// The text columns a search looks through.
    fn searched(&self) -> Vec<&Field> {
        self.editable()
            .into_iter()
            .filter(|f| f.kind == Kind::Str && !f.optional)
            .collect()
    }
}

/// Whether a field's name says it holds something a page must not show.
/// The admin leaves these out of its forms and says they are not shown.
fn secret(name: &str) -> bool {
    const WORDS: &[&str] = &["password", "secret", "token", "hash", "key"];
    let name = name.to_lowercase();
    WORDS.iter().any(|w| name == *w || name.contains(w))
}

/// The Kiln source of the pages the program asks for, or None when it
/// asks for none. `models` is every struct the program declares.
pub fn generate(module: &ast::Module) -> Option<String> {
    let wanted = wanted_models(module);
    if wanted.is_empty() {
        return None;
    }
    let declared = declared_models(module);
    let mut out = String::new();
    let mut any = false;
    for name in &wanted {
        let Some(model) = declared.get(name) else {
            continue;
        };
        if model.key().is_none() {
            continue;
        }
        if !any {
            out.push_str(SHARED);
            any = true;
        }
        out.push_str(&model_source(model));
    }
    any.then_some(out)
}

/// The models of `admin.pages<T>(...)` calls, in the order they are
/// written.
fn wanted_models(module: &ast::Module) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut found = |e: &ast::Expr| {
        let ast::ExprKind::Call {
            callee, type_args, ..
        } = &e.kind
        else {
            return;
        };
        let ast::ExprKind::Field { expr, name } = &callee.kind else {
            return;
        };
        if name.name != "pages" || !matches!(&expr.kind, ast::ExprKind::Ident(m) if m == "admin") {
            return;
        }
        if let Some(model) = type_args.first().and_then(type_name) {
            if !out.contains(&model) {
                out.push(model);
            }
        }
    };
    for item in &module.items {
        if let ast::Item::Fn(f) = item {
            kiln_check::orm::walk_block(&f.body, &mut found);
        }
    }
    out
}

/// Every struct the program declares, by name.
fn declared_models(module: &ast::Module) -> BTreeMap<String, Model> {
    let mut out = BTreeMap::new();
    for item in &module.items {
        let ast::Item::Struct(s) = item else { continue };
        if !s.generics.is_empty() {
            continue;
        }
        let fields = s
            .fields
            .iter()
            .map(|f| {
                let (kind, optional) = field_kind(&f.ty);
                Field {
                    name: f.name.name.clone(),
                    kind,
                    optional,
                    has_default: f.default.is_some(),
                }
            })
            .collect();
        out.insert(
            s.name.name.clone(),
            Model {
                name: s.name.name.clone(),
                fields,
            },
        );
    }
    out
}

/// The name a type expression names, for `Post` and `module.Post`.
fn type_name(t: &ast::TypeExpr) -> Option<String> {
    match &t.kind {
        ast::TypeKind::Named { path, args } if args.is_empty() => {
            path.last().map(|i| i.name.clone())
        }
        _ => None,
    }
}

/// What a field's written type is in a form.
fn field_kind(t: &ast::TypeExpr) -> (Kind, bool) {
    match &t.kind {
        ast::TypeKind::Optional(inner) => (field_kind(inner).0, true),
        ast::TypeKind::Named { path, args } if args.is_empty() => {
            let kind = match path.last().map(|i| i.name.as_str()) {
                Some("String") => Kind::Str,
                Some("Int") => Kind::Int,
                Some("Float") => Kind::Float,
                Some("Bool") => Kind::Bool,
                _ => Kind::Other,
            };
            (kind, false)
        }
        _ => (Kind::Other, false),
    }
}

/// A Kiln string literal holding `text`.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '$' => out.push_str("\\$"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The name of a generated function.
fn fn_name(model: &str, what: &str) -> String {
    format!("__kiln_admin_{model}_{what}")
}

/// The part of the pages that does not depend on a model.
const SHARED: &str = r##"
// ----- the admin's pages, generated for `admin.pages` -----
import web, sql, orm, auth, admin, validate, html

const __KILN_ADMIN_CSS: String = "*{box-sizing:border-box}body{margin:0;font:16px/1.5 system-ui,sans-serif;color:#1F232B;background:#F7F3EC}header{padding:1rem 1.5rem;background:#1F232B;color:#F7F3EC}header a{color:inherit;font-weight:600;text-decoration:none}main{padding:1.5rem;max-width:60rem}h1{font-size:1.5rem;margin:0 0 1rem}table{border-collapse:collapse;width:100%;background:#fff}th,td{text-align:left;padding:.5rem .75rem;border-bottom:1px solid #e4ded3}th a{color:inherit}tr:hover td{background:#faf7f2}label{display:block;margin:0 0 1rem}label span{display:block;font-weight:600;font-size:.875rem}input[type=text],input[type=number]{width:100%;padding:.5rem;border:1px solid #c9c2b5;border-radius:.25rem;font:inherit}.problem{color:#9b2c16;font-size:.875rem}.secret{color:#8a8578}button{font:inherit;padding:.5rem 1rem;border:0;border-radius:.25rem;background:#C65A34;color:#fff;cursor:pointer}a.button{display:inline-block;padding:.5rem 1rem;border-radius:.25rem;background:#1F232B;color:#F7F3EC;text-decoration:none}form.search{margin:0 0 1rem;display:flex;gap:.5rem}nav{margin:1rem 0;display:flex;gap:.75rem}"

fn __kiln_admin_page(site: admin.Site, title: String, body: String, status: Int) -> web.Response {
    let head = "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>${html.escape(title)} - ${html.escape(site.title)}</title><style>${__KILN_ADMIN_CSS}</style></head><body>"
    let header = "<header><a href=\"${html.escape(site.home)}\">${html.escape(site.title)}</a></header><main><h1>${html.escape(title)}</h1>"
    let res = web.html("${head}${header}${body}</main></body></html>", status: status)
    res.headers["x-frame-options"] = "DENY"
    res.headers["referrer-policy"] = "same-origin"
    res.headers["cache-control"] = "no-store"
    return res
}

// Nothing is allowed by default: a page is for this identity only when
// the program's own rule says so.
fn __kiln_admin_allow(site: admin.Site, req: web.Request, action: String, model: String) -> auth.Identity ! {
    let who = try site.sessions.require(req)
    let decide = site.authorize
    if not decide(who, action, model) {
        throw web.error(403, "you cannot ${action} ${model}")
    }
    return who
}

fn __kiln_admin_input(label: String, name: String, kind: String, value: String, problem: String) -> String {
    let step = if kind == "decimal" { " step=\"any\"" } else { "" }
    let ty = if kind == "decimal" { "number" } else { kind }
    let field = "<input type=\"${ty}\"${step} id=\"${html.escape(name)}\" name=\"${html.escape(name)}\" value=\"${html.escape(value)}\">"
    let said = if problem == "" { "" } else { "<span class=\"problem\">${html.escape(problem)}</span>" }
    return "<label><span>${html.escape(label)}</span>${field}${said}</label>"
}

fn __kiln_admin_checkbox(label: String, name: String, on: Bool) -> String {
    let mark = if on { " checked" } else { "" }
    return "<label><span>${html.escape(label)}</span><input type=\"checkbox\" name=\"${html.escape(name)}\" value=\"on\"${mark}></label>"
}

fn __kiln_admin_shown(label: String, value: String) -> String {
    return "<label><span>${html.escape(label)}</span><span class=\"secret\">${html.escape(value)}</span></label>"
}

// A number a form sent, and whether it is one at all.
fn __kiln_admin_int(text: String) -> Int? {
    if text.trim() == "" {
        return none
    }
    return text.trim().to_int()
}

fn __kiln_admin_float(text: String) -> Float? {
    if text.trim() == "" {
        return none
    }
    return text.trim().to_float()
}

fn __kiln_admin_bad_int(text: String, required: Bool) -> Bool {
    if text.trim() == "" {
        return required
    }
    return match text.trim().to_int() {
        some(n) => false
        none => true
    }
}

fn __kiln_admin_bad_float(text: String, required: Bool) -> Bool {
    if text.trim() == "" {
        return required
    }
    return match text.trim().to_float() {
        some(n) => false
        none => true
    }
}

// What to tell an operator about a database that refused a change. The
// error's own message names tables and columns, which belongs in the log
// and not on the page.
fn __kiln_admin_why(err: Error) -> String {
    return match sql.error_kind(err) {
        "unique" => "another row already has that value"
        "foreign_key" => "that points at a row which is not there"
        "not_null" => "something this needs was left out"
        "check" => "the database does not allow that value"
        "constraint" => "the database does not allow that value"
        _ => "the change could not be saved"
    }
}

fn __kiln_admin_search(prefix: String, term: String) -> String {
    return "<form class=\"search\" method=\"get\" action=\"${html.escape(prefix)}\"><input type=\"text\" name=\"q\" value=\"${html.escape(term)}\" placeholder=\"Search\"><button type=\"submit\">Search</button></form>"
}

// Text as a value in a link's query.
fn __kiln_admin_param(text: String) -> String {
    let safe = text.replace("%", "%25").replace("&", "%26").replace("#", "%23").replace("+", "%2B").replace("=", "%3D").replace("?", "%3F")
    return safe.replace(" ", "+")
}

fn __kiln_admin_pager(prefix: String, term: String, sort: String, dir: String, page: Int, pages: Int) -> String {
    if pages < 2 {
        return ""
    }
    let searched = if term == "" { "" } else { "&q=${__kiln_admin_param(term)}" }
    let query = html.escape("&sort=${__kiln_admin_param(sort)}&dir=${dir}${searched}")
    var out = "<nav>"
    if page > 1 {
        out = out + "<a href=\"${html.escape(prefix)}?page=${page - 1}${query}\">Back</a>"
    }
    out = out + "<span>Page ${page} of ${pages}</span>"
    if page < pages {
        out = out + "<a href=\"${html.escape(prefix)}?page=${page + 1}${query}\">Next</a>"
    }
    return out + "</nav>"
}

fn __kiln_admin_heading(prefix: String, term: String, label: String, column: String, sort: String, dir: String) -> String {
    let next = if sort == column and dir == "asc" { "desc" } else { "asc" }
    let mark = if sort == column { if dir == "asc" { " ^" } else { " v" } } else { "" }
    let searched = if term == "" { "" } else { "&q=${__kiln_admin_param(term)}" }
    let query = html.escape("sort=${__kiln_admin_param(column)}&dir=${next}${searched}")
    return "<th><a href=\"${html.escape(prefix)}?${query}\">${html.escape(label)}${mark}</a>"
}
"##;

/// The pages of one model.
fn model_source(model: &Model) -> String {
    let name = &model.name;
    let mut out = format!("\n// ----- {name} -----\n");
    out.push_str(&rows_source(model));
    out.push_str(&values_source(model));
    out.push_str(&fields_source(model));
    out.push_str(&form_source(model));
    out.push_str(&list_source(model));
    out.push_str(&read_source(model));
    out.push_str(&write_source(model));
    out.push_str(&routes_source(model));
    out
}

/// The condition that searches the text columns, or None when the model
/// has none to search.
fn search_condition(model: &Model) -> Option<String> {
    let columns = model.searched();
    if columns.is_empty() {
        return None;
    }
    let parts: Vec<String> = columns
        .iter()
        .map(|f| format!("__row.{}.contains(term)", f.name))
        .collect();
    Some(format!(".where(fn(__row) {{ {} }})", parts.join(" or ")))
}

/// `<model>_rows`: the page of rows a list shows, in the order it asks
/// for. Every order is a query of its own, so that the compiler writes
/// the SQL and a visitor cannot.
fn rows_source(model: &Model) -> String {
    let name = &model.name;
    let search = search_condition(model);
    let mut out = format!(
        "\nfn {}(site: admin.Site, term: String, sort: String, dir: String, size: Int, skip: Int) -> List<{name}> ! {{\n\
         \x20   let db = site.db\n\
         \x20   let wanted = \"${{sort}}/${{dir}}\"\n",
        fn_name(name, "rows"),
    );
    let arm = |cond: &str, order: &str, search: &str| {
        format!(
            "        {cond} => try orm.find<{name}>(db){search}{order}.limit(size).offset(skip).all()\n"
        )
    };
    for (i, with_search) in [false, true].into_iter().enumerate() {
        let s = match (&search, with_search) {
            (Some(s), true) => s.as_str(),
            (_, true) => continue,
            _ => "",
        };
        let guard = if i == 0 {
            "    if term == \"\" {\n"
        } else {
            "    } else {\n"
        };
        out.push_str(guard);
        out.push_str("        return match wanted {\n");
        for column in model.columns() {
            for dir in ["asc", "desc"] {
                let descending = if dir == "desc" {
                    ", descending: true"
                } else {
                    ""
                };
                let order = format!(".order_by(.{}{descending})", column.name);
                out.push_str(&arm(&quote(&format!("{}/{dir}", column.name)), &order, s));
            }
        }
        out.push_str(&arm("_", ".order_by(.id)", s));
        out.push_str("        }\n");
        if search.is_none() {
            break;
        }
    }
    if search.is_some() {
        out.push_str("    }\n");
    } else {
        out.push_str("    }\n    return []\n");
    }
    out.push_str("}\n");
    out.push_str(&format!(
        "\nfn {}(site: admin.Site, term: String) -> Int ! {{\n\
         \x20   if term == \"\" {{\n\
         \x20       return try orm.find<{name}>(site.db).count()\n\
         \x20   }}\n\
         \x20   return try orm.find<{name}>(site.db){}.count()\n\
         }}\n",
        fn_name(name, "count"),
        search.as_deref().unwrap_or(""),
    ));
    out
}

/// `<model>_values`: a row as the text a form holds.
fn values_source(model: &Model) -> String {
    let mut out = format!(
        "\nfn {}(row: {}) -> Map<String, String> {{\n    var values: Map<String, String> = Map()\n",
        fn_name(&model.name, "values"),
        model.name,
    );
    for f in model.editable() {
        let name = &f.name;
        let text = match (f.kind, f.optional) {
            (Kind::Bool, _) => format!("if row.{name} {{ \"on\" }} else {{ \"\" }}"),
            (_, true) => format!("match row.{name} {{\n        some(__v) => \"${{__v}}\"\n        none => \"\"\n    }}"),
            (Kind::Str, false) => format!("row.{name}"),
            _ => format!("\"${{row.{name}}}\""),
        };
        out.push_str(&format!("    values[{}] = {text}\n", quote(name)));
    }
    out.push_str("    return values\n}\n");
    out
}

/// `<model>_fields`: the form's fields, with what is wrong with them.
fn fields_source(model: &Model) -> String {
    let mut out = format!(
        "\nfn {}(values: Map<String, String>, checked: validate.Checked) -> String {{\n    var out = \"\"\n",
        fn_name(&model.name, "fields"),
    );
    for f in &model.fields {
        let (name, label) = (quote(&f.name), quote(&f.label()));
        if f.name == "id" {
            continue;
        }
        if secret(&f.name) {
            out.push_str(&format!(
                "    out = out + __kiln_admin_shown({label}, \"not shown here\")\n"
            ));
            continue;
        }
        match f.kind {
            Kind::Bool => out.push_str(&format!(
                "    out = out + __kiln_admin_checkbox({label}, {name}, (values.get({name}) ?? \"\") != \"\")\n"
            )),
            Kind::Other => out.push_str(&format!(
                "    out = out + __kiln_admin_shown({label}, \"not changed here\")\n"
            )),
            _ => out.push_str(&format!(
                "    out = out + __kiln_admin_input({label}, {name}, {}, values.get({name}) ?? \"\", checked.error({name}) ?? \"\")\n",
                quote(f.input()),
            )),
        }
    }
    out.push_str("    return out\n}\n");
    out
}

/// `<model>_form`: the page that creates or changes a row. `key` is 0 for
/// a new row.
fn form_source(model: &Model) -> String {
    let name = &model.name;
    let shown = model.shown();
    let deletable = format!(
        "    if key != 0 {{\n\
         \x20       body = body + \"<p><a href=\\\"${{html.escape(prefix)}}/${{key}}/delete\\\">Delete this {}</a>\"\n\
         \x20   }}\n",
        shown.to_lowercase(),
    );
    format!(
        "\nfn {}(req: web.Request, prefix: String, site: admin.Site, key: Int, values: Map<String, String>, checked: validate.Checked, status: Int) -> web.Response ! {{\n\
         \x20   let token = try site.sessions.csrf(req)\n\
         \x20   let action = if key == 0 {{ prefix }} else {{ \"${{prefix}}/${{key}}\" }}\n\
         \x20   let title = if key == 0 {{ \"New {shown}\" }} else {{ \"{shown} ${{key}}\" }}\n\
         \x20   var body = \"\"\n\
         \x20   for problem in checked.problems {{\n\
         \x20       if problem.field == \"\" {{\n\
         \x20           body = body + \"<p class=\\\"problem\\\">${{html.escape(problem.message)}}\"\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   body = body + \"<form method=\\\"post\\\" action=\\\"${{html.escape(action)}}\\\"><input type=\\\"hidden\\\" name=\\\"csrf\\\" value=\\\"${{html.escape(token)}}\\\">\"\n\
         \x20   body = body + {fields}(values, checked)\n\
         \x20   body = body + \"<button type=\\\"submit\\\">Save</button></form>\"\n\
         {deletable}\
         \x20   body = body + \"<p><a href=\\\"${{html.escape(prefix)}}\\\">Back to the list</a>\"\n\
         \x20   return __kiln_admin_page(site, title, body, status)\n\
         }}\n",
        fn_name(name, "form"),
        fields = fn_name(name, "fields"),
    )
}

/// `<model>_list`: the list page, with its search, order and pages.
fn list_source(model: &Model) -> String {
    let name = &model.name;
    let mut headings = String::new();
    for column in model.columns() {
        headings.push_str(&format!(
            "    head = head + __kiln_admin_heading(prefix, term, {}, {}, sort, dir)\n",
            quote(&column.label()),
            quote(&column.name),
        ));
    }
    let mut cells = String::new();
    for (i, column) in model.columns().iter().enumerate() {
        let shown = if secret(&column.name) {
            "\"not shown here\"".to_string()
        } else if column.optional {
            format!(
                "match row.{} {{\n            some(__v) => \"${{__v}}\"\n            none => \"\"\n        }}",
                column.name
            )
        } else {
            format!("\"${{row.{}}}\"", column.name)
        };
        if i == 0 {
            cells.push_str(
                "        body = body + \"<td><a href=\\\"${html.escape(prefix)}/${row.id}\\\">${row.id}</a>\"\n",
            );
        } else {
            cells.push_str(&format!(
                "        let __cell_{i} = {shown}\n        body = body + \"<td>${{html.escape(__cell_{i})}}\"\n"
            ));
        }
    }
    let new = if model.creatable() {
        "    body = body + \"<p><a class=\\\"button\\\" href=\\\"${html.escape(prefix)}/new\\\">New {name}</a>\"\n"
            .replace("{name}", model.shown())
    } else {
        String::new()
    };
    format!(
        "\nfn {}(req: web.Request, prefix: String, site: admin.Site) -> web.Response ! {{\n\
         \x20   let who = try __kiln_admin_allow(site, req, \"list\", {model})\n\
         \x20   let term = req.query(\"q\") ?? \"\"\n\
         \x20   let asked = (req.query(\"page\") ?? \"1\").to_int() ?? 1\n\
         \x20   let page = asked.max(1)\n\
         \x20   let size = site.per_page.clamp(1, 200)\n\
         \x20   let sort = req.query(\"sort\") ?? \"id\"\n\
         \x20   let dir = if (req.query(\"dir\") ?? \"\") == \"desc\" {{ \"desc\" }} else {{ \"asc\" }}\n\
         \x20   let total = try {count}(site, term)\n\
         \x20   let pages = if total == 0 {{ 1 }} else {{ (total + size - 1) / size }}\n\
         \x20   let rows = try {rows}(site, term, sort, dir, size, (page.min(pages) - 1) * size)\n\
         \x20   var body = __kiln_admin_search(prefix, term)\n\
         \x20   var head = \"<table><tr>\"\n\
         {headings}\
         \x20   body = body + head\n\
         \x20   for row in rows {{\n\
         \x20       body = body + \"<tr>\"\n\
         {cells}\
         \x20   }}\n\
         \x20   body = body + \"</table>\"\n\
         \x20   if rows.len() == 0 {{\n\
         \x20       body = body + \"<p>Nothing here yet.\"\n\
         \x20   }}\n\
         \x20   body = body + __kiln_admin_pager(prefix, term, sort, dir, page.min(pages), pages)\n\
         {new}\
         \x20   return __kiln_admin_page(site, \"{plural} (${{total}})\", body, 200)\n\
         }}\n",
        fn_name(name, "list"),
        plural = kiln_check::orm::plural(model.shown()),
        model = quote(model.shown()),
        count = fn_name(name, "count"),
        rows = fn_name(name, "rows"),
    )
}

/// `<model>_read`: the form a request sent, as text and what is wrong
/// with it.
fn read_source(model: &Model) -> String {
    let name = &model.name;
    let mut record = vec!["csrf: String".to_string()];
    for f in model.editable() {
        // A browser sends every text field, but a form posted some other
        // way may leave one out, which is the same as leaving it empty.
        let ty = if f.kind == Kind::Bool {
            "Bool"
        } else {
            "String?"
        };
        record.push(format!("{}: {ty}", f.name));
    }
    let mut values = String::new();
    let mut rules = Vec::new();
    for f in model.editable() {
        let name = quote(&f.name);
        let text = if f.kind == Kind::Bool {
            format!("if form.{} {{ \"on\" }} else {{ \"\" }}", f.name)
        } else {
            format!("form.{} ?? \"\"", f.name)
        };
        values.push_str(&format!("    values[{name}] = {text}\n"));
        let required = if f.optional || f.has_default {
            "false"
        } else {
            "true"
        };
        match f.kind {
            Kind::Str if !f.optional && !f.has_default => {
                rules.push(format!("validate.required({name}, (form.{} ?? \"\"))", f.name))
            }
            Kind::Int => rules.push(format!(
                "validate.when(__kiln_admin_bad_int((form.{} ?? \"\"), {required}), {name}, \"this needs a whole number\")",
                f.name,
            )),
            Kind::Float => rules.push(format!(
                "validate.when(__kiln_admin_bad_float((form.{} ?? \"\"), {required}), {name}, \"this needs a number\")",
                f.name,
            )),
            _ => {}
        }
    }
    let rules = if rules.is_empty() {
        String::new()
    } else {
        format!("        {},\n", rules.join(",\n        "))
    };
    format!(
        "\nfn {}(req: web.Request) -> {{ values: Map<String, String>, checked: validate.Checked, csrf: String }} ! {{\n\
         \x20   let form = try req.form<{{ {record} }}>()\n\
         \x20   var values: Map<String, String> = Map()\n\
         {values}\
         \x20   let checked = validate.check([\n{rules}    ])\n\
         \x20   return {{ values: values, checked: checked, csrf: form.csrf }}\n\
         }}\n",
        fn_name(name, "read"),
        record = record.join(", "),
    )
}

/// The value of one field, out of the form's text.
fn from_values(f: &Field) -> String {
    let name = quote(&f.name);
    let text = format!("(values.get({name}) ?? \"\")");
    match (f.kind, f.optional) {
        (Kind::Bool, _) => format!("{text} != \"\""),
        (Kind::Str, true) => format!("if {text} == \"\" {{ none }} else {{ {text} }}"),
        (Kind::Str, false) => text,
        (Kind::Int, true) => format!("__kiln_admin_int({text})"),
        (Kind::Int, false) => format!("__kiln_admin_int({text}) ?? 0"),
        (Kind::Float, true) => format!("__kiln_admin_float({text})"),
        (Kind::Float, false) => format!("__kiln_admin_float({text}) ?? 0.0"),
        (Kind::Other, _) => "none".to_string(),
    }
}

/// `<model>_create`, `_update` and `_delete`: the handlers that change
/// the database, each of which checks the session's token first.
fn write_source(model: &Model) -> String {
    let name = &model.name;
    let mut out = String::new();
    if model.creatable() {
        let mut made = vec!["id: 0".to_string()];
        for f in model.editable() {
            made.push(format!("{}: {}", f.name, from_values(f)));
        }
        out.push_str(&format!(
            "\nfn {}(req: web.Request, prefix: String, site: admin.Site) -> web.Response ! {{\n\
             \x20   let who = try __kiln_admin_allow(site, req, \"create\", {model})\n\
             \x20   let sent = try {read}(req)\n\
             \x20   try site.sessions.check(req, sent.csrf)\n\
             \x20   if not sent.checked.valid {{\n\
             \x20       return try {form}(req, prefix, site, 0, sent.values, sent.checked, 422)\n\
             \x20   }}\n\
             \x20   let values = sent.values\n\
             \x20   let made = {name}({made})\n\
             \x20   let stored = orm.insert<{name}>(site.db, made) catch err {{\n\
             \x20       let why = validate.check([validate.problem(\"\", __kiln_admin_why(err))])\n\
             \x20       return try {form}(req, prefix, site, 0, sent.values, why, 422)\n\
             \x20   }}\n\
             \x20   return web.redirect(\"${{prefix}}/${{stored.id}}\", status: 303)\n\
             }}\n\
             \nfn {}(req: web.Request, prefix: String, site: admin.Site) -> web.Response ! {{\n\
             \x20   let who = try __kiln_admin_allow(site, req, \"create\", {model})\n\
             \x20   var values: Map<String, String> = Map()\n\
             \x20   return try {form}(req, prefix, site, 0, values, validate.check([]), 200)\n\
             }}\n",
            fn_name(name, "create"),
            fn_name(name, "new"),
            model = quote(model.shown()),
            read = fn_name(name, "read"),
            form = fn_name(name, "form"),
            made = made.join(", "),
        ));
    }
    let mut changed = vec!["id: row.id".to_string()];
    for f in &model.fields {
        if f.name == "id" {
            continue;
        }
        if f.in_form() && !secret(&f.name) {
            changed.push(format!("{}: {}", f.name, from_values(f)));
        } else {
            changed.push(format!("{}: row.{}", f.name, f.name));
        }
    }
    out.push_str(&format!(
        "\nfn {edit}(req: web.Request, prefix: String, site: admin.Site, key: Int) -> web.Response ! {{\n\
         \x20   let who = try __kiln_admin_allow(site, req, \"view\", {model})\n\
         \x20   let found = try orm.get<{name}>(site.db, key)\n\
         \x20   if found is some(row) {{\n\
         \x20       return try {form}(req, prefix, site, key, {values}(row), validate.check([]), 200)\n\
         \x20   }}\n\
         \x20   return web.not_found()\n\
         }}\n\
         \nfn {update}(req: web.Request, prefix: String, site: admin.Site, key: Int) -> web.Response ! {{\n\
         \x20   let who = try __kiln_admin_allow(site, req, \"update\", {model})\n\
         \x20   let sent = try {read}(req)\n\
         \x20   try site.sessions.check(req, sent.csrf)\n\
         \x20   let found = try orm.get<{name}>(site.db, key)\n\
         \x20   if found is some(row) {{\n\
         \x20       if not sent.checked.valid {{\n\
         \x20           return try {form}(req, prefix, site, key, sent.values, sent.checked, 422)\n\
         \x20       }}\n\
         \x20       let values = sent.values\n\
         \x20       let changed = {name}({changed})\n\
         \x20       orm.update<{name}>(site.db, changed) catch err {{\n\
         \x20           let why = validate.check([validate.problem(\"\", __kiln_admin_why(err))])\n\
         \x20           return try {form}(req, prefix, site, key, sent.values, why, 422)\n\
         \x20       }}\n\
         \x20       return web.redirect(\"${{prefix}}/${{key}}\", status: 303)\n\
         \x20   }}\n\
         \x20   return web.not_found()\n\
         }}\n\
         \nfn {asked}(req: web.Request, prefix: String, site: admin.Site, key: Int) -> web.Response ! {{\n\
         \x20   let who = try __kiln_admin_allow(site, req, \"delete\", {model})\n\
         \x20   let token = try site.sessions.csrf(req)\n\
         \x20   let found = try orm.get<{name}>(site.db, key)\n\
         \x20   if found is some(row) {{\n\
         \x20       let body = \"<p>Delete {shown} ${{key}}? This cannot be undone.<form method=\\\"post\\\" action=\\\"${{html.escape(prefix)}}/${{key}}/delete\\\"><input type=\\\"hidden\\\" name=\\\"csrf\\\" value=\\\"${{html.escape(token)}}\\\"><button type=\\\"submit\\\">Delete</button></form><p><a href=\\\"${{html.escape(prefix)}}/${{key}}\\\">Keep it</a>\"\n\
         \x20       return __kiln_admin_page(site, \"Delete {shown} ${{key}}\", body, 200)\n\
         \x20   }}\n\
         \x20   return web.not_found()\n\
         }}\n\
         \nfn {delete}(req: web.Request, prefix: String, site: admin.Site, key: Int) -> web.Response ! {{\n\
         \x20   let who = try __kiln_admin_allow(site, req, \"delete\", {model})\n\
         \x20   let form = try req.form<{{ csrf: String }}>()\n\
         \x20   try site.sessions.check(req, form.csrf)\n\
         \x20   let found = try orm.get<{name}>(site.db, key)\n\
         \x20   if found is some(row) {{\n\
         \x20       orm.delete<{name}>(site.db, row) catch err {{\n\
         \x20           let why = validate.check([validate.problem(\"\", __kiln_admin_why(err))])\n\
         \x20           return try {form_fn}(req, prefix, site, key, {values}(row), why, 422)\n\
         \x20       }}\n\
         \x20       return web.redirect(prefix, status: 303)\n\
         \x20   }}\n\
         \x20   return web.not_found()\n\
         }}\n",
        edit = fn_name(name, "edit"),
        update = fn_name(name, "update"),
        asked = fn_name(name, "asked"),
        delete = fn_name(name, "delete"),
        model = quote(model.shown()),
        shown = model.shown(),
        read = fn_name(name, "read"),
        form = fn_name(name, "form"),
        form_fn = fn_name(name, "form"),
        values = fn_name(name, "values"),
        changed = changed.join(", "),
    ));
    out
}

/// `__kiln_admin_<Model>`: the routes, which `admin.pages<Model>` calls.
fn routes_source(model: &Model) -> String {
    let name = &model.name;
    let mut out = format!(
        "\nfn __kiln_admin_{name}(app: web.App, prefix: String, site: admin.Site) {{\n\
         \x20   app.get(prefix, fn(req) {{ return try {list}(req, prefix, site) }})\n",
        list = fn_name(name, "list"),
    );
    if model.creatable() {
        out.push_str(&format!(
            "    app.get(\"${{prefix}}/new\", fn(req) {{ return try {new}(req, prefix, site) }})\n\
             \x20   app.post(prefix, fn(req) {{ return try {create}(req, prefix, site) }})\n",
            new = fn_name(name, "new"),
            create = fn_name(name, "create"),
        ));
    }
    out.push_str(&format!(
        "    app.get(\"${{prefix}}/{{key: Int}}\", fn(req, key) {{ return try {edit}(req, prefix, site, key) }})\n\
         \x20   app.post(\"${{prefix}}/{{key: Int}}\", fn(req, key) {{ return try {update}(req, prefix, site, key) }})\n\
         \x20   app.get(\"${{prefix}}/{{key: Int}}/delete\", fn(req, key) {{ return try {asked}(req, prefix, site, key) }})\n\
         \x20   app.post(\"${{prefix}}/{{key: Int}}/delete\", fn(req, key) {{ return try {delete}(req, prefix, site, key) }})\n\
         }}\n",
        edit = fn_name(name, "edit"),
        update = fn_name(name, "update"),
        asked = fn_name(name, "asked"),
        delete = fn_name(name, "delete"),
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(src: &str) -> Model {
        let parsed = kiln_syntax::parse(src);
        declared_models(&parsed.module)
            .into_values()
            .next()
            .unwrap()
    }

    #[test]
    fn a_field_is_a_form_field_only_when_a_form_can_carry_it() {
        let m = model(
            "struct Post {\n    id: Int\n    title: String\n    summary: String? = none\n    \
             views: Int = 0\n    published: Bool = false\n    tags: List<String> = []\n}\n",
        );
        let names: Vec<&str> = m.editable().iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["title", "summary", "views", "published"]);
        assert_eq!(m.searched().len(), 1);
        assert!(m.creatable());
        assert!(m.key().is_some());
    }

    #[test]
    fn a_secret_is_never_a_form_field_or_a_column() {
        let m =
            model("struct Account {\n    id: Int\n    email: String\n    password: String\n}\n");
        let names: Vec<&str> = m.editable().iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["email"]);
        // A password with no default cannot be filled in from a form, so
        // the admin does not offer to create an account.
        assert!(!m.creatable());
    }

    #[test]
    fn only_the_models_the_program_asks_for_get_pages() {
        let src = "struct Post {\n    id: Int\n    title: String\n}\n\
                   struct Other {\n    id: Int\n}\n\
                   fn main() {\n    admin.pages<Post>(app, \"/admin/posts\", site)\n}\n";
        let parsed = kiln_syntax::parse(src);
        assert_eq!(wanted_models(&parsed.module), ["Post"]);
        let text = generate(&parsed.module).unwrap();
        assert!(text.contains("fn __kiln_admin_Post(app: web.App"));
        assert!(!text.contains("Other"));
        // Every page asks before it shows anything.
        assert_eq!(text.matches("__kiln_admin_allow(site, req").count(), 7);
    }

    #[test]
    fn a_program_without_admin_pages_generates_nothing() {
        let parsed = kiln_syntax::parse("fn main() {\n    print(\"hi\")\n}\n");
        assert!(generate(&parsed.module).is_none());
    }
}
