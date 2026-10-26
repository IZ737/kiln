//! The checker's side of `orm`: it recognizes a query chain as it checks
//! it, builds the model and the SQL with [`crate::orm`], and records the
//! plan for the lowering.

use crate::checker::{Checker, Scope};
use crate::orm::{self, Model, Param, QueryError, QueryPlan};
use crate::types::*;
use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_syntax::ast::{self, Expr, ExprKind};

/// The methods of `orm.Query` that run it.
pub(crate) const TERMINALS: &[&str] = &["all", "first", "count", "delete", "sql"];

impl Checker {
    pub(crate) fn orm_prims(&self) -> orm::Prims {
        let module_type = |module: &str, name: &str| {
            self.modules
                .iter()
                .find(|m| m.name == module)
                .and_then(|m| m.types.get(name))
                .copied()
        };
        orm::Prims {
            int: self.global_types["Int"],
            float: self.global_types["Float"],
            bool_: self.global_types["Bool"],
            string: self.global_types["String"],
            duration: self.global_types["Duration"],
            list: self.global_types["List"],
            instant: module_type("time", "Instant"),
            date: self.global_types.get("Date").copied(),
        }
    }

    /// The id of a type of the `orm` module.
    fn orm_type(&self, name: &str) -> Option<AdtId> {
        self.modules
            .iter()
            .find(|m| m.name == "orm")
            .and_then(|m| m.types.get(name))
            .copied()
    }

    pub(crate) fn is_query_ty(&self, ty: &Ty) -> bool {
        matches!(ty, Ty::Adt(id, _) if Some(*id) == self.orm_type("Query"))
    }

    /// The model of a struct, reported and remembered once.
    pub(crate) fn model(&mut self, adt: AdtId, span: Span) -> Option<Model> {
        if let Some(model) = self.models.get(&adt) {
            return model.clone();
        }
        let prims = self.orm_prims();
        let mapping = self.mappings.get(&adt).cloned();
        let built = orm::model_of(
            &self.adts,
            self.std_adts,
            &prims,
            adt,
            mapping.as_ref(),
            span,
        );
        let model = match built {
            Ok(model) => Some(model),
            Err(diag) => {
                self.error(*diag);
                None
            }
        };
        self.models.insert(adt, model.clone());
        if model.is_some() && !self.model_order.contains(&adt) {
            self.model_order.push(adt);
        }
        model
    }

    /// The model a call's type argument names.
    fn model_arg(&mut self, explicit: &[Ty], span: Span) -> Option<Model> {
        let ty = match explicit.first() {
            Some(t) => t.clone(),
            None => self
                .instantiations
                .get(&span)
                .and_then(|m| m.get("T"))
                .cloned()?,
        };
        match self.zonk(&ty) {
            Ty::Adt(adt, _) => self.model(adt, span),
            other => {
                let shown = self.show(&other);
                self.error(
                    Diagnostic::error(codes::BAD_MODEL, format!("`{shown}` is not a model"), span)
                        .with_help("write the model out, as in `orm.find<Post>(db)`"),
                );
                None
            }
        }
    }

    /// `admin.pages<Post>(app, prefix, site)`: the model must be one the
    /// admin can make pages for, and the pages themselves are written by
    /// the compiler (see `kiln_ir::admin`).
    pub(crate) fn admin_pages(&mut self, explicit: &[Ty], span: Span) {
        let Some(model) = self.model_arg(explicit, span) else {
            return;
        };
        let key = model.key_column();
        if !self.is_prim(&key.ty, "Int") || key.optional {
            let shown = self.show(&key.ty);
            self.error(
                Diagnostic::error(
                    codes::BAD_MODEL,
                    format!(
                        "the admin's pages need a model whose key is an `Int`, and `{}.{}` is `{shown}`",
                        model.name, key.name
                    ),
                    span,
                )
                .with_help("write the pages for this model yourself, with `orm` and a glaze"),
            );
            return;
        }
        // The generator names its functions after the struct as the
        // checker knows it, which a folder module has renamed.
        let declared = match explicit.first() {
            Some(Ty::Adt(id, _)) => self.adts[*id].name.clone(),
            _ => model.name.clone(),
        };
        self.admins.insert(span, declared);
    }

    /// `orm.find<Post>(db)` and the other functions of the module, after
    /// their arguments have been checked. Returns true when the call was
    /// an `orm` function that needs a model.
    pub(crate) fn orm_call(&mut self, name: &str, args: &[ast::Arg], explicit: &[Ty], span: Span) {
        if name == "model" {
            return;
        }
        let Some(model) = self.model_arg(explicit, span) else {
            self.bad_queries.insert(span);
            return;
        };
        let Some(db) = args.first().map(|a| a.value.clone()) else {
            return;
        };
        let mut plan = QueryPlan::new(model, db);
        if name == "find" {
            self.queries.insert(span, plan);
            return;
        }
        let value = args.get(1).map(|a| a.value.clone());
        if name == "get" {
            let m = &plan.plan_model();
            plan.conds
                .push(format!("{}.{} = $1", m.table, m.key_column().name));
            if let Some(key) = value.clone() {
                plan.params.push(Param::Value(key));
            }
        }
        if matches!(name, "get" | "insert" | "update" | "delete") {
            self.query_runs.insert(
                span,
                Query {
                    plan,
                    terminal: name.to_string(),
                    value,
                },
            );
        }
    }

    /// A method of `orm.Query`, after its arguments have been checked.
    pub(crate) fn orm_step(&mut self, recv: &Expr, name: &str, args: &[ast::Arg], span: Span) {
        if self.bad_queries.contains(&recv.span) {
            self.bad_queries.insert(span);
            return;
        }
        let Some(mut plan) = self.queries.get(&recv.span).cloned() else {
            self.bad_queries.insert(span);
            self.error(
                Diagnostic::error(
                    codes::BAD_QUERY,
                    format!("`{name}` must be called on a query that is built here"),
                    recv.span,
                )
                .with_help(
                    "build and run a query in one chain, as in `orm.find<Post>(db).where(.published).all()`",
                ),
            );
            return;
        };
        // `descending:` decides the SQL, so it is read before the field.
        let mut down = false;
        if name == "order_by" {
            let descending = args
                .iter()
                .find(|a| a.name.as_ref().is_some_and(|n| n.name == "descending"))
                .map(|a| &a.value)
                .or_else(|| args.get(1).map(|a| &a.value));
            match descending.map(|d| &d.kind) {
                None => {}
                Some(ExprKind::Bool(b)) => down = *b,
                Some(_) => self.bad_order(
                    Diagnostic::error(
                        codes::BAD_QUERY,
                        "`descending` must be written as `true` or `false` here, because the order is part of the SQL",
                        descending.map_or(span, |d| d.span),
                    )
                    .with_help("to sort either way, write the two queries"),
                ),
            }
        }
        let mut failed = None;
        let mut field = None;
        {
            let prims = self.orm_prims();
            let compiler = orm::Compiler {
                adts: &self.adts,
                prims: &prims,
                types: &self.expr_types,
            };
            match name {
                "where" => match args.first() {
                    Some(a) => match compiler.condition(&mut plan, &a.value) {
                        Ok(sql) => plan.conds.push(sql),
                        Err(diag) => failed = Some(diag),
                    },
                    None => return,
                },
                "order_by" | "include" => {
                    let Some(a) = args.first() else { return };
                    match compiler.field_of(&plan, &a.value) {
                        Ok(f) => field = Some(f),
                        Err(diag) => failed = Some(diag),
                    }
                }
                "limit" | "offset" => {
                    let Some(a) = args.first() else { return };
                    if name == "limit" {
                        plan.limit = Some(a.value.clone());
                    } else {
                        plan.offset = Some(a.value.clone());
                    }
                }
                _ if TERMINALS.contains(&name) => {}
                _ => return,
            }
        }
        if let Some((field, fspan)) = field {
            match name {
                "order_by" => match self.query_column(&plan, &field, fspan) {
                    Some(col) => {
                        let dir = if down { " desc" } else { "" };
                        plan.order.push(format!("{col}{dir}"));
                    }
                    None => {
                        self.bad_queries.insert(span);
                        return;
                    }
                },
                _ => match self.relation_index(&plan, &field, fspan) {
                    Some(i) => {
                        if !plan.includes.contains(&i) {
                            plan.includes.push(i);
                        }
                        let target = plan.model.relations[i].target;
                        self.model(target, fspan);
                    }
                    None => {
                        self.bad_queries.insert(span);
                        return;
                    }
                },
            }
        }
        if let Some(why) = failed {
            if let QueryError::At(diag) = why {
                self.error(*diag);
            }
            self.bad_queries.insert(span);
            return;
        }
        if TERMINALS.contains(&name) {
            self.query_runs.insert(
                span,
                Query {
                    plan: plan.clone(),
                    terminal: name.to_string(),
                    value: None,
                },
            );
        }
        self.queries.insert(span, plan);
    }

    fn bad_order(&mut self, diag: Diagnostic) {
        self.error(diag);
    }

    /// `posts.title` for a field of the model, reported when there is no
    /// such column.
    fn query_column(&mut self, plan: &QueryPlan, field: &str, span: Span) -> Option<String> {
        let m = &plan.model;
        let names = self.field_names(m);
        match names
            .iter()
            .position(|n| n == field)
            .and_then(|i| m.columns.iter().find(|c| c.field == i))
        {
            Some(col) => Some(format!("{}.{}", m.table, col.name)),
            None if !names.iter().any(|n| n == field) => None,
            None => {
                let mut diag = Diagnostic::error(
                    codes::BAD_QUERY,
                    format!("`{}` has no column `{field}` to sort by", m.name),
                    span,
                );
                let candidates: Vec<&str> = m
                    .columns
                    .iter()
                    .filter_map(|c| names.get(c.field).map(String::as_str))
                    .collect();
                if let Some(s) = similar(field, candidates) {
                    diag = diag.with_suggestion(format!("did you mean `{s}`?"), span, s);
                }
                self.error(diag);
                None
            }
        }
    }

    fn relation_index(&mut self, plan: &QueryPlan, field: &str, span: Span) -> Option<usize> {
        let m = &plan.model;
        let fields = self.field_names(m);
        match m.relations.iter().position(|r| r.name == field) {
            Some(i) => Some(i),
            None if !fields.iter().any(|n| n == field) => None,
            None => {
                let mut diag = Diagnostic::error(
                    codes::BAD_QUERY,
                    format!("`{}` has no relation `{field}` to include", m.name),
                    span,
                );
                let names: Vec<&str> = m.relations.iter().map(|r| r.name.as_str()).collect();
                if names.is_empty() {
                    diag = diag.with_help(format!(
                        "a relation is a field of `{}` holding another model, or a list of one",
                        m.name
                    ));
                } else if let Some(s) = similar(field, names) {
                    diag = diag.with_suggestion(format!("did you mean `{s}`?"), span, s);
                }
                self.error(diag);
                None
            }
        }
    }

    fn field_names(&self, m: &Model) -> Vec<String> {
        match &self.adts[m.adt].kind {
            AdtKind::Struct(fields) => fields.iter().map(|f| f.name.clone()).collect(),
            _ => Vec::new(),
        }
    }

    /// Reads the mappings of `orm.model<T>(...)` constants, which apply
    /// wherever the model is used, before any query is checked.
    pub(crate) fn collect_mappings(&mut self, module: &ast::Module) {
        for item in &module.items {
            let ast::Item::Const(c) = item else { continue };
            let ExprKind::Call {
                callee,
                type_args,
                args,
            } = &c.value.kind
            else {
                continue;
            };
            let ExprKind::Field { expr, name } = &callee.kind else {
                continue;
            };
            if name.name != "model"
                || !matches!(&expr.kind, ExprKind::Ident(i) if self.imports.get(i).is_some_and(|m| self.modules[*m].name == "orm"))
            {
                continue;
            }
            let Some(t) = type_args.first() else { continue };
            let Ty::Adt(adt, _) = self.resolve_type(t, &[], Scope::Global) else {
                continue;
            };
            let mut mapping = orm::Mapping {
                span: c.value.span,
                ..Default::default()
            };
            for (i, a) in args.iter().enumerate() {
                let field = a.name.as_ref().map(|n| n.name.clone()).unwrap_or_else(|| {
                    ["table", "key", "columns", "generated", "readonly"]
                        .get(i)
                        .unwrap_or(&"")
                        .to_string()
                });
                match field.as_str() {
                    "table" => mapping.table = text_of(&a.value),
                    "key" => mapping.key = field_name_of(&a.value),
                    "columns" => {
                        if let ExprKind::Record(fields) = &a.value.kind {
                            for f in fields {
                                if let Some(name) = f.value.as_ref().and_then(text_of) {
                                    mapping.columns.insert(f.name.name.clone(), name);
                                }
                            }
                        }
                    }
                    "generated" | "readonly" => {
                        if let ExprKind::List(items) = &a.value.kind {
                            let names: Vec<String> =
                                items.iter().filter_map(field_name_of).collect();
                            if field == "generated" {
                                mapping.generated = names;
                            } else {
                                mapping.readonly = names;
                            }
                        }
                    }
                    _ => {}
                }
            }
            self.mappings.insert(adt, mapping);
        }
    }
}

/// A string literal's text.
fn text_of(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::Str(pieces) => match pieces.as_slice() {
            [ast::StrPiece::Text(t)] => Some(t.clone()),
            [] => Some(String::new()),
            _ => None,
        },
        _ => None,
    }
}

/// The name in `.field`.
fn field_name_of(e: &Expr) -> Option<String> {
    match &e.kind {
        ExprKind::FieldShorthand(name) => Some(name.name.clone()),
        _ => None,
    }
}

/// The plan of a query, for the lowering: the SQL, what it binds, and the
/// relations to load afterwards.
#[derive(Clone, Debug)]
pub struct Query {
    pub plan: QueryPlan,
    /// Which method or function ran it: a method of `orm.Query`, or
    /// `get`, `insert`, `update` or `delete`.
    pub terminal: String,
    /// The row `insert`, `update` and `delete` were given.
    pub value: Option<Expr>,
}

impl Query {
    /// The SQL this query runs, and the values it binds.
    pub fn sql(&self) -> (String, Vec<Param>) {
        match self.terminal.as_str() {
            "count" => self.plan.count_sql(),
            "delete" => self.plan.delete_sql(),
            "first" | "get" => {
                let one = QueryPlan {
                    one: true,
                    ..self.plan.clone()
                };
                one.rows_sql()
            }
            _ => self.plan.rows_sql(),
        }
    }
}
