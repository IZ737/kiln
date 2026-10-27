//! `kiln db`: the schema a program's models describe, the database it
//! runs against, and the migrations between them.
//!
//! Nothing here changes a database by itself. `diff` writes a migration
//! for a person to read; `migrate` applies the migrations that are
//! committed, in order, each once, checking that the ones already applied
//! have not been edited since. `pull` reads an existing database and
//! writes the models for it, and never writes to the database at all.

use kiln_check::orm::{ColumnKind, Model};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// What `kiln db` was asked to do.
pub enum Action {
    Status,
    Diff {
        name: String,
        allow_destructive: bool,
    },
    Migrate {
        dry_run: bool,
    },
    Verify,
    Pull {
        out: Option<PathBuf>,
        check: bool,
    },
}

/// The schema a program's models describe, or a database holds.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Schema {
    pub tables: BTreeMap<String, Table>,
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Table {
    pub columns: Vec<Column>,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Column {
    pub name: String,
    /// The SQL type, as the dialect writes it.
    pub ty: String,
    pub not_null: bool,
    pub key: bool,
    /// `table(column)` this column points at, when it does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub references: Option<String>,
}

impl Table {
    fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
}

/// The type of a column in a schema: `integer`, `real`, `text` or
/// `boolean`, whatever the database calls it. Migrations are written per
/// dialect, in its own words ([`ddl_type`]).
fn sql_type(kind: ColumnKind) -> &'static str {
    match kind {
        ColumnKind::Int | ColumnKind::Duration => "integer",
        ColumnKind::Bool => "boolean",
        ColumnKind::Float => "real",
        ColumnKind::Text | ColumnKind::Time | ColumnKind::Json => "text",
    }
}

/// The databases `kiln db` works on, each with its own migrations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
    Mysql,
}

impl Dialect {
    pub const ALL: [Dialect; 3] = [Dialect::Sqlite, Dialect::Postgres, Dialect::Mysql];

    pub fn of_url(url: &str) -> Option<Dialect> {
        if url.starts_with("sqlite:") {
            Some(Dialect::Sqlite)
        } else if url.starts_with("postgres://") || url.starts_with("postgresql://") {
            Some(Dialect::Postgres)
        } else if url.starts_with("mysql://") || url.starts_with("mariadb://") {
            Some(Dialect::Mysql)
        } else {
            None
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Dialect::Sqlite => "sqlite",
            Dialect::Postgres => "postgres",
            Dialect::Mysql => "mysql",
        }
    }
}

/// A schema's type as `dialect` writes it in a migration.
fn ddl_type(dialect: Dialect, ty: &str) -> String {
    let t = match (dialect, ty) {
        (Dialect::Sqlite, "boolean") => "integer",
        (Dialect::Postgres | Dialect::Mysql, "integer") => "bigint",
        (Dialect::Postgres, "real") => "double precision",
        (Dialect::Mysql, "real") => "double",
        (_, other) => other,
    };
    t.to_string()
}

/// The type a database reports for a column, as a schema's type.
fn schema_type(ty: &str) -> String {
    let t = ty.to_lowercase();
    if t.contains("bool") || t == "tinyint(1)" {
        "boolean"
    } else if t.contains("int") || t.contains("serial") {
        "integer"
    } else if t.contains("real")
        || t.contains("doub")
        || t.contains("floa")
        || t.contains("numeric")
        || t.contains("decimal")
    {
        "real"
    } else {
        "text"
    }
    .to_string()
}

/// The schema the models describe.
pub fn schema_of(models: &[Model]) -> Schema {
    let mut tables = BTreeMap::new();
    for m in models {
        let key = m.key_column().name.clone();
        let columns = m
            .columns
            .iter()
            .map(|c| {
                let points_at = m.relations.iter().find_map(|r| {
                    let links = r.key_field == c.field
                        && r.kind == kiln_check::orm::RelationKind::BelongsTo;
                    let target = models.iter().find(|t| t.adt == r.target)?;
                    links.then(|| format!("{}({})", target.table, target.key_column().name))
                });
                Column {
                    name: c.name.clone(),
                    ty: sql_type(c.kind).to_string(),
                    not_null: !c.optional && c.name != key,
                    key: c.name == key,
                    references: points_at,
                }
            })
            .collect();
        tables.insert(m.table.clone(), Table { columns });
    }
    Schema { tables }
}

/// One change between two schemas.
#[derive(Debug)]
pub enum Change {
    CreateTable(String),
    DropTable(String),
    AddColumn(String, String),
    DropColumn(String, String),
    /// A column whose type or nullability changed, which SQLite cannot
    /// alter in place.
    AlterColumn(String, String, String, String),
}

impl Change {
    pub fn destructive(&self) -> bool {
        matches!(
            self,
            Change::DropTable(_) | Change::DropColumn(..) | Change::AlterColumn(..)
        )
    }
}

/// The changes that take `from` to `to`.
pub fn diff(from: &Schema, to: &Schema) -> Vec<Change> {
    diff_by(from, to, |a, b| a == b)
}

/// How a database differs from what its migrations describe: SQLite keeps
/// a boolean as an integer.
fn drift(dialect: Dialect, live: &Schema, expected: &Schema) -> Vec<Change> {
    let plain = |t: &str| {
        let t = schema_type(t);
        if dialect == Dialect::Sqlite && t == "boolean" {
            "integer".to_string()
        } else {
            t
        }
    };
    diff_by(live, expected, |a, b| plain(a) == plain(b))
}

fn diff_by(from: &Schema, to: &Schema, same: impl Fn(&str, &str) -> bool) -> Vec<Change> {
    let mut changes = Vec::new();
    for (name, table) in &to.tables {
        let Some(old) = from.tables.get(name) else {
            changes.push(Change::CreateTable(name.clone()));
            continue;
        };
        for c in &table.columns {
            match old.column(&c.name) {
                None => changes.push(Change::AddColumn(name.clone(), c.name.clone())),
                Some(before) if !same(&before.ty, &c.ty) || before.not_null != c.not_null => {
                    changes.push(Change::AlterColumn(
                        name.clone(),
                        c.name.clone(),
                        describe(before),
                        describe(c),
                    ));
                }
                Some(_) => {}
            }
        }
        for c in &old.columns {
            if table.column(&c.name).is_none() {
                changes.push(Change::DropColumn(name.clone(), c.name.clone()));
            }
        }
    }
    for name in from.tables.keys() {
        if !to.tables.contains_key(name) {
            changes.push(Change::DropTable(name.clone()));
        }
    }
    changes
}

/// A value of this type for rows that have none yet.
fn zero(dialect: Dialect, ty: &str) -> &'static str {
    match (dialect, schema_type(ty).as_str()) {
        (_, "integer") => "0",
        (_, "real") => "0.0",
        (Dialect::Postgres, "boolean") => "false",
        (_, "boolean") => "0",
        // MySQL takes a default for text only as an expression.
        (Dialect::Mysql, _) => "('')",
        _ => "''",
    }
}

fn describe(c: &Column) -> String {
    format!("{}{}", c.ty, if c.not_null { " not null" } else { "" })
}

/// `create table posts (...)`, with the columns in the model's order.
fn create_table(dialect: Dialect, name: &str, table: &Table) -> String {
    let mut sql = format!("create table {name} (\n");
    let mut lines: Vec<String> = table
        .columns
        .iter()
        .map(|c| {
            let ty = ddl_type(dialect, &c.ty);
            let mut line = format!("    {} {ty}", c.name);
            if c.key {
                line.push_str(match (dialect, ty.as_str()) {
                    (Dialect::Postgres, "bigint") => {
                        " generated by default as identity primary key"
                    }
                    (Dialect::Mysql, "bigint") => " auto_increment primary key",
                    _ => " primary key",
                });
            } else if c.not_null {
                line.push_str(" not null");
            }
            // MySQL ignores `references` written on a column.
            if let (Some(target), false) = (&c.references, dialect == Dialect::Mysql) {
                line.push_str(&format!(" references {target}"));
            }
            line
        })
        .collect();
    if dialect == Dialect::Mysql {
        for c in &table.columns {
            if let Some(target) = &c.references {
                lines.push(format!("    foreign key ({}) references {target}", c.name));
            }
        }
    }
    sql.push_str(&lines.join(",\n"));
    sql.push_str("\n)");
    sql
}

/// The SQL of the changes, with a comment on each that needs a person.
pub fn migration_sql(dialect: Dialect, to: &Schema, changes: &[Change]) -> String {
    let mut sql = String::new();
    for change in in_order(to, changes) {
        match change {
            Change::CreateTable(name) => {
                let _ = writeln!(sql, "{};\n", create_table(dialect, name, &to.tables[name]));
            }
            Change::AddColumn(table, column) => {
                let c = to.tables[table].column(column).expect("the new column");
                // A column the rows that exist have no value for needs a
                // default, which is what they get.
                let (note, rest) = if c.not_null {
                    (
                        "-- The rows that are already there get this default. If they need\n-- another value, set it here or in a migration of its own.\n",
                        format!(" not null default {}", zero(dialect, &c.ty)),
                    )
                } else {
                    ("", String::new())
                };
                let _ = writeln!(
                    sql,
                    "{note}alter table {table} add column {} {}{rest};\n",
                    c.name,
                    ddl_type(dialect, &c.ty)
                );
            }
            Change::DropColumn(table, column) => {
                let _ = writeln!(
                    sql,
                    "-- Dropping a column throws its data away.\nalter table {table} drop column {column};\n"
                );
            }
            Change::DropTable(table) => {
                let _ = writeln!(
                    sql,
                    "-- Dropping a table throws its data away.\ndrop table {table};\n"
                );
            }
            Change::AlterColumn(table, column, before, after) => {
                let how = match dialect {
                    Dialect::Sqlite => "SQLite cannot\n-- change a column in place: make the new table, copy the rows over,\n-- drop the old one and rename it, inside one transaction.",
                    Dialect::Postgres => "Write the\n-- `alter table ... alter column` that changes it, with `using` to\n-- convert the rows that are there.",
                    Dialect::Mysql => "Write the\n-- `alter table ... modify column` that changes it, after checking\n-- that every row converts.",
                };
                let _ = writeln!(
                    sql,
                    "-- `{table}.{column}` was `{before}` and is now `{after}`. {how}\n"
                );
            }
        }
    }
    sql
}

/// The changes with the new tables first, each after the tables it
/// references, which PostgreSQL and MySQL need to exist already.
fn in_order<'a>(to: &Schema, changes: &'a [Change]) -> Vec<&'a Change> {
    let mut created: Vec<&Change> = Vec::new();
    let mut waiting: Vec<&Change> = changes
        .iter()
        .filter(|c| matches!(c, Change::CreateTable(_)))
        .collect();
    let name = |c: &Change| match c {
        Change::CreateTable(n) => n.clone(),
        _ => String::new(),
    };
    while !waiting.is_empty() {
        let ready = waiting.iter().position(|c| {
            let table = &to.tables[&name(c)];
            table.columns.iter().all(|col| match &col.references {
                Some(target) => {
                    let target = target.split('(').next().unwrap_or("");
                    target == name(c) || !waiting.iter().any(|w| name(w) == target)
                }
                None => true,
            })
        });
        // A cycle of references is written in name order.
        created.push(waiting.remove(ready.unwrap_or(0)));
    }
    created.extend(
        changes
            .iter()
            .filter(|c| !matches!(c, Change::CreateTable(_))),
    );
    created
}

// ----- the database -----

/// The database `kiln db` works on: SQLite, or a server.
enum Database {
    Sqlite(rusqlite::Connection),
    Server(Box<dyn kiln_dbwire::Client>, Dialect),
}

/// Tables the runtime and `kiln db` keep for themselves, which no model
/// describes.
fn own_table(name: &str) -> bool {
    name == LEDGER || name == "kiln_sessions"
}

fn text(v: &kiln_dbwire::Value) -> String {
    match v {
        kiln_dbwire::Value::Text(s) => s.clone(),
        kiln_dbwire::Value::Int(n) => n.to_string(),
        kiln_dbwire::Value::Blob(b) => String::from_utf8_lossy(b).into_owned(),
        _ => String::new(),
    }
}

impl Database {
    fn open(url: &str) -> Result<Database, String> {
        let dialect = Dialect::of_url(url).ok_or_else(|| {
            format!(
                "{} is not a database URL: it should start with sqlite:, postgres:// or mysql://",
                redact(url)
            )
        })?;
        if dialect != Dialect::Sqlite {
            let client = kiln_dbwire::open(url, None)
                .map_err(|e| format!("cannot open {}: {e}", redact(url)))?;
            return Ok(Database::Server(client, dialect));
        }
        let path = url.strip_prefix("sqlite:").unwrap_or(url);
        let path = path.strip_prefix("//").unwrap_or(path);
        let conn = if path == ":memory:" {
            rusqlite::Connection::open_in_memory()
        } else {
            rusqlite::Connection::open(path)
        }
        .map_err(|e| format!("cannot open {path}: {e}"))?;
        Ok(Database::Sqlite(conn))
    }

    fn dialect(&self) -> Dialect {
        match self {
            Database::Sqlite(_) => Dialect::Sqlite,
            Database::Server(_, d) => *d,
        }
    }

    /// Rows of text from a query without values, on a server.
    fn rows(&mut self, sql: &str) -> Result<Vec<Vec<String>>, String> {
        let Database::Server(client, _) = self else {
            unreachable!("a SQLite database is read through rusqlite")
        };
        let rows = client.query(sql, &[]).map_err(|e| e.to_string())?;
        Ok(rows
            .rows
            .iter()
            .map(|r| r.iter().map(text).collect())
            .collect())
    }

    /// The schema the database holds now.
    fn schema(&mut self) -> Result<Schema, String> {
        match self.dialect() {
            Dialect::Sqlite => self.sqlite_schema(),
            dialect => self.server_schema(dialect),
        }
    }

    /// A server's schema, from `information_schema`: the tables of the
    /// schema (PostgreSQL) or database (MySQL) the URL names.
    fn server_schema(&mut self, dialect: Dialect) -> Result<Schema, String> {
        let (here, ty) = match dialect {
            Dialect::Postgres => ("current_schema()", "data_type"),
            _ => ("database()", "column_type"),
        };
        let columns = self.rows(&format!(
            "select table_name, column_name, {ty}, is_nullable from information_schema.columns where table_schema = {here} order by table_name, ordinal_position"
        ))?;
        let keys = self.rows(&format!(
            "select k.table_name, k.column_name from information_schema.table_constraints c join information_schema.key_column_usage k on k.constraint_name = c.constraint_name and k.table_schema = c.table_schema and k.table_name = c.table_name where c.constraint_type = 'PRIMARY KEY' and c.table_schema = {here}"
        ))?;
        let references = match dialect {
            Dialect::Postgres => self.rows(
                "select k.table_name, k.column_name, u.table_name, u.column_name from information_schema.table_constraints c join information_schema.key_column_usage k on k.constraint_name = c.constraint_name and k.table_schema = c.table_schema join information_schema.constraint_column_usage u on u.constraint_name = c.constraint_name and u.table_schema = c.table_schema where c.constraint_type = 'FOREIGN KEY' and c.table_schema = current_schema()",
            )?,
            _ => self.rows(
                "select table_name, column_name, referenced_table_name, referenced_column_name from information_schema.key_column_usage where table_schema = database() and referenced_table_name is not null",
            )?,
        };
        let mut tables: BTreeMap<String, Table> = BTreeMap::new();
        for row in columns {
            let [table, column, ty, nullable] = &row[..] else {
                continue;
            };
            if own_table(table) {
                continue;
            }
            let key = keys.iter().any(|k| k[0] == *table && k[1] == *column);
            tables
                .entry(table.clone())
                .or_default()
                .columns
                .push(Column {
                    name: column.clone(),
                    ty: schema_type(ty),
                    not_null: nullable == "NO" && !key,
                    key,
                    references: None,
                });
        }
        for r in references {
            if let Some(c) = tables
                .get_mut(&r[0])
                .and_then(|t| t.columns.iter_mut().find(|c| c.name == r[1]))
            {
                c.references = Some(format!("{}({})", r[2], r[3]));
            }
        }
        Ok(Schema { tables })
    }

    fn sqlite_schema(&self) -> Result<Schema, String> {
        let Database::Sqlite(conn) = self else {
            unreachable!()
        };
        let mut tables = BTreeMap::new();
        let names: Vec<String> = {
            let mut stmt = conn
                .prepare("select name from sqlite_master where type = 'table' and name not like 'sqlite_%' order by name")
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<_, _>>().map_err(|e| e.to_string())?
        };
        for name in names {
            if own_table(&name) {
                continue;
            }
            let mut columns = Vec::new();
            let mut stmt = conn
                .prepare(&format!("pragma table_info({name})"))
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(5)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (column, ty, not_null, pk) = row.map_err(|e| e.to_string())?;
                columns.push(Column {
                    name: column,
                    ty: schema_type(&ty),
                    not_null: not_null != 0 && pk == 0,
                    key: pk > 0,
                    references: None,
                });
            }
            // Foreign keys, so a pulled model knows its relations.
            let mut stmt = conn
                .prepare(&format!("pragma foreign_key_list({name})"))
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, Option<String>>(4)?,
                    ))
                })
                .map_err(|e| e.to_string())?;
            for row in rows {
                let (target, from, to) = row.map_err(|e| e.to_string())?;
                if let Some(c) = columns.iter_mut().find(|c| c.name == from) {
                    c.references = Some(format!("{target}({})", to.unwrap_or_else(|| "id".into())));
                }
            }
            tables.insert(name, Table { columns });
        }
        Ok(Schema { tables })
    }

    fn applied(&mut self) -> Result<Vec<(String, String)>, String> {
        let ledger = format!(
            "create table if not exists {LEDGER} (version varchar(32) primary key, name varchar(200) not null, checksum varchar(80) not null, applied_at varchar(32) not null)"
        );
        let select = format!("select version, checksum from {LEDGER} order by version");
        match self {
            Database::Sqlite(conn) => {
                conn.execute_batch(&ledger).map_err(|e| e.to_string())?;
                let mut stmt = conn.prepare(&select).map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                    .map_err(|e| e.to_string())?;
                rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
            }
            Database::Server(client, _) => {
                client.exec(&ledger, &[]).map_err(|e| e.to_string())?;
                Ok(self
                    .rows(&select)?
                    .into_iter()
                    .map(|r| (r[0].clone(), r[1].clone()))
                    .collect())
            }
        }
    }

    /// Applies one migration and records it, all or nothing where the
    /// database can: MySQL commits each change to a table as it makes it.
    fn apply(&mut self, m: &Migration) -> Result<(), String> {
        let record = [m.version.clone(), m.name.clone(), m.checksum.clone(), now()];
        match self {
            Database::Sqlite(conn) => {
                let tx = conn.transaction().map_err(|e| e.to_string())?;
                tx.execute_batch(&m.sql)
                    .map_err(|e| format!("{}: {e}", m.file_name()))?;
                tx.execute(
                    &format!("insert into {LEDGER} (version, name, checksum, applied_at) values (?1, ?2, ?3, ?4)"),
                    rusqlite::params![record[0], record[1], record[2], record[3]],
                )
                .map_err(|e| e.to_string())?;
                tx.commit().map_err(|e| e.to_string())
            }
            Database::Server(client, dialect) => {
                let marks = match dialect {
                    Dialect::Postgres => "$1, $2, $3, $4",
                    _ => "?, ?, ?, ?",
                };
                let values: Vec<kiln_dbwire::Value> =
                    record.into_iter().map(kiln_dbwire::Value::Text).collect();
                let run = |client: &mut Box<dyn kiln_dbwire::Client>| -> Result<(), String> {
                    client
                        .exec(&m.sql, &[])
                        .map_err(|e| format!("{}: {e}", m.file_name()))?;
                    client
                        .exec(
                            &format!("insert into {LEDGER} (version, name, checksum, applied_at) values ({marks})"),
                            &values,
                        )
                        .map_err(|e| e.to_string())?;
                    Ok(())
                };
                client.exec("begin", &[]).map_err(|e| e.to_string())?;
                match run(client) {
                    Ok(()) => client
                        .exec("commit", &[])
                        .map(|_| ())
                        .map_err(|e| e.to_string()),
                    Err(e) => {
                        let _ = client.exec("rollback", &[]);
                        Err(e)
                    }
                }
            }
        }
    }
}

const LEDGER: &str = "kiln_migrations";

fn now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}")
}

/// A URL with its password replaced, for messages.
fn redact(url: &str) -> String {
    match url.split_once("://").and_then(|(scheme, rest)| {
        let (auth, host) = rest.rsplit_once('@')?;
        Some(format!(
            "{scheme}://{}:***@{host}",
            auth.split(':').next().unwrap_or("")
        ))
    }) {
        Some(safe) => safe,
        None => url.to_string(),
    }
}

// ----- migration files -----

#[derive(Debug)]
pub struct Migration {
    pub version: String,
    pub name: String,
    pub sql: String,
    pub checksum: String,
}

impl Migration {
    fn file_name(&self) -> String {
        format!("{}_{}.sql", self.version, self.name)
    }
}

fn checksum(sql: &str) -> String {
    // Line ends differ between systems; the statements do not.
    let text: String = sql.replace("\r\n", "\n");
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256-{hex}")
}

/// Each database's migrations are in a folder of their own:
/// `migrations/sqlite`, `migrations/postgres`, `migrations/mysql`.
fn migrations_dir(root: &Path, dialect: Dialect) -> PathBuf {
    root.join("migrations").join(dialect.name())
}

fn snapshot_path(root: &Path) -> PathBuf {
    root.join("migrations/schema.json")
}

fn read_migrations(root: &Path, dialect: Dialect) -> Result<Vec<Migration>, String> {
    let dir = migrations_dir(root, dialect);
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|e| e == "sql"))
        .collect();
    files.sort();
    let mut out = Vec::new();
    for file in files {
        let stem = file.file_stem().unwrap_or_default().to_string_lossy();
        let Some((version, name)) = stem.split_once('_') else {
            return Err(format!(
                "{} is not named <version>_<name>.sql",
                file.display()
            ));
        };
        let sql = std::fs::read_to_string(&file)
            .map_err(|e| format!("cannot read {}: {e}", file.display()))?;
        out.push(Migration {
            version: version.to_string(),
            name: name.to_string(),
            checksum: checksum(&sql),
            sql,
        });
    }
    Ok(out)
}

fn read_snapshot(root: &Path) -> Result<Schema, String> {
    let path = snapshot_path(root);
    if !path.is_file() {
        return Ok(Schema::default());
    }
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

// ----- the command -----

pub fn run(action: Action, path: &Path, url: Option<String>) -> ExitCode {
    match go(action, path, url) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn database_url(url: Option<String>) -> Result<String, String> {
    url.or_else(|| std::env::var("DATABASE_URL").ok())
        .ok_or_else(|| {
            "no database: pass `--url sqlite:./data/app.db` or set DATABASE_URL".to_string()
        })
}

fn go(action: Action, path: &Path, url: Option<String>) -> Result<(), String> {
    let root = crate::project::discover(path)
        .map_err(|err| format!("{err:?}"))
        .map(|p| p.root)
        .unwrap_or_else(|_| path.to_path_buf());
    match action {
        Action::Pull { out, check } => pull(&root, &database_url(url)?, out, check),
        Action::Status => status(&root, &database_url(url)?),
        Action::Verify => verify(&root),
        Action::Diff {
            name,
            allow_destructive,
        } => {
            // The migrations are written for each database the program
            // has migrations for, and for the one it is given.
            let given = url
                .or_else(|| std::env::var("DATABASE_URL").ok())
                .and_then(|u| Dialect::of_url(&u))
                .unwrap_or(Dialect::Sqlite);
            write_diff(&root, path, &name, allow_destructive, given)
        }
        Action::Migrate { dry_run } => migrate(&root, &database_url(url)?, dry_run),
    }
}

/// The models the program uses, which is the schema it needs.
fn models(path: &Path) -> Result<Vec<Model>, String> {
    let (program, _) = crate::check_program_of(path)?;
    Ok(program.models)
}

fn write_diff(
    root: &Path,
    path: &Path,
    name: &str,
    allow_destructive: bool,
    given: Dialect,
) -> Result<(), String> {
    let models = models(path)?;
    if models.is_empty() {
        return Err(
            "the program has no models yet: a model is a struct the program uses with `orm`".into(),
        );
    }
    let wanted = schema_of(&models);
    let before = read_snapshot(root)?;
    let changes = diff(&before, &wanted);
    let destructive: Vec<&Change> = changes.iter().filter(|c| c.destructive()).collect();
    if !destructive.is_empty() && !allow_destructive {
        let mut message = String::from(
            "this change would drop or rewrite data, so it needs `--allow-destructive`:\n",
        );
        for change in destructive {
            let _ = writeln!(message, "  {}", describe_change(change));
        }
        return Err(message);
    }
    let dialects: Vec<Dialect> = Dialect::ALL
        .into_iter()
        .filter(|d| *d == given || migrations_dir(root, *d).is_dir())
        .collect();
    let mut existing = Vec::new();
    for d in &dialects {
        existing.extend(read_migrations(root, *d)?.into_iter().map(|m| m.version));
    }
    let last = existing
        .iter()
        .filter_map(|v| v.parse::<u32>().ok())
        .max()
        .unwrap_or(0);
    // A database that only now gets migrations takes the latest version,
    // so the folders stay in step.
    let next = if changes.is_empty() {
        last.max(1)
    } else {
        last + 1
    };
    let version = format!("{next:04}");
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    let mut wrote = false;
    for dialect in dialects {
        let dir = migrations_dir(root, dialect);
        // A database that has no migrations yet gets the whole schema.
        let changes = if dir.is_dir() {
            diff(&before, &wanted)
        } else {
            diff(&Schema::default(), &wanted)
        };
        if changes.is_empty() {
            continue;
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
        let file = dir.join(format!("{version}_{safe}.sql"));
        let mut sql = format!(
            "-- {version} {safe}\n-- Written by `kiln db diff` for {}. Read it before applying it.\n\n",
            dialect.name()
        );
        sql.push_str(&migration_sql(dialect, &wanted, &changes));
        std::fs::write(&file, &sql).map_err(|e| format!("cannot write {}: {e}", file.display()))?;
        println!("wrote {}", file.display());
        for change in &changes {
            println!("  {}", describe_change(change));
        }
        wrote = true;
    }
    if !wrote {
        println!("the migrations already describe the models: nothing to write");
        return Ok(());
    }
    let snapshot = snapshot_path(root);
    let text = serde_json::to_string_pretty(&wanted).map_err(|e| e.to_string())?;
    std::fs::write(&snapshot, format!("{text}\n"))
        .map_err(|e| format!("cannot write {}: {e}", snapshot.display()))?;
    println!("Read it, then apply it with `kiln db migrate`.");
    Ok(())
}

fn describe_change(change: &Change) -> String {
    match change {
        Change::CreateTable(t) => format!("create table {t}"),
        Change::DropTable(t) => format!("drop table {t} (throws its data away)"),
        Change::AddColumn(t, c) => format!("add {t}.{c}"),
        Change::DropColumn(t, c) => format!("drop {t}.{c} (throws its data away)"),
        Change::AlterColumn(t, c, before, after) => {
            format!("{t}.{c} was `{before}` and is now `{after}` (needs a hand-written migration)")
        }
    }
}

/// How the database differs from the migrations, said from the
/// database's side.
fn describe_drift(change: &Change) -> String {
    match change {
        Change::CreateTable(t) => {
            format!("the migrations describe a table {t}, which is not there")
        }
        Change::DropTable(t) => format!("the database has a table {t} that no migration describes"),
        Change::AddColumn(t, c) => {
            format!("the migrations describe {t}.{c}, which is not there")
        }
        Change::DropColumn(t, c) => {
            format!("the database has {t}.{c}, which no migration describes")
        }
        Change::AlterColumn(t, c, live, expected) => {
            format!("{t}.{c} is `{live}` in the database and `{expected}` in the migrations")
        }
    }
}

fn status(root: &Path, url: &str) -> Result<(), String> {
    let mut db = Database::open(url)?;
    let dialect = db.dialect();
    let applied = db.applied()?;
    let files = read_migrations(root, dialect)?;
    if files.is_empty() {
        println!(
            "no migrations in {}",
            migrations_dir(root, dialect).display()
        );
    }
    for m in &files {
        let state = match applied.iter().find(|(v, _)| *v == m.version) {
            Some((_, sum)) if *sum == m.checksum => "applied",
            Some(_) => "applied, but the file has changed since",
            None => "pending",
        };
        println!("{} {}  {state}", m.version, m.name);
    }
    for (version, _) in &applied {
        if !files.iter().any(|m| m.version == *version) {
            println!("{version} ?  applied, but its file is missing");
        }
    }
    // What the database holds, against what the migrations describe.
    let live = db.schema()?;
    let expected = read_snapshot(root)?;
    let drift = drift(dialect, &live, &expected);
    if drift.is_empty() {
        println!("the database matches the migrations");
    } else {
        println!("the database differs from the migrations:");
        for change in &drift {
            println!("  {}", describe_drift(change));
        }
    }
    Ok(())
}

fn verify(root: &Path) -> Result<(), String> {
    for dialect in Dialect::ALL {
        if !migrations_dir(root, dialect).is_dir() {
            continue;
        }
        let files = read_migrations(root, dialect)?;
        let mut seen = Vec::new();
        for m in &files {
            if seen.contains(&m.version) {
                return Err(format!(
                    "two {} migrations are version {}",
                    dialect.name(),
                    m.version
                ));
            }
            seen.push(m.version.clone());
        }
        println!(
            "{}: {} migration(s), each with a version of its own",
            dialect.name(),
            files.len()
        );
    }
    let snapshot = snapshot_path(root);
    if snapshot.is_file() {
        read_snapshot(root)?;
        println!("{} reads", snapshot.display());
    }
    Ok(())
}

fn migrate(root: &Path, url: &str, dry_run: bool) -> Result<(), String> {
    let mut db = Database::open(url)?;
    let applied = db.applied()?;
    let files = read_migrations(root, db.dialect())?;
    for (version, sum) in &applied {
        match files.iter().find(|m| m.version == *version) {
            Some(m) if m.checksum == *sum => {}
            Some(m) => {
                return Err(format!(
                    "migration {} has changed since it was applied; a migration that has run is history, so write a new one instead",
                    m.file_name()
                ))
            }
            None => {
                return Err(format!(
                    "migration {version} was applied but its file is missing; restore it before migrating"
                ))
            }
        }
    }
    let pending: Vec<&Migration> = files
        .iter()
        .filter(|m| !applied.iter().any(|(v, _)| *v == m.version))
        .collect();
    if pending.is_empty() {
        println!("nothing to apply: the database is up to date");
        return Ok(());
    }
    // A migration that is older than one already applied would change the
    // order in which they ran.
    if let Some(last) = applied.last() {
        if let Some(m) = pending.iter().find(|m| m.version <= last.0) {
            return Err(format!(
                "{} comes before {}, which has already been applied; give it a later version",
                m.file_name(),
                last.0
            ));
        }
    }
    for m in pending {
        if dry_run {
            println!("would apply {}:\n{}", m.file_name(), m.sql);
            continue;
        }
        db.apply(m)?;
        println!("applied {}", m.file_name());
    }
    Ok(())
}

/// Applies a program's migrations to a new database, quietly: a test's
/// database starts with the schema the program's migrations give.
pub fn prepare_test_database(root: &Path, url: &str) -> Result<(), String> {
    let mut db = Database::open(url)?;
    let applied = db.applied()?;
    for m in read_migrations(root, db.dialect())? {
        if !applied.iter().any(|(v, _)| *v == m.version) {
            db.apply(&m)?;
        }
    }
    Ok(())
}

/// `kiln db pull`: the models for the tables a database already has.
fn pull(root: &Path, url: &str, out: Option<PathBuf>, check: bool) -> Result<(), String> {
    let mut db = Database::open(url)?;
    let live = db.schema()?;
    if live.tables.is_empty() {
        return Err(format!("{} has no tables", redact(url)));
    }
    let source = models_source(&live);
    if check {
        let expected = read_snapshot(root)?;
        let changes = drift(db.dialect(), &expected, &live);
        if changes.is_empty() {
            println!("the database matches the models");
            return Ok(());
        }
        println!("the database differs from the models:");
        for change in &changes {
            println!("  {}", describe_drift(change));
        }
        return Ok(());
    }
    let path = out.unwrap_or_else(|| root.join("models.kiln"));
    if path.exists() {
        return Err(format!(
            "{} is already there; `kiln db pull` never overwrites code you have edited. Write it elsewhere with `--out`, or move it aside",
            path.display()
        ));
    }
    std::fs::write(&path, &source).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    let snapshot = snapshot_path(root);
    if !snapshot.exists() {
        if let Some(parent) = snapshot.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let text = serde_json::to_string_pretty(&live).map_err(|e| e.to_string())?;
        let _ = std::fs::write(&snapshot, format!("{text}\n"));
    }
    println!(
        "wrote {} with {} model(s). Read them, then check them with `kiln check`.",
        path.display(),
        live.tables.len()
    );
    Ok(())
}

/// The Kiln source of the models for a schema.
pub fn models_source(schema: &Schema) -> String {
    let mut out = String::from(
        "// Written by `kiln db pull` from a database that already exists.\n// These are ordinary structs: edit them, and `kiln db pull --check`\n// reports what the database has that they do not.\n",
    );
    for (table, t) in &schema.tables {
        let name = type_name(table);
        let mut mapping = Vec::new();
        if kiln_check::orm::table_name(&name) != *table {
            mapping.push(format!("table: {table:?}"));
        }
        let _ = write!(out, "\nstruct {name} {{\n");
        for c in &t.columns {
            let field = kiln_check::orm::snake_case(&c.name);
            let ty = kiln_type(&c.ty);
            let optional = if c.not_null || c.key { "" } else { "?" };
            let default = if c.not_null || c.key { "" } else { " = none" };
            let _ = writeln!(out, "    {field}: {ty}{optional}{default}");
        }
        let _ = writeln!(out, "}}");
        if !mapping.is_empty() {
            let _ = writeln!(
                out,
                "\nconst {} = orm.model<{name}>({})",
                kiln_check::orm::snake_case(table),
                mapping.join(", ")
            );
        }
    }
    out
}

/// `posts` to `Post`: the singular, in upper camel case.
fn type_name(table: &str) -> String {
    let singular = if let Some(stem) = table.strip_suffix("ies") {
        format!("{stem}y")
    } else if let Some(stem) = table.strip_suffix("es") {
        if stem.ends_with(['s', 'x', 'z']) || stem.ends_with("ch") || stem.ends_with("sh") {
            stem.to_string()
        } else {
            format!("{stem}e")
        }
    } else {
        table.strip_suffix('s').unwrap_or(table).to_string()
    };
    singular
        .split('_')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut chars = p.chars();
            match chars.next() {
                Some(c) => format!("{}{}", c.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        })
        .collect()
}

/// The Kiln type for a column's SQL type.
fn kiln_type(ty: &str) -> &'static str {
    let ty = ty.to_lowercase();
    if ty.contains("int") {
        "Int"
    } else if ty.contains("real") || ty.contains("floa") || ty.contains("doub") {
        "Float"
    } else if ty.contains("bool") {
        "Bool"
    } else {
        "String"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A column in a test schema: name, type, not null, key.
    type TestColumn<'a> = (&'a str, &'a str, bool, bool);

    fn schema(sql: &[(&str, &[TestColumn])]) -> Schema {
        let mut tables = BTreeMap::new();
        for (name, columns) in sql {
            tables.insert(
                name.to_string(),
                Table {
                    columns: columns
                        .iter()
                        .map(|(n, t, not_null, key)| Column {
                            name: n.to_string(),
                            ty: t.to_string(),
                            not_null: *not_null,
                            key: *key,
                            references: None,
                        })
                        .collect(),
                },
            );
        }
        Schema { tables }
    }

    #[test]
    fn changes_are_additive_or_reported_as_destructive() {
        let before = schema(&[("posts", &[("id", "integer", false, true)])]);
        let after = schema(&[
            (
                "posts",
                &[
                    ("id", "integer", false, true),
                    ("title", "text", true, false),
                ],
            ),
            ("tags", &[("id", "integer", false, true)]),
        ]);
        let changes = diff(&before, &after);
        assert_eq!(changes.len(), 2);
        assert!(!changes.iter().any(Change::destructive));
        let sql = migration_sql(Dialect::Sqlite, &after, &changes);
        assert!(sql.contains("alter table posts add column title text not null default ''"));
        assert!(sql.contains("create table tags"));
        let back = diff(&after, &before);
        assert!(back.iter().any(Change::destructive));
        assert!(diff(&after, &after).is_empty());
    }

    #[test]
    fn pulled_models_are_named_after_their_tables() {
        assert_eq!(type_name("posts"), "Post");
        assert_eq!(type_name("categories"), "Category");
        assert_eq!(type_name("blog_posts"), "BlogPost");
        let source = models_source(&schema(&[(
            "posts",
            &[
                ("id", "integer", false, true),
                ("title", "text", true, false),
                ("summary", "text", false, false),
            ],
        )]));
        assert!(source.contains("struct Post {"));
        assert!(source.contains("    title: String\n"));
        assert!(source.contains("    summary: String? = none\n"));
    }
}
