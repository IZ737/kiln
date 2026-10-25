//! The `base64` module: standard Base64 (RFC 4648, with padding), which the
//! compiler also uses to carry files such as images inside programs.

use base64::engine::general_purpose::STANDARD;
use base64::Engine;

use crate::value::{bytes, kv, ptr, string, KStr};

/// The bytes of `s` in Base64.
///
/// # Safety
///
/// `s` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_base64_encode(s: *mut KStr) -> *mut KStr {
    ptr(string(STANDARD.encode(bytes(kv(s))).as_bytes()))
}

/// The bytes that `s` encodes, which need not be UTF-8. On failure it
/// returns null and stores the reason in `*error`.
///
/// # Safety
///
/// `s` is a Kiln string, and `error` can be written.
#[no_mangle]
pub unsafe extern "C" fn k_rt_base64_decode(s: *mut KStr, error: *mut *mut KStr) -> *mut KStr {
    match STANDARD.decode(bytes(kv(s))) {
        Ok(data) => ptr(string(&data)),
        Err(e) => {
            *error = ptr(string(format!("cannot read Base64: {e}").as_bytes()));
            std::ptr::null_mut()
        }
    }
}
