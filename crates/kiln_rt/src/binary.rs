//! `Bytes`: binary data, such as an image, a download or an upload.
//!
//! A `Bytes` value is held like a string (a `kstr`), but the type system
//! keeps the two apart: a `String` is UTF-8 text, so `bytes.text()` checks
//! that the bytes are before it gives them back as one.

use std::ffi::c_char;

use crate::value::*;

extern "C" {
    fn k_rt_fail(message: *mut KStr);
    fn k_web_response(status: i64, body: *mut KStr, content_type: *const c_char) -> *mut Kv;
}

fn fail(message: &str) {
    unsafe { k_rt_fail(ptr(string(message.as_bytes()))) }
}

/// An I/O error as the C runtime words one: without Rust's "(os error 2)".
fn io_message(what: &str, path: &[u8], e: &std::io::Error) -> String {
    let text = e.to_string();
    let reason = text.split(" (os error").next().unwrap_or(&text);
    format!("cannot {what} {}: {reason}", String::from_utf8_lossy(path))
}

/// `text.bytes()`: the text's UTF-8 bytes, which are already how it is held.
#[no_mangle]
pub extern "C" fn k_rt_string_bytes(s: Kv) -> Kv {
    s
}

/// `data.len()`.
///
/// # Safety
///
/// `data` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_bytes_len(data: Kv) -> Kv {
    bytes(data).len() as Kv
}

/// `data.is_empty()`.
///
/// # Safety
///
/// `data` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_bytes_is_empty(data: Kv) -> Kv {
    bytes(data).is_empty() as Kv
}

/// `data.slice(start, end)`: the bytes from `start` up to `end`, each kept
/// within the data, as `String.slice` does for characters.
///
/// # Safety
///
/// `data` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_bytes_slice(data: Kv, start: Kv, end: Kv) -> Kv {
    let b = bytes(data);
    let at = |i: Kv| (i as i64).clamp(0, b.len() as i64) as usize;
    let (start, end) = (at(start), at(end));
    string(if end > start { &b[start..end] } else { &[] })
}

/// `data.text()`: the bytes as text, failing when they are not UTF-8.
///
/// # Safety
///
/// `data` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_bytes_text(data: Kv) -> Kv {
    match std::str::from_utf8(bytes(data)) {
        Ok(_) => data,
        Err(e) => {
            fail(&format!(
                "the bytes are not UTF-8 text: byte {} is not part of a character",
                e.valid_up_to()
            ));
            0
        }
    }
}

/// `fs.read_bytes(path)`.
///
/// # Safety
///
/// `path` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_fs_read_bytes(path: Kv) -> Kv {
    let p = bytes(path);
    match std::fs::read(String::from_utf8_lossy(p).as_ref()) {
        Ok(data) => string(&data),
        Err(e) => {
            fail(&io_message("read", p, &e));
            0
        }
    }
}

/// `fs.write_bytes(path, data)`.
///
/// # Safety
///
/// `path` and `data` are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_fs_write_bytes(path: Kv, data: Kv) -> Kv {
    let p = bytes(path);
    if let Err(e) = std::fs::write(String::from_utf8_lossy(p).as_ref(), bytes(data)) {
        fail(&io_message("write", p, &e));
    }
    0
}

/// `web.bytes(data, content_type:, status:)`; the lowering adds `headers:`.
///
/// # Safety
///
/// `data` and `content_type` are Kiln strings.
#[no_mangle]
pub unsafe extern "C" fn k_rt_web_bytes(data: Kv, content_type: Kv, status: Kv) -> Kv {
    let mut kind: Vec<u8> = bytes(content_type)
        .iter()
        .copied()
        .filter(|&c| c != 0 && c != b'\r' && c != b'\n')
        .collect();
    kind.push(0);
    kv(k_web_response(
        status as i64,
        ptr(data),
        kind.as_ptr() as *const c_char,
    ))
}

/// `req.bytes()`: the request's body as it came.
///
/// # Safety
///
/// `request` is a `Request`'s slots.
#[no_mangle]
pub unsafe extern "C" fn k_rt_web_request_bytes(request: Kv) -> Kv {
    *ptr::<Kv>(request).add(4)
}
