//! Models and typed queries: what a struct says about the table it is
//! stored in, and the SQL a query's Kiln conditions become.
//!
//! The checker runs this while it checks a query chain, so that a
//! misspelled field, a comparison of the wrong type or a condition the
//! compiler cannot turn into SQL is an error at `kiln check`, and records
//! the plan it builds for the lowering, which has nothing left to decide.

use crate::types::*;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast;
use std::collections::HashMap;

// ----- models -----

/// How a column is stored, which decides its SQL type and how values of
/// it are bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ColumnKind {
    Int,
    Float,
    Bool,
    Text,
    /// A duration, in nanoseconds.
    Duration,
    /// A time or a date, as ISO 8601 text.
    Time,
    /// Anything else with a JSON form, as JSON text.
    Json,
}

#[derive(Clone, Debug)]
pub struct Column {
    /// Its field's index in the struct.
    pub field: usize,
    pub name: String,
    pub ty: Ty,
    pub kind: ColumnKind,
    pub optional: bool,
    /// Whether the database fills it in, so inserts leave it out.
    pub generated: bool,
    /// Whether it is never written.
    pub readonly: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationKind {
    /// `author: User?` next to a column `author_id`.
    BelongsTo,
    /// `comments: List<Comment>`, the rows whose `post_id` is this key.
    HasMany,
}

#[derive(Clone, Debug)]
pub struct Relation {
    pub field: usize,
    pub name: String,
    pub target: AdtId,
    pub kind: RelationKind,
    /// The field of this model whose value is matched.
    pub key_field: usize,
    /// The column of the other table it is matched against.
    pub fk: String,
}

/// What a model struct says about its table.
#[derive(Clone, Debug)]
pub struct Model {
    pub adt: AdtId,
    pub name: String,
    pub table: String,
    /// The index in `columns` of the key column.
    pub key: usize,
    pub columns: Vec<Column>,
    pub relations: Vec<Relation>,
}

impl Model {
    pub fn key_column(&self) -> &Column {
        &self.columns[self.key]
    }

    pub fn column(&self, field: usize) -> Option<&Column> {
        self.columns.iter().find(|c| c.field == field)
    }

    /// `select posts.id, posts.title from posts`: every column, named, so
    /// a join cannot change what a row holds.
    pub fn select(&self) -> String {
        let cols: Vec<String> = self
            .columns
            .iter()
            .map(|c| format!("{}.{}", self.table, c.name))
            .collect();
        format!("select {} from {}", cols.join(", "), self.table)
    }
}

/// How a model's names were overridden, from `orm.model<T>(...)`.
#[derive(Clone, Debug, Default)]
pub struct Mapping {
    pub table: Option<String>,
    pub key: Option<String>,
    pub columns: HashMap<String, String>,
    pub generated: Vec<String>,
    pub readonly: Vec<String>,
    /// Where it was written, for diagnostics.
    pub span: Span,
}

/// `Post` to `posts`, `BlogPost` to `blog_posts`.
pub fn table_name(name: &str) -> String {
    plural(&snake_case(declared_name(name)))
}

/// The name as the program writes it. A type of a folder module is
/// renamed before it is checked, so that the files of a project share one
/// namespace; a table is named after what the code calls the struct.
pub fn declared_name(name: &str) -> &str {
    match name.rsplit_once("_symbol_") {
        Some((_, name)) => name,
        None => name,
    }
}

pub fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// `Post` is `Posts`, for a heading that names many of them.
pub fn plural(name: &str) -> String {
    let ends = |s: &str| name.ends_with(s);
    let vowel_y = ["ay", "ey", "iy", "oy", "uy"].iter().any(|v| ends(v));
    if ends("s") || ends("x") || ends("z") || ends("ch") || ends("sh") {
        format!("{name}es")
    } else if ends("y") && !vowel_y {
        format!("{}ies", &name[..name.len() - 1])
    } else {
        format!("{name}s")
    }
}

/// Names a table or column cannot have, because the compiler writes them
/// into SQL as they are.
const RESERVED: &[&str] = &[
    "all",
    "alter",
    "and",
    "as",
    "asc",
    "between",
    "by",
    "case",
    "check",
    "column",
    "constraint",
    "create",
    "default",
    "delete",
    "desc",
    "distinct",
    "drop",
    "else",
    "end",
    "exists",
    "foreign",
    "from",
    "group",
    "having",
    "in",
    "index",
    "inner",
    "insert",
    "into",
    "is",
    "join",
    "key",
    "left",
    "like",
    "limit",
    "not",
    "null",
    "offset",
    "on",
    "or",
    "order",
    "outer",
    "primary",
    "references",
    "right",
    "select",
    "set",
    "table",
    "then",
    "union",
    "unique",
    "update",
    "using",
    "values",
    "when",
    "where",
];

fn usable(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !RESERVED.contains(&name)
}

/// The column kind for a field type, or None when it has no SQL form.
fn column_kind(ty: &Ty, prims: &Prims) -> Option<ColumnKind> {
    match ty {
        Ty::Adt(id, _) if *id == prims.int => Some(ColumnKind::Int),
        Ty::Adt(id, _) if *id == prims.float => Some(ColumnKind::Float),
        Ty::Adt(id, _) if *id == prims.bool_ => Some(ColumnKind::Bool),
        Ty::Adt(id, _) if *id == prims.string => Some(ColumnKind::Text),
        Ty::Adt(id, _) if *id == prims.duration => Some(ColumnKind::Duration),
        Ty::Adt(id, _) if Some(id) == prims.instant.as_ref() || Some(id) == prims.date.as_ref() => {
            Some(ColumnKind::Time)
        }
        Ty::Fn(_) | Ty::Style(_) => None,
        Ty::Adt(..) | Ty::Record(_) => Some(ColumnKind::Json),
        _ => None,
    }
}

/// The ids of the types a model's fields may have.
#[derive(Clone, Debug)]
pub struct Prims {
    pub int: AdtId,
    pub float: AdtId,
    pub bool_: AdtId,
    pub string: AdtId,
    pub duration: AdtId,
    pub list: AdtId,
    pub instant: Option<AdtId>,
    pub date: Option<AdtId>,
}

/// Builds a model from a struct, reporting what makes it unusable.
pub fn model_of(
    adts: &[AdtDef],
    std_adts: usize,
    prims: &Prims,
    adt: AdtId,
    mapping: Option<&Mapping>,
    span: Span,
) -> Result<Model, Box<Diagnostic>> {
    let def = &adts[adt];
    let bad = |message: String| {
        Box::new(Diagnostic::error(codes::BAD_MODEL, message, span).with_help(
            "a model is a struct whose fields are its columns, with an `id: Int` key; map the exceptions with `orm.model<T>(...)`",
        ))
    };
    let AdtKind::Struct(fields) = &def.kind else {
        return Err(bad(format!(
            "`{}` is an enum, so it is not a model",
            def.name
        )));
    };
    if adt < std_adts {
        return Err(bad(format!(
            "`{}` is a standard library type, so it is not a model",
            def.name
        )));
    }
    let table = mapping
        .and_then(|m| m.table.clone())
        .unwrap_or_else(|| table_name(&def.name));
    if !usable(&table) {
        return Err(bad(format!(
            "`{}` would be stored in a table called `{table}`, which SQL cannot use as a name; give it one with `orm.model<{}>(table: \"...\")`",
            def.name, def.name
        )));
    }
    let key_name = mapping
        .and_then(|m| m.key.clone())
        .unwrap_or_else(|| "id".into());
    let mut columns = Vec::new();
    let mut relations = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        let (inner, optional) = match &f.ty {
            Ty::Opt(inner) => ((**inner).clone(), true),
            ty => (ty.clone(), false),
        };
        // A struct or a list of structs is a relation, not a column.
        if let Some(rel) = relation_of(adts, std_adts, prims, adt, &table, i, f, &inner, optional) {
            match rel {
                Ok(r) => relations.push(r),
                Err(message) => return Err(bad(message)),
            }
            continue;
        }
        let name = mapping
            .and_then(|m| m.columns.get(&f.name).cloned())
            .unwrap_or_else(|| f.name.clone());
        if !usable(&name) {
            return Err(bad(format!(
                "the field `{}` would be the column `{name}`, which SQL cannot use as a name; give it one with `orm.model<{}>(columns: {{ {}: \"...\" }})`",
                f.name, def.name, f.name
            )));
        }
        let Some(kind) = column_kind(&inner, prims) else {
            return Err(bad(format!(
                "the field `{}` of `{}` cannot be stored in a column",
                f.name, def.name
            )));
        };
        let generated = mapping.is_some_and(|m| m.generated.contains(&f.name))
            || (f.name == key_name && kind == ColumnKind::Int);
        columns.push(Column {
            field: i,
            name,
            ty: f.ty.clone(),
            kind,
            optional,
            generated,
            readonly: mapping.is_some_and(|m| m.readonly.contains(&f.name)),
        });
    }
    let Some(key) = columns.iter().position(|c| {
        fields
            .get(c.field)
            .is_some_and(|f: &FieldDef| f.name == key_name)
    }) else {
        return Err(bad(format!(
            "`{}` has no field `{key_name}` to be its key; add one, or name it with `orm.model<{}>(key: .field)`",
            def.name, def.name
        )));
    };
    if columns[key].optional {
        return Err(bad(format!(
            "the key `{}` of `{}` is optional, and a key is never missing",
            columns[key].name, def.name
        )));
    }
    Ok(Model {
        adt,
        name: declared_name(&def.name).to_string(),
        table,
        key,
        columns,
        relations,
    })
}

#[allow(clippy::too_many_arguments)]
fn relation_of(
    adts: &[AdtDef],
    std_adts: usize,
    prims: &Prims,
    owner: AdtId,
    table: &str,
    field: usize,
    f: &FieldDef,
    inner: &Ty,
    optional: bool,
) -> Option<Result<Relation, String>> {
    let is_model = |id: AdtId| id >= std_adts && matches!(adts[id].kind, AdtKind::Struct(_));
    let AdtKind::Struct(fields) = &adts[owner].kind else {
        return None;
    };
    let (target, kind) = match inner {
        Ty::Adt(id, args) if *id == prims.list => match &args[0] {
            Ty::Adt(target, _) if is_model(*target) => (*target, RelationKind::HasMany),
            _ => return None,
        },
        Ty::Adt(id, _) if is_model(*id) => (*id, RelationKind::BelongsTo),
        _ => return None,
    };
    let needed = match kind {
        RelationKind::BelongsTo => format!("{}_id", f.name),
        RelationKind::HasMany => format!("{}_id", snake_case(declared_name(&adts[owner].name))),
    };
    match kind {
        RelationKind::BelongsTo => {
            if !optional {
                return Some(Err(format!(
                    "the relation `{}` of `{}` must be optional (`{}?`), because it is loaded only when a query asks for it with `include`",
                    f.name, adts[owner].name, adts[target].name
                )));
            }
            let Some(key_field) = fields.iter().position(|o| o.name == needed) else {
                return Some(Err(format!(
                    "the relation `{}` of `{}` needs a field `{needed}` holding the key of the `{}` it belongs to",
                    f.name, adts[owner].name, adts[target].name
                )));
            };
            Some(Ok(Relation {
                field,
                name: f.name.clone(),
                target,
                kind,
                key_field,
                fk: "id".into(),
            }))
        }
        RelationKind::HasMany => {
            let AdtKind::Struct(their) = &adts[target].kind else {
                return None;
            };
            if !their.iter().any(|o| o.name == needed) {
                return Some(Err(format!(
                    "the relation `{}` of `{}` needs a field `{needed}` on `{}`, holding the key of the {} row",
                    f.name, adts[owner].name, adts[target].name, table
                )));
            }
            let Some(key_field) = fields.iter().position(|o| o.name == "id") else {
                return Some(Err(format!(
                    "the relation `{}` of `{}` needs its `id` field",
                    f.name, adts[owner].name
                )));
            };
            Some(Ok(Relation {
                field,
                name: f.name.clone(),
                target,
                kind,
                key_field,
                fk: needed,
            }))
        }
    }
}

// ----- queries -----

/// A value a query sends to the database.
#[derive(Clone, Debug)]
pub enum Param {
    /// The value of a Kiln expression.
    Value(ast::Expr),
    /// A `like` pattern made from a Kiln expression at run time.
    Like(ast::Expr, LikeMode),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LikeMode {
    Contains = 0,
    StartsWith = 1,
    EndsWith = 2,
}

/// A query, as its chain of calls builds it up.
#[derive(Clone, Debug)]
pub struct QueryPlan {
    pub model: Model,
    /// The `sql.Db` the query runs on.
    pub db: ast::Expr,
    /// Conditions, joined by `and`.
    pub conds: Vec<String>,
    pub params: Vec<Param>,
    /// `order by` keys, such as `posts.id desc`.
    pub order: Vec<String>,
    pub limit: Option<ast::Expr>,
    pub offset: Option<ast::Expr>,
    /// The relations to load, by their index in the model.
    pub includes: Vec<usize>,
    /// Whether only the first row is wanted, as `first` and `get` want.
    pub one: bool,
}

impl QueryPlan {
    pub fn plan_model(&self) -> Model {
        self.model.clone()
    }

    pub fn new(model: Model, db: ast::Expr) -> Self {
        QueryPlan {
            model,
            db,
            conds: Vec::new(),
            params: Vec::new(),
            order: Vec::new(),
            limit: None,
            offset: None,
            includes: Vec::new(),
            one: false,
        }
    }

    fn tail(&self, params: &mut Vec<Param>) -> String {
        let mut sql = String::new();
        if !self.conds.is_empty() {
            sql.push_str(&format!(" where {}", self.conds.join(" and ")));
        }
        if !self.order.is_empty() {
            sql.push_str(&format!(" order by {}", self.order.join(", ")));
        }
        if let Some(limit) = &self.limit {
            params.push(Param::Value(limit.clone()));
            sql.push_str(&format!(" limit ${}", params.len()));
        } else if self.one {
            sql.push_str(" limit 1");
        }
        if let Some(offset) = &self.offset {
            // Some databases need a limit before an offset.
            if self.limit.is_none() {
                sql.push_str(" limit -1");
            }
            params.push(Param::Value(offset.clone()));
            sql.push_str(&format!(" offset ${}", params.len()));
        }
        sql
    }

    /// The SQL of `all` and `first`, with the values it binds in order.
    pub fn rows_sql(&self) -> (String, Vec<Param>) {
        let mut params = self.params.clone();
        let sql = format!("{}{}", self.model.select(), self.tail(&mut params));
        (sql, params)
    }

    /// The SQL of `count`.
    pub fn count_sql(&self) -> (String, Vec<Param>) {
        let mut params = self.params.clone();
        let (order, limit, offset) = (self.order.clone(), self.limit.clone(), self.offset.clone());
        let bare = QueryPlan {
            order: Vec::new(),
            ..self.clone()
        };
        let sql = format!(
            "select count(*) from {}{}",
            self.model.table,
            bare.tail(&mut params)
        );
        let _ = (order, limit, offset);
        (sql, params)
    }

    /// `insert into posts (title, body) values ($1, $2)`, with the
    /// fields whose values it binds, in order.
    pub fn insert_sql(&self) -> (String, Vec<usize>) {
        let m = &self.model;
        let cols: Vec<&Column> = m
            .columns
            .iter()
            .filter(|c| !c.generated && !c.readonly)
            .collect();
        let names: Vec<&str> = cols.iter().map(|c| c.name.as_str()).collect();
        let marks: Vec<String> = (1..=cols.len()).map(|i| format!("${i}")).collect();
        let sql = if cols.is_empty() {
            format!("insert into {} default values", m.table)
        } else {
            format!(
                "insert into {} ({}) values ({})",
                m.table,
                names.join(", "),
                marks.join(", ")
            )
        };
        (sql, cols.iter().map(|c| c.field).collect())
    }

    /// `update posts set title = $1 where id = $2`, with the fields whose
    /// values it binds, the key last.
    pub fn update_sql(&self) -> (String, Vec<usize>) {
        let m = &self.model;
        let key = m.key_column();
        let cols: Vec<&Column> = m
            .columns
            .iter()
            .filter(|c| c.field != key.field && !c.readonly)
            .collect();
        let sets: Vec<String> = cols
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{} = ${}", c.name, i + 1))
            .collect();
        let sql = format!(
            "update {} set {} where {} = ${}",
            m.table,
            sets.join(", "),
            key.name,
            cols.len() + 1
        );
        let mut fields: Vec<usize> = cols.iter().map(|c| c.field).collect();
        fields.push(key.field);
        (sql, fields)
    }

    /// `delete from posts where id = $1`, binding the key.
    pub fn delete_row_sql(&self) -> (String, Vec<usize>) {
        let m = &self.model;
        let key = m.key_column();
        (
            format!("delete from {} where {} = $1", m.table, key.name),
            vec![key.field],
        )
    }

    /// For each relation to include, the SQL that loads it and the fields
    /// that link the two models.
    pub fn include_steps(&self, models: &dyn Fn(AdtId) -> Option<Model>) -> Vec<IncludeStep> {
        let mut steps = Vec::new();
        for &i in &self.includes {
            let r = &self.model.relations[i];
            let Some(child) = models(r.target) else {
                continue;
            };
            let Some(fk) = child.columns.iter().find(|c| c.name == r.fk) else {
                continue;
            };
            steps.push(IncludeStep {
                field: r.field,
                sql: format!("{} where {}.{} in ", child.select(), child.table, r.fk),
                key_field: r.key_field,
                fk_field: fk.field,
                list: r.kind == RelationKind::HasMany,
            });
        }
        steps
    }

    /// The SQL of `delete`.
    pub fn delete_sql(&self) -> (String, Vec<Param>) {
        let mut params = self.params.clone();
        let m = &self.model;
        let mut sql = format!("delete from {}", m.table);
        if !self.conds.is_empty() || self.limit.is_some() || self.offset.is_some() {
            // The conditions name the table, so they are written as a
            // subquery on the keys, which every dialect allows.
            let inner = QueryPlan {
                model: m.clone(),
                ..self.clone()
            };
            let key = &m.key_column().name;
            let tail = inner.tail(&mut params);
            sql.push_str(&format!(
                " where {key} in (select {}.{key} from {}{tail})",
                m.table, m.table
            ));
        }
        (sql, params)
    }
}

/// One relation a query loads after its rows.
#[derive(Clone, Debug)]
pub struct IncludeStep {
    /// The field of the model that holds it.
    pub field: usize,
    /// `select comments.* from comments where comments.post_id in `, to
    /// which the runtime adds the keys it loads for.
    pub sql: String,
    /// The field of the model whose value is matched,
    pub key_field: usize,
    /// and the field of the other model it is matched against.
    pub fk_field: usize,
    /// Whether the field is a list.
    pub list: bool,
}

/// Where a query's conditions are compiled, so the message can say which
/// call cannot be built.
pub struct Compiler<'a> {
    pub adts: &'a [AdtDef],
    pub prims: &'a Prims,
    /// The type of every expression checked so far, by span.
    pub types: &'a HashMap<Span, Ty>,
}

/// Why a query could not be compiled: a mistake of its own, or one the
/// type checker has already reported, such as a misspelled field.
#[derive(Debug)]
pub enum QueryError {
    Reported,
    At(Box<Diagnostic>),
}

impl From<Diagnostic> for QueryError {
    fn from(d: Diagnostic) -> Self {
        QueryError::At(Box::new(d))
    }
}

type Compiled = Result<String, QueryError>;

impl Compiler<'_> {
    /// Compiles `test`, the argument of `where`, into SQL.
    pub fn condition(&self, plan: &mut QueryPlan, test: &ast::Expr) -> Compiled {
        match &test.kind {
            // `.published`, a field used as the whole condition.
            ast::ExprKind::FieldShorthand(name) => {
                let col = self.column(plan, &name.name, name.span)?;
                Ok(truth(&col))
            }
            ast::ExprKind::Closure { params, body, .. } => {
                let Some(binding) = params.first().map(|p| p.name.name.clone()) else {
                    return Err(self.cannot(test.span, "a condition with no parameter"));
                };
                let value = body_value(body).ok_or_else(|| {
                    self.cannot(body.span, "a condition that is more than one expression")
                })?;
                let sql = self.expr(plan, &binding, value)?;
                Ok(match sql.strip_prefix("__col:") {
                    Some(col) => truth(col),
                    None => sql,
                })
            }
            _ => Err(self.cannot(test.span, "this condition")),
        }
    }

    /// Compiles the argument of `order_by` or `include`: a field of the
    /// model, written `.title` or `fn(p) { p.title }`.
    pub fn field_of(
        &self,
        plan: &QueryPlan,
        key: &ast::Expr,
    ) -> Result<(String, Span), QueryError> {
        match &key.kind {
            ast::ExprKind::FieldShorthand(name) => Ok((name.name.clone(), name.span)),
            ast::ExprKind::Closure { params, body, .. } => {
                let binding = params.first().map(|p| p.name.name.clone());
                match body_value(body).map(|v| &v.kind) {
                    Some(ast::ExprKind::Field { expr, name }) if matches!(&expr.kind, ast::ExprKind::Ident(i) if Some(i) == binding.as_ref()) => {
                        Ok((name.name.clone(), name.span))
                    }
                    _ => Err(self.cannot(key.span, "this field")),
                }
            }
            _ => {
                let _ = plan;
                Err(self.cannot(key.span, "this field"))
            }
        }
    }

    fn column(&self, plan: &QueryPlan, name: &str, span: Span) -> Compiled {
        let m = &plan.model;
        match m.columns.iter().find(|c| self.field_name(m, c) == name) {
            Some(c) => Ok(format!("{}.{}", m.table, c.name)),
            // A name that is no field at all is the type checker's to
            // report, so that one mistake is one error.
            None if !self.is_field(m, name) => Err(QueryError::Reported),
            None => {
                let mut diag = Diagnostic::error(
                    codes::BAD_QUERY,
                    format!("`{}` has no column `{name}` to query on", m.name),
                    span,
                );
                let names: Vec<String> = m.columns.iter().map(|c| self.field_name(m, c)).collect();
                if let Some(s) = similar(name, names.iter().map(String::as_str)) {
                    diag = diag.with_suggestion(format!("did you mean `{s}`?"), span, s);
                }
                Err(diag.into())
            }
        }
    }

    /// Whether the model's struct has a field of this name at all.
    pub fn is_field(&self, m: &Model, name: &str) -> bool {
        match &self.adts[m.adt].kind {
            AdtKind::Struct(fields) => fields.iter().any(|f| f.name == name),
            _ => false,
        }
    }

    fn field_name(&self, m: &Model, c: &Column) -> String {
        match &self.adts[m.adt].kind {
            AdtKind::Struct(fields) => fields[c.field].name.clone(),
            _ => c.name.clone(),
        }
    }

    fn cannot(&self, span: Span, what: &str) -> QueryError {
        Diagnostic::error(
            codes::BAD_QUERY,
            format!("the compiler cannot turn {what} into SQL"),
            span,
        )
        .with_help(
            "queries compare the model's fields with values; work out anything else before the query, or write the SQL with `db.query`",
        )
        .into()
    }

    /// Compiles an expression inside a condition. A column alone comes
    /// back as `__col:<sql>`, so that `p.published` on its own becomes
    /// `= 1` and not a bare column.
    fn expr(&self, plan: &mut QueryPlan, binding: &str, e: &ast::Expr) -> Compiled {
        use ast::ExprKind as E;
        match &e.kind {
            E::Field { expr, name } if self.is_binding(expr, binding) => {
                let col = self.column(plan, &name.name, name.span)?;
                Ok(format!("__col:{col}"))
            }
            E::Binary { op, lhs, rhs } => self.binary(plan, binding, *op, lhs, rhs, e.span),
            E::Unary { op, expr } => match op {
                ast::UnaryOp::Not => {
                    let inner = self.value(plan, binding, expr)?;
                    Ok(format!("not ({inner})"))
                }
                ast::UnaryOp::Neg => {
                    let inner = self.value(plan, binding, expr)?;
                    Ok(format!("-({inner})"))
                }
            },
            E::Is { expr, pattern } => {
                let value = self.value(plan, binding, expr)?;
                match &pattern.kind {
                    ast::PatternKind::None => Ok(format!("{value} is null")),
                    _ => Err(self.cannot(e.span, "this pattern")),
                }
            }
            E::Call { callee, args, .. } => self.call(plan, binding, callee, args, e.span),
            E::Block(b) => match body_value(b) {
                Some(v) => self.expr(plan, binding, v),
                None => Err(self.cannot(e.span, "this condition")),
            },
            _ if !self.mentions(e, binding) => Ok(self.param(plan, Param::Value(e.clone()))),
            _ => Err(self.cannot(e.span, "this part of the condition")),
        }
    }

    /// Like [`Compiler::expr`], with a lone column read as its value.
    fn value(&self, plan: &mut QueryPlan, binding: &str, e: &ast::Expr) -> Compiled {
        let sql = self.expr(plan, binding, e)?;
        Ok(sql.strip_prefix("__col:").unwrap_or(&sql).to_string())
    }

    fn param(&self, plan: &mut QueryPlan, p: Param) -> String {
        plan.params.push(p);
        format!("${}", plan.params.len())
    }

    #[allow(clippy::too_many_arguments)]
    fn binary(
        &self,
        plan: &mut QueryPlan,
        binding: &str,
        op: ast::BinaryOp,
        lhs: &ast::Expr,
        rhs: &ast::Expr,
        span: Span,
    ) -> Compiled {
        use ast::BinaryOp as O;
        let null = |e: &ast::Expr| matches!(e.kind, ast::ExprKind::None);
        if matches!(op, O::Eq | O::Ne) && (null(lhs) || null(rhs)) {
            let other = if null(lhs) { rhs } else { lhs };
            let value = self.value(plan, binding, other)?;
            let not = if op == O::Ne { " not" } else { "" };
            return Ok(format!("{value} is{not} null"));
        }
        let text = match op {
            O::Or => "or",
            O::And => "and",
            O::Eq => "=",
            O::Ne => "<>",
            O::Lt => "<",
            O::Le => "<=",
            O::Gt => ">",
            O::Ge => ">=",
            O::Add => "+",
            O::Sub => "-",
            O::Mul => "*",
            O::Div => "/",
            O::Rem => "%",
            O::Coalesce => {
                let (a, b) = (
                    self.value(plan, binding, lhs)?,
                    self.value(plan, binding, rhs)?,
                );
                return Ok(format!("coalesce({a}, {b})"));
            }
            O::Range => return Err(self.cannot(span, "a range")),
        };
        let a = self.value(plan, binding, lhs)?;
        let b = self.value(plan, binding, rhs)?;
        Ok(format!("({a} {text} {b})"))
    }

    fn call(
        &self,
        plan: &mut QueryPlan,
        binding: &str,
        callee: &ast::Expr,
        args: &[ast::Arg],
        span: Span,
    ) -> Compiled {
        let ast::ExprKind::Field { expr: recv, name } = &callee.kind else {
            return Err(self.cannot(span, "this call"));
        };
        // A method on a column: text searches, and `len`.
        let on_model = self.mentions(recv, binding);
        if !on_model {
            if args.iter().any(|a| self.mentions(&a.value, binding)) {
                return Err(self.cannot(span, "this call"));
            }
            let whole = ast::Expr {
                kind: ast::ExprKind::Call {
                    callee: Box::new(callee.clone()),
                    type_args: Vec::new(),
                    args: args.to_vec(),
                },
                span,
            };
            return Ok(self.param(plan, Param::Value(whole)));
        }
        let col = self.value(plan, binding, recv)?;
        let mode = match name.name.as_str() {
            "contains" => LikeMode::Contains,
            "starts_with" => LikeMode::StartsWith,
            "ends_with" => LikeMode::EndsWith,
            "len" if args.is_empty() => return Ok(format!("length({col})")),
            "lower" if args.is_empty() => return Ok(format!("lower({col})")),
            "upper" if args.is_empty() => return Ok(format!("upper({col})")),
            _ => return Err(self.cannot(span, format!("`.{}` in a query", name.name).as_str())),
        };
        let Some(arg) = args.first() else {
            return Err(self.cannot(span, "this call"));
        };
        if args.len() != 1 || self.mentions(&arg.value, binding) {
            return Err(self.cannot(span, "this call"));
        }
        let ty = self.types.get(&recv.span);
        if !matches!(ty, Some(Ty::Adt(id, _)) if *id == self.prims.string) {
            return Err(self.cannot(span, "a text search on a column that is not text"));
        }
        let p = self.param(plan, Param::Like(arg.value.clone(), mode));
        // `!` rather than a backslash, which MySQL reads as an escape
        // inside the string literal itself.
        Ok(format!("{col} like {p} escape '!'"))
    }

    fn is_binding(&self, e: &ast::Expr, binding: &str) -> bool {
        matches!(&e.kind, ast::ExprKind::Ident(name) if name == binding)
    }

    /// Whether the expression reads the condition's parameter, which is
    /// what decides between SQL and a bound value.
    fn mentions(&self, e: &ast::Expr, binding: &str) -> bool {
        let mut found = false;
        walk(e, &mut |e| {
            if let ast::ExprKind::Ident(name) = &e.kind {
                if name == binding {
                    found = true;
                }
            }
        });
        found
    }
}

/// `col = 1`: the SQL for a boolean column used as a condition. Databases
/// store booleans as 1 and 0, or as true and false, and both compare
/// equal to 1 everywhere Kiln supports.
fn truth(col: &str) -> String {
    format!("{col} = 1")
}

/// The value a block produces: its last statement, when that is an
/// expression or a `return`.
fn body_value(b: &ast::Block) -> Option<&ast::Expr> {
    match b.stmts.last() {
        Some(ast::Stmt::Expr(e)) if b.stmts.len() == 1 => Some(e),
        Some(ast::Stmt::Return { value, .. }) if b.stmts.len() == 1 => value.as_ref(),
        _ => None,
    }
}

/// Calls `f` on an expression and every expression inside it.
pub fn walk(e: &ast::Expr, f: &mut impl FnMut(&ast::Expr)) {
    use ast::ExprKind as E;
    f(e);
    match &e.kind {
        E::Unary { expr, .. } => walk(expr, f),
        E::Binary { lhs, rhs, .. } => {
            walk(lhs, f);
            walk(rhs, f);
        }
        E::Call { callee, args, .. } => {
            walk(callee, f);
            for a in args {
                walk(&a.value, f);
            }
        }
        E::Field { expr, .. } | E::Try(expr) | E::Throw(expr) | E::Is { expr, .. } => walk(expr, f),
        E::Index { expr, index } => {
            walk(expr, f);
            walk(index, f);
        }
        E::List(items) => items.iter().for_each(|i| walk(i, f)),
        E::Record(fields) => fields
            .iter()
            .filter_map(|r| r.value.as_ref())
            .for_each(|v| walk(v, f)),
        E::Str(pieces) => pieces.iter().for_each(|p| {
            if let ast::StrPiece::Expr(e) = p {
                walk(e, f)
            }
        }),
        E::Closure { body, .. } | E::Block(body) => walk_block(body, f),
        E::If {
            cond,
            then,
            otherwise,
        } => {
            walk(cond, f);
            walk_block(then, f);
            if let Some(o) = otherwise {
                walk(o, f);
            }
        }
        E::Match { scrutinee, arms } => {
            walk(scrutinee, f);
            for arm in arms {
                walk(&arm.body, f);
                if let Some(g) = &arm.guard {
                    walk(g, f);
                }
            }
        }
        E::Catch { expr, handler, .. } => {
            walk(expr, f);
            walk_block(handler, f);
        }
        _ => {}
    }
}

pub fn walk_block(b: &ast::Block, f: &mut impl FnMut(&ast::Expr)) {
    for stmt in &b.stmts {
        match stmt {
            ast::Stmt::Let { value, .. } | ast::Stmt::Expr(value) => walk(value, f),
            ast::Stmt::Assign { target, value, .. } => {
                walk(target, f);
                walk(value, f);
            }
            ast::Stmt::Return { value, .. } => {
                if let Some(v) = value {
                    walk(v, f)
                }
            }
            ast::Stmt::For {
                iter, filter, body, ..
            } => {
                walk(iter, f);
                if let Some(c) = filter {
                    walk(c, f)
                }
                walk_block(body, f);
            }
            ast::Stmt::While { cond, body, .. } => {
                walk(cond, f);
                walk_block(body, f);
            }
            ast::Stmt::Break(_) | ast::Stmt::Continue(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_names_follow_the_struct() {
        assert_eq!(table_name("Post"), "posts");
        assert_eq!(table_name("BlogPost"), "blog_posts");
        assert_eq!(table_name("Category"), "categories");
        assert_eq!(table_name("Address"), "addresses");
        assert_eq!(table_name("Day"), "days");
    }

    #[test]
    fn sql_names_are_checked() {
        assert!(usable("post_title"));
        assert!(!usable("order"));
        assert!(!usable("Title"));
        assert!(!usable("a-b"));
    }
}
