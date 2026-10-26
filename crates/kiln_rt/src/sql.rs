//! `sql`: databases, behind one driver interface.
//!
//! A `sql.Db` is a pool of connections to one database. Its Kiln value is
//! a small heap object holding the pool's id and, for the handle a
//! transaction gets, the connection it is pinned to. The pools themselves
//! live here, in Rust's heap, and hold no Kiln values: queries take their
//! parameters as Rust values (`SqlValue`) and give rows back the same way,
//! and only then are the rows turned into Kiln values, by the type
//! description the backends pass.
//!
//! Kiln code runs on one thread, its tasks taking turns. A query that does
//! not run in a transaction borrows a connection for the length of the
//! call; a transaction keeps its connection until it commits or rolls back,
//! which can be after the task has waited and other tasks have run. A
//! task that finds every connection taken waits for one with
//! `k_rt_sleep`, so the other tasks go on.
//!
//! Queries are written with `$1`, `$2`, ... placeholders whatever the
//! database. `placeholders` rewrites them for the driver: `?1` for SQLite,
//! `?` (with the values repeated in order) for MySQL; PostgreSQL takes them
//! as they are. Values are always bound, never written into the text.

use crate::value::*;
use std::cell::RefCell;
use std::time::{Duration, Instant};

extern "C" {
    /// Stores the message of a failing library call (`k_lib_error`).
    fn k_rt_fail(message: *mut KStr);
    /// Sleeps, letting other tasks run when there are any.
    fn k_rt_sleep(ns: i64);
}

/// Makes the current library call fail with `message`.
pub fn fail(message: &str) {
    unsafe { k_rt_fail(ptr(string(message.as_bytes()))) }
}

/// Waits `d`, letting the other tasks run.
pub fn pause(d: Duration) {
    unsafe { k_rt_sleep(d.as_nanos().min(i64::MAX as u128) as i64) }
}

// ----- values, rows and errors -----

#[derive(Clone, Debug, PartialEq)]
pub enum SqlValue {
    Null,
    Int(i64),
    Float(f64),
    Text(String),
    Bool(bool),
    Blob(Vec<u8>),
}

impl SqlValue {
    fn kind(&self) -> &'static str {
        match self {
            SqlValue::Null => "NULL",
            SqlValue::Int(_) => "an integer",
            SqlValue::Float(_) => "a float",
            SqlValue::Text(_) => "text",
            SqlValue::Bool(_) => "a boolean",
            SqlValue::Blob(_) => "binary data",
        }
    }
}

#[derive(Default, Debug)]
pub struct Rows {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<SqlValue>>,
}

/// What `exec` reports: the rows it changed, and the key the database
/// generated for an insert, when it says.
#[derive(Default, Debug)]
pub struct Done {
    pub changed: i64,
    pub key: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Connection,
    Syntax,
    Unique,
    ForeignKey,
    NotNull,
    Check,
    Constraint,
    Decode,
    Busy,
    Transaction,
    Unsupported,
    Other,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Connection => "connection",
            Kind::Syntax => "syntax",
            Kind::Unique => "unique",
            Kind::ForeignKey => "foreign_key",
            Kind::NotNull => "not_null",
            Kind::Check => "check",
            Kind::Constraint => "constraint",
            Kind::Decode => "decode",
            Kind::Busy => "busy",
            Kind::Transaction => "transaction",
            Kind::Unsupported => "unsupported",
            Kind::Other => "other",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DbError {
    pub kind: Kind,
    pub message: String,
}

impl DbError {
    pub fn new(kind: Kind, message: impl Into<String>) -> Self {
        DbError {
            kind,
            message: message.into(),
        }
    }

    /// The message a Kiln `Error` gets: the database's words, then the
    /// kind, which `sql.error_kind` reads back.
    pub fn text(&self) -> String {
        format!("{} (sql {})", self.message, self.kind.name())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    Sqlite,
    Postgres,
    Mysql,
}

impl Dialect {
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Sqlite => "sqlite",
            Dialect::Postgres => "postgres",
            Dialect::Mysql => "mysql",
        }
    }
}

/// A connection to a database.
pub trait Driver {
    fn query(&mut self, sql: &str, params: &[SqlValue]) -> Result<Rows, DbError>;
    fn exec(&mut self, sql: &str, params: &[SqlValue]) -> Result<Done, DbError>;
}

// ----- placeholders -----

/// Rewrites `$n` placeholders for `dialect`, outside quotes and comments,
/// and returns the values in the order the rewritten text binds them.
pub fn placeholders(
    sql: &str,
    params: &[SqlValue],
    dialect: Dialect,
) -> Result<(String, Vec<SqlValue>), DbError> {
    let b = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut order = Vec::new();
    let mut highest = 0usize;
    let mut i = 0;
    let mut copied = 0;
    while i < b.len() {
        match b[i] {
            q @ (b'\'' | b'"' | b'`') => {
                i += 1;
                while i < b.len() {
                    if b[i] == q {
                        // A doubled quote is a quote inside.
                        if i + 1 < b.len() && b[i + 1] == q {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
            }
            b'$' if b.get(i + 1).is_some_and(u8::is_ascii_digit) => {
                let start = i;
                i += 1;
                while i < b.len() && b[i].is_ascii_digit() {
                    i += 1;
                }
                let n: usize = sql[start + 1..i].parse().unwrap_or(0);
                if n == 0 {
                    return Err(DbError::new(
                        Kind::Syntax,
                        "placeholders count from $1, so $0 is not one",
                    ));
                }
                highest = highest.max(n);
                out.push_str(&sql[copied..start]);
                copied = i;
                match dialect {
                    Dialect::Sqlite => out.push_str(&format!("?{n}")),
                    Dialect::Postgres => out.push_str(&format!("${n}")),
                    Dialect::Mysql => out.push('?'),
                }
                order.push(n - 1);
            }
            _ => i += 1,
        }
    }
    out.push_str(&sql[copied.min(sql.len())..]);
    if highest != params.len() {
        let message = if highest > params.len() {
            format!(
                "the query uses ${highest} but {} value(s) were given",
                params.len()
            )
        } else if highest == 0 {
            format!(
                "{} value(s) were given but the query has no placeholders; write $1, $2, ... where they go",
                params.len()
            )
        } else {
            format!(
                "{} values were given but the query uses only up to ${highest}",
                params.len()
            )
        };
        return Err(DbError::new(Kind::Syntax, message));
    }
    let values = match dialect {
        Dialect::Mysql => order.iter().map(|&i| params[i].clone()).collect(),
        _ => params.to_vec(),
    };
    Ok((out, values))
}

// ----- SQLite -----

pub struct Sqlite {
    conn: rusqlite::Connection,
}

/// The path or `:memory:` of a `sqlite:` URL.
fn sqlite_path(url: &str) -> &str {
    let rest = url.strip_prefix("sqlite:").unwrap_or(url);
    rest.strip_prefix("//").unwrap_or(rest)
}

impl Sqlite {
    pub fn open(url: &str) -> Result<Sqlite, DbError> {
        let path = sqlite_path(url);
        let conn = if path == ":memory:" {
            rusqlite::Connection::open_in_memory()
        } else {
            rusqlite::Connection::open(path)
        }
        .map_err(|e| DbError::new(Kind::Connection, format!("cannot open {path}: {e}")))?;
        let mut driver = Sqlite { conn };
        // Foreign keys are checked, as other databases check them. A file
        // gets the write-ahead log, so readers do not wait for a writer.
        driver.exec("PRAGMA foreign_keys = ON", &[])?;
        if path != ":memory:" {
            let _ = driver.query("PRAGMA journal_mode = WAL", &[]);
        }
        Ok(driver)
    }
}

fn sqlite_error(e: rusqlite::Error) -> DbError {
    use rusqlite::ffi::ErrorCode as C;
    let message = e.to_string();
    let kind = match &e {
        rusqlite::Error::SqliteFailure(f, _) => match f.code {
            C::ConstraintViolation => match f.extended_code {
                rusqlite::ffi::SQLITE_CONSTRAINT_UNIQUE
                | rusqlite::ffi::SQLITE_CONSTRAINT_PRIMARYKEY => Kind::Unique,
                rusqlite::ffi::SQLITE_CONSTRAINT_FOREIGNKEY => Kind::ForeignKey,
                rusqlite::ffi::SQLITE_CONSTRAINT_NOTNULL => Kind::NotNull,
                rusqlite::ffi::SQLITE_CONSTRAINT_CHECK => Kind::Check,
                _ => Kind::Constraint,
            },
            C::DatabaseBusy | C::DatabaseLocked => Kind::Busy,
            C::CannotOpen | C::NotADatabase | C::PermissionDenied | C::ReadOnly => Kind::Connection,
            _ if message.contains("syntax error")
                || message.contains("no such")
                || message.contains("has no column") =>
            {
                Kind::Syntax
            }
            _ => Kind::Other,
        },
        rusqlite::Error::SqlInputError { .. } => Kind::Syntax,
        _ if message.contains("syntax error") || message.contains("no such") => Kind::Syntax,
        _ => Kind::Other,
    };
    DbError::new(kind, message)
}

/// A statement SQLite cannot prepare is one it cannot read, unless the
/// error says otherwise.
fn prepare_error(e: rusqlite::Error) -> DbError {
    let mut err = sqlite_error(e);
    if err.kind == Kind::Other {
        err.kind = Kind::Syntax;
    }
    err
}

fn sqlite_params(params: &[SqlValue]) -> Vec<rusqlite::types::Value> {
    use rusqlite::types::Value as V;
    params
        .iter()
        .map(|p| match p {
            SqlValue::Null => V::Null,
            SqlValue::Int(n) => V::Integer(*n),
            SqlValue::Float(f) => V::Real(*f),
            SqlValue::Text(s) => V::Text(s.clone()),
            SqlValue::Bool(b) => V::Integer(*b as i64),
            SqlValue::Blob(b) => V::Blob(b.clone()),
        })
        .collect()
}

impl Driver for Sqlite {
    fn query(&mut self, sql: &str, params: &[SqlValue]) -> Result<Rows, DbError> {
        let (sql, params) = placeholders(sql, params, Dialect::Sqlite)?;
        let mut stmt = self.conn.prepare(&sql).map_err(prepare_error)?;
        let columns: Vec<String> = stmt.column_names().map(|c| c.to_string()).collect();
        let values = sqlite_params(&params);
        let mut rows = stmt
            .query(rusqlite::params_from_iter(values.iter()))
            .map_err(sqlite_error)?;
        let mut out = Rows {
            columns,
            rows: Vec::new(),
        };
        while let Some(row) = rows.next().map_err(sqlite_error)? {
            let mut values = Vec::with_capacity(out.columns.len());
            for i in 0..out.columns.len() {
                use rusqlite::types::ValueRef as R;
                let v = match row.get_ref(i).map_err(sqlite_error)? {
                    R::Null => SqlValue::Null,
                    R::Integer(n) => SqlValue::Int(n),
                    R::Real(f) => SqlValue::Float(f),
                    R::Text(t) => match std::str::from_utf8(t) {
                        Ok(s) => SqlValue::Text(s.to_string()),
                        Err(_) => SqlValue::Blob(t.to_vec()),
                    },
                    R::Blob(b) => SqlValue::Blob(b.to_vec()),
                };
                values.push(v);
            }
            out.rows.push(values);
        }
        Ok(out)
    }

    fn exec(&mut self, sql: &str, params: &[SqlValue]) -> Result<Done, DbError> {
        let (sql, params) = placeholders(sql, params, Dialect::Sqlite)?;
        let mut stmt = self.conn.prepare(&sql).map_err(prepare_error)?;
        let values = sqlite_params(&params);
        // A statement that returns rows (`PRAGMA`, `... RETURNING`) is
        // stepped to its end.
        if stmt.column_count() > 0 {
            let mut rows = stmt
                .query(rusqlite::params_from_iter(values.iter()))
                .map_err(sqlite_error)?;
            let mut key = None;
            while let Some(row) = rows.next().map_err(sqlite_error)? {
                if key.is_none() {
                    key = row.get::<_, i64>(0).ok();
                }
            }
            drop(rows);
            return Ok(Done {
                changed: self.conn.changes() as i64,
                key,
            });
        }
        let changed = stmt
            .execute(rusqlite::params_from_iter(values.iter()))
            .map_err(sqlite_error)?;
        Ok(Done {
            changed: changed as i64,
            key: Some(self.conn.last_insert_rowid()),
        })
    }
}

// ----- pools -----

struct Pool {
    url: String,
    dialect: Dialect,
    max: usize,
    wait: Duration,
    /// Each connection, `None` while a call or a transaction has it.
    conns: Vec<Option<Box<dyn Driver>>>,
    /// For each connection, the transaction that has it, if any.
    tx: Vec<Option<u64>>,
    closed: bool,
}

thread_local! {
    static POOLS: RefCell<Vec<Pool>> = const { RefCell::new(Vec::new()) };
    static NEXT_TX: RefCell<u64> = const { RefCell::new(1) };
}

/// The fields of a `sql.Db`: its pool, and for a transaction's handle the
/// connection and the transaction.
#[repr(C)]
struct DbHandle {
    pool: i64,
    conn: i64,
    tx: u64,
}

fn new_handle(pool: usize, conn: Option<(usize, u64)>) -> Kv {
    let h = unsafe { k_alloc(std::mem::size_of::<DbHandle>()) } as *mut DbHandle;
    unsafe {
        (*h).pool = pool as i64;
        (*h).conn = conn.map_or(-1, |c| c.0 as i64);
        (*h).tx = conn.map_or(0, |c| c.1);
    }
    kv(h)
}

fn handle(v: Kv) -> (usize, Option<(usize, u64)>) {
    let h = ptr::<DbHandle>(v);
    unsafe {
        let conn = ((*h).conn >= 0).then_some(((*h).conn as usize, (*h).tx));
        ((*h).pool as usize, conn)
    }
}

/// The URL with any password replaced, for messages.
pub fn redact(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    let (auth, host) = match rest.rsplit_once('@') {
        Some(parts) => parts,
        None => return url.to_string(),
    };
    let user = auth.split(':').next().unwrap_or("");
    format!("{scheme}://{user}:***@{host}")
}

fn connect(url: &str) -> Result<(Dialect, Box<dyn Driver>), DbError> {
    if url.starts_with("sqlite:") {
        return Ok((Dialect::Sqlite, Box::new(Sqlite::open(url)?)));
    }
    if url.starts_with("postgres://") || url.starts_with("postgresql://") {
        return crate::remote::postgres(url).map(|d| (Dialect::Postgres, d));
    }
    if url.starts_with("mysql://") || url.starts_with("mariadb://") {
        return crate::remote::mysql(url).map(|d| (Dialect::Mysql, d));
    }
    Err(DbError::new(
        Kind::Connection,
        format!(
            "cannot open {}: the URL should start with sqlite:, postgres:// or mysql://",
            redact(url)
        ),
    ))
}

pub fn open(url: &str, max: i64, wait: Duration) -> Result<usize, DbError> {
    let (dialect, first) = connect(url)?;
    let memory = dialect == Dialect::Sqlite && sqlite_path(url) == ":memory:";
    // Each connection to `:memory:` would be a database of its own.
    let max = if memory {
        1
    } else {
        max.clamp(1, 256) as usize
    };
    Ok(POOLS.with(|p| {
        let mut pools = p.borrow_mut();
        pools.push(Pool {
            url: url.to_string(),
            dialect,
            max,
            wait,
            conns: vec![Some(first)],
            tx: vec![None],
            closed: false,
        });
        pools.len() - 1
    }))
}

/// A connection taken from a pool, with its place there.
type Conn = (usize, Box<dyn Driver>);

/// Takes a connection of the pool: the transaction's own, or an idle one,
/// opening a new one or waiting when there is none.
fn checkout(pool: usize, conn: Option<(usize, u64)>) -> Result<Conn, DbError> {
    let start = Instant::now();
    loop {
        let found = POOLS.with(|p| -> Result<Option<Conn>, DbError> {
            let mut pools = p.borrow_mut();
            let Some(pl) = pools.get_mut(pool) else {
                return Err(DbError::new(Kind::Connection, "not an open database"));
            };
            if pl.closed {
                return Err(DbError::new(Kind::Connection, "the database is closed"));
            }
            if let Some((c, tx)) = conn {
                if pl.tx.get(c) != Some(&Some(tx)) {
                    return Err(DbError::new(
                        Kind::Transaction,
                        "the transaction has already ended",
                    ));
                }
                return Ok(pl.conns[c].take().map(|d| (c, d)));
            }
            for i in 0..pl.conns.len() {
                if pl.tx[i].is_none() {
                    if let Some(d) = pl.conns[i].take() {
                        return Ok(Some((i, d)));
                    }
                }
            }
            Ok(None)
        })?;
        if let Some(found) = found {
            return Ok(found);
        }
        // None idle: open another while the pool may grow.
        let (url, grow, wait) = POOLS.with(|p| {
            let pools = p.borrow();
            let pl = &pools[pool];
            (pl.url.clone(), pl.conns.len() < pl.max, pl.wait)
        });
        if grow && conn.is_none() {
            let (_, d) = connect(&url)?;
            let i = POOLS.with(|p| {
                let mut pools = p.borrow_mut();
                let pl = &mut pools[pool];
                pl.conns.push(None);
                pl.tx.push(None);
                pl.conns.len() - 1
            });
            return Ok((i, d));
        }
        if start.elapsed() >= wait {
            return Err(DbError::new(
                Kind::Busy,
                "every connection is in use (raise `max_connections` in `sql.open`)",
            ));
        }
        pause(Duration::from_millis(1));
    }
}

fn checkin(pool: usize, i: usize, d: Box<dyn Driver>) {
    POOLS.with(|p| {
        if let Some(pl) = p.borrow_mut().get_mut(pool) {
            if let Some(slot) = pl.conns.get_mut(i) {
                *slot = Some(d);
            }
        }
    });
}

/// Runs `f` on a connection of `db`. A busy database is tried again until
/// the pool's wait is over, letting other tasks run in between.
pub fn with_conn<T>(
    db: Kv,
    mut f: impl FnMut(&mut dyn Driver) -> Result<T, DbError>,
) -> Result<T, DbError> {
    let (pool, conn) = handle(db);
    let (i, mut d) = checkout(pool, conn)?;
    let wait = POOLS.with(|p| p.borrow()[pool].wait);
    let start = Instant::now();
    let mut delay = Duration::from_micros(200);
    let result = loop {
        match f(d.as_mut()) {
            Err(e) if e.kind == Kind::Busy && start.elapsed() < wait && conn.is_none() => {
                pause(delay);
                delay = (delay * 2).min(Duration::from_millis(20));
            }
            r => break r,
        }
    };
    checkin(pool, i, d);
    result
}

pub fn dialect_of(db: Kv) -> Dialect {
    let (pool, _) = handle(db);
    POOLS.with(|p| {
        p.borrow()
            .get(pool)
            .map_or(Dialect::Sqlite, |pl| pl.dialect)
    })
}

// ----- Kiln values -----

/// The parameters of a query: a `List<sql.Value>`.
unsafe fn params_of(list: Kv) -> Vec<SqlValue> {
    let l = &*ptr::<KList>(list);
    (0..l.len)
        .map(|i| {
            let e = &*ptr::<KEnum>(*l.items.add(i as usize));
            let field = *e.f.as_ptr();
            match e.tag {
                1 => SqlValue::Int(field as i64),
                2 => SqlValue::Float(f64::from_bits(field)),
                3 => SqlValue::Text(String::from_utf8_lossy(bytes(field)).into_owned()),
                4 => SqlValue::Bool(field != 0),
                _ => SqlValue::Null,
            }
        })
        .collect()
}

/// A `sql.Value`.
fn value_kv(v: &SqlValue) -> Kv {
    let (tag, field) = match v {
        SqlValue::Null => return kv(unsafe { k_enum(0, 0, std::ptr::null()) }),
        SqlValue::Int(n) => (1, *n as Kv),
        SqlValue::Float(f) => (2, f.to_bits()),
        SqlValue::Text(s) => (3, string(s.as_bytes())),
        SqlValue::Bool(b) => (4, *b as Kv),
        SqlValue::Blob(b) => (3, string(b)),
    };
    kv(unsafe { k_enum(tag, 1, &field) })
}

/// The SQL value of a Kiln value of the type `d` describes: optionals are
/// NULL or their value, times and dates text in ISO 8601, an enum
/// variant without fields its name, and other values their JSON.
unsafe fn to_sql(d: &KDesc, v: Kv) -> SqlValue {
    match d.kind {
        D_OPT => {
            if v == 0 {
                SqlValue::Null
            } else {
                to_sql(d.part(0), (*ptr::<KBox>(v)).v)
            }
        }
        D_INT | D_DURATION => SqlValue::Int(v as i64),
        D_FLOAT => SqlValue::Float(f64::from_bits(v)),
        D_BOOL => SqlValue::Bool(v != 0),
        D_STR => SqlValue::Text(String::from_utf8_lossy(bytes(v)).into_owned()),
        D_STRUCT if d.name() == "Instant" && d.n == 1 => {
            SqlValue::Text(crate::timefmt::iso(*ptr::<i64>(v)))
        }
        D_STRUCT if d.name() == "Date" && d.n == 3 => {
            let f = ptr::<i64>(v);
            SqlValue::Text(format!("{:04}-{:02}-{:02}", *f, *f.add(1), *f.add(2)))
        }
        D_ENUM => {
            let e = &*ptr::<KEnum>(v);
            let variant = d.part(e.tag);
            if variant.n == 0 {
                SqlValue::Text(variant.name().to_string())
            } else {
                json_text(d, v)
            }
        }
        _ => json_text(d, v),
    }
}

unsafe fn json_text(d: &KDesc, v: Kv) -> SqlValue {
    let s = crate::json::k_json_encode(d, v);
    SqlValue::Text(String::from_utf8_lossy((*s).bytes()).into_owned())
}

/// Reads a column into the type `d` describes. `what` names the column for
/// messages.
unsafe fn from_sql(d: &KDesc, v: &SqlValue, what: &str) -> Result<Kv, String> {
    let mismatch = || {
        format!(
            "cannot read {what}: expected {}, found {}",
            expected(d),
            v.kind()
        )
    };
    match (d.kind, v) {
        (D_OPT, SqlValue::Null) => Ok(0),
        (D_OPT, _) => {
            let inner = from_sql(d.part(0), v, what)?;
            Ok(kv(k_box(inner)))
        }
        (_, SqlValue::Null) => Err(format!(
            "cannot read {what}: it is NULL, so its Kiln type should be optional (`{}?`)",
            expected_type(d)
        )),
        (D_INT | D_DURATION, SqlValue::Int(n)) => Ok(*n as Kv),
        (D_INT | D_DURATION, SqlValue::Bool(b)) => Ok(*b as Kv),
        (D_INT | D_DURATION, SqlValue::Float(f)) if f.fract() == 0.0 && f.abs() < 9.2e18 => {
            Ok(*f as i64 as Kv)
        }
        (D_INT | D_DURATION, SqlValue::Text(t)) => match t.trim().parse::<i64>() {
            // MySQL and PostgreSQL send some integers (sums, counts of
            // decimals) as text.
            Ok(n) if t.trim() == t => Ok(n as Kv),
            _ => Err(mismatch()),
        },
        (D_FLOAT, SqlValue::Float(f)) => Ok(f.to_bits()),
        (D_FLOAT, SqlValue::Int(n)) => Ok((*n as f64).to_bits()),
        (D_FLOAT, SqlValue::Text(t)) => match t.parse::<f64>() {
            Ok(f) => Ok(f.to_bits()),
            Err(_) => Err(mismatch()),
        },
        (D_BOOL, SqlValue::Bool(b)) => Ok(*b as Kv),
        (D_BOOL, SqlValue::Int(0)) => Ok(0),
        (D_BOOL, SqlValue::Int(1)) => Ok(1),
        (D_BOOL, SqlValue::Text(t)) if t == "t" || t == "f" => Ok((t == "t") as Kv),
        (D_STR, SqlValue::Text(t)) => Ok(string(t.as_bytes())),
        (D_STR, SqlValue::Blob(b)) => match std::str::from_utf8(b) {
            Ok(s) => Ok(string(s.as_bytes())),
            Err(_) => Err(format!(
                "cannot read {what}: it is binary data that is not UTF-8 text"
            )),
        },
        (D_STRUCT, SqlValue::Text(t)) if d.name() == "Instant" && d.n == 1 => {
            match crate::timefmt::parse(t) {
                Some(ns) => {
                    let s = slots(1);
                    *s = ns as Kv;
                    Ok(kv(s))
                }
                None => Err(format!("cannot read {what}: {t:?} is not a time")),
            }
        }
        (D_STRUCT, SqlValue::Int(secs)) if d.name() == "Instant" && d.n == 1 => {
            let s = slots(1);
            *s = secs.saturating_mul(1_000_000_000) as Kv;
            Ok(kv(s))
        }
        (D_STRUCT, SqlValue::Text(t)) if d.name() == "Date" && d.n == 3 => {
            match crate::timefmt::date(t) {
                Some((y, m, day)) => {
                    let s = slots(3);
                    *s = y as Kv;
                    *s.add(1) = m as Kv;
                    *s.add(2) = day as Kv;
                    Ok(kv(s))
                }
                None => Err(format!("cannot read {what}: {t:?} is not a date")),
            }
        }
        (D_ENUM, SqlValue::Text(t)) => {
            if let Some(i) = (0..d.n).find(|&i| d.part(i).name() == t) {
                if d.part(i).n == 0 {
                    return Ok(kv(k_enum(i, 0, std::ptr::null())));
                }
            }
            json_value(d, t, what)
        }
        (D_LIST | D_MAP | D_SET | D_STRUCT | D_RECORD, SqlValue::Text(t)) => json_value(d, t, what),
        _ => Err(mismatch()),
    }
}

unsafe fn json_value(d: &KDesc, text: &str, what: &str) -> Result<Kv, String> {
    let mut error: *mut KStr = std::ptr::null_mut();
    let s = string(text.as_bytes());
    let v = crate::json::k_rt_json_decode(d, ptr(s), &mut error);
    if error.is_null() {
        Ok(v)
    } else {
        let msg = String::from_utf8_lossy((*error).bytes()).into_owned();
        Err(format!("cannot read {what}: {msg}"))
    }
}

fn expected(d: &KDesc) -> &'static str {
    match d.kind {
        D_INT => "an integer",
        D_DURATION => "a duration in nanoseconds",
        D_FLOAT => "a number",
        D_BOOL => "a boolean (0 or 1)",
        D_STR => "text",
        D_OPT => "a value or NULL",
        D_STRUCT if d.name() == "Instant" => "a time",
        D_STRUCT if d.name() == "Date" => "a date",
        D_ENUM => "a variant's name",
        _ => "JSON text",
    }
}

fn expected_type(d: &KDesc) -> String {
    match d.kind {
        D_INT => "Int".into(),
        D_FLOAT => "Float".into(),
        D_BOOL => "Bool".into(),
        D_STR => "String".into(),
        D_DURATION => "Duration".into(),
        _ if !d.name().is_empty() => d.name().to_string(),
        _ => "T".into(),
    }
}

/// Whether rows of the type `d` describes are read by column name: a
/// struct or record, other than a time or a date.
fn is_row(d: &KDesc) -> bool {
    matches!(d.kind, D_STRUCT | D_RECORD)
        && !(d.kind == D_STRUCT && matches!(d.name(), "Instant" | "Date"))
}

/// One row as the type `d` describes: a struct or record by column name,
/// anything else from the row's only column.
pub unsafe fn row_kv(d: &KDesc, columns: &[String], row: &[SqlValue]) -> Result<Kv, String> {
    if !is_row(d) {
        if row.len() != 1 {
            return Err(format!(
                "cannot read a {} from a row of {} columns; select one column, or read the row into a struct",
                expected_type(d),
                row.len()
            ));
        }
        let what = format!("column `{}`", columns[0]);
        return from_sql(d, &row[0], &what);
    }
    let s = slots(d.n);
    for i in 0..d.n {
        let label = d.label(i).unwrap_or("");
        let part = d.part(i);
        let found = columns.iter().position(|c| c == label);
        let v = match found {
            Some(c) => from_sql(part, &row[c], &format!("column `{label}`"))?,
            None if part.kind == D_OPT => 0,
            None => {
                return Err(format!(
                    "cannot read the field `{label}`: the query returns no column `{label}` (it returns {})",
                    columns
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            }
        };
        *s.add(i as usize) = v;
    }
    Ok(kv(s))
}

fn report<T>(r: Result<T, DbError>) -> Option<T> {
    match r {
        Ok(v) => Some(v),
        Err(e) => {
            fail(&e.text());
            None
        }
    }
}

fn decode_error(message: String) -> DbError {
    DbError::new(Kind::Decode, message)
}

// ----- the `sql` module -----

/// `sql.open(url, max_connections, busy_timeout)`.
///
/// # Safety
///
/// The arguments are Kiln values of the declared types.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_open(url: Kv, max: Kv, wait: Kv) -> Kv {
    let url = String::from_utf8_lossy(bytes(url)).into_owned();
    let wait = Duration::from_nanos((wait as i64).max(0) as u64);
    report(open(&url, max as i64, wait).map(|p| new_handle(p, None))).unwrap_or(0)
}

/// `sql.__value(value)`: a query parameter.
///
/// # Safety
///
/// `d` describes `v`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_value(d: *const KDesc, v: Kv) -> Kv {
    value_kv(&to_sql(&*d, v))
}

fn sql_text(v: Kv) -> String {
    String::from_utf8_lossy(bytes(v)).into_owned()
}

/// `db.query<T>(sql, args...)`: `d` describes `List<T>`.
///
/// # Safety
///
/// The arguments are Kiln values of the declared types.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_query(d: *const KDesc, db: Kv, sql: Kv, args: Kv) -> Kv {
    let (sql, params) = (sql_text(sql), params_of(args));
    let item = (*d).part(0);
    let r = with_conn(db, |c| c.query(&sql, &params)).and_then(|rows| {
        let list = k_list(0, std::ptr::null());
        for row in &rows.rows {
            let v = row_kv(item, &rows.columns, row).map_err(decode_error)?;
            k_list_push(list, v);
        }
        Ok(kv(list))
    });
    report(r).unwrap_or(0)
}

/// `db.one<T>(sql, args...)`: `d` describes `T?`.
///
/// # Safety
///
/// The arguments are Kiln values of the declared types.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_one(d: *const KDesc, db: Kv, sql: Kv, args: Kv) -> Kv {
    let (sql, params) = (sql_text(sql), params_of(args));
    let item = (*d).part(0);
    let r = with_conn(db, |c| c.query(&sql, &params)).and_then(|rows| match rows.rows.first() {
        None => Ok(0),
        Some(row) => {
            let v = row_kv(item, &rows.columns, row).map_err(decode_error)?;
            Ok(kv(k_box(v)))
        }
    });
    report(r).unwrap_or(0)
}

/// `db.scalar<T>(sql, args...)`: exactly one row of one column.
///
/// # Safety
///
/// The arguments are Kiln values of the declared types.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_scalar(d: *const KDesc, db: Kv, sql: Kv, args: Kv) -> Kv {
    let (sql, params) = (sql_text(sql), params_of(args));
    let r = with_conn(db, |c| c.query(&sql, &params)).and_then(|rows| {
        if rows.rows.len() != 1 || rows.columns.len() != 1 {
            return Err(DbError::new(
                Kind::Decode,
                format!(
                    "`scalar` expects one row of one column, but the query returned {} row(s) of {} column(s)",
                    rows.rows.len(),
                    rows.columns.len()
                ),
            ));
        }
        let what = format!("column `{}`", rows.columns[0]);
        from_sql(&*d, &rows.rows[0][0], &what).map_err(decode_error)
    });
    report(r).unwrap_or(0)
}

/// `db.exec(sql, args...)`: the number of rows changed.
///
/// # Safety
///
/// The arguments are Kiln values of the declared types.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_exec(db: Kv, sql: Kv, args: Kv) -> Kv {
    let (sql, params) = (sql_text(sql), params_of(args));
    report(with_conn(db, |c| c.exec(&sql, &params)).map(|d| d.changed as Kv)).unwrap_or(0)
}

/// `db.insert(sql, args...)`: the key the database generated.
///
/// # Safety
///
/// The arguments are Kiln values of the declared types.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_insert(db: Kv, sql: Kv, args: Kv) -> Kv {
    let (sql, params) = (sql_text(sql), params_of(args));
    let r = with_conn(db, |c| c.exec(&sql, &params)).and_then(|d| {
        d.key.ok_or_else(|| {
            DbError::new(
                Kind::Unsupported,
                "the database reported no generated key; on PostgreSQL end the insert with `RETURNING id`",
            )
        })
    });
    report(r).map_or(0, |k| k as Kv)
}

/// Starts a transaction, returning the handle that runs in it.
///
/// # Safety
///
/// `db` is a `sql.Db`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_begin(db: Kv) -> Kv {
    let (pool, conn) = handle(db);
    if conn.is_some() {
        fail(
            &DbError::new(
                Kind::Transaction,
                "a transaction cannot start inside another; use the handle you have",
            )
            .text(),
        );
        return 0;
    }
    let dialect = dialect_of(db);
    let r = checkout(pool, None).and_then(|(i, mut d)| {
        let begin = match dialect {
            Dialect::Sqlite => "BEGIN IMMEDIATE",
            _ => "BEGIN",
        };
        match d.exec(begin, &[]) {
            Ok(_) => {
                let tx = NEXT_TX.with(|n| {
                    let mut n = n.borrow_mut();
                    *n += 1;
                    *n
                });
                POOLS.with(|p| p.borrow_mut()[pool].tx[i] = Some(tx));
                checkin(pool, i, d);
                Ok(new_handle(pool, Some((i, tx))))
            }
            Err(e) => {
                checkin(pool, i, d);
                Err(e)
            }
        }
    });
    report(r).unwrap_or(0)
}

unsafe fn finish_tx(tx: Kv, sql: &str) -> Result<(), DbError> {
    let (pool, conn) = handle(tx);
    let Some((i, id)) = conn else {
        return Err(DbError::new(Kind::Transaction, "not a transaction"));
    };
    let r = with_conn(tx, |c| c.exec(sql, &[]).map(|_| ()));
    POOLS.with(|p| {
        if let Some(pl) = p.borrow_mut().get_mut(pool) {
            if pl.tx.get(i) == Some(&Some(id)) {
                pl.tx[i] = None;
            }
        }
    });
    r
}

/// Commits a transaction.
///
/// # Safety
///
/// `tx` is a `sql.Db` from `k_rt_sql_begin`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_commit(tx: Kv) -> Kv {
    report(finish_tx(tx, "COMMIT"));
    0
}

/// Rolls a transaction back. It cannot fail: a connection that cannot
/// roll back is closed, which rolls back too.
///
/// # Safety
///
/// `tx` is a `sql.Db` from `k_rt_sql_begin`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_rollback(tx: Kv) -> Kv {
    if finish_tx(tx, "ROLLBACK").is_err() {
        let (pool, conn) = handle(tx);
        if let Some((i, _)) = conn {
            POOLS.with(|p| {
                if let Some(pl) = p.borrow_mut().get_mut(pool) {
                    if let Some(slot) = pl.conns.get_mut(i) {
                        // Reopened on its next use.
                        *slot = None;
                        pl.tx[i] = None;
                    }
                }
            });
            reopen(pool, i);
        }
    }
    0
}

fn reopen(pool: usize, i: usize) {
    let url = POOLS.with(|p| p.borrow()[pool].url.clone());
    if let Ok((_, d)) = connect(&url) {
        checkin(pool, i, d);
    }
}

/// `db.close()`.
///
/// # Safety
///
/// `db` is a `sql.Db`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_close(db: Kv) -> Kv {
    let (pool, _) = handle(db);
    POOLS.with(|p| {
        if let Some(pl) = p.borrow_mut().get_mut(pool) {
            pl.closed = true;
            pl.conns.clear();
            pl.tx.clear();
        }
    });
    0
}

/// `db.dialect()`.
///
/// # Safety
///
/// `db` is a `sql.Db`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_dialect(db: Kv) -> Kv {
    string(dialect_of(db).name().as_bytes())
}

/// `sql.error_kind(err)`: the kind a database error's message ends with,
/// or "" for an error that did not come from a database.
///
/// # Safety
///
/// `err` is an `Error`, whose first field is its message.
#[no_mangle]
pub unsafe extern "C" fn k_rt_sql_error_kind(err: Kv) -> Kv {
    let message = bytes(*ptr::<Kv>(err));
    let text = String::from_utf8_lossy(message);
    let kind = text
        .trim_end()
        .strip_suffix(')')
        .and_then(|t| t.rsplit_once("(sql "))
        .map_or("", |(_, k)| k);
    string(kind.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(sql: &str, n: usize, d: Dialect) -> Result<String, String> {
        let params: Vec<SqlValue> = (0..n).map(|i| SqlValue::Int(i as i64)).collect();
        placeholders(sql, &params, d)
            .map(|(s, _)| s)
            .map_err(|e| e.message)
    }

    #[test]
    fn placeholders_are_rewritten_outside_quotes() {
        assert_eq!(
            rewrite(
                "select * from t where a = $1 and b = '$2' -- $3\n and c = $2",
                2,
                Dialect::Sqlite
            )
            .unwrap(),
            "select * from t where a = ?1 and b = '$2' -- $3\n and c = ?2"
        );
        assert_eq!(
            rewrite("select $2, $1, $2", 2, Dialect::Mysql).unwrap(),
            "select ?, ?, ?"
        );
        let params = vec![SqlValue::Int(1), SqlValue::Int(2)];
        let (_, order) = placeholders("select $2, $1, $2", &params, Dialect::Mysql).unwrap();
        assert_eq!(
            order,
            vec![SqlValue::Int(2), SqlValue::Int(1), SqlValue::Int(2)]
        );
        assert!(rewrite("select $3", 2, Dialect::Postgres).is_err());
        assert!(rewrite("select 1", 1, Dialect::Postgres).is_err());
        assert_eq!(
            rewrite("select 'it''s $1', \"$1\"", 0, Dialect::Sqlite).unwrap(),
            "select 'it''s $1', \"$1\""
        );
    }

    #[test]
    fn passwords_are_redacted() {
        assert_eq!(
            redact("postgres://kiln:secret@db:5432/app"),
            "postgres://kiln:***@db:5432/app"
        );
        assert_eq!(redact("sqlite:./a.db"), "sqlite:./a.db");
    }
}
