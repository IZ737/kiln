//! The requests an integration test sends to the app under test: plain
//! HTTP/1.1 to 127.0.0.1, one connection each, with the test's cookies.
//! Redirects are not followed, so a test sees the `303` it expects.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

pub struct Response {
    pub status: u16,
    /// By lower-case name; a header sent more than once is joined by `, `.
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// Sends one request to `base` (`http://127.0.0.1:port`).
pub fn send(
    base: &str,
    method: &str,
    path: &str,
    headers: &BTreeMap<String, String>,
    body: &str,
    cookies: &mut BTreeMap<String, String>,
    timeout: Duration,
) -> Result<Response, String> {
    let host = base.trim_start_matches("http://");
    let path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    if path.contains(['\r', '\n', ' ']) || method.contains(['\r', '\n', ' ']) {
        return Err("the method and path cannot hold spaces or line breaks".into());
    }
    let mut stream =
        TcpStream::connect(host).map_err(|e| format!("cannot connect to {base}: {e}"))?;
    stream.set_read_timeout(Some(timeout)).ok();
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    let mut has_origin = false;
    for (name, value) in headers {
        if name.contains(['\r', '\n', ':']) || value.contains(['\r', '\n']) {
            return Err(format!("header `{name}` cannot hold line breaks"));
        }
        has_origin |= name.eq_ignore_ascii_case("origin");
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    // Forms a page sends come from its own origin, as a browser's do.
    if !has_origin && method != "GET" && method != "HEAD" {
        head.push_str(&format!("Origin: {base}\r\n"));
    }
    if !cookies.is_empty() {
        let jar: Vec<String> = cookies.iter().map(|(k, v)| format!("{k}={v}")).collect();
        head.push_str(&format!("Cookie: {}\r\n", jar.join("; ")));
    }
    head.push_str(&format!("Content-Length: {}\r\n\r\n", body.len()));
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(body.as_bytes()))
        .map_err(|e| format!("cannot send the request: {e}"))?;
    let mut raw = Vec::new();
    stream
        .read_to_end(&mut raw)
        .map_err(|e| format!("no response from the app: {e}"))?;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("the app sent an incomplete response")?;
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut body = raw[split + 4..].to_vec();
    let mut lines = head.split("\r\n");
    let status = lines
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .and_then(|s| s.parse().ok())
        .ok_or("the app sent no status")?;
    let mut out = BTreeMap::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim().to_string();
        if name == "set-cookie" {
            if let Some((k, v)) = value.split(';').next().and_then(|c| c.split_once('=')) {
                let expired = value.to_ascii_lowercase().contains("max-age=0");
                if expired || v.is_empty() {
                    cookies.remove(k.trim());
                } else {
                    cookies.insert(k.trim().to_string(), v.trim().to_string());
                }
            }
        }
        out.entry(name)
            .and_modify(|v: &mut String| {
                v.push_str(", ");
                v.push_str(&value);
            })
            .or_insert(value);
    }
    if out
        .get("transfer-encoding")
        .is_some_and(|v| v.contains("chunked"))
    {
        body = dechunk(&body);
    }
    Ok(Response {
        status,
        headers: out,
        body: String::from_utf8_lossy(&body).into_owned(),
    })
}

pub fn dechunk(mut data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let Some(end) = data.windows(2).position(|w| w == b"\r\n") else {
            break;
        };
        let size = std::str::from_utf8(&data[..end])
            .ok()
            .and_then(|s| usize::from_str_radix(s.split(';').next()?.trim(), 16).ok())
            .unwrap_or(0);
        data = &data[end + 2..];
        if size == 0 || data.len() < size {
            break;
        }
        out.extend_from_slice(&data[..size]);
        data = &data[(size + 2).min(data.len())..];
    }
    out
}

/// Percent-encodes text as a form does.
pub fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn chunks_and_encoding() {
        assert_eq!(super::dechunk(b"3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n"), b"abcde");
        assert_eq!(super::encode("a b&c=é"), "a+b%26c%3D%C3%A9");
    }
}
