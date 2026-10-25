//! The `web` server's request data and status phrases. The connections
//! are in `server.rs`.

use std::ffi::{c_char, CStr};

use percent_encoding::percent_decode;

use crate::value::{bytes, k_box, kv, string, KBox, KStr};

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
            k_box(string(c.value().as_bytes()))
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
}
