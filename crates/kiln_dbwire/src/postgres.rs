//! PostgreSQL's protocol, version 3, on the messages `postgres-protocol`
//! writes and reads.
//!
//! A statement with values goes through the extended protocol: `Parse`
//! with a type for each value (or none, for text, so the server takes the
//! type the statement needs), `Bind`, `Describe`, `Execute` and `Sync`.
//! Values are sent as text, and binary data in binary. Rows come back as
//! text and are read by their columns' types. A statement without values
//! is a simple query, which may hold several statements.

use crate::{Client, Done, Error, Kind, Rows, Stream, Target, Value, Waiter};
use bytes::BytesMut;
use fallible_iterator::FallibleIterator;
use postgres_protocol::authentication::{self, sasl};
use postgres_protocol::message::backend::{DataRowBody, ErrorResponseBody, Message};
use postgres_protocol::message::frontend;
use postgres_protocol::IsNull;

pub struct Postgres {
    stream: Stream,
    buf: BytesMut,
}

// The type numbers of the values a statement is sent.
const INT8: u32 = 20;
const FLOAT8: u32 = 701;
const BOOL: u32 = 16;
const BYTEA: u32 = 17;

impl Postgres {
    pub fn connect(url: &str, wait: Option<Waiter>) -> Result<Postgres, Error> {
        let target = Target::parse(url, 5432)?;
        if let Some(mode) = target.option("sslmode") {
            if !matches!(mode, "disable" | "allow" | "prefer") {
                return Err(Error::new(
                    Kind::Unsupported,
                    format!("sslmode={mode} asks for TLS, which Kiln does not speak to PostgreSQL yet; use a private network or a local socket proxy"),
                ));
            }
        }
        let stream = Stream::connect(&target.host, target.port, wait)?;
        let mut pg = Postgres {
            stream,
            buf: BytesMut::new(),
        };
        let user = if target.user.is_empty() {
            "postgres".to_string()
        } else {
            target.user.clone()
        };
        let database = if target.database.is_empty() {
            user.clone()
        } else {
            target.database.clone()
        };
        let mut out = BytesMut::new();
        frontend::startup_message(
            [
                ("user", user.as_str()),
                ("database", database.as_str()),
                ("client_encoding", "UTF8"),
                ("application_name", "kiln"),
            ],
            &mut out,
        )
        .map_err(|e| Error::new(Kind::Connection, e.to_string()))?;
        pg.stream.write_all(&out)?;
        pg.sign_in(&user, &target.password)?;
        Ok(pg)
    }

    fn sign_in(&mut self, user: &str, password: &str) -> Result<(), Error> {
        let mut scram: Option<sasl::ScramSha256> = None;
        loop {
            match self.next()? {
                Message::AuthenticationOk => {}
                Message::AuthenticationCleartextPassword => {
                    self.send_password(password.as_bytes())?;
                }
                Message::AuthenticationMd5Password(body) => {
                    let hash =
                        authentication::md5_hash(user.as_bytes(), password.as_bytes(), body.salt());
                    self.send_password(hash.as_bytes())?;
                }
                Message::AuthenticationSasl(body) => {
                    let mut mechanisms = body.mechanisms();
                    let mut scram_ok = false;
                    while let Some(m) = mechanisms.next().map_err(io_error)? {
                        scram_ok |= m == sasl::SCRAM_SHA_256;
                    }
                    if !scram_ok {
                        return Err(Error::new(
                            Kind::Unsupported,
                            "the server asks for a sign-in other than SCRAM-SHA-256",
                        ));
                    }
                    let s = sasl::ScramSha256::new(
                        password.as_bytes(),
                        sasl::ChannelBinding::unsupported(),
                    );
                    let mut out = BytesMut::new();
                    frontend::sasl_initial_response(sasl::SCRAM_SHA_256, s.message(), &mut out)
                        .map_err(io_error)?;
                    self.stream.write_all(&out)?;
                    scram = Some(s);
                }
                Message::AuthenticationSaslContinue(body) => {
                    let s = scram
                        .as_mut()
                        .ok_or_else(|| protocol("SCRAM out of order"))?;
                    s.update(body.data()).map_err(sign_in_failed)?;
                    let mut out = BytesMut::new();
                    frontend::sasl_response(s.message(), &mut out).map_err(io_error)?;
                    self.stream.write_all(&out)?;
                }
                Message::AuthenticationSaslFinal(body) => {
                    let mut s = scram.take().ok_or_else(|| protocol("SCRAM out of order"))?;
                    s.finish(body.data()).map_err(sign_in_failed)?;
                }
                Message::ErrorResponse(body) => {
                    let mut e = server_error(&body);
                    if e.kind == Kind::Other {
                        e.kind = Kind::Connection;
                    }
                    return Err(e);
                }
                Message::ReadyForQuery(_) => return Ok(()),
                Message::AuthenticationKerberosV5
                | Message::AuthenticationScmCredential
                | Message::AuthenticationGss
                | Message::AuthenticationSspi => {
                    return Err(Error::new(
                        Kind::Unsupported,
                        "the server asks for a sign-in Kiln does not speak; use a password",
                    ))
                }
                // Parameter statuses, the key for cancelling, notices.
                _ => {}
            }
        }
    }

    fn send_password(&mut self, password: &[u8]) -> Result<(), Error> {
        let mut out = BytesMut::new();
        frontend::password_message(password, &mut out).map_err(io_error)?;
        self.stream.write_all(&out)
    }

    /// The next message from the server, reading as much as it takes.
    fn next(&mut self) -> Result<Message, Error> {
        loop {
            if let Some(m) = Message::parse(&mut self.buf).map_err(io_error)? {
                return Ok(m);
            }
            let mut chunk = [0u8; 16 * 1024];
            let n = self.stream.read_some(&mut chunk)?;
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }

    /// Runs `sql`, collecting its rows and the count of its last command.
    fn run(&mut self, sql: &str, params: &[Value]) -> Result<(Rows, i64), Error> {
        let mut out = BytesMut::new();
        if params.is_empty() {
            frontend::query(sql, &mut out).map_err(io_error)?;
        } else {
            let types: Vec<u32> = params
                .iter()
                .map(|p| match p {
                    Value::Int(_) => INT8,
                    Value::Float(_) => FLOAT8,
                    Value::Bool(_) => BOOL,
                    Value::Blob(_) => BYTEA,
                    // Text and NULL take the type the statement needs.
                    Value::Text(_) | Value::Null => 0,
                })
                .collect();
            frontend::parse("", sql, types, &mut out).map_err(io_error)?;
            let formats: Vec<i16> = params
                .iter()
                .map(|p| matches!(p, Value::Blob(_)) as i16)
                .collect();
            frontend::bind(
                "",
                "",
                formats,
                params,
                |p, buf| {
                    match p {
                        Value::Null => return Ok(IsNull::Yes),
                        Value::Int(n) => buf.extend_from_slice(n.to_string().as_bytes()),
                        Value::Float(f) => buf.extend_from_slice(float_text(*f).as_bytes()),
                        Value::Bool(b) => buf.extend_from_slice(if *b { b"t" } else { b"f" }),
                        Value::Text(s) => buf.extend_from_slice(s.as_bytes()),
                        Value::Blob(b) => buf.extend_from_slice(b),
                    }
                    Ok(IsNull::No)
                },
                [0i16],
                &mut out,
            )
            .map_err(|_| protocol("a value could not be sent"))?;
            frontend::describe(b'P', "", &mut out).map_err(io_error)?;
            frontend::execute("", 0, &mut out).map_err(io_error)?;
            frontend::sync(&mut out);
        }
        self.stream.write_all(&out)?;

        let mut rows = Rows::default();
        let mut types: Vec<u32> = Vec::new();
        let mut changed = 0;
        let mut failure: Option<Error> = None;
        loop {
            match self.next()? {
                Message::RowDescription(body) => {
                    // A simple query of several statements describes each;
                    // the rows kept are the last statement's.
                    rows = Rows::default();
                    types.clear();
                    let mut fields = body.fields();
                    while let Some(f) = fields.next().map_err(io_error)? {
                        rows.columns.push(f.name().to_string());
                        types.push(f.type_oid());
                    }
                }
                Message::DataRow(body) => {
                    if failure.is_none() {
                        match row(&body, &types) {
                            Ok(r) => rows.rows.push(r),
                            Err(e) => failure = Some(e),
                        }
                    }
                }
                Message::CommandComplete(body) => {
                    let tag = body.tag().map_err(io_error)?;
                    changed = tag
                        .rsplit(' ')
                        .next()
                        .and_then(|n| n.parse().ok())
                        .unwrap_or(0);
                }
                Message::ErrorResponse(body) => {
                    if failure.is_none() {
                        failure = Some(server_error(&body));
                    }
                }
                Message::ReadyForQuery(_) => break,
                _ => {}
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok((rows, changed)),
        }
    }
}

impl Client for Postgres {
    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Rows, Error> {
        self.run(sql, params).map(|(rows, _)| rows)
    }

    fn exec(&mut self, sql: &str, params: &[Value]) -> Result<Done, Error> {
        let (rows, changed) = self.run(sql, params)?;
        // `insert ... returning id` gives the key as its first column.
        let key = rows.rows.first().and_then(|r| match r.first() {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        });
        Ok(Done { changed, key })
    }
}

impl Drop for Postgres {
    fn drop(&mut self) {
        let mut out = BytesMut::new();
        frontend::terminate(&mut out);
        let _ = self.stream.write_all(&out);
    }
}

/// A float as text PostgreSQL reads back to the same value.
fn float_text(f: f64) -> String {
    if f.is_nan() {
        "NaN".into()
    } else if f.is_infinite() {
        if f > 0.0 {
            "Infinity".into()
        } else {
            "-Infinity".into()
        }
    } else {
        format!("{f:?}")
    }
}

/// A row's values, read by their columns' types.
fn row(body: &DataRowBody, types: &[u32]) -> Result<Vec<Value>, Error> {
    let buffer = body.buffer();
    let mut out = Vec::with_capacity(types.len());
    let mut ranges = body.ranges();
    let mut i = 0;
    while let Some(range) = ranges.next().map_err(io_error)? {
        let ty = types.get(i).copied().unwrap_or(0);
        i += 1;
        let Some(range) = range else {
            out.push(Value::Null);
            continue;
        };
        let bytes = &buffer[range];
        let text = || {
            std::str::from_utf8(bytes)
                .map(str::to_string)
                .map_err(|_| Error::new(Kind::Decode, "a column is not UTF-8 text"))
        };
        out.push(match ty {
            // int2, int4, int8, oid
            21 | 23 | 20 | 26 => Value::Int(text()?.parse().map_err(|_| {
                Error::new(Kind::Decode, "a whole number column held something else")
            })?),
            // float4, float8, numeric
            700 | 701 | 1700 => Value::Float(match text()?.as_str() {
                "NaN" => f64::NAN,
                "Infinity" => f64::INFINITY,
                "-Infinity" => f64::NEG_INFINITY,
                t => t
                    .parse()
                    .map_err(|_| Error::new(Kind::Decode, "a number column held something else"))?,
            }),
            BOOL => Value::Bool(bytes == b"t"),
            BYTEA => Value::Blob(hex_bytes(bytes)?),
            _ => Value::Text(text()?),
        });
    }
    Ok(out)
}

/// `bytea` in its text form, `\x` and hex digits.
fn hex_bytes(text: &[u8]) -> Result<Vec<u8>, Error> {
    let bad = || {
        Error::new(
            Kind::Decode,
            "binary data came back in a form Kiln cannot read",
        )
    };
    let digits = text.strip_prefix(b"\\x").ok_or_else(bad)?;
    let value = |c: u8| (c as char).to_digit(16).map(|d| d as u8).ok_or_else(bad);
    digits
        .chunks(2)
        .map(|pair| match pair {
            [h, l] => Ok(value(*h)? << 4 | value(*l)?),
            _ => Err(bad()),
        })
        .collect()
}

/// The server's error, with its kind from its SQLSTATE code.
fn server_error(body: &ErrorResponseBody) -> Error {
    let mut code = String::new();
    let mut message = String::new();
    let mut detail = String::new();
    let mut fields = body.fields();
    while let Ok(Some(f)) = fields.next() {
        let value = String::from_utf8_lossy(f.value_bytes()).into_owned();
        match f.type_() {
            b'C' => code = value,
            b'M' => message = value,
            b'D' => detail = value,
            _ => {}
        }
    }
    // The detail names the values (`Key (email)=(a@b.c) already exists`),
    // and so does a value the server could not read (`invalid input syntax
    // for type integer: "abc"`): an error must not repeat them.
    let _ = detail;
    let kind = kind_of(&code);
    let message = match (kind, message.rfind(": \"")) {
        (Kind::Decode, Some(at)) => message[..at].to_string(),
        _ => message,
    };
    Error::new(kind, message)
}

/// The kind of a SQLSTATE code.
pub fn kind_of(code: &str) -> Kind {
    match code {
        "23505" => Kind::Unique,
        "23503" => Kind::ForeignKey,
        "23502" => Kind::NotNull,
        "23514" => Kind::Check,
        "40001" | "40P01" | "55P03" | "57014" => Kind::Busy,
        "22P02" | "22003" | "22007" | "22008" | "22021" => Kind::Decode,
        "0A000" => Kind::Unsupported,
        c if c.starts_with("23") => Kind::Constraint,
        c if c.starts_with("42") => Kind::Syntax,
        c if c.starts_with("08") || c.starts_with("28") || c.starts_with("57P") || c == "3D000" => {
            Kind::Connection
        }
        c if c.starts_with("25") || c.starts_with("2D") => Kind::Transaction,
        _ => Kind::Other,
    }
}

fn io_error(e: std::io::Error) -> Error {
    Error::new(
        Kind::Connection,
        format!("PostgreSQL's answer could not be read: {e}"),
    )
}

fn protocol(what: &str) -> Error {
    Error::new(
        Kind::Connection,
        format!("PostgreSQL's answer could not be read: {what}"),
    )
}

fn sign_in_failed(e: std::io::Error) -> Error {
    Error::new(
        Kind::Connection,
        format!("signing in to PostgreSQL failed: {e}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_have_kinds() {
        assert_eq!(kind_of("23505"), Kind::Unique);
        assert_eq!(kind_of("23P01"), Kind::Constraint);
        assert_eq!(kind_of("42P01"), Kind::Syntax);
        assert_eq!(kind_of("28P01"), Kind::Connection);
        assert_eq!(kind_of("XX000"), Kind::Other);
    }

    #[test]
    fn bytea_text_is_read() {
        assert_eq!(hex_bytes(b"\\x00ff10").unwrap(), vec![0, 255, 16]);
        assert!(hex_bytes(b"00").is_err());
    }
}
