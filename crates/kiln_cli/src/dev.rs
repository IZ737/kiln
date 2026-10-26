//! `kiln dev`: runs a program's server while you change it. kiln watches
//! the project's files, builds again after a change and restarts the
//! server; when a build fails, the server that was running keeps running
//! and the errors are shown. Pages reach the server through a small proxy
//! that adds a script to each HTML page, which reloads it after a restart.
//! The script is only ever added by `kiln dev`, never to a built program.

use crate::project;
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitCode, Stdio};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Where the reload script waits for the next restart.
const VERSION_PATH: &str = "/_kiln/dev/version";

const RELOAD_SCRIPT: &str = r#"<script>/* kiln dev: reloads the page after a rebuild */(()=>{let v=null;const poll=()=>fetch("/_kiln/dev/version"+(v===null?"":"?v="+v)).then(r=>r.text()).then(t=>{if(v!==null&&t!==v){location.reload();return}v=t;poll()}).catch(()=>setTimeout(poll,1000));poll()})()</script>"#;

pub struct Options {
    pub dir: PathBuf,
    pub host: String,
    pub port: u16,
}

/// The server that is running now, and which build it is.
struct State {
    app: Option<String>,
    version: u64,
}

type Shared = Arc<(Mutex<State>, Condvar)>;

pub fn run(options: Options, builder: crate::testing::Builder) -> ExitCode {
    let root = match project::root(&options.dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {}: {e}", options.dir.display());
            return ExitCode::from(2);
        }
    };
    let address = format!("{}:{}", options.host, options.port);
    let listener = match TcpListener::bind(&address) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "error: cannot listen on {address}: {e}; is another server using it? Choose another port with --port"
            );
            return ExitCode::from(2);
        }
    };
    let loopback = matches!(options.host.as_str(), "127.0.0.1" | "localhost" | "::1");
    if !loopback {
        eprintln!("warning: kiln dev is listening on {address}, so other machines can reach it; it is for development only");
    }
    let shown = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or(address);
    let shared: Shared = Arc::new((
        Mutex::new(State {
            app: None,
            version: 0,
        }),
        Condvar::new(),
    ));
    let proxy_state = shared.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let state = proxy_state.clone();
            std::thread::spawn(move || {
                let _ = serve(stream, &state);
            });
        }
    });
    let work = std::env::temp_dir().join(format!("kiln-dev-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&work);
    let mut server: Option<Child> = None;
    let mut last = crate::testing::snapshot(&root);
    let mut build = 0u32;
    let mut first = true;
    loop {
        build += 1;
        let started = Instant::now();
        if !first {
            eprintln!("\nkiln dev: files changed, building again");
        }
        match start(&root, &work, build, builder) {
            Ok((child, url)) => {
                if let Some(mut old) = server.take() {
                    let _ = old.kill();
                    let _ = old.wait();
                }
                server = Some(child);
                let (lock, cv) = &*shared;
                let mut s = lock.lock().unwrap();
                s.app = Some(url);
                s.version += 1;
                cv.notify_all();
                drop(s);
                eprintln!(
                    "kiln dev: serving {} on http://{shown} (built in {:.1} s); press Ctrl+C to stop",
                    root.display(),
                    started.elapsed().as_secs_f64()
                );
            }
            Err(e) => {
                if !e.is_empty() {
                    eprintln!("{e}");
                }
                if server.is_some() {
                    eprintln!("kiln dev: the build failed; the last good build is still running on http://{shown}");
                } else {
                    eprintln!("kiln dev: the build failed; fix the errors and save to try again");
                }
            }
        }
        first = false;
        // Wait for a change, then for a burst of saves to settle.
        loop {
            std::thread::sleep(Duration::from_millis(300));
            if let Some(child) = server.as_mut() {
                if let Ok(Some(status)) = child.try_wait() {
                    eprintln!("kiln dev: the server stopped ({status}); it starts again after the next change");
                    server = None;
                }
            }
            let now = crate::testing::snapshot(&root);
            if now != last {
                std::thread::sleep(Duration::from_millis(150));
                last = crate::testing::snapshot(&root);
                break;
            }
        }
    }
}

/// Builds the program and starts it on a port of its own.
fn start(
    root: &Path,
    work: &Path,
    build: u32,
    builder: crate::testing::Builder,
) -> Result<(Child, String), String> {
    let project = project::discover(root).map_err(|err| {
        let (file, diag) = project::error_diagnostic(&err, &root.join("kiln.toml"));
        diag.render(&file)
    })?;
    let exe = work.join(crate::Target::host().executable(&format!("app-{build}")));
    if !builder(&project, &exe) {
        return Err(String::new());
    }
    let mut child = Command::new(&exe)
        .env("KILN_LISTEN", "127.0.0.1:0")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("error: cannot start the program: {e}"))?;
    let stderr = child.stderr.take().unwrap();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut sent = false;
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if !sent {
                if let Some(url) = line.strip_prefix("info listening on ") {
                    let _ = tx.send(url.trim().replace("localhost", "127.0.0.1"));
                    sent = true;
                    continue;
                }
            }
            eprintln!("{line}");
        }
    });
    match rx.recv_timeout(Duration::from_secs(30)) {
        Ok(url) => Ok((child, url.trim_start_matches("http://").to_string())),
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            Err("error: the program did not start a server; kiln dev runs programs that call `app.listen`".into())
        }
    }
}

/// One connection from the browser: answered by kiln for the reload
/// script, otherwise passed to the program.
fn serve(mut client: TcpStream, shared: &Shared) -> std::io::Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(30)))?;
    let (head, body) = read_message(&mut client)?;
    let target = head
        .lines()
        .next()
        .and_then(|l| l.split(' ').nth(1))
        .unwrap_or("/")
        .to_string();
    if let Some(query) = target.strip_prefix(VERSION_PATH) {
        let seen: Option<u64> = query.strip_prefix("?v=").and_then(|v| v.parse().ok());
        let (lock, cv) = &**shared;
        let mut state = lock.lock().unwrap();
        if let Some(seen) = seen {
            let until = Instant::now() + Duration::from_secs(25);
            while state.version == seen && Instant::now() < until {
                state = cv
                    .wait_timeout(state, until.saturating_duration_since(Instant::now()))
                    .unwrap()
                    .0;
            }
        }
        let version = state.version.to_string();
        drop(state);
        return write!(
            client,
            "HTTP/1.1 200 OK\r\ncontent-type: text/plain\r\ncache-control: no-store\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{version}",
            version.len()
        );
    }
    let app = shared.0.lock().unwrap().app.clone();
    let Some(app) = app else {
        let page = "<!doctype html><title>Building</title><p>kiln dev is building the program; this page reloads when it starts.</p>";
        return respond_html(&mut client, 503, &format!("{page}{RELOAD_SCRIPT}"));
    };
    let mut upstream = match TcpStream::connect(&app) {
        Ok(s) => s,
        Err(e) => {
            return respond_html(
                &mut client,
                502,
                &format!("<!doctype html><title>Not running</title><p>The program is not answering ({e}).</p>{RELOAD_SCRIPT}"),
            )
        }
    };
    // One request per connection keeps the proxy simple.
    let mut lines = head.split("\r\n");
    let mut out = format!("{}\r\n", lines.next().unwrap_or_default());
    for line in lines {
        let lower = line.to_ascii_lowercase();
        if line.is_empty() || lower.starts_with("connection:") || lower.starts_with("keep-alive:") {
            continue;
        }
        out.push_str(line);
        out.push_str("\r\n");
    }
    out.push_str("connection: close\r\n\r\n");
    upstream.write_all(out.as_bytes())?;
    upstream.write_all(&body)?;
    let mut response = Vec::new();
    upstream.read_to_end(&mut response)?;
    let Some(split) = response.windows(4).position(|w| w == b"\r\n\r\n") else {
        return client.write_all(&response);
    };
    let head = String::from_utf8_lossy(&response[..split]).to_string();
    let mut body = response[split + 4..].to_vec();
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    let mut status_line = String::new();
    let mut kept = Vec::new();
    for (i, line) in head.split("\r\n").enumerate() {
        if i == 0 {
            status_line = line.to_string();
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.insert(k.trim().to_ascii_lowercase(), v.trim().to_string());
            let lower = k.trim().to_ascii_lowercase();
            if !matches!(lower.as_str(), "content-length" | "transfer-encoding" | "connection") {
                kept.push(line.to_string());
            }
        }
    }
    if headers
        .get("transfer-encoding")
        .is_some_and(|v| v.contains("chunked"))
    {
        body = crate::testing::http::dechunk(&body);
    }
    let html = headers
        .get("content-type")
        .is_some_and(|t| t.starts_with("text/html"));
    if html {
        let text = String::from_utf8_lossy(&body).to_string();
        let text = match text.rfind("</body>") {
            Some(i) => format!("{}{RELOAD_SCRIPT}{}", &text[..i], &text[i..]),
            None => format!("{text}{RELOAD_SCRIPT}"),
        };
        body = text.into_bytes();
    }
    let mut out = format!("{status_line}\r\n");
    for line in kept {
        out.push_str(&line);
        out.push_str("\r\n");
    }
    out.push_str(&format!("content-length: {}\r\nconnection: close\r\n\r\n", body.len()));
    client.write_all(out.as_bytes())?;
    client.write_all(&body)
}

fn respond_html(client: &mut TcpStream, status: u16, page: &str) -> std::io::Result<()> {
    write!(
        client,
        "HTTP/1.1 {status} Kiln dev\r\ncontent-type: text/html; charset=utf-8\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{page}",
        page.len()
    )
}

/// Reads a request's head and its body, by its `content-length`.
fn read_message(stream: &mut TcpStream) -> std::io::Result<(String, Vec<u8>)> {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    let split = loop {
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i;
        }
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return Err(std::io::Error::other("the connection closed"));
        }
        data.extend_from_slice(&buf[..n]);
        if data.len() > 1 << 20 {
            return Err(std::io::Error::other("the request head is too large"));
        }
    };
    let head = String::from_utf8_lossy(&data[..split]).to_string();
    let length: usize = head
        .lines()
        .find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim()
                .eq_ignore_ascii_case("content-length")
                .then(|| v.trim().parse().ok())?
        })
        .unwrap_or(0);
    let mut body = data[split + 4..].to_vec();
    while body.len() < length {
        let n = stream.read(&mut buf)?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&buf[..n]);
    }
    Ok((head, body))
}
