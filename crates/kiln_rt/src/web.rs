//! The `web` server's request data and status phrases. The connections
//! are in `server.rs`.

use std::ffi::{c_char, CStr};

use percent_encoding::{percent_decode, utf8_percent_encode, AsciiSet, CONTROLS};

use crate::value::{bytes, k_box, k_map_set, kv, ptr, string, KBox, KEnum, KMap, KStr, Kv};

extern "C" {
    fn k_panic_c(message: *const c_char) -> !;
}

/// What a cookie value cannot hold as it is (RFC 6265's cookie-octet), and
/// `%`, so that decoding gives back what was encoded.
const COOKIE_VALUE: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b',')
    .add(b';')
    .add(b'\\')
    .add(b'%');

/// Whether `name` can be a cookie's name: an HTTP token.
fn cookie_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || b"!#$&'*+-.^_`|~".contains(&c))
}

/// The `Set-Cookie` header value for `res.set_cookie(...)`. `same_site` is
/// the variant's index in `web.SameSite`: Lax, Strict, None.
fn cookie_header(
    name: &str,
    value: &str,
    max_age: Option<i64>,
    path: &str,
    same_site: i64,
    http_only: bool,
    secure: bool,
) -> String {
    let mut out = format!("{name}={}", utf8_percent_encode(value, COOKIE_VALUE));
    if let Some(seconds) = max_age {
        out.push_str(&format!("; Max-Age={}", seconds.max(0)));
    }
    if !path.is_empty() {
        let path: String = path
            .chars()
            .filter(|c| !matches!(c, ';' | '\r' | '\n'))
            .collect();
        out.push_str(&format!("; Path={path}"));
    }
    out.push_str(match same_site {
        1 => "; SameSite=Strict",
        2 => "; SameSite=None",
        _ => "; SameSite=Lax",
    });
    if http_only {
        out.push_str("; HttpOnly");
    }
    if secure || same_site == 2 {
        out.push_str("; Secure");
    }
    out
}

/// Adds a `set-cookie` header to a response's headers. A response can set
/// several cookies, so they are kept in one entry, a line each, which the
/// server sends as separate headers.
///
/// # Safety
///
/// `res` is a `Response`, whose slots are status, headers, body.
pub unsafe fn add_set_cookie(res: Kv, line: &str) {
    let headers = ptr::<KMap>(*ptr::<Kv>(res).add(1));
    let key = string(b"set-cookie");
    let map = &*headers;
    let mut joined = line.to_string();
    for i in 0..map.used as usize {
        if *map.live.add(i) && bytes(*map.keys.add(i)) == b"set-cookie" {
            let old = String::from_utf8_lossy(bytes(*map.vals.add(i))).into_owned();
            joined = format!("{old}\n{line}");
        }
    }
    k_map_set(headers, key, string(joined.as_bytes()));
}

/// `res.set_cookie(name, value, max_age:, path:, same_site:, http_only:,
/// secure:)`, which returns the response.
///
/// # Safety
///
/// `res` is a `Response`, `name`, `value` and `path` Kiln strings,
/// `max_age` a boxed `Duration` or NULL, and `same_site` a `SameSite`.
#[no_mangle]
pub unsafe extern "C" fn k_rt_web_set_cookie(
    res: Kv,
    name: Kv,
    value: Kv,
    max_age: Kv,
    path: Kv,
    same_site: Kv,
    http_only: Kv,
    secure: Kv,
) -> Kv {
    let name = String::from_utf8_lossy(bytes(name)).into_owned();
    if !cookie_name(name.as_bytes()) {
        k_panic_c(c"a cookie's name can only have letters, digits and !#$&'*+-.^_`|~".as_ptr());
    }
    let value = String::from_utf8_lossy(bytes(value)).into_owned();
    let path = String::from_utf8_lossy(bytes(path)).into_owned();
    let max_age = (max_age != 0).then(|| (*ptr::<KBox>(max_age)).v as i64 / 1_000_000_000);
    let same_site = (*ptr::<KEnum>(same_site)).tag;
    let line = cookie_header(
        &name,
        &value,
        max_age,
        &path,
        same_site,
        http_only != 0,
        secure != 0,
    );
    add_set_cookie(res, &line);
    res
}

/// The reason phrase for an HTTP status, or "" for one without a phrase
/// here.
#[no_mangle]
pub extern "C" fn k_web_reason(status: i64) -> *const c_char {
    reason(status).as_ptr()
}

pub fn reason(status: i64) -> &'static CStr {
    match status {
        100 => c"Continue",
        200 => c"OK",
        201 => c"Created",
        202 => c"Accepted",
        204 => c"No Content",
        301 => c"Moved Permanently",
        302 => c"Found",
        303 => c"See Other",
        304 => c"Not Modified",
        307 => c"Temporary Redirect",
        308 => c"Permanent Redirect",
        400 => c"Bad Request",
        401 => c"Unauthorized",
        403 => c"Forbidden",
        404 => c"Not Found",
        405 => c"Method Not Allowed",
        408 => c"Request Timeout",
        409 => c"Conflict",
        410 => c"Gone",
        411 => c"Length Required",
        413 => c"Content Too Large",
        415 => c"Unsupported Media Type",
        418 => c"I'm a Teapot",
        422 => c"Unprocessable Content",
        429 => c"Too Many Requests",
        431 => c"Request Header Fields Too Large",
        500 => c"Internal Server Error",
        501 => c"Not Implemented",
        502 => c"Bad Gateway",
        503 => c"Service Unavailable",
        _ => c"",
    }
}

/// A path segment with its `%XX` escapes decoded. A `%` not followed by
/// two hex digits stays as it is, and bytes that are not UTF-8 become
/// U+FFFD.
///
/// # Safety
///
/// `s` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_path_decode(s: *mut KStr) -> *mut KStr {
    let text = bytes(kv(s));
    let decoded = percent_decode(text).decode_utf8_lossy();
    crate::value::ptr(string(decoded.as_bytes()))
}

/// The value of `name` in a query string (`a=1&b=two`), decoded as a form
/// is, or NULL.
///
/// # Safety
///
/// Both are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_query(query: *mut KStr, name: *mut KStr) -> *mut KBox {
    let name = bytes(kv(name));
    form_urlencoded::parse(bytes(kv(query)))
        .find(|(key, _)| key.as_bytes() == name)
        .map_or(std::ptr::null_mut(), |(_, value)| unsafe {
            k_box(string(value.as_bytes()))
        })
}

/// The value of the cookie `name` in a `cookie` header, or NULL.
///
/// # Safety
///
/// Both are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_cookie(header: *mut KStr, name: *mut KStr) -> *mut KBox {
    let name = bytes(kv(name));
    let header = String::from_utf8_lossy(bytes(kv(header)));
    cookie::Cookie::split_parse(header.as_ref())
        .flatten()
        .find(|c| c.name().as_bytes() == name)
        .map_or(std::ptr::null_mut(), |c| unsafe {
            let value = percent_decode(c.value().as_bytes()).decode_utf8_lossy();
            k_box(string(value.as_bytes()))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons() {
        assert_eq!(reason(404), c"Not Found");
        assert_eq!(reason(299), c"");
    }

    #[test]
    fn cookies() {
        assert_eq!(
            cookie_header("theme", "dark", None, "/", 0, true, true),
            "theme=dark; Path=/; SameSite=Lax; HttpOnly; Secure"
        );
        assert_eq!(
            cookie_header("n", "a b;c%", Some(60), "", 1, false, false),
            "n=a%20b%3Bc%25; Max-Age=60; SameSite=Strict"
        );
        // `SameSite=None` is only kept by browsers with `Secure`.
        assert!(cookie_header("n", "v", Some(-5), "/", 2, false, false)
            .ends_with("Max-Age=0; Path=/; SameSite=None; Secure"));
        assert!(cookie_name(b"session_id"));
        assert!(!cookie_name(b"a=b") && !cookie_name(b""));
    }
}
