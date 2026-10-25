//! The `html` module: escaping values for HTML, which rendered glazes call
//! for every value they output, and the check on URLs that glazes put in
//! attributes such as `href`.

use std::borrow::Cow;

use crate::value::{bytes, kv, ptr, string, KStr};

/// The text with `&`, `<`, `>`, `"` and `'` escaped, so that it is safe in
/// HTML text and in a quoted attribute value. Bytes that are not UTF-8
/// become U+FFFD.
///
/// # Safety
///
/// `s` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_html_escape(s: *mut KStr) -> *mut KStr {
    let text = String::from_utf8_lossy(bytes(kv(s)));
    match html_escape::encode_quoted_attribute(&text) {
        // Nothing to change.
        Cow::Borrowed(_) if matches!(text, Cow::Borrowed(_)) => s,
        escaped => ptr(string(escaped.as_bytes())),
    }
}

/// What a URL that is not safe in an attribute becomes: a URL that goes
/// nowhere and says why.
pub const UNSAFE_URL: &str = "about:invalid#kiln-unsafe-url";

/// The URL if it is relative or has a scheme that cannot run code
/// (`http`, `https`, `mailto` or `tel`), otherwise [`UNSAFE_URL`].
///
/// # Safety
///
/// `s` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_html_safe_url(s: *mut KStr) -> *mut KStr {
    if is_safe_url(bytes(kv(s))) {
        s
    } else {
        ptr(string(UNSAFE_URL.as_bytes()))
    }
}

/// Whether a URL is safe to put in an attribute that browsers follow, such
/// as `href` or `src`. The scheme is found as browsers find it (the WHATWG
/// URL standard): leading and trailing spaces and control characters are
/// ignored, and tabs and newlines anywhere are removed, so `java\tscript:`
/// is `javascript:`. A URL without a scheme is relative, and safe.
pub fn is_safe_url(url: &[u8]) -> bool {
    const SAFE: &[&str] = &["http", "https", "mailto", "tel"];
    let trimmed = trim_c0_space(url);
    let mut scheme = Vec::new();
    for &b in trimmed {
        match b {
            b'\t' | b'\n' | b'\r' => continue,
            b':' => {
                let scheme = String::from_utf8_lossy(&scheme).to_ascii_lowercase();
                return SAFE.contains(&scheme.as_str());
            }
            b'a'..=b'z' | b'A'..=b'Z' => scheme.push(b),
            b'0'..=b'9' | b'+' | b'-' | b'.' if !scheme.is_empty() => scheme.push(b),
            // Not a scheme, so the URL is relative: `/a:b`, `?x=1:2`, `#a:b`.
            _ => return true,
        }
    }
    // No `:` at all.
    true
}

/// Without leading and trailing C0 control characters and spaces.
fn trim_c0_space(s: &[u8]) -> &[u8] {
    let is_c0_space = |b: &u8| *b <= b' ';
    let start = s.iter().position(|b| !is_c0_space(b)).unwrap_or(s.len());
    let end = s
        .iter()
        .rposition(|b| !is_c0_space(b))
        .map_or(start, |i| i + 1);
    &s[start..end]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_urls() {
        for url in [
            "/posts/1",
            "posts/1",
            "https://example.com/a?b=c:d",
            "HTTP://example.com",
            "mailto:kiln@example.com",
            "tel:+1-555",
            "?q=a:b",
            "#top",
            "./a:b",
            "",
            "1:2",
            "/javascript:alert(1)",
        ] {
            assert!(is_safe_url(url.as_bytes()), "{url}");
        }
    }

    #[test]
    fn unsafe_urls() {
        for url in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            " javascript:alert(1)",
            "\u{1}javascript:alert(1)",
            "java\tscript:alert(1)",
            "java\nscript:alert(1)",
            "vbscript:msgbox(1)",
            "data:text/html,<script>alert(1)</script>",
            "file:///etc/passwd",
            "x-custom+scheme.1:a",
        ] {
            assert!(!is_safe_url(url.as_bytes()), "{url:?}");
        }
    }
}
