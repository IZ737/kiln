//! `kiln fire`: a release build, and with `--static` a static site.
//!
//! A static export builds the program and runs it with `KILN_EXPORT` set:
//! its `app.listen` then renders every page the site has and writes them
//! to the output folder instead of serving (`k_web_export` in `kiln.c`,
//! and `kiln_rt/src/export.rs`), with a report of what it wrote and could
//! not write. Then this checks the pages: their links, forms and files
//! must point at pages the site has, and no ember on them may call a
//! `server fn`.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use kiln_diagnostics::{codes, Diagnostic, SourceFile, Span};
use kiln_native::Target;

use crate::{build, report_signal, Backend, Emit};

/// Where the project remembers the last static export, so that the next
/// one may replace that folder.
const LAST_EXPORT: &str = ".kiln/fire-static.json";

pub struct Options {
    pub dir: PathBuf,
    pub output: Option<PathBuf>,
    pub backend: Backend,
}

/// `kiln fire`: an executable, as `kiln build` makes it. Phase 7 makes
/// this the optimized release build.
pub fn release(opts: Options) -> ExitCode {
    let target = Target::host();
    let out = opts
        .output
        .unwrap_or_else(|| crate::default_output(&opts.dir, target));
    match build(&opts.dir, &out, Emit::default(), opts.backend, target) {
        Ok(()) => {
            eprintln!("built {}", out.display());
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// `kiln fire --static`.
pub fn static_site(opts: Options) -> ExitCode {
    let project = match crate::project::discover(&opts.dir) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };
    let root = project.root.clone();
    let out = opts.output.unwrap_or_else(|| root.join("dist"));
    if let Err(message) = clear_output(&root, &out) {
        eprintln!("error: {message}");
        return ExitCode::from(2);
    }
    let scratch = match kiln_codegen::scratch_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };
    let result = export(&opts.dir, &root, &out, &scratch, opts.backend);
    let _ = std::fs::remove_dir_all(&scratch);
    result
}

/// Makes `out` an empty folder. A folder that is not empty is replaced
/// only when it is the one the last export of this project wrote.
fn clear_output(root: &Path, out: &Path) -> Result<(), String> {
    let empty = match std::fs::read_dir(out) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) if !out.exists() => true,
        Err(err) => return Err(format!("cannot read {}: {err}", out.display())),
    };
    if !empty {
        let last = std::fs::read_to_string(root.join(LAST_EXPORT))
            .ok()
            .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
            .and_then(|v| v["output"].as_str().map(PathBuf::from));
        let same = last
            .and_then(|l| l.canonicalize().ok())
            .is_some_and(|l| out.canonicalize().is_ok_and(|o| o == l));
        if !same {
            return Err(format!(
                "{} is not empty, and kiln did not write it; remove it or give another `--output`",
                out.display()
            ));
        }
        std::fs::remove_dir_all(out).map_err(|e| format!("cannot remove {}: {e}", out.display()))?;
    }
    std::fs::create_dir_all(out).map_err(|e| format!("cannot create {}: {e}", out.display()))
}

fn export(dir: &Path, root: &Path, out: &Path, scratch: &Path, backend: Backend) -> ExitCode {
    let exe = scratch.join(Target::host().executable("program"));
    let calls = RefCell::new(Vec::new());
    let emit = Emit {
        server_calls: Some(&calls),
        ..Emit::default()
    };
    if let Err(code) = build(dir, &exe, emit, backend, Target::host()) {
        return code;
    }
    let report_path = scratch.join("report.json");
    let out_abs = out.canonicalize().unwrap_or_else(|_| out.to_path_buf());
    let status = std::process::Command::new(&exe)
        .env("KILN_EXPORT", &out_abs)
        .env("KILN_EXPORT_REPORT", &report_path)
        .env_remove("KILN_LISTEN")
        .status();
    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            if s.code().is_none() {
                report_signal(&s);
            }
            eprintln!("error: the program failed before it wrote the site");
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: cannot run the program: {err}");
            return ExitCode::from(2);
        }
    }
    let Some(report) = std::fs::read_to_string(&report_path)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
    else {
        eprintln!(
            "error: the program finished without calling `app.listen`, so there is no site to write"
        );
        return ExitCode::FAILURE;
    };
    let pages: Vec<(String, String)> = report["pages"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            (
                p["path"].as_str().unwrap_or("").to_string(),
                p["file"].as_str().unwrap_or("").to_string(),
            )
        })
        .collect();
    let sources = sources(root);
    let mut errors = 0;
    // Links to the routes these errors are about are not reported again.
    let mut reported = Vec::new();
    for problem in report["problems"].as_array().into_iter().flatten() {
        errors += 1;
        eprintln!("{}", problem_message(problem, &sources));
        reported.push(problem["pattern"].as_str().unwrap_or("").to_string());
    }
    let site = Site::new(&pages);
    let mut calls = calls.into_inner();
    calls.dedup();
    let mut ember_pages: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    // Each missing page once, with the pages that point to it.
    let mut missing: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
    let mut note = |from: &str, link: &Link| {
        if let Some(problem) = site.check(from, link) {
            let key = match &problem {
                Problem::Post(target) => format!("post {target}"),
                Problem::Missing(target) => target.clone(),
            };
            if let Problem::Missing(target) = &problem {
                if reported.iter().any(|p| matches_pattern(p, target)) {
                    return;
                }
            }
            let entry = missing
                .entry(key)
                .or_insert_with(|| (problem.message(link.what), Vec::new()));
            if !entry.1.iter().any(|f| f == from) {
                entry.1.push(from.to_string());
            }
        }
    };
    for (path, file) in &pages {
        let Ok(body) = std::fs::read(out.join(file)) else {
            continue;
        };
        let text = String::from_utf8_lossy(&body);
        if file.ends_with(".html") {
            for link in links(&text) {
                note(path, &link);
            }
            for (name, _) in &calls {
                if text.contains(&format!("data-k-ember=\"{name}\"")) {
                    ember_pages.entry(name).or_default().push(path);
                }
            }
        } else if file.ends_with(".css") {
            for url in css_urls(&text) {
                let link = Link {
                    url,
                    what: "a stylesheet's `url(...)`",
                    post: false,
                };
                note(path, &link);
            }
        }
    }
    for (message, from) in missing.values() {
        errors += 1;
        let shown = if from.len() > 3 {
            format!("{} and {} more", from[..3].join(", "), from.len() - 3)
        } else {
            from.join(", ")
        };
        eprintln!("{message}  = note: on the page {shown}\n");
    }
    for (name, rendered) in &calls {
        if let Some(on) = ember_pages.get(name.as_str()) {
            errors += 1;
            let mut on: Vec<&str> = on.clone();
            on.dedup();
            eprintln!("{rendered}  = note: on the page {}\n", on.join(", "));
        }
    }
    let written: Vec<&str> = pages.iter().map(|(_, f)| f.as_str()).collect();
    let _ = std::fs::create_dir_all(root.join(".kiln"));
    let _ = std::fs::write(
        root.join(LAST_EXPORT),
        serde_json::json!({ "output": out_abs, "files": written }).to_string(),
    );
    if errors > 0 {
        eprintln!(
            "{errors} error(s): the site in {} is not complete. Run `kiln explain <code>` for details.",
            out.display()
        );
        return ExitCode::FAILURE;
    }
    eprintln!(
        "wrote {} file(s) to {}; any static host can serve it",
        pages.len(),
        out.display()
    );
    ExitCode::SUCCESS
}

/// The project's `.kiln` files, to point at the routes problems are about.
fn sources(root: &Path) -> Vec<SourceFile> {
    let Ok(project) = crate::project::discover(root) else {
        return Vec::new();
    };
    project
        .files
        .iter()
        .filter(|p| p.extension().is_some_and(|e| e == "kiln"))
        .filter_map(|p| {
            let text = crate::project::read(p).ok()?;
            Some(SourceFile::new(p.display().to_string(), text))
        })
        .collect()
}

/// The error for a page the program could not write, at its route.
fn problem_message(problem: &serde_json::Value, sources: &[SourceFile]) -> String {
    let get = |k: &str| problem[k].as_str().unwrap_or("").to_string();
    let (pattern, path, kind, message) = (get("pattern"), get("path"), get("kind"), get("message"));
    let (code, text, help): (&str, String, Option<String>) = match kind.as_str() {
        "unlisted" => (
            codes::STATIC_PAGES,
            format!("the route {pattern} has parameters, so a static site has only the pages it lists"),
            Some("list them with `.static(fn() { [...] })`, or leave the route out with `.dynamic()`".into()),
        ),
        "pages" => (
            codes::STATIC_PAGES,
            format!("cannot list the pages of {pattern}: {message}"),
            None,
        ),
        "reads" => (
            codes::STATIC_PAGE,
            format!("the page {path} reads {message}, which a static host gives no page"),
            Some("leave the route out of the static site with `.dynamic()`".into()),
        ),
        "status" => (
            codes::STATIC_PAGE,
            format!("{message}, so it cannot be a page of the site"),
            Some("a page of a static site answers 200 or redirects; leave the route out with `.dynamic()`".into()),
        ),
        "type" => (
            codes::STATIC_PAGE,
            message,
            Some(format!("give its route a file extension, such as `{}.txt`", pattern.trim_end_matches('/'))),
        ),
        _ => (codes::STATIC_PAGE, message, None),
    };
    let quoted = format!("\"{pattern}\"");
    for file in sources {
        if let Some(at) = file.text.find(&quoted) {
            let mut diag =
                Diagnostic::error(code, text.clone(), Span::new(at, at + quoted.len()));
            if let Some(help) = &help {
                diag = diag.with_help(help.clone());
            }
            return diag.render(file);
        }
    }
    let mut out = format!("error[{code}]: {text}\n");
    if let Some(help) = help {
        out.push_str(&format!("  = help: {help}\n"));
    }
    out
}

/// The URL paths the exported site answers.
struct Site {
    paths: BTreeSet<String>,
}

impl Site {
    fn new(pages: &[(String, String)]) -> Site {
        let mut paths = BTreeSet::new();
        for (_, file) in pages {
            paths.insert(format!("/{file}"));
            if let Some(dir) = file.strip_suffix("index.html") {
                paths.insert(format!("/{dir}"));
                if !dir.is_empty() {
                    paths.insert(format!("/{}", dir.trim_end_matches('/')));
                }
            }
        }
        Site { paths }
    }

    /// What is wrong with a link on the page at `from`, if anything.
    fn check(&self, from: &str, link: &Link) -> Option<Problem> {
        let target = resolve(from, &link.url)?;
        if link.post {
            return Some(Problem::Post(target));
        }
        if self.paths.contains(&target) {
            return None;
        }
        Some(Problem::Missing(target))
    }
}

/// A link a static site cannot follow.
enum Problem {
    /// A form that sends a POST here.
    Post(String),
    /// A page or file the site does not have.
    Missing(String),
}

impl Problem {
    fn message(&self, what: &str) -> String {
        match self {
            Problem::Post(target) => format!(
                "error[{}]: a form sends a POST to {target}, which a static host cannot answer\n  = help: leave the page out with `.dynamic()`, or make the form send a GET to a page the site has\n",
                codes::STATIC_LINK
            ),
            Problem::Missing(target) => format!(
                "error[{}]: {what} points to {target}, which the static site does not have\n  = help: list that page with `.static(...)`, or point to one the site has\n",
                codes::STATIC_LINK
            ),
        }
    }
}

/// Whether `path` is one of the pages of the route `pattern`.
fn matches_pattern(pattern: &str, path: &str) -> bool {
    let p: Vec<&str> = pattern.trim_start_matches('/').split('/').collect();
    let s: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    p.len() == s.len()
        && p.iter().zip(&s).all(|(p, s)| {
            (p.starts_with('{') && p.ends_with('}') && !s.is_empty()) || p == s
        })
}

/// A URL on a page, and what it is.
#[derive(Debug, PartialEq)]
struct Link {
    url: String,
    what: &'static str,
    /// A form that sends a POST.
    post: bool,
}

/// The path on this site that `url`, on the page at `from`, points to, or
/// None for another site's, the same page's or one that is not a page.
fn resolve(from: &str, url: &str) -> Option<String> {
    let url = url.trim();
    if url.is_empty() || url.starts_with('#') || url.starts_with('?') || url.starts_with("//") {
        return None;
    }
    let scheme = url
        .split_once(':')
        .is_some_and(|(s, _)| !s.is_empty() && !s.contains(['/', '?', '#']));
    if scheme {
        return None;
    }
    let url = url.split(['#', '?']).next().unwrap_or("");
    let joined = if url.starts_with('/') {
        url.to_string()
    } else {
        let base = &from[..from.rfind('/').map_or(0, |i| i + 1)];
        format!("{base}{url}")
    };
    let mut parts: Vec<&str> = Vec::new();
    let segments: Vec<&str> = joined.split('/').skip(1).collect();
    for (i, seg) in segments.iter().enumerate() {
        match *seg {
            "." => {}
            ".." => {
                parts.pop();
            }
            s if s.is_empty() && i + 1 < segments.len() => {}
            s => parts.push(s),
        }
    }
    let ends_dir = joined.ends_with('/') || joined.ends_with("/.") || joined.ends_with("/..");
    let mut path = format!("/{}", parts.join("/"));
    if ends_dir && !path.ends_with('/') {
        path.push('/');
    }
    Some(percent_decode(&path))
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The links, files and form targets of an HTML page: what `href`, `src`,
/// `srcset`, `action`, `formaction` and `poster` point to, outside
/// comments, scripts and styles.
fn links(html: &str) -> Vec<Link> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(open) = rest.find('<') {
        rest = &rest[open + 1..];
        if let Some(after) = rest.strip_prefix("!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        let name_end = rest
            .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
            .unwrap_or(rest.len());
        let name = rest[..name_end].to_ascii_lowercase();
        if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_alphabetic()) {
            continue;
        }
        let (attrs, after) = attributes(&rest[name_end..]);
        rest = after;
        let get = |n: &str| attrs.iter().find(|(k, _)| k == n).map(|(_, v)| v.as_str());
        for (key, value) in &attrs {
            let what = match key.as_str() {
                "href" => "a link",
                "src" | "poster" => "a file",
                "action" => "a form",
                "formaction" => "a button's form target",
                "srcset" => "a file",
                _ => continue,
            };
            if key == "href" && matches!(name.as_str(), "link") {
                let rel = get("rel").unwrap_or("").to_ascii_lowercase();
                if rel.split_whitespace().any(|r| {
                    matches!(r, "preconnect" | "dns-prefetch" | "canonical" | "alternate")
                }) {
                    continue;
                }
            }
            let post = key == "action"
                && name == "form"
                && get("method").is_some_and(|m| !m.eq_ignore_ascii_case("get"));
            let urls: Vec<String> = if key == "srcset" {
                value
                    .split(',')
                    .filter_map(|c| c.split_whitespace().next())
                    .map(str::to_string)
                    .collect()
            } else {
                vec![value.clone()]
            };
            for url in urls {
                out.push(Link { url, what, post });
            }
        }
        if name == "script" || name == "style" {
            let close = format!("</{name}");
            rest = rest.find(&close).map_or("", |end| &rest[end..]);
        }
    }
    out
}

/// A tag's attributes, with entities in their values decoded, and the text
/// after the tag.
fn attributes(mut s: &str) -> (Vec<(String, String)>, &str) {
    let mut out = Vec::new();
    loop {
        s = s.trim_start_matches(|c: char| c.is_whitespace() || c == '/');
        if s.is_empty() {
            return (out, s);
        }
        if let Some(after) = s.strip_prefix('>') {
            return (out, after);
        }
        let end = s
            .find(|c: char| c.is_whitespace() || c == '=' || c == '>' || c == '/')
            .unwrap_or(s.len())
            .max(1);
        let key = s[..end].to_ascii_lowercase();
        s = s[end..].trim_start();
        let mut value = String::new();
        if let Some(after) = s.strip_prefix('=') {
            s = after.trim_start();
            let (v, after) = match s.chars().next() {
                Some(q @ ('"' | '\'')) => {
                    let body = &s[1..];
                    let close = body.find(q).unwrap_or(body.len());
                    (&body[..close], body.get(close + 1..).unwrap_or(""))
                }
                _ => {
                    let close = s
                        .find(|c: char| c.is_whitespace() || c == '>')
                        .unwrap_or(s.len());
                    (&s[..close], &s[close..])
                }
            };
            value = decode_entities(v);
            s = after;
        }
        out.push((key, value));
    }
}

fn decode_entities(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&#x27;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The URLs of a stylesheet's `url(...)`s.
fn css_urls(css: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = css;
    while let Some(at) = rest.find("url(") {
        rest = &rest[at + 4..];
        let close = rest.find(')').unwrap_or(rest.len());
        let url = rest[..close].trim().trim_matches(['"', '\'']).to_string();
        if !url.starts_with("data:") {
            out.push(url);
        }
        rest = &rest[close..];
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_links() {
        assert_eq!(resolve("/docs/a", "b").as_deref(), Some("/docs/b"));
        assert_eq!(resolve("/docs/", "b").as_deref(), Some("/docs/b"));
        assert_eq!(resolve("/docs/a", "../x/").as_deref(), Some("/x/"));
        assert_eq!(resolve("/", "/a%20b?x#y").as_deref(), Some("/a b"));
        assert_eq!(resolve("/", "https://example.com/"), None);
        assert_eq!(resolve("/", "mailto:a@b.c"), None);
        assert_eq!(resolve("/", "#top"), None);
        assert_eq!(resolve("/", "//cdn.example/x.js"), None);
    }

    #[test]
    fn finds_links() {
        let html = r#"<!-- <a href="/gone"> --><a class=x href="/a?b=1&amp;c=2">a</a>
<img src='/i.png' srcset="/i1.png 1x, /i2.png 2x"><script>let s = "<a href='/no'>"</script>
<form method="post" action="/send"><link rel="canonical" href="https://k/">"#;
        let found: Vec<(String, bool)> = links(html).into_iter().map(|l| (l.url, l.post)).collect();
        assert_eq!(
            found,
            [
                ("/a?b=1&c=2".to_string(), false),
                ("/i.png".to_string(), false),
                ("/i1.png".to_string(), false),
                ("/i2.png".to_string(), false),
                ("/send".to_string(), true),
            ]
        );
    }

    #[test]
    fn site_paths() {
        let site = Site::new(&[
            ("/".into(), "index.html".into()),
            ("/install".into(), "install/index.html".into()),
            ("/llms.txt".into(), "llms.txt".into()),
        ]);
        let link = |url: &str| Link {
            url: url.into(),
            what: "a link",
            post: false,
        };
        assert!(site.check("/", &link("/install")).is_none());
        assert!(site.check("/", &link("/install/")).is_none());
        assert!(site.check("/", &link("llms.txt")).is_none());
        assert!(site.check("/", &link("/index.html")).is_none());
        assert!(site.check("/", &link("/missing")).is_some());
        assert!(matches_pattern("/docs/{page}", "/docs/x"));
        assert!(!matches_pattern("/docs/{page}", "/docs/x/y"));
        assert!(matches_pattern("/search", "/search"));
    }
}
