//! Database servers: PostgreSQL and MySQL.

use crate::sql::{DbError, Driver, Kind};

pub fn postgres(_url: &str) -> Result<Box<dyn Driver>, DbError> {
    Err(DbError::new(
        Kind::Unsupported,
        "PostgreSQL is not supported yet",
    ))
}

pub fn mysql(_url: &str) -> Result<Box<dyn Driver>, DbError> {
    Err(DbError::new(
        Kind::Unsupported,
        "MySQL is not supported yet",
    ))
}
