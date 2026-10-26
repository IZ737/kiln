//! MySQL's and MariaDB's protocol (4.1 and later).
//!
//! A statement with values is prepared, run with its values in the binary
//! protocol, and closed; one without values is a text query, which may
//! hold several statements. Signing in speaks `mysql_native_password` and
//! `caching_sha2_password`; over a connection without TLS the latter's
//! full sign-in sends the password encrypted with the server's RSA key,
//! as MySQL's own clients do.

use crate::{Client, Done, Error, Kind, Rows, Stream, Target, Value, Waiter};
use sha1::Sha1;
use sha2::{Digest, Sha256};

const LONG_PASSWORD: u32 = 1;
const FOUND_ROWS: u32 = 2;
const LONG_FLAG: u32 = 4;
const CONNECT_WITH_DB: u32 = 8;
const PROTOCOL_41: u32 = 0x200;
const TRANSACTIONS: u32 = 0x2000;
const SECURE_CONNECTION: u32 = 0x8000;
const MULTI_STATEMENTS: u32 = 0x1_0000;
const MULTI_RESULTS: u32 = 0x2_0000;
const PS_MULTI_RESULTS: u32 = 0x4_0000;
const PLUGIN_AUTH: u32 = 0x8_0000;
const PLUGIN_AUTH_LENENC: u32 = 0x20_0000;

const MORE_RESULTS: u16 = 0x0008;

// Column types.
const T_DECIMAL: u8 = 0;
const T_TINY: u8 = 1;
const T_SHORT: u8 = 2;
const T_LONG: u8 = 3;
const T_FLOAT: u8 = 4;
const T_DOUBLE: u8 = 5;
const T_NULL: u8 = 6;
const T_TIMESTAMP: u8 = 7;
const T_LONGLONG: u8 = 8;
const T_INT24: u8 = 9;
const T_DATE: u8 = 10;
const T_TIME: u8 = 11;
const T_DATETIME: u8 = 12;
const T_YEAR: u8 = 13;
const T_NEWDECIMAL: u8 = 246;
const T_BLOB: u8 = 252;
const T_VAR_STRING: u8 = 253;

const UNSIGNED: u16 = 32;
const BINARY_CHARSET: u16 = 63;

pub struct Mysql {
    stream: Stream,
    seq: u8,
}

struct Column {
    name: String,
    ty: u8,
    flags: u16,
    charset: u16,
}

impl Mysql {
    pub fn connect(url: &str, wait: Option<Waiter>) -> Result<Mysql, Error> {
        let target = Target::parse(url, 3306)?;
        for key in ["ssl-mode", "sslmode", "ssl"] {
            if let Some(mode) = target.option(key) {
                if !matches!(
                    mode.to_ascii_lowercase().as_str(),
                    "disabled" | "disable" | "false" | "preferred" | "prefer"
                ) {
                    return Err(Error::new(
                        Kind::Unsupported,
                        format!("{key}={mode} asks for TLS, which Kiln does not speak to MySQL yet; use a private network or a local socket proxy"),
                    ));
                }
            }
        }
        let stream = Stream::connect(&target.host, target.port, wait)?;
        let mut db = Mysql { stream, seq: 0 };
        db.sign_in(&target)?;
        // Times are UTC, as Kiln stores them.
        db.exec("set time_zone = '+00:00'", &[])?;
        Ok(db)
    }

    fn sign_in(&mut self, target: &Target) -> Result<(), Error> {
        let hello = self.read()?;
        if hello.first() == Some(&0xff) {
            return Err(connection(server_error(&hello)));
        }
        let mut r = Reader::new(&hello);
        let version = r.u8()?;
        if version != 10 {
            return Err(Error::new(
                Kind::Unsupported,
                "the server speaks a protocol older than MySQL 4.1",
            ));
        }
        r.nul_str()?;
        r.skip(4)?;
        let mut scramble = r.take(8)?.to_vec();
        r.skip(1)?;
        let mut caps = r.u16()? as u32;
        let mut plugin = String::from("mysql_native_password");
        if r.left() > 0 {
            r.skip(1 + 2)?;
            caps |= (r.u16()? as u32) << 16;
            let auth_len = r.u8()? as usize;
            r.skip(10)?;
            if caps & SECURE_CONNECTION != 0 {
                let n = auth_len.saturating_sub(8).max(13);
                let part = r.take(n.min(r.left()))?;
                let part = part.strip_suffix(&[0]).unwrap_or(part);
                scramble.extend_from_slice(part);
            }
            if caps & PLUGIN_AUTH != 0 {
                plugin = r.nul_str().unwrap_or_default();
            }
        }
        if caps & PROTOCOL_41 == 0 {
            return Err(Error::new(
                Kind::Unsupported,
                "the server speaks a protocol older than MySQL 4.1",
            ));
        }
        let mine = LONG_PASSWORD
            | FOUND_ROWS
            | LONG_FLAG
            | PROTOCOL_41
            | TRANSACTIONS
            | SECURE_CONNECTION
            | MULTI_STATEMENTS
            | MULTI_RESULTS
            | PS_MULTI_RESULTS
            | PLUGIN_AUTH
            | PLUGIN_AUTH_LENENC
            | if target.database.is_empty() {
                0
            } else {
                CONNECT_WITH_DB
            };
        let password = target.password.as_bytes();
        let answer = scrambled(&plugin, password, &scramble)?;
        let mut out = Vec::new();
        out.extend_from_slice(&(mine & caps | PROTOCOL_41).to_le_bytes());
        out.extend_from_slice(&(16u32 << 20).to_le_bytes());
        // utf8mb4
        out.push(45);
        out.extend_from_slice(&[0; 23]);
        out.extend_from_slice(target.user.as_bytes());
        out.push(0);
        put_lenenc(&mut out, answer.len() as u64);
        out.extend_from_slice(&answer);
        if !target.database.is_empty() {
            out.extend_from_slice(target.database.as_bytes());
            out.push(0);
        }
        out.extend_from_slice(plugin.as_bytes());
        out.push(0);
        self.write(&out)?;

        loop {
            let reply = self.read()?;
            match reply.first() {
                Some(0x00) => return Ok(()),
                Some(0xff) => return Err(connection(server_error(&reply))),
                // Another sign-in, with its own scramble.
                Some(0xfe) => {
                    let mut r = Reader::new(&reply[1..]);
                    plugin = r.nul_str()?;
                    let data = r.rest();
                    scramble = data.strip_suffix(&[0]).unwrap_or(data).to_vec();
                    let answer = scrambled(&plugin, password, &scramble)?;
                    self.write(&answer)?;
                }
                // `caching_sha2_password`: 3 is signed in from its cache,
                // and 4 asks for the password itself.
                Some(0x01) if reply.get(1) == Some(&3) => {}
                Some(0x01) if reply.get(1) == Some(&4) => {
                    self.write(&[2])?;
                    let key = self.read()?;
                    let pem = key.strip_prefix(&[1]).unwrap_or(&key);
                    let mut plain = password.to_vec();
                    plain.push(0);
                    for (i, b) in plain.iter_mut().enumerate() {
                        *b ^= scramble[i % scramble.len()];
                    }
                    let sealed = rsa_oaep_encrypt(pem, &plain)?;
                    self.write(&sealed)?;
                }
                _ => return Err(protocol("an answer to signing in")),
            }
        }
    }

    // ----- packets -----

    fn read(&mut self) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        loop {
            let mut head = [0u8; 4];
            self.stream.read_exact(&mut head)?;
            let len = u32::from_le_bytes([head[0], head[1], head[2], 0]) as usize;
            self.seq = head[3].wrapping_add(1);
            let start = out.len();
            out.resize(start + len, 0);
            self.stream.read_exact(&mut out[start..])?;
            if len < 0xff_ffff {
                return Ok(out);
            }
        }
    }

    fn write(&mut self, payload: &[u8]) -> Result<(), Error> {
        let mut chunks = payload.chunks(0xff_ffff).peekable();
        let mut data = Vec::with_capacity(payload.len() + 8);
        if payload.is_empty() {
            data.extend_from_slice(&[0, 0, 0, self.seq]);
            self.seq = self.seq.wrapping_add(1);
        }
        while let Some(chunk) = chunks.next() {
            let len = chunk.len() as u32;
            data.extend_from_slice(&len.to_le_bytes()[..3]);
            data.push(self.seq);
            self.seq = self.seq.wrapping_add(1);
            data.extend_from_slice(chunk);
            if chunks.peek().is_none() && chunk.len() == 0xff_ffff {
                data.extend_from_slice(&[0, 0, 0, self.seq]);
                self.seq = self.seq.wrapping_add(1);
            }
        }
        self.stream.write_all(&data)
    }

    fn command(&mut self, code: u8, body: &[u8]) -> Result<(), Error> {
        self.seq = 0;
        let mut payload = Vec::with_capacity(body.len() + 1);
        payload.push(code);
        payload.extend_from_slice(body);
        self.write(&payload)
    }

    fn columns(&mut self, n: usize) -> Result<Vec<Column>, Error> {
        let mut out = Vec::with_capacity(n);
        for _ in 0..n {
            let p = self.read()?;
            let mut r = Reader::new(&p);
            for _ in 0..4 {
                r.lenenc_bytes()?;
            }
            let name = String::from_utf8_lossy(r.lenenc_bytes()?).into_owned();
            r.lenenc_bytes()?;
            r.lenenc_int()?;
            let charset = r.u16()?;
            r.skip(4)?;
            let ty = r.u8()?;
            let flags = r.u16()?;
            out.push(Column {
                name,
                ty,
                flags,
                charset,
            });
        }
        // The end of the definitions.
        if n > 0 {
            self.read()?;
        }
        Ok(out)
    }

    /// Reads one statement's results: the rows of the last result that
    /// had any, the rows the statements changed and the last key made.
    fn results(&mut self, binary: bool) -> Result<(Rows, Done), Error> {
        let mut rows = Rows::default();
        let mut done = Done::default();
        let mut failure = None;
        loop {
            let first = self.read()?;
            let status = match first.first() {
                Some(0x00) => {
                    let mut r = Reader::new(&first[1..]);
                    done.changed += r.lenenc_int()? as i64;
                    let key = r.lenenc_int()?;
                    if key > 0 {
                        done.key = Some(key as i64);
                    }
                    r.u16()?
                }
                Some(0xff) => {
                    // An error ends the statements that follow.
                    failure.get_or_insert(server_error(&first));
                    0
                }
                _ => {
                    let n = Reader::new(&first).lenenc_int()? as usize;
                    let columns = self.columns(n)?;
                    rows = Rows {
                        columns: columns.iter().map(|c| c.name.clone()).collect(),
                        rows: Vec::new(),
                    };
                    let status;
                    loop {
                        let p = self.read()?;
                        if p.first() == Some(&0xfe) && p.len() < 9 {
                            status = if p.len() >= 5 {
                                u16::from_le_bytes([p[3], p[4]])
                            } else {
                                0
                            };
                            break;
                        }
                        if p.first() == Some(&0xff) {
                            failure.get_or_insert(server_error(&p));
                            status = 0;
                            break;
                        }
                        if failure.is_some() {
                            continue;
                        }
                        let row = if binary {
                            binary_row(&p, &columns)
                        } else {
                            text_row(&p, &columns)
                        };
                        match row {
                            Ok(row) => rows.rows.push(row),
                            Err(e) => failure = Some(e),
                        }
                    }
                    status
                }
            };
            if status & MORE_RESULTS == 0 {
                break;
            }
        }
        match failure {
            Some(e) => Err(e),
            None => Ok((rows, done)),
        }
    }

    fn run(&mut self, sql: &str, params: &[Value]) -> Result<(Rows, Done), Error> {
        if params.is_empty() {
            self.command(0x03, sql.as_bytes())?;
            return self.results(false);
        }
        self.command(0x16, sql.as_bytes())?;
        let prepared = self.read()?;
        if prepared.first() == Some(&0xff) {
            return Err(server_error(&prepared));
        }
        let mut r = Reader::new(&prepared[1..]);
        let id = r.u32()?;
        let n_columns = r.u16()? as usize;
        let n_params = r.u16()? as usize;
        self.columns(n_params)?;
        self.columns(n_columns)?;
        let result = if n_params != params.len() {
            Err(Error::new(
                Kind::Syntax,
                format!(
                    "the statement takes {n_params} value(s) but {} were given",
                    params.len()
                ),
            ))
        } else {
            self.execute(id, params)
        };
        // Closing has no answer.
        self.command(0x19, &id.to_le_bytes())?;
        result
    }

    fn execute(&mut self, id: u32, params: &[Value]) -> Result<(Rows, Done), Error> {
        let mut body = Vec::new();
        body.extend_from_slice(&id.to_le_bytes());
        body.push(0);
        body.extend_from_slice(&1u32.to_le_bytes());
        let mut nulls = vec![0u8; params.len().div_ceil(8)];
        for (i, p) in params.iter().enumerate() {
            if *p == Value::Null {
                nulls[i / 8] |= 1 << (i % 8);
            }
        }
        body.extend_from_slice(&nulls);
        body.push(1);
        for p in params {
            let ty = match p {
                Value::Null => T_NULL,
                Value::Int(_) => T_LONGLONG,
                Value::Float(_) => T_DOUBLE,
                Value::Bool(_) => T_TINY,
                Value::Text(_) => T_VAR_STRING,
                Value::Blob(_) => T_BLOB,
            };
            body.extend_from_slice(&[ty, 0]);
        }
        for p in params {
            match p {
                Value::Null => {}
                Value::Int(n) => body.extend_from_slice(&n.to_le_bytes()),
                Value::Float(f) => body.extend_from_slice(&f.to_le_bytes()),
                Value::Bool(b) => body.push(*b as u8),
                Value::Text(s) => {
                    put_lenenc(&mut body, s.len() as u64);
                    body.extend_from_slice(s.as_bytes());
                }
                Value::Blob(b) => {
                    put_lenenc(&mut body, b.len() as u64);
                    body.extend_from_slice(b);
                }
            }
        }
        self.command(0x17, &body)?;
        self.results(true)
    }
}

impl Client for Mysql {
    fn query(&mut self, sql: &str, params: &[Value]) -> Result<Rows, Error> {
        self.run(sql, params).map(|(rows, _)| rows)
    }

    fn exec(&mut self, sql: &str, params: &[Value]) -> Result<Done, Error> {
        self.run(sql, params).map(|(_, done)| done)
    }
}

impl Drop for Mysql {
    fn drop(&mut self) {
        let _ = self.command(0x01, &[]);
    }
}

// ----- values -----

fn text_row(p: &[u8], columns: &[Column]) -> Result<Vec<Value>, Error> {
    let mut r = Reader::new(p);
    let mut out = Vec::with_capacity(columns.len());
    for c in columns {
        if r.peek() == Some(0xfb) {
            r.skip(1)?;
            out.push(Value::Null);
            continue;
        }
        let bytes = r.lenenc_bytes()?;
        out.push(from_text(bytes, c)?);
    }
    Ok(out)
}

fn from_text(bytes: &[u8], c: &Column) -> Result<Value, Error> {
    let text = || {
        std::str::from_utf8(bytes)
            .map_err(|_| Error::new(Kind::Decode, "a column is not UTF-8 text"))
    };
    Ok(match c.ty {
        T_TINY | T_SHORT | T_LONG | T_LONGLONG | T_INT24 | T_YEAR => Value::Int(
            text()?
                .parse()
                .map_err(|_| Error::new(Kind::Decode, "a whole number is too large for an Int"))?,
        ),
        T_FLOAT | T_DOUBLE | T_DECIMAL | T_NEWDECIMAL => Value::Float(
            text()?
                .parse()
                .map_err(|_| Error::new(Kind::Decode, "a number column held something else"))?,
        ),
        _ if c.charset == BINARY_CHARSET && !is_temporal(c.ty) => Value::Blob(bytes.to_vec()),
        _ => Value::Text(text()?.to_string()),
    })
}

fn is_temporal(ty: u8) -> bool {
    matches!(ty, T_DATE | T_TIME | T_DATETIME | T_TIMESTAMP | T_YEAR)
}

fn binary_row(p: &[u8], columns: &[Column]) -> Result<Vec<Value>, Error> {
    let mut r = Reader::new(p);
    r.skip(1)?;
    let nulls = r.take((columns.len() + 9) / 8)?.to_vec();
    let mut out = Vec::with_capacity(columns.len());
    for (i, c) in columns.iter().enumerate() {
        let bit = i + 2;
        if nulls[bit / 8] & (1 << (bit % 8)) != 0 {
            out.push(Value::Null);
            continue;
        }
        let unsigned = c.flags & UNSIGNED != 0;
        let v = match c.ty {
            T_TINY => {
                let b = r.u8()?;
                Value::Int(if unsigned { b as i64 } else { b as i8 as i64 })
            }
            T_SHORT | T_YEAR => {
                let n = r.u16()?;
                Value::Int(if unsigned { n as i64 } else { n as i16 as i64 })
            }
            T_LONG | T_INT24 => {
                let n = r.u32()?;
                Value::Int(if unsigned { n as i64 } else { n as i32 as i64 })
            }
            T_LONGLONG => {
                let n = u64::from_le_bytes(r.take(8)?.try_into().unwrap());
                if unsigned && n > i64::MAX as u64 {
                    return Err(Error::new(
                        Kind::Decode,
                        "a whole number is too large for an Int",
                    ));
                }
                Value::Int(n as i64)
            }
            T_FLOAT => Value::Float(f32::from_le_bytes(r.take(4)?.try_into().unwrap()) as f64),
            T_DOUBLE => Value::Float(f64::from_le_bytes(r.take(8)?.try_into().unwrap())),
            T_DATE | T_DATETIME | T_TIMESTAMP => {
                let n = r.u8()? as usize;
                let b = r.take(n)?;
                let (y, mo, d) = if n >= 4 {
                    (u16::from_le_bytes([b[0], b[1]]), b[2], b[3])
                } else {
                    (0, 0, 0)
                };
                let mut s = format!("{y:04}-{mo:02}-{d:02}");
                if c.ty != T_DATE {
                    let (h, mi, se) = if n >= 7 {
                        (b[4], b[5], b[6])
                    } else {
                        (0, 0, 0)
                    };
                    s.push_str(&format!(" {h:02}:{mi:02}:{se:02}"));
                    if n >= 11 {
                        let micro = u32::from_le_bytes([b[7], b[8], b[9], b[10]]);
                        s.push_str(&format!(".{micro:06}"));
                    }
                }
                Value::Text(s)
            }
            T_TIME => {
                let n = r.u8()? as usize;
                let b = r.take(n)?;
                if n >= 8 {
                    let days = u32::from_le_bytes([b[1], b[2], b[3], b[4]]);
                    let hours = days * 24 + b[5] as u32;
                    let sign = if b[0] == 1 { "-" } else { "" };
                    Value::Text(format!("{sign}{hours:02}:{:02}:{:02}", b[6], b[7]))
                } else {
                    Value::Text("00:00:00".into())
                }
            }
            _ => from_text(r.lenenc_bytes()?, c)?,
        };
        out.push(v);
    }
    Ok(out)
}

// ----- errors -----

fn server_error(p: &[u8]) -> Error {
    let code = if p.len() >= 3 {
        u16::from_le_bytes([p[1], p[2]])
    } else {
        0
    };
    let rest = p.get(3..).unwrap_or(&[]);
    let (state, text) = match rest.strip_prefix(b"#") {
        Some(r) if r.len() >= 5 => (String::from_utf8_lossy(&r[..5]).into_owned(), &r[5..]),
        _ => (String::new(), rest),
    };
    let message = String::from_utf8_lossy(text).into_owned();
    let kind = kind_of(code, &state);
    Error::new(kind, without_values(code, &message))
}

/// The kind of a MySQL error number, or of its SQLSTATE.
pub fn kind_of(code: u16, state: &str) -> Kind {
    match code {
        1062 | 1586 => Kind::Unique,
        1216 | 1217 | 1451 | 1452 => Kind::ForeignKey,
        1048 | 1364 => Kind::NotNull,
        3819 | 4025 => Kind::Check,
        1064 | 1146 | 1054 | 1149 | 1109 | 1060 | 1050 | 1051 => Kind::Syntax,
        1205 | 1213 => Kind::Busy,
        1044 | 1045 | 1049 | 1251 | 2006 | 2013 => Kind::Connection,
        1264 | 1265 | 1292 | 1366 | 1406 => Kind::Decode,
        1235 => Kind::Unsupported,
        _ if state.starts_with("23") => Kind::Constraint,
        _ if state.starts_with("42") => Kind::Syntax,
        _ if state.starts_with("08") || state.starts_with("28") => Kind::Connection,
        _ if state.starts_with("25") => Kind::Transaction,
        _ => Kind::Other,
    }
}

/// The message without the values MySQL quotes in some errors
/// (`Duplicate entry 'ada@example.com' for key ...`): an error is logged
/// and shown, and must not carry what a person typed.
fn without_values(code: u16, message: &str) -> String {
    match code {
        1062 | 1586 => match message.find(" for key ") {
            Some(at) => format!("Duplicate entry{}", &message[at..]),
            None => "Duplicate entry".into(),
        },
        1264 | 1265 | 1292 | 1366 | 1406 => match message.find(" for column ") {
            Some(at) => format!("Incorrect value{}", &message[at..]),
            None => message.to_string(),
        },
        _ => message.to_string(),
    }
}

fn connection(mut e: Error) -> Error {
    if e.kind == Kind::Other || e.kind == Kind::Syntax {
        e.kind = Kind::Connection;
    }
    e
}

fn protocol(what: &str) -> Error {
    Error::new(
        Kind::Connection,
        format!("MySQL's answer could not be read: {what}"),
    )
}

// ----- signing in -----

fn scrambled(plugin: &str, password: &[u8], scramble: &[u8]) -> Result<Vec<u8>, Error> {
    if password.is_empty() {
        return Ok(Vec::new());
    }
    let scramble = &scramble[..scramble.len().min(20)];
    match plugin {
        "mysql_native_password" => {
            // SHA1(password) XOR SHA1(scramble + SHA1(SHA1(password)))
            let one = Sha1::digest(password);
            let two = Sha1::digest(one);
            let mut h = Sha1::new();
            h.update(scramble);
            h.update(two);
            let mask = h.finalize();
            Ok(one.iter().zip(mask.iter()).map(|(a, b)| a ^ b).collect())
        }
        "caching_sha2_password" => {
            // SHA256(password) XOR SHA256(SHA256(SHA256(password)) + scramble)
            let one = Sha256::digest(password);
            let two = Sha256::digest(one);
            let mut h = Sha256::new();
            h.update(two);
            h.update(scramble);
            let mask = h.finalize();
            Ok(one.iter().zip(mask.iter()).map(|(a, b)| a ^ b).collect())
        }
        other => Err(Error::new(
            Kind::Unsupported,
            format!("the server asks for the sign-in `{other}`, which Kiln does not speak; use mysql_native_password or caching_sha2_password"),
        )),
    }
}

/// RSA-OAEP with SHA-1, MySQL's choice, for the key in `pem`.
fn rsa_oaep_encrypt(pem: &[u8], message: &[u8]) -> Result<Vec<u8>, Error> {
    use base64::Engine;
    use num_bigint::BigUint;
    let bad = || {
        Error::new(
            Kind::Connection,
            "the server's public key could not be read",
        )
    };
    let text = std::str::from_utf8(pem).map_err(|_| bad())?;
    let pkcs1 = text.contains("BEGIN RSA PUBLIC KEY");
    let body: String = text
        .lines()
        .filter(|l| !l.starts_with("-----"))
        .collect::<Vec<_>>()
        .join("");
    let der = base64::engine::general_purpose::STANDARD
        .decode(body.trim())
        .map_err(|_| bad())?;
    let mut d = Der(&der);
    let mut key = d.sequence().ok_or_else(bad)?;
    if !pkcs1 {
        // SubjectPublicKeyInfo: the algorithm, then the key in a bit string.
        key.element().ok_or_else(bad)?;
        let bits = key.bit_string().ok_or_else(bad)?;
        key = Der(bits).sequence().ok_or_else(bad)?;
    }
    let n = BigUint::from_bytes_be(key.integer().ok_or_else(bad)?);
    let e = BigUint::from_bytes_be(key.integer().ok_or_else(bad)?);
    let k = n.to_bytes_be().len();
    const H: usize = 20;
    if message.len() + 2 * H + 2 > k {
        return Err(Error::new(
            Kind::Connection,
            "the password is too long for the server's key",
        ));
    }
    let mut db = Vec::with_capacity(k - H - 1);
    db.extend_from_slice(&Sha1::digest(b""));
    db.resize(k - message.len() - H - 2, 0);
    db.push(1);
    db.extend_from_slice(message);
    let mut seed = [0u8; H];
    getrandom::fill(&mut seed).map_err(|_| Error::new(Kind::Connection, "no random numbers"))?;
    for (b, m) in db.iter_mut().zip(mgf1(&seed, k - H - 1)) {
        *b ^= m;
    }
    for (b, m) in seed.iter_mut().zip(mgf1(&db, H)) {
        *b ^= m;
    }
    let mut em = vec![0u8];
    em.extend_from_slice(&seed);
    em.extend_from_slice(&db);
    let c = BigUint::from_bytes_be(&em).modpow(&e, &n).to_bytes_be();
    let mut out = vec![0u8; k - c.len()];
    out.extend_from_slice(&c);
    Ok(out)
}

fn mgf1(seed: &[u8], len: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(len + 20);
    let mut counter = 0u32;
    while out.len() < len {
        let mut h = Sha1::new();
        h.update(seed);
        h.update(counter.to_be_bytes());
        out.extend_from_slice(&h.finalize());
        counter += 1;
    }
    out.truncate(len);
    out
}

/// Just enough DER for an RSA public key.
struct Der<'a>(&'a [u8]);

impl<'a> Der<'a> {
    fn next(&mut self, tag: Option<u8>) -> Option<&'a [u8]> {
        let (&t, rest) = self.0.split_first()?;
        if tag.is_some_and(|want| want != t) {
            return None;
        }
        let (&first, mut rest) = rest.split_first()?;
        let len = if first < 0x80 {
            first as usize
        } else {
            let n = (first & 0x7f) as usize;
            if n > 4 || rest.len() < n {
                return None;
            }
            let len = rest[..n].iter().fold(0usize, |a, &b| a << 8 | b as usize);
            rest = &rest[n..];
            len
        };
        if rest.len() < len {
            return None;
        }
        let (value, after) = rest.split_at(len);
        self.0 = after;
        Some(value)
    }
    fn sequence(&mut self) -> Option<Der<'a>> {
        self.next(Some(0x30)).map(Der)
    }
    fn element(&mut self) -> Option<&'a [u8]> {
        self.next(None)
    }
    fn bit_string(&mut self) -> Option<&'a [u8]> {
        self.next(Some(0x03)).and_then(|b| b.get(1..))
    }
    fn integer(&mut self) -> Option<&'a [u8]> {
        self.next(Some(0x02))
    }
}

// ----- reading and writing fields -----

fn put_lenenc(out: &mut Vec<u8>, n: u64) {
    if n < 251 {
        out.push(n as u8);
    } else if n < 1 << 16 {
        out.push(0xfc);
        out.extend_from_slice(&(n as u16).to_le_bytes());
    } else if n < 1 << 24 {
        out.push(0xfd);
        out.extend_from_slice(&(n as u32).to_le_bytes()[..3]);
    } else {
        out.push(0xfe);
        out.extend_from_slice(&n.to_le_bytes());
    }
}

struct Reader<'a> {
    data: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, at: 0 }
    }
    fn left(&self) -> usize {
        self.data.len() - self.at
    }
    fn peek(&self) -> Option<u8> {
        self.data.get(self.at).copied()
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        if self.left() < n {
            return Err(protocol("a packet ended early"));
        }
        let out = &self.data[self.at..self.at + n];
        self.at += n;
        Ok(out)
    }
    fn skip(&mut self, n: usize) -> Result<(), Error> {
        self.take(n).map(|_| ())
    }
    fn rest(&mut self) -> &'a [u8] {
        let out = &self.data[self.at..];
        self.at = self.data.len();
        out
    }
    fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, Error> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn u32(&mut self) -> Result<u32, Error> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn nul_str(&mut self) -> Result<String, Error> {
        let rest = &self.data[self.at..];
        let end = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or_else(|| protocol("a string has no end"))?;
        self.at += end + 1;
        Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
    }
    fn lenenc_int(&mut self) -> Result<u64, Error> {
        Ok(match self.u8()? {
            n @ 0..=250 => n as u64,
            0xfc => self.u16()? as u64,
            0xfd => {
                let b = self.take(3)?;
                u32::from_le_bytes([b[0], b[1], b[2], 0]) as u64
            }
            0xfe => u64::from_le_bytes(self.take(8)?.try_into().unwrap()),
            _ => return Err(protocol("a length")),
        })
    }
    fn lenenc_bytes(&mut self) -> Result<&'a [u8], Error> {
        let n = self.lenenc_int()? as usize;
        self.take(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_password_scramble() {
        // From MySQL's documentation of the handshake.
        let scramble = b"abcdefghijklmnopqrst";
        let a = scrambled("mysql_native_password", b"secret", scramble).unwrap();
        assert_eq!(a.len(), 20);
        assert!(scrambled("mysql_native_password", b"", scramble)
            .unwrap()
            .is_empty());
        assert_eq!(
            scrambled("caching_sha2_password", b"secret", scramble)
                .unwrap()
                .len(),
            32
        );
        assert!(scrambled("client_ed25519", b"x", scramble).is_err());
    }

    #[test]
    fn values_are_left_out_of_errors() {
        assert_eq!(
            without_values(1062, "Duplicate entry 'ada@example.com' for key 'email'"),
            "Duplicate entry for key 'email'"
        );
        assert_eq!(
            without_values(
                1366,
                "Incorrect integer value: 'abc' for column 'n' at row 1"
            ),
            "Incorrect value for column 'n' at row 1"
        );
    }

    #[test]
    fn lengths_round_trip() {
        for n in [0u64, 250, 251, 65535, 65536, 1 << 24, u64::MAX] {
            let mut out = Vec::new();
            put_lenenc(&mut out, n);
            assert_eq!(Reader::new(&out).lenenc_int().unwrap(), n);
        }
    }

    #[test]
    fn a_public_key_is_read_and_used() {
        // A 1024-bit test key (n, e = 65537) in SubjectPublicKeyInfo.
        let pem = b"-----BEGIN PUBLIC KEY-----
MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQC7JHoJfg6yNzLMOWet8Z49a4KD
0dCspMAYvo2YAMB7/wdEycocujbhJ2n/seONi+5XqTqqFkM5VBl8rmkkFPZk/7x0
xmdsTPECSWnHK+HhoaNDFPR3j8jQhVo1laxiqcEhAHegi5cwtFosuJAvSKAFKEvy
D43si00DQnXWrYHAEQIDAQAB
-----END PUBLIC KEY-----
";
        let sealed = rsa_oaep_encrypt(pem, b"secret\0").unwrap();
        assert_eq!(sealed.len(), 128);
    }
}
