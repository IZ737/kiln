//! Kiln's documentation site (`site/`), a Kiln program, from end to end:
//! its generated content matches `docs/`, it builds for Linux and Windows,
//! and when it runs, every page and file it links to is there, the pages
//! have what browsers and screen readers need, the stylesheet has its
//! themes and breakpoints, and what a request puts in a page is escaped.
//! The other theme builds and renders the same pages differently.

use std::collections::{BTreeSet, VecDeque};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-site-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn kiln() -> Command {
    Command::new(env!("CARGO_BIN_EXE_kiln"))
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_dir() {
            copy_dir(&path, &to.join(entry.file_name()));
        } else {
            std::fs::copy(&path, to.join(entry.file_name())).unwrap();
        }
    }
}

#[test]
fn content_is_generated_from_the_docs() {
    // The generator writes `site/content.kiln` from `docs/`, relative to
    // where it runs, so it runs on a copy of both.
    let dir = scratch("content");
    copy_dir(&root().join("docs"), &dir.join("docs"));
    copy_dir(&root().join("site/tools"), &dir.join("site/tools"));
    let out = kiln()
        .args(["run", "site/tools/content"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let made = std::fs::read_to_string(dir.join("site/content.kiln")).unwrap();
    let committed = std::fs::read_to_string(root().join("site/content.kiln")).unwrap();
    assert!(
        made == committed,
        "site/content.kiln is out of date with docs/: run `kiln run site/tools/content` from the repository's root"
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn builds_for_windows() {
    let dir = scratch("windows");
    let out = kiln()
        .args(["build", "--target", "windows", "-o"])
        .arg(dir.join("site.exe"))
        .arg(root().join("site"))
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn every_page_and_file_is_served() {
    let dir = scratch("crawl");
    let exe = build(&root().join("site"), &dir);
    let server = Server::start(&exe);
    let mut queue = VecDeque::from(["/".to_string()]);
    let mut seen = BTreeSet::from(["/".to_string()]);
    let mut pages = 0;
    let mut css = String::new();
    while let Some(path) = queue.pop_front() {
        let r = server.get(&path);
        assert_eq!(r.status, 200, "{path}: {}", r.body);
        let kind = r.header("content-type");
        if path.ends_with(".css") {
            assert_eq!(kind, "text/css; charset=utf-8", "{path}");
            css.push_str(&r.body);
            for url in css_urls(&r.body) {
                if seen.insert(url.clone()) {
                    queue.push_back(url);
                }
            }
            continue;
        }
        if path.ends_with(".svg") {
            assert_eq!(kind, "image/svg+xml", "{path}");
            continue;
        }
        if path.ends_with(".js") {
            // The embers' runtime, which loads their WebAssembly.
            assert_eq!(kind, "text/javascript; charset=utf-8", "{path}");
            let wasm = r.body.split("const KILN_WASM = \"").nth(1).unwrap();
            let wasm = &wasm[..wasm.find('"').unwrap()];
            let w = server.get(wasm);
            assert_eq!(w.status, 200, "{wasm}");
            assert_eq!(w.header("content-type"), "application/wasm", "{wasm}");
            continue;
        }
        if path.ends_with(".png") {
            assert_eq!(kind, "image/png", "{path}");
            continue;
        }
        assert_eq!(kind, "text/html; charset=utf-8", "{path}");
        pages += 1;
        check_page(&path, &r.body);
        for link in links(&r.body) {
            let Some(target) = resolve(&path, &link) else {
                continue;
            };
            if seen.insert(target.clone()) {
                queue.push_back(target);
            }
        }
    }
    // The landing page, install and getting started, and every page of
    // docs/ that the generator carries.
    assert!(pages >= 18, "only {pages} pages: {seen:?}");
    // Without WebAssembly the search box is a form that loads /search.
    let found = server.get("/search?q=glaze");
    assert_eq!(found.status, 200);
    check_page("/search", &found.body);
    assert!(
        found.body.contains("href=\"/docs/reference/glazes\""),
        "{}",
        found.body
    );
    let none = server.get("/search?q=%3Cscript%3Exyzzy");
    assert!(
        none.body.contains("No pages match “&lt;script&gt;xyzzy”"),
        "{}",
        none.body
    );
    assert!(css.contains(".theme-forge{"), "{css}");
    assert!(css.contains(".theme-paper{"), "{css}");
    assert!(css.contains("@media (min-width:900px)"), "{css}");
    assert!(css.contains("@layer kiln.global,kiln.modules,kiln.scoped,kiln.ui"));

    // Files from the program are cached; their names change with them.
    let page = server.get("/");
    let sheet = attribute_values(&page.body, "href")
        .into_iter()
        .find(|h| h.starts_with("/_kiln/") && h.ends_with(".css"))
        .unwrap();
    let r = server.get(&sheet);
    assert!(
        r.header("cache-control").contains("immutable"),
        "{}",
        r.headers
    );

    // Unknown pages are 404, in the site's own page, and what the request
    // put in the message is escaped.
    let r = server.get("/docs/%3Cscript%3Ealert(1)%3C%2Fscript%3E");
    assert_eq!(r.status, 404);
    assert!(r.body.contains("This page is not here"));
    assert!(!r.body.contains("<script>alert(1)"), "{}", r.body);
    assert_eq!(server.get("/docs").status, 301);
    server.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_other_theme_restyles_the_same_pages() {
    let dir = scratch("theme");
    let forge = build(&root().join("site"), &dir);
    let paper_src = dir.join("paper");
    copy_dir(&root().join("site"), &paper_src);
    let theme = paper_src.join("theme.kiln");
    let text = std::fs::read_to_string(&theme).unwrap();
    assert!(text.contains("    return forge()\n"));
    std::fs::write(
        &theme,
        text.replace("    return forge()\n", "    return paper()\n"),
    )
    .unwrap();
    let paper = build(&paper_src, &dir);
    let (a, b) = (Server::start(&forge), Server::start(&paper));
    for path in ["/", "/docs/reference/glazes"] {
        let (x, y) = (a.get(path).body, b.get(path).body);
        assert!(x.contains("class=\"theme-forge "), "{x}");
        assert!(y.contains("class=\"theme-paper "), "{y}");
        // The same text, differently styled.
        assert_eq!(text_of(&x), text_of(&y), "{path}");
        assert_ne!(x, y);
    }
    // Forge lays the page out in a grid with a sidebar, Paper in a column.
    let frame = |s: &Server| {
        let body = s.get("/").body;
        let at = body.find("<nav id=\"site-nav\"").unwrap();
        let start = body[..at].rfind("<div").unwrap();
        let classes = attribute_values(&body[start..at], "class").remove(0);
        let css = stylesheet(s, &body);
        classes
            .split(' ')
            .filter_map(|c| rule(&css, c))
            .collect::<Vec<_>>()
    };
    let forge_frame = frame(&a);
    let paper_frame = frame(&b);
    assert!(
        forge_frame.contains(&"display:grid".to_string()),
        "{forge_frame:?}"
    );
    assert!(
        paper_frame.contains(&"display:flex".to_string()),
        "{paper_frame:?}"
    );
    assert!(paper_frame.contains(&"flex-direction:column".to_string()));
    a.stop();
    b.stop();
    std::fs::remove_dir_all(&dir).unwrap();
}

/// What a page must have: a language, a title, one `<h1>`, images with
/// text alternatives, and no scripts but the embers' runtime.
fn check_page(path: &str, html: &str) {
    assert!(html.starts_with("<!doctype html>"), "{path}");
    assert!(html.contains("<html lang=\"en\">"), "{path}");
    assert!(html.contains("<meta name=\"viewport\""), "{path}");
    assert!(
        html.contains("<title>") && html.contains(" · Kiln</title>"),
        "{path}"
    );
    assert_eq!(html.matches("<h1").count(), 1, "{path}: one <h1>");
    assert!(html.contains("<main id=\"content\""), "{path}");
    assert!(
        html.contains("href=\"#content\""),
        "{path}: a link past the navigation"
    );
    for img in html.split("<img").skip(1) {
        let tag = &img[..img.find('>').unwrap()];
        assert!(tag.contains(" alt=\""), "{path}: <img{tag}>");
    }
    // The only script is the embers' runtime: the search box and copy
    // buttons are embers, and the rest is HTML and CSS.
    for script in html.split("<script").skip(1) {
        assert!(
            script.starts_with(" type=\"module\" src=\"/_kiln/ember-"),
            "{path}: <script{}",
            &script[..script.find('>').unwrap_or(script.len())]
        );
    }
    assert_eq!(html.matches("<script").count(), 1, "{path}: one script");
}

fn build(program: &Path, dir: &Path) -> PathBuf {
    let exe = dir.join(format!(
        "{}-site{}",
        program.file_name().unwrap().to_string_lossy(),
        std::env::consts::EXE_SUFFIX
    ));
    let out = kiln()
        .arg("build")
        .arg(program)
        .arg("-o")
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    exe
}

/// The values of every `name="..."` attribute of the tags in `html`.
/// Text, such as code that shows HTML, has `&lt;` for `<`, so it has no
/// tags.
fn attribute_values(html: &str, name: &str) -> Vec<String> {
    let needle = format!(" {name}=\"");
    html.split('<')
        .skip(1)
        .filter_map(|tag| tag.split('>').next())
        .flat_map(|tag| tag.split(needle.as_str()).skip(1))
        .map(|rest| rest[..rest.find('"').unwrap()].to_string())
        .collect()
}

fn links(html: &str) -> Vec<String> {
    let mut out = attribute_values(html, "href");
    out.extend(attribute_values(html, "src"));
    out
}

fn css_urls(css: &str) -> Vec<String> {
    css.split("url(")
        .skip(1)
        .map(|rest| {
            rest[..rest.find(')').unwrap()]
                .trim_matches(['"', '\''])
                .to_string()
        })
        .chain(
            css.split("@import \"")
                .skip(1)
                .map(|rest| rest[..rest.find('"').unwrap()].to_string()),
        )
        .filter(|u| u.starts_with('/'))
        .collect()
}

/// The site path a link goes to, or `None` for other sites and fragments.
fn resolve(from: &str, link: &str) -> Option<String> {
    let link = link.split('#').next().unwrap();
    if link.is_empty() || link.contains("://") || link.starts_with("mailto:") {
        return None;
    }
    let joined = if link.starts_with('/') {
        link.to_string()
    } else {
        format!("{}{link}", &from[..from.rfind('/').unwrap() + 1])
    };
    let mut parts: Vec<&str> = Vec::new();
    for p in joined.split('/').skip(1) {
        match p {
            ".." => {
                parts.pop();
            }
            "." => {}
            _ => parts.push(p),
        }
    }
    let path = format!("/{}", parts.join("/"));
    // The docs link to the repository's README, which the site sends to
    // GitHub.
    (path != "/README").then_some(path)
}

/// The page's text, without its tags and attributes.
fn text_of(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    let mut in_style = false;
    for (i, c) in html.char_indices() {
        match c {
            '<' => {
                in_tag = true;
                if html[i..].starts_with("<style") {
                    in_style = true;
                } else if html[i..].starts_with("</style") {
                    in_style = false;
                }
            }
            '>' => in_tag = false,
            _ if !in_tag && !in_style => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn stylesheet(s: &Server, html: &str) -> String {
    let sheet = attribute_values(html, "href")
        .into_iter()
        .find(|h| h.starts_with("/_kiln/app-"))
        .unwrap();
    s.get(&sheet).body
}

/// The declaration of the rule for `.class`, outside media queries.
fn rule(css: &str, class: &str) -> Option<String> {
    let needle = format!(".{class}{{");
    let at = css.find(&needle)? + needle.len();
    Some(css[at..at + css[at..].find('}')?].to_string())
}

struct Response {
    status: u16,
    headers: String,
    body: String,
}

impl Response {
    fn header(&self, name: &str) -> String {
        self.headers
            .lines()
            .find_map(|l| {
                let (n, v) = l.split_once(':')?;
                n.eq_ignore_ascii_case(name).then(|| v.trim().to_string())
            })
            .unwrap_or_default()
    }
}

struct Server {
    child: Child,
    port: u16,
}

impl Server {
    fn start(exe: &Path) -> Server {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(exe)
            .env("PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(30);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "the site did not start");
            std::thread::sleep(Duration::from_millis(20));
        }
        Server { child, port }
    }

    fn get(&self, path: &str) -> Response {
        let mut c = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(
            c,
            "GET {path} HTTP/1.1\r\nhost: localhost\r\nconnection: close\r\n\r\n"
        )
        .unwrap();
        let mut raw = Vec::new();
        c.read_to_end(&mut raw).unwrap();
        let text = String::from_utf8_lossy(&raw).into_owned();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        let (status_line, headers) = head.split_once("\r\n").unwrap_or((head, ""));
        let status = status_line.split(' ').nth(1).unwrap().parse().unwrap();
        Response {
            status,
            headers: headers.replace("\r\n", "\n"),
            body: body.to_string(),
        }
    }

    fn stop(mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
