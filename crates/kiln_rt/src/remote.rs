//! Database servers: PostgreSQL and MySQL, through `kiln_dbwire`, whose
//! sockets wait on Kiln's tasks, so other requests go on while a query
//! runs.

use crate::server::Sock;
use crate::sql::{DbError, Dialect, Done, Driver, Kind, Rows, SqlValue};
use kiln_dbwire::{Client, Value};
use std::time::Duration;

extern "C" {
    fn k_task_wait(fd: Sock, write: bool, deadline: i64) -> bool;
    fn k_mono_now() -> i64;
}

fn wait(sock: kiln_dbwire::RawSock, write: bool, limit: Option<Duration>) -> bool {
    let deadline = match limit {
        Some(d) => {
            unsafe { k_mono_now() }.saturating_add(d.as_nanos().min(i64::MAX as u128) as i64)
        }
        None => i64::MAX,
    };
    unsafe { k_task_wait(sock as Sock, write, deadline) }
}

struct Remote {
    client: Box<dyn Client>,
    dialect: Dialect,
}

pub fn postgres(url: &str) -> Result<Box<dyn Driver>, DbError> {
    open(url, Dialect::Postgres)
}

pub fn mysql(url: &str) -> Result<Box<dyn Driver>, DbError> {
    open(url, Dialect::Mysql)
}

fn open(url: &str, dialect: Dialect) -> Result<Box<dyn Driver>, DbError> {
    let client = kiln_dbwire::open(url, Some(wait)).map_err(error)?;
    Ok(Box::new(Remote { client, dialect }))
}

fn error(e: kiln_dbwire::Error) -> DbError {
    use kiln_dbwire::Kind as W;
    let kind = match e.kind {
        W::Connection => Kind::Connection,
        W::Syntax => Kind::Syntax,
        W::Unique => Kind::Unique,
        W::ForeignKey => Kind::ForeignKey,
        W::NotNull => Kind::NotNull,
        W::Check => Kind::Check,
        W::Constraint => Kind::Constraint,
        W::Decode => Kind::Decode,
        W::Busy => Kind::Busy,
        W::Transaction => Kind::Transaction,
        W::Unsupported => Kind::Unsupported,
        W::Other => Kind::Other,
    };
    DbError::new(kind, e.message)
}

fn values(params: &[SqlValue]) -> Vec<Value> {
    params
        .iter()
        .map(|p| match p {
            SqlValue::Null => Value::Null,
            SqlValue::Int(n) => Value::Int(*n),
            SqlValue::Float(f) => Value::Float(*f),
            SqlValue::Text(s) => Value::Text(s.clone()),
            SqlValue::Bool(b) => Value::Bool(*b),
            SqlValue::Blob(b) => Value::Blob(b.clone()),
        })
        .collect()
}

impl Driver for Remote {
    fn query(&mut self, sql: &str, params: &[SqlValue]) -> Result<Rows, DbError> {
        let (sql, params) = crate::sql::placeholders(sql, params, self.dialect)?;
        let rows = self.client.query(&sql, &values(&params)).map_err(error)?;
        Ok(Rows {
            columns: rows.columns,
            rows: rows
                .rows
                .into_iter()
                .map(|r| {
                    r.into_iter()
                        .map(|v| match v {
                            Value::Null => SqlValue::Null,
                            Value::Int(n) => SqlValue::Int(n),
                            Value::Float(f) => SqlValue::Float(f),
                            Value::Text(s) => SqlValue::Text(s),
                            Value::Bool(b) => SqlValue::Bool(b),
                            Value::Blob(b) => SqlValue::Blob(b),
                        })
                        .collect()
                })
                .collect(),
        })
    }

    fn exec(&mut self, sql: &str, params: &[SqlValue]) -> Result<Done, DbError> {
        let (sql, params) = crate::sql::placeholders(sql, params, self.dialect)?;
        let done = self.client.exec(&sql, &values(&params)).map_err(error)?;
        Ok(Done {
            changed: done.changed,
            key: done.key,
        })
    }
}
