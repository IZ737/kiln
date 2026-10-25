//! The `web` server.

use std::ffi::{c_char, CStr};

/// The reason phrase for an HTTP status, or "" for one without a phrase
/// here.
#[no_mangle]
pub extern "C" fn k_web_reason(status: i64) -> *const c_char {
    reason(status).as_ptr()
}

fn reason(status: i64) -> &'static CStr {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasons() {
        assert_eq!(reason(404), c"Not Found");
        assert_eq!(reason(299), c"");
    }
}
