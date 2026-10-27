//! Static sites: the paths of a `.static` route's pages, and the files
//! `kiln fire --static` writes. `k_web_export` in `kiln.c` renders each
//! page and hands it here; at the end this writes the files and a report,
//! which `kiln` reads to check the site and to report what it could not
//! write.

use std::ffi::{c_char, c_int, CStr};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use percent_encoding::{percent_decode_str, utf8_percent_encode, AsciiSet, CONTROLS};
use serde_json::{json, Value};

use crate::value::{bytes, k_list, k_list_push, kv, ptr, string, KList, KMap, KStr, Kv};

/// What a path segment cannot hold as it is: RFC 3986's `pchar` without
/// the characters some hosts treat specially.
const SEGMENT: &AsciiSet = &CONTROLS
    .add(b' ')
    .add(b'"')
    .add(b'#')
    .add(b'%')
    .add(b'/')
    .add(b'<')
    .add(b'>')
    .add(b'?')
    .add(b'[')
    .add(b'\\')
    .add(b']')
    .add(b'^')
    .add(b'`')
    .add(b'{')
    .add(b'|')
    .add(b'}');

/// A route pattern's segments: `Some(name)` for a parameter.
fn segments(pattern: &str) -> Vec<(String, Option<String>)> {
    pattern
        .trim_start_matches('/')
        .split('/')
        .map(|s| {
            if s.len() >= 2 && s.starts_with('{') && s.ends_with('}') {
                let inner = &s[1..s.len() - 1];
                let name = inner.split(':').next().unwrap_or("").trim().to_string();
                (s.to_string(), Some(name))
            } else {
                (s.to_string(), None)
            }
        })
        .collect()
}

/// A parameter's value as a path segment.
fn segment(value: &Value) -> Result<String, String> {
    let text = match value {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        other => return Err(format!("{other} is not a route parameter")),
    };
    if text.is_empty() || text == "." || text == ".." || text.contains(['/', '\\']) {
        return Err(format!("{text:?} cannot be a part of a path"));
    }
    Ok(utf8_percent_encode(&text, SEGMENT).to_string())
}

/// The paths of the pages `items` (a JSON array) lists for `pattern`.
pub fn paths(pattern: &str, items: &str) -> Result<Vec<String>, String> {
    let items: Vec<Value> = serde_json::from_str(items).map_err(|e| e.to_string())?;
    let segs = segments(pattern);
    let params: Vec<&String> = segs.iter().filter_map(|(_, p)| p.as_ref()).collect();
    let mut out = Vec::new();
    for item in &items {
        let mut path = String::new();
        for (text, param) in &segs {
            path.push('/');
            match param {
                None => path.push_str(text),
                Some(name) => {
                    let value = if params.len() == 1 {
                        item
                    } else {
                        item.get(name.as_str()).unwrap_or(&Value::Null)
                    };
                    let seg =
                        segment(value).map_err(|e| format!("the page {item} of {pattern}: {e}"))?;
                    path.push_str(&seg);
                }
            }
        }
        if !out.contains(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

/// The paths of a `.static` route's pages, as Kiln strings, from the JSON
/// of the list its function returned; NULL with `*error` set when one
/// cannot be a path.
///
/// # Safety
///
/// `pattern` and `json` are Kiln strings, and `error` can be written.
#[no_mangle]
pub unsafe extern "C" fn k_rt_static_paths(
    pattern: *mut KStr,
    json: *mut KStr,
    error: *mut *mut KStr,
) -> *mut KList {
    let pattern = String::from_utf8_lossy(bytes(kv(pattern))).into_owned();
    let items = String::from_utf8_lossy(bytes(kv(json))).into_owned();
    match paths(&pattern, &items) {
        Ok(paths) => {
            let list = k_list(0, std::ptr::null());
            for p in paths {
                k_list_push(list, string(p.as_bytes()));
            }
            list
        }
        Err(message) => {
            *error = ptr(string(message.as_bytes()));
            std::ptr::null_mut()
        }
    }
}

/// A page as its route answered it.
struct Page {
    pattern: String,
    path: String,
    status: i64,
    content_type: String,
    location: Option<String>,
    body: Vec<u8>,
    reads: Option<String>,
}

#[derive(Default)]
struct Export {
    pages: Vec<Page>,
    /// Routes that gave no pages: their pattern, a kind and a message.
    problems: Vec<(String, String, String)>,
    not_found: Option<Vec<u8>>,
}

static EXPORT: Mutex<Option<Export>> = Mutex::new(None);

fn with<R>(f: impl FnOnce(&mut Export) -> R) -> R {
    let mut guard = EXPORT.lock().unwrap_or_else(|e| e.into_inner());
    f(guard.get_or_insert_with(Export::default))
}

fn text(v: Kv) -> String {
    String::from_utf8_lossy(bytes(v)).into_owned()
}

/// A response's header, by name in any case.
unsafe fn header(res: *const Kv, name: &str) -> Option<String> {
    let map = &*ptr::<KMap>(*res.add(1));
    for i in 0..map.used as usize {
        if !*map.live.add(i) {
            continue;
        }
        if text(*map.keys.add(i)).eq_ignore_ascii_case(name) {
            return Some(text(*map.vals.add(i)));
        }
    }
    None
}

/// # Safety
///
/// Called once, before the pages.
#[no_mangle]
pub unsafe extern "C" fn k_rt_export_start() {
    with(|e| *e = Export::default());
}

/// A page a route answered: `res` is a `web.Response`, and `reads` says
/// what of the request it read, or is NULL.
///
/// # Safety
///
/// `pattern` and `path` are Kiln strings, `res` a response, and `reads`
/// NULL or a C string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_export_page(
    pattern: *mut KStr,
    path: *mut KStr,
    res: *const Kv,
    reads: *const c_char,
) {
    let page = Page {
        pattern: text(kv(pattern)),
        path: text(kv(path)),
        status: *res as i64,
        content_type: header(res, "content-type").unwrap_or_default(),
        location: header(res, "location"),
        body: bytes(*res.add(2)).to_vec(),
        reads: (!reads.is_null()).then(|| CStr::from_ptr(reads).to_string_lossy().into_owned()),
    };
    with(|e| e.pages.push(page));
}

/// A route that gave no pages: `kind` is `unlisted` (it has parameters and
/// no `.static`) or `pages` (its function failed, with `message`).
///
/// # Safety
///
/// `pattern` and `message` are Kiln strings, `kind` a C string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_export_problem(
    pattern: *mut KStr,
    kind: *const c_char,
    message: *mut KStr,
) {
    let problem = (
        text(kv(pattern)),
        CStr::from_ptr(kind).to_string_lossy().into_owned(),
        text(kv(message)),
    );
    with(|e| e.problems.push(problem));
}

/// The page `app.on_error` makes for 404, written as `404.html`.
///
/// # Safety
///
/// `res` is a response.
#[no_mangle]
pub unsafe extern "C" fn k_rt_export_not_found(res: *const Kv) {
    let body = bytes(*res.add(2)).to_vec();
    with(|e| e.not_found = Some(body));
}

/// The file a page's path is written to, relative to the site's folder:
/// `/` and `/docs/` are folders' `index.html`, and so is another path of
/// an HTML page, so `/install` is `install/index.html`, which every static
/// host serves at `/install` or `/install/`. A file that is not HTML, and
/// an HTML page whose path ends in `.html`, keeps its path. `None` for a
/// path no file can have.
pub fn file_of(path: &str, html: bool) -> Option<String> {
    let decoded = percent_decode_str(path).decode_utf8().ok()?;
    let rel = decoded.trim_start_matches('/');
    let parts: Vec<&str> = rel.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        let last = i == parts.len() - 1;
        if (part.is_empty() && !last)
            || *part == "."
            || *part == ".."
            || part.contains(['\\', ':', '\0', '*', '?', '"', '<', '>', '|'])
        {
            return None;
        }
    }
    let last = parts.last().copied().unwrap_or("");
    let lower = last.to_ascii_lowercase();
    Some(if last.is_empty() {
        format!("{rel}index.html")
    } else if !html || lower.ends_with(".html") || lower.ends_with(".htm") {
        rel.to_string()
    } else {
        format!("{rel}/index.html")
    })
}

/// A page that sends the browser on to `to`, for a redirect.
fn redirect_page(to: &str) -> Vec<u8> {
    let to = html_escape::encode_double_quoted_attribute(to);
    format!(
        "<!doctype html>\n<meta charset=\"utf-8\">\n<title>Redirecting</title>\n\
         <meta http-equiv=\"refresh\" content=\"0; url={to}\">\n\
         <link rel=\"canonical\" href=\"{to}\">\n\
         <p><a href=\"{to}\">Continue</a></p>\n"
    )
    .into_bytes()
}

fn is_html(content_type: &str) -> bool {
    content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .eq_ignore_ascii_case("text/html")
}

/// What to write, and what is wrong, from the pages: `(file, body, path)`
/// for each file, and a problem `(pattern, path, kind, message)` for each
/// page that cannot be written.
#[allow(clippy::type_complexity)]
fn plan(e: &Export) -> (Vec<(String, Vec<u8>, String)>, Vec<Value>) {
    let mut files: Vec<(String, Vec<u8>, String, bool)> = Vec::new();
    let mut problems: Vec<Value> = e
        .problems
        .iter()
        .map(|(pattern, kind, message)| {
            json!({"pattern": pattern, "path": "", "kind": kind, "message": message})
        })
        .collect();
    let problem = |problems: &mut Vec<Value>, p: &Page, kind: &str, message: String| {
        problems
            .push(json!({"pattern": p.pattern, "path": p.path, "kind": kind, "message": message}));
    };
    for p in &e.pages {
        let redirect = matches!(p.status, 301 | 302 | 303 | 307 | 308) && p.location.is_some();
        let Some(file) = file_of(&p.path, redirect || is_html(&p.content_type)) else {
            problem(
                &mut problems,
                p,
                "path",
                format!("{} cannot be a file's path", p.path),
            );
            continue;
        };
        if let Some(reads) = &p.reads {
            problem(&mut problems, p, "reads", reads.clone());
            continue;
        }
        let body = if redirect {
            let to = p.location.clone().unwrap_or_default();
            // `/docs` sending the browser to `/docs/` is what hosts do.
            if to.starts_with('/') && file_of(&to, true).as_deref() == Some(file.as_str()) {
                continue;
            }
            redirect_page(&to)
        } else if p.status == 200 {
            let extension = p.path.rsplit('/').next().is_some_and(|l| l.contains('.'));
            if !is_html(&p.content_type) && !extension {
                problem(
                    &mut problems,
                    p,
                    "type",
                    format!(
                        "{} answers `{}`, but a static host serves a path without a file extension as HTML",
                        p.path, p.content_type
                    ),
                );
                continue;
            }
            p.body.clone()
        } else {
            problem(
                &mut problems,
                p,
                "status",
                format!("{} answered {}", p.path, p.status),
            );
            continue;
        };
        match files.iter_mut().find(|f| f.0 == file) {
            // A page wins over a redirect to it.
            Some(f) if f.3 && !redirect => *f = (file, body, p.path.clone(), redirect),
            Some(f) if !f.3 && !redirect => problem(
                &mut problems,
                p,
                "clash",
                format!("{} and {} are both written to {file}", f.2, p.path),
            ),
            Some(_) => {}
            None => files.push((file, body, p.path.clone(), redirect)),
        }
    }
    if let Some(body) = &e.not_found {
        if !files.iter().any(|f| f.0 == "404.html") {
            files.push(("404.html".into(), body.clone(), "/404.html".into(), false));
        }
    }
    (
        files.into_iter().map(|(f, b, p, _)| (f, b, p)).collect(),
        problems,
    )
}

fn write_all(dir: &Path, files: &[(String, Vec<u8>, String)]) -> std::io::Result<()> {
    for (file, body, _) in files {
        let at: PathBuf = dir.join(file);
        if let Some(parent) = at.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&at, body)?;
    }
    Ok(())
}

/// Writes the pages to `dir`, and the report to `report` when it is not
/// NULL. Returns the program's exit status: 1 when a file could not be
/// written.
///
/// # Safety
///
/// `dir` is a C string, `report` NULL or one.
#[no_mangle]
pub unsafe extern "C" fn k_rt_export_finish(dir: *const c_char, report: *const c_char) -> c_int {
    let dir = PathBuf::from(CStr::from_ptr(dir).to_string_lossy().into_owned());
    let (files, problems) = with(|e| plan(e));
    if let Err(err) = write_all(&dir, &files) {
        eprintln!("error cannot write the site to {}: {err}", dir.display());
        return 1;
    }
    let pages: Vec<Value> = files
        .iter()
        .map(|(file, _, path)| json!({"path": path, "file": file}))
        .collect();
    let doc = json!({"pages": pages, "problems": problems});
    if !report.is_null() {
        let at = CStr::from_ptr(report).to_string_lossy().into_owned();
        if let Err(err) = std::fs::write(&at, doc.to_string()) {
            eprintln!("error cannot write {at}: {err}");
            return 1;
        }
    } else {
        for p in &problems {
            eprintln!("error {}", p["message"].as_str().unwrap_or(""));
        }
        eprintln!("info wrote {} file(s) to {}", files.len(), dir.display());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files() {
        assert_eq!(file_of("/", true).as_deref(), Some("index.html"));
        assert_eq!(file_of("/docs/", true).as_deref(), Some("docs/index.html"));
        assert_eq!(
            file_of("/install", true).as_deref(),
            Some("install/index.html")
        );
        assert_eq!(
            file_of("/phase-6.5", true).as_deref(),
            Some("phase-6.5/index.html")
        );
        assert_eq!(file_of("/old.html", true).as_deref(), Some("old.html"));
        assert_eq!(file_of("/llms.txt", false).as_deref(), Some("llms.txt"));
        assert_eq!(file_of("/a%20b", true).as_deref(), Some("a b/index.html"));
        assert_eq!(file_of("/../x", true), None);
        assert_eq!(file_of("/%2e%2e/x", true), None);
        assert_eq!(file_of("/a//b", true), None);
    }

    #[test]
    fn pages() {
        assert_eq!(
            paths("/posts/{id: Int}", "[1,2,2]").unwrap(),
            ["/posts/1", "/posts/2"]
        );
        assert_eq!(
            paths("/d/{dir}/{page}", r#"[{"dir":"a b","page":"x?"}]"#).unwrap(),
            ["/d/a%20b/x%3F"]
        );
        assert!(paths("/p/{name}", r#"["x/y"]"#).is_err());
        assert!(paths("/p/{name}", r#"[".."]"#).is_err());
    }
}
