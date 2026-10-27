//! `auth`: passwords, sessions kept in the database, and the identity of
//! a request.
//!
//! A password is stored as an Argon2id hash, never in a form it can be
//! read back from. A session is a long random value the browser keeps in a
//! cookie; the database holds only its SHA-256 digest, so a copy of the
//! table is not a set of usable sessions. Signing in starts a new session
//! and leaves the old one behind, which is what stops a session someone
//! else fixed in the browser from becoming a signed-in one. Every session
//! has a token of its own for forms, so a page on another site cannot post
//! to this one with the visitor's cookie.

use std::time::SystemTime;

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use sha2::{Digest, Sha256};

use crate::sql::{self, DbError, Dialect, SqlValue};
use crate::value::*;

extern "C" {
    /// Fails the library call that runs with an error the server answers
    /// `status` with, as `throw web.error(status, message)` does.
    fn k_rt_web_fail(status: i64, message: *mut KStr);
    fn k_rt_fail(message: *mut KStr);
    fn k_rt_header(request: *mut Kv, name: *mut KStr) -> *mut KStr;
}

/// The table the sessions are in. It is the runtime's own, not one of the
/// program's models, so the migrations do not describe it.
const TABLE: &str = "kiln_sessions";

/// The slots of the handle `auth.sessions` returns.
const S_DB: usize = 0;
const S_COOKIE: usize = 1;
const S_IDLE: usize = 2;
const S_LIFETIME: usize = 3;
const S_SECURE: usize = 4;
const S_SLOTS: i64 = 5;

/// The slots of an `Identity`, in the field order of `std/auth.kiln`.
const I_SUBJECT: usize = 0;
const I_ROLES: usize = 1;
const I_STARTED: usize = 2;

fn fail(e: &DbError) {
    sql::fail(&e.text())
}

unsafe fn text(v: Kv) -> String {
    String::from_utf8_lossy(bytes(v)).into_owned()
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

/// `n` bytes from the system's random numbers, as URL-safe text. Without
/// them there is no safe session, so this is the one place the runtime
/// gives up rather than carrying on.
fn random(n: usize) -> String {
    let mut bytes = vec![0u8; n.clamp(16, 256)];
    if getrandom::fill(&mut bytes).is_err() {
        panic!("the system has no random numbers to make a session with");
    }
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    bytes
        .iter()
        .map(|b| ALPHABET[(*b & 63) as usize] as char)
        .collect()
}

/// The digest a session's row is found by.
fn digest(token: &str) -> String {
    let out = Sha256::digest(token.as_bytes());
    out.iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether two secrets are the same, in a time that does not depend on
/// how much of them matches.
fn same(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// `auth.hash_password(password)`: the Argon2id hash to store.
///
/// # Safety
///
/// `password` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_hash_password(password: Kv) -> Kv {
    // Argon2id with the crate's recommended parameters, and a random
    // salt of its own for every password.
    let hashed: Result<String, _> = Argon2::default()
        .hash_password(bytes(password))
        .map(|h: PasswordHash| h.to_string());
    match hashed {
        Ok(hash) => string(hash.as_bytes()),
        Err(e) => {
            k_rt_fail(ptr(string(
                format!("cannot hash the password: {e}").as_bytes(),
            )));
            0
        }
    }
}

/// `auth.verify_password(password, hash)`: whether the password is the one
/// the hash was made from. A hash this cannot read is not a match.
///
/// # Safety
///
/// Both are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_verify_password(password: Kv, hash: Kv) -> Kv {
    let stored = text(hash);
    let ok = stored.parse::<PasswordHash>().is_ok_and(|parsed| {
        Argon2::default()
            .verify_password(bytes(password), &parsed)
            .is_ok()
    });
    ok as Kv
}

/// `auth.token(bytes)`: a value no one can guess, for a link in an email
/// or an API key.
#[no_mangle]
pub extern "C" fn k_rt_auth_token(n: Kv) -> Kv {
    string(random(n as usize).as_bytes())
}

/// `auth.sha256(text)`: the SHA-256 digest of the text's bytes, in hex.
///
/// # Safety
///
/// `text` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_sha256(text: Kv) -> Kv {
    let digest = Sha256::digest(bytes(text));
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    string(hex.as_bytes())
}

/// `auth.sessions(db, ...)`: sessions kept in `db`. It makes the table
/// when it is not there yet, and nothing else: it never changes a table
/// that is.
///
/// # Safety
///
/// `db` is an open `sql.Db` and `cookie` a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_sessions(
    db: Kv,
    cookie: Kv,
    idle: Kv,
    lifetime: Kv,
    secure: Kv,
) -> Kv {
    // `text` is a key everywhere but MySQL, which needs a length.
    let id = match sql::dialect_of(db) {
        Dialect::Mysql => "varchar(64)",
        _ => "text",
    };
    let ddl = format!(
        "create table if not exists {TABLE} (
            id {id} primary key,
            subject text not null,
            roles text not null,
            csrf text not null,
            started bigint not null,
            seen bigint not null)"
    );
    if let Err(e) = sql::with_conn(db, |c| c.exec(&ddl, &[])) {
        fail(&e);
        return 0;
    }
    let out = slots(S_SLOTS);
    *out.add(S_DB) = db;
    *out.add(S_COOKIE) = cookie;
    *out.add(S_IDLE) = idle;
    *out.add(S_LIFETIME) = lifetime;
    *out.add(S_SECURE) = secure;
    kv(out)
}

/// The settings of a handle.
struct Settings {
    db: Kv,
    cookie: String,
    /// Seconds; 0 is no limit.
    idle: i64,
    lifetime: i64,
    secure: bool,
}

unsafe fn settings(sessions: Kv) -> Settings {
    let s = ptr::<Kv>(sessions);
    Settings {
        db: *s.add(S_DB),
        cookie: text(*s.add(S_COOKIE)),
        idle: (*s.add(S_IDLE) as i64) / 1_000_000_000,
        lifetime: (*s.add(S_LIFETIME) as i64) / 1_000_000_000,
        secure: *s.add(S_SECURE) != 0,
    }
}

/// A session's row.
struct Row {
    subject: String,
    roles: String,
    csrf: String,
    started: i64,
}

/// The cookie the request sent, if any.
unsafe fn sent(req: Kv, name: &str) -> Option<String> {
    let header = k_rt_header(ptr(req), ptr(string(b"cookie")));
    if header.is_null() {
        return None;
    }
    let header = String::from_utf8_lossy((*header).bytes()).into_owned();
    cookie::Cookie::split_parse(header)
        .flatten()
        .find(|c| c.name() == name)
        .map(|c| c.value().to_string())
}

/// The session a request has, if it is still a valid one. A session that
/// has sat unused for longer than `idle`, or started longer ago than
/// `lifetime`, is over, and its row is deleted.
unsafe fn session(s: &Settings, req: Kv) -> Result<Option<Row>, DbError> {
    let Some(token) = sent(req, &s.cookie) else {
        return Ok(None);
    };
    let id = digest(&token);
    let found = sql::with_conn(s.db, |c| {
        c.query(
            &format!("select subject, roles, csrf, started, seen from {TABLE} where id = $1"),
            &[SqlValue::Text(id.clone())],
        )
    })?;
    let Some(row) = found.rows.first() else {
        return Ok(None);
    };
    let field = |i: usize| match &row[i] {
        SqlValue::Text(t) => t.clone(),
        other => format!("{other:?}"),
    };
    let number = |i: usize| match row[i] {
        SqlValue::Int(n) => n,
        _ => 0,
    };
    let (started, seen) = (number(3), number(4));
    let now = now();
    let idle_over = s.idle > 0 && now - seen > s.idle;
    let life_over = s.lifetime > 0 && now - started > s.lifetime;
    if idle_over || life_over {
        let _ = sql::with_conn(s.db, |c| {
            c.exec(
                &format!("delete from {TABLE} where id = $1"),
                &[SqlValue::Text(id.clone())],
            )
        })?;
        return Ok(None);
    }
    // The last time it was used, kept roughly: a write for every request
    // would cost more than it is worth.
    if now - seen > 60 {
        let _ = sql::with_conn(s.db, |c| {
            c.exec(
                &format!("update {TABLE} set seen = $1 where id = $2"),
                &[SqlValue::Int(now), SqlValue::Text(id.clone())],
            )
        })?;
    }
    Ok(Some(Row {
        subject: field(0),
        roles: field(1),
        csrf: field(2),
        started,
    }))
}

/// A session's row as an `Identity`.
unsafe fn identity(row: &Row) -> Kv {
    let roles = k_list(0, std::ptr::null());
    for role in row.roles.split(',').filter(|r| !r.is_empty()) {
        k_list_push(roles, string(role.as_bytes()));
    }
    let started = slots(1);
    *started = (row.started as u64).wrapping_mul(1_000_000_000);
    let out = slots(3);
    *out.add(I_SUBJECT) = string(row.subject.as_bytes());
    *out.add(I_ROLES) = kv(roles);
    *out.add(I_STARTED) = kv(started);
    kv(out)
}

/// The `set-cookie` header for a session, or for clearing one.
fn set_cookie(s: &Settings, value: &str, seconds: i64) -> String {
    let mut out = format!("{}={value}; Path=/; HttpOnly; SameSite=Lax", s.cookie);
    if s.secure {
        out.push_str("; Secure");
    }
    out.push_str(&format!("; Max-Age={seconds}"));
    out
}

/// Puts a `set-cookie` header on a response, and returns it.
unsafe fn with_cookie(res: Kv, value: &str) -> Kv {
    crate::web::add_set_cookie(res, value);
    res
}

/// `sessions.sign_in(res, subject, roles)`.
///
/// # Safety
///
/// `sessions` is a handle, `res` a `Response`, `subject` a Kiln string and
/// `roles` a `List<String>`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_sign_in(sessions: Kv, res: Kv, subject: Kv, roles: Kv) -> Kv {
    let s = settings(sessions);
    // The session the request had, if any, is left behind: a new sign-in
    // is never the session someone else put in the browser.
    let list = &*ptr::<KList>(roles);
    let mut names = Vec::new();
    for i in 0..list.len {
        let role = text(*list.items.add(i as usize));
        if role.contains(',') {
            k_rt_fail(ptr(string(b"a role cannot have a comma in its name")));
            return 0;
        }
        names.push(role);
    }
    let token = random(32);
    let row = (
        digest(&token),
        text(subject),
        names.join(","),
        random(32),
        now(),
    );
    let done = sql::with_conn(s.db, |c| {
        c.exec(
            &format!(
                "insert into {TABLE} (id, subject, roles, csrf, started, seen)
                 values ($1, $2, $3, $4, $5, $5)"
            ),
            &[
                SqlValue::Text(row.0.clone()),
                SqlValue::Text(row.1.clone()),
                SqlValue::Text(row.2.clone()),
                SqlValue::Text(row.3.clone()),
                SqlValue::Int(row.4),
            ],
        )
    });
    if let Err(e) = done {
        fail(&e);
        return 0;
    }
    let life = if s.lifetime > 0 {
        s.lifetime
    } else {
        30 * 86400
    };
    with_cookie(res, &set_cookie(&s, &token, life))
}

/// `sessions.sign_out(req, res)`: ends this request's session.
///
/// # Safety
///
/// `req` is a `Request` and `res` a `Response`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_sign_out(sessions: Kv, req: Kv, res: Kv) -> Kv {
    let s = settings(sessions);
    if let Some(token) = sent(req, &s.cookie) {
        let done = sql::with_conn(s.db, |c| {
            c.exec(
                &format!("delete from {TABLE} where id = $1"),
                &[SqlValue::Text(digest(&token))],
            )
        });
        if let Err(e) = done {
            fail(&e);
            return 0;
        }
    }
    with_cookie(res, &set_cookie(&s, "", 0))
}

/// `sessions.sign_out_all(subject)`: ends every session of a subject,
/// after a password change.
///
/// # Safety
///
/// `subject` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_sign_out_all(sessions: Kv, subject: Kv) -> Kv {
    let s = settings(sessions);
    let done = sql::with_conn(s.db, |c| {
        c.exec(
            &format!("delete from {TABLE} where subject = $1"),
            &[SqlValue::Text(text(subject))],
        )
    });
    match done {
        Ok(done) => done.changed as Kv,
        Err(e) => {
            fail(&e);
            0
        }
    }
}

/// `sessions.sweep()`: deletes the sessions that are over.
///
/// # Safety
///
/// `sessions` is a handle.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_sweep(sessions: Kv) -> Kv {
    let s = settings(sessions);
    let now = now();
    let mut sql_text = format!("delete from {TABLE} where 0 = 1");
    let mut params = Vec::new();
    if s.idle > 0 {
        params.push(SqlValue::Int(now - s.idle));
        sql_text.push_str(&format!(" or seen < ${}", params.len()));
    }
    if s.lifetime > 0 {
        params.push(SqlValue::Int(now - s.lifetime));
        sql_text.push_str(&format!(" or started < ${}", params.len()));
    }
    match sql::with_conn(s.db, |c| c.exec(&sql_text, &params)) {
        Ok(done) => done.changed as Kv,
        Err(e) => {
            fail(&e);
            0
        }
    }
}

/// `sessions.current(req)`: who the request is from, or `none`.
///
/// # Safety
///
/// `req` is a `Request`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_current(sessions: Kv, req: Kv) -> Kv {
    let s = settings(sessions);
    match session(&s, req) {
        Ok(Some(row)) => kv(k_box(identity(&row))),
        Ok(None) => 0,
        Err(e) => {
            fail(&e);
            0
        }
    }
}

/// `sessions.require(req)`: who the request is from, or an error the
/// server answers 401 with.
///
/// # Safety
///
/// `req` is a `Request`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_require(sessions: Kv, req: Kv) -> Kv {
    let s = settings(sessions);
    match session(&s, req) {
        Ok(Some(row)) => identity(&row),
        Ok(None) => {
            k_rt_web_fail(401, ptr(string(b"sign in to see this")));
            0
        }
        Err(e) => {
            fail(&e);
            0
        }
    }
}

/// `sessions.csrf(req)`: the token for a form in this session, or "" when
/// the request has no session.
///
/// # Safety
///
/// `req` is a `Request`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_csrf(sessions: Kv, req: Kv) -> Kv {
    let s = settings(sessions);
    match session(&s, req) {
        Ok(Some(row)) => string(row.csrf.as_bytes()),
        Ok(None) => string(b""),
        Err(e) => {
            fail(&e);
            0
        }
    }
}

/// `sessions.check(req, token)`: lets the request through only when it
/// carries this session's token, and otherwise fails with 403.
///
/// # Safety
///
/// `req` is a `Request` and `token` a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_auth_check(sessions: Kv, req: Kv, token: Kv) -> Kv {
    let s = settings(sessions);
    let found = match session(&s, req) {
        Ok(found) => found,
        Err(e) => {
            fail(&e);
            return 0;
        }
    };
    let sent = text(token);
    let ok = found.is_some_and(|row| !row.csrf.is_empty() && same(&row.csrf, &sent));
    if !ok {
        k_rt_web_fail(
            403,
            ptr(string(b"this form has expired: open the page again")),
        );
    }
    0
}

/// `admin.site(...)`: what the admin's pages work on, as the slots of a
/// `Site` in the field order of `std/admin.kiln`.
///
/// # Safety
///
/// The arguments are the values of those fields.
#[no_mangle]
pub unsafe extern "C" fn k_rt_admin_site(
    db: Kv,
    sessions: Kv,
    authorize: Kv,
    title: Kv,
    home: Kv,
    per_page: Kv,
) -> Kv {
    let out = slots(6);
    for (i, v) in [db, sessions, authorize, title, home, per_page]
        .into_iter()
        .enumerate()
    {
        *out.add(i) = v;
    }
    kv(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_password_verifies_against_its_own_hash_only() {
        let hash: String = Argon2::default()
            .hash_password(b"open sesame")
            .map(|h: PasswordHash| h.to_string())
            .unwrap();
        let parsed: PasswordHash = hash.parse().unwrap();
        assert!(Argon2::default()
            .verify_password(b"open sesame", &parsed)
            .is_ok());
        assert!(Argon2::default()
            .verify_password(b"open sesamf", &parsed)
            .is_err());
        assert!(hash.starts_with("$argon2id$"));
    }

    #[test]
    fn a_token_is_long_and_different_every_time() {
        let a = random(32);
        assert_eq!(a.len(), 32);
        assert!(a
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
        assert_ne!(a, random(32));
    }

    #[test]
    fn a_digest_hides_the_token_it_is_of() {
        let token = random(32);
        let d = digest(&token);
        assert_eq!(d.len(), 64);
        assert!(!d.contains(&token));
        assert_eq!(d, digest(&token));
        assert_ne!(d, digest(&random(32)));
    }

    #[test]
    fn two_secrets_match_only_when_they_are_the_same() {
        assert!(same("abc", "abc"));
        assert!(!same("abc", "abd"));
        assert!(!same("abc", "abcd"));
        assert!(!same("", "a"));
    }
}
