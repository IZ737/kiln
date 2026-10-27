//! The browser of `kiln test`: a Chromium that kiln starts and drives over
//! the Chrome DevTools Protocol. kiln does not carry a browser. It uses the
//! one `KILN_TEST_BROWSER` or `[test] browser` in kiln.toml names, else one
//! that Playwright installed or the system has, else the one
//! `kiln test --install-browser` downloads into kiln's cache.
//!
//! Each page is a browser context of its own, so tests share no cookies or
//! storage. Elements are found the way a person finds them: by role and
//! accessible name (Chromium's own accessibility tree answers), by label,
//! by text. An action waits until its element is there, visible, enabled,
//! not covered and, inside an ember, hydrated.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::BufRead;
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{Message, WebSocket};

/// How long one browser action waits for its element.
const ACTION_TIMEOUT: Duration = Duration::from_secs(10);

/// The Chrome for Testing release `--install-browser` downloads.
pub const PINNED_VERSION: &str = "131.0.6778.204";

type Pending = Arc<Mutex<HashMap<u64, mpsc::Sender<Value>>>>;

/// What a page did that a test may ask about.
#[derive(Default)]
struct PageEvents {
    loads: u64,
    errors: Vec<String>,
}

type Events = Arc<Mutex<HashMap<String, PageEvents>>>;

pub struct Browser {
    child: Mutex<Option<Child>>,
    out: Mutex<mpsc::Sender<String>>,
    pending: Pending,
    events: Events,
    next: AtomicU64,
    profile: PathBuf,
}

pub struct Page {
    /// The id tests know the page by: its target.
    pub id: String,
    session: String,
    context: String,
    base: String,
}

/// Where a browser is, if there is one.
pub fn find(root: &Path) -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KILN_TEST_BROWSER") {
        return Some(PathBuf::from(p));
    }
    if let Ok(Some(m)) = crate::ware::manifest::Manifest::load(root) {
        if let Some(b) = &m.test.browser {
            return Some(root.join(b));
        }
    }
    let installed = installed_path();
    if installed.is_file() {
        return Some(installed);
    }
    let mut caches: Vec<PathBuf> = Vec::new();
    if let Some(p) = std::env::var_os("PLAYWRIGHT_BROWSERS_PATH") {
        caches.push(PathBuf::from(p));
    }
    if let Some(home) = std::env::var_os("HOME") {
        caches.push(Path::new(&home).join(".cache/ms-playwright"));
        caches.push(Path::new(&home).join("Library/Caches/ms-playwright"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        caches.push(Path::new(&local).join("ms-playwright"));
    }
    caches.push(PathBuf::from("/opt/pw-browsers"));
    for cache in caches {
        let Ok(entries) = std::fs::read_dir(&cache) else {
            continue;
        };
        let mut dirs: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
        dirs.sort();
        dirs.reverse();
        // The headless shell first: it is smaller and starts faster.
        for prefix in ["chromium_headless_shell-", "chromium-"] {
            for d in &dirs {
                let name = d
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                if !name.starts_with(prefix) {
                    continue;
                }
                for exe in [
                    "chrome-headless-shell-linux64/chrome-headless-shell",
                    "chrome-linux64/chrome",
                    "chrome-linux/chrome",
                    "chrome-headless-shell-win64/chrome-headless-shell.exe",
                    "chrome-win64/chrome.exe",
                    "chrome-win/chrome.exe",
                    "chrome-headless-shell-mac-arm64/chrome-headless-shell",
                    "chrome-mac/Chromium.app/Contents/MacOS/Chromium",
                ] {
                    if d.join(exe).is_file() {
                        return Some(d.join(exe));
                    }
                }
            }
        }
    }
    let system = [
        "/usr/bin/chromium",
        "/usr/bin/chromium-browser",
        "/usr/bin/google-chrome",
        "/usr/bin/google-chrome-stable",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    ];
    system.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// Where `--install-browser` puts the browser, in kiln's per-user cache.
pub fn installed_dir() -> PathBuf {
    crate::ware::cache::dir()
        .join("browsers")
        .join(format!("chrome-headless-shell-{PINNED_VERSION}"))
}

fn platform() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Some("linux64"),
        ("windows", "x86_64") => Some("win64"),
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        _ => None,
    }
}

fn installed_path() -> PathBuf {
    let platform = platform().unwrap_or("linux64");
    let exe = if cfg!(windows) {
        "chrome-headless-shell.exe"
    } else {
        "chrome-headless-shell"
    };
    installed_dir()
        .join(format!("chrome-headless-shell-{platform}"))
        .join(exe)
}

/// `kiln test --install-browser`: downloads the pinned Chrome for Testing
/// headless shell into kiln's cache, once.
pub fn install() -> Result<String, String> {
    let exe = installed_path();
    if exe.is_file() {
        return Ok(format!("the test browser is installed: {}", exe.display()));
    }
    let platform = platform().ok_or(
        "Chrome for Testing has no build for this system; set KILN_TEST_BROWSER to a Chromium",
    )?;
    let url = format!(
        "https://storage.googleapis.com/chrome-for-testing-public/{PINNED_VERSION}/{platform}/chrome-headless-shell-{platform}.zip"
    );
    eprintln!("downloading {url}");
    let agent = crate::upgrade::agent(Duration::from_secs(600))?;
    let bytes = crate::upgrade::get(&agent, &url)?
        .ok_or_else(|| format!("{url} is not there; set KILN_TEST_BROWSER to a Chromium"))?;
    let dir = installed_dir();
    let staging = dir.with_extension("partial");
    let _ = std::fs::remove_dir_all(&staging);
    std::fs::create_dir_all(&staging).map_err(|e| format!("{}: {e}", staging.display()))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("the download is not a zip archive: {e}"))?;
    for i in 0..zip.len() {
        let mut file = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = file.enclosed_name() else {
            return Err(format!(
                "the archive holds an unsafe path: {:?}",
                file.name_raw()
            ));
        };
        let out = staging.join(name);
        if file.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut w = std::fs::File::create(&out).map_err(|e| e.to_string())?;
        std::io::copy(&mut file, &mut w).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::rename(&staging, &dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !exe.is_file() {
        return Err(format!("the archive did not hold {}", exe.display()));
    }
    Ok(format!("installed the test browser: {}", exe.display()))
}

impl Browser {
    pub fn launch(root: &Path, headed: bool) -> Result<Browser, String> {
        let exe = find(root).ok_or(
            "no browser for browser tests: run `kiln test --install-browser`, or set KILN_TEST_BROWSER to a Chromium or Chrome",
        )?;
        let profile = std::env::temp_dir().join(format!(
            "kiln-test-browser-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&profile);
        let mut command = Command::new(&exe);
        command.args([
            "--remote-debugging-port=0",
            "--no-first-run",
            "--no-default-browser-check",
            "--disable-background-networking",
            "--disable-extensions",
            "--disable-sync",
            "--disable-dev-shm-usage",
            "--password-store=basic",
            "--use-mock-keychain",
            "--hide-scrollbars",
            "--mute-audio",
        ]);
        command.arg(format!("--user-data-dir={}", profile.display()));
        if !headed {
            command.arg("--headless");
        }
        // Chromium's sandbox needs a user namespace, which containers and
        // CI machines running as root do not give.
        #[cfg(unix)]
        if std::fs::read_to_string("/proc/self/status").is_ok_and(|s| {
            s.lines()
                .any(|l| l.starts_with("Uid:") && l.split_whitespace().nth(1) == Some("0"))
        }) || std::env::var("USER").is_ok_and(|u| u == "root")
            || std::env::var_os("CI").is_some()
            || std::fs::metadata("/.dockerenv").is_ok()
        {
            command.arg("--no-sandbox");
        }
        command.arg("about:blank");
        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start the browser {}: {e}", exe.display()))?;
        let stderr = child.stderr.take().unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut said = false;
            let mut log = String::new();
            for line in std::io::BufReader::new(stderr)
                .lines()
                .map_while(Result::ok)
            {
                if !said {
                    if let Some(at) = line.find("ws://") {
                        let _ = tx.send(Ok(line[at..].trim().to_string()));
                        said = true;
                    } else {
                        log.push_str(&line);
                        log.push('\n');
                    }
                }
            }
            if !said {
                let _ = tx.send(Err(log));
            }
        });
        let ws = match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(Ok(ws)) => ws,
            Ok(Err(log)) => {
                let _ = child.kill();
                return Err(format!("the browser {} stopped:\n{log}", exe.display()));
            }
            Err(_) => {
                let _ = child.kill();
                return Err(format!("the browser {} did not start", exe.display()));
            }
        };
        let host = ws
            .trim_start_matches("ws://")
            .split('/')
            .next()
            .unwrap_or("")
            .to_string();
        let stream =
            TcpStream::connect(&host).map_err(|e| format!("cannot reach the browser: {e}"))?;
        let (socket, _) = tungstenite::client(ws.as_str(), stream)
            .map_err(|e| format!("cannot reach the browser: {e}"))?;
        socket
            .get_ref()
            .set_read_timeout(Some(Duration::from_millis(5)))
            .ok();
        let pending: Pending = Arc::default();
        let events: Events = Arc::default();
        let (out, out_rx) = mpsc::channel::<String>();
        let (p, e) = (pending.clone(), events.clone());
        std::thread::spawn(move || pump(socket, out_rx, p, e));
        Ok(Browser {
            child: Mutex::new(Some(child)),
            out: Mutex::new(out),
            pending,
            events,
            next: AtomicU64::new(1),
            profile,
        })
    }

    fn call(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        self.call_for(session, method, params, Duration::from_secs(30))
    }

    fn call_for(
        &self,
        session: Option<&str>,
        method: &str,
        params: Value,
        limit: Duration,
    ) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            message["sessionId"] = json!(s);
        }
        let (tx, rx) = mpsc::channel();
        self.pending.lock().unwrap().insert(id, tx);
        self.out
            .lock()
            .unwrap()
            .send(message.to_string())
            .map_err(|_| "the browser closed".to_string())?;
        let reply = rx.recv_timeout(limit).map_err(|_| {
            self.pending.lock().unwrap().remove(&id);
            format!("the browser did not answer {method}")
        })?;
        if let Some(error) = reply.get("error") {
            return Err(error["message"]
                .as_str()
                .unwrap_or("browser error")
                .to_string());
        }
        Ok(reply["result"].clone())
    }

    pub fn page(&self, base: &str, width: u32, height: u32) -> Result<Page, String> {
        let context = self.call(
            None,
            "Target.createBrowserContext",
            json!({ "disposeOnDetach": true }),
        )?["browserContextId"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let target = self.call(
            None,
            "Target.createTarget",
            json!({ "url": "about:blank", "browserContextId": context }),
        )?["targetId"]
            .as_str()
            .unwrap_or("")
            .to_string();
        let session = self.call(
            None,
            "Target.attachToTarget",
            json!({ "targetId": target, "flatten": true }),
        )?["sessionId"]
            .as_str()
            .unwrap_or("")
            .to_string();
        self.events
            .lock()
            .unwrap()
            .insert(session.clone(), PageEvents::default());
        let s = Some(session.as_str());
        for domain in [
            "Page.enable",
            "Runtime.enable",
            "Network.enable",
            "Log.enable",
            "DOM.enable",
            "Accessibility.enable",
        ] {
            self.call(s, domain, json!({}))?;
        }
        let page = Page {
            id: target,
            session,
            context,
            base: base.trim_end_matches('/').to_string(),
        };
        self.viewport(&page, width, height)?;
        Ok(page)
    }

    fn viewport(&self, page: &Page, width: u32, height: u32) -> Result<(), String> {
        self.call(
            Some(&page.session),
            "Emulation.setDeviceMetricsOverride",
            json!({ "width": width, "height": height, "deviceScaleFactor": 1, "mobile": false }),
        )
        .map(|_| ())
    }

    pub fn close_page(&self, page: &Page) {
        let _ = self.call(None, "Target.closeTarget", json!({ "targetId": page.id }));
        let _ = self.call(
            None,
            "Target.disposeBrowserContext",
            json!({ "browserContextId": page.context }),
        );
        self.events.lock().unwrap().remove(&page.session);
    }

    pub fn close(&self) {
        let _ = self.call_for(None, "Browser.close", json!({}), Duration::from_secs(5));
        if let Some(mut child) = self.child.lock().unwrap().take() {
            let until = Instant::now() + Duration::from_secs(5);
            while child.try_wait().ok().flatten().is_none() && Instant::now() < until {
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.profile);
    }

    pub fn errors(&self, page: &Page) -> Vec<String> {
        self.events
            .lock()
            .unwrap()
            .get(&page.session)
            .map(|e| e.errors.clone())
            .unwrap_or_default()
    }

    fn loads(&self, page: &Page) -> u64 {
        self.events
            .lock()
            .unwrap()
            .get(&page.session)
            .map_or(0, |e| e.loads)
    }

    pub fn screenshot(&self, page: &Page, dir: &Path, name: &str) -> Result<String, String> {
        let data = self.call(
            Some(&page.session),
            "Page.captureScreenshot",
            json!({ "format": "png" }),
        )?["data"]
            .as_str()
            .unwrap_or("")
            .to_string();
        use base64::Engine;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|e| e.to_string())?;
        let safe: String = name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("{safe}.png"));
        std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        Ok(path.display().to_string())
    }

    /// Evaluates JavaScript in the page and returns its JSON value.
    fn eval(&self, page: &Page, expression: &str) -> Result<Value, String> {
        let r = self.call(
            Some(&page.session),
            "Runtime.evaluate",
            json!({ "expression": expression, "returnByValue": true, "awaitPromise": true }),
        )?;
        if let Some(e) = r.get("exceptionDetails") {
            return Err(exception_text(e));
        }
        Ok(r["result"]["value"].clone())
    }

    /// Calls a function on an element and returns its JSON value.
    fn on(
        &self,
        page: &Page,
        object: &str,
        function: &str,
        args: &[Value],
    ) -> Result<Value, String> {
        let arguments: Vec<Value> = args.iter().map(|a| json!({ "value": a })).collect();
        let r = self.call(
            Some(&page.session),
            "Runtime.callFunctionOn",
            json!({
                "objectId": object,
                "functionDeclaration": function,
                "arguments": arguments,
                "returnByValue": true,
                "awaitPromise": true,
            }),
        )?;
        if let Some(e) = r.get("exceptionDetails") {
            return Err(exception_text(e));
        }
        Ok(r["result"]["value"].clone())
    }

    /// The elements a locator matches now, as remote objects.
    fn resolve(&self, page: &Page, loc: &Locator) -> Result<Vec<String>, String> {
        if loc.by == "role" {
            let doc = self.call(
                Some(&page.session),
                "Runtime.evaluate",
                json!({ "expression": "document" }),
            )?["result"]["objectId"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let mut params = json!({ "objectId": doc, "role": loc.value });
            if !loc.name.is_empty() {
                params["accessibleName"] = json!(loc.name);
            }
            let nodes = self.call(Some(&page.session), "Accessibility.queryAXTree", params)?;
            let mut out = Vec::new();
            for node in nodes["nodes"].as_array().cloned().unwrap_or_default() {
                if node["ignored"].as_bool() == Some(true) {
                    continue;
                }
                let Some(backend) = node["backendDOMNodeId"].as_u64() else {
                    continue;
                };
                let r = self.call(
                    Some(&page.session),
                    "DOM.resolveNode",
                    json!({ "backendNodeId": backend }),
                )?;
                if let Some(id) = r["object"]["objectId"].as_str() {
                    out.push(id.to_string());
                }
            }
            return Ok(out);
        }
        let expression = format!("({FIND})({}, {})", json!(loc.by), json!(loc.value));
        let r = self.call(
            Some(&page.session),
            "Runtime.evaluate",
            json!({ "expression": expression }),
        )?;
        if let Some(e) = r.get("exceptionDetails") {
            return Err(exception_text(e));
        }
        let Some(array) = r["result"]["objectId"].as_str() else {
            return Ok(Vec::new());
        };
        let props = self.call(
            Some(&page.session),
            "Runtime.getProperties",
            json!({ "objectId": array, "ownProperties": true }),
        )?;
        let mut out = Vec::new();
        for p in props["result"].as_array().cloned().unwrap_or_default() {
            if p["name"]
                .as_str()
                .is_some_and(|n| n.parse::<usize>().is_ok())
            {
                if let Some(id) = p["value"]["objectId"].as_str() {
                    out.push(id.to_string());
                }
            }
        }
        Ok(out)
    }

    /// [`Browser::resolve`], again while the page is between documents.
    fn settled(&self, page: &Page, loc: &Locator, until: Instant) -> Result<Vec<String>, String> {
        loop {
            match self.resolve(page, loc) {
                Ok(found) => return Ok(found),
                Err(e) if Instant::now() >= until => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    /// Waits for the one element a locator names to be ready for `need`.
    fn element(
        &self,
        page: &Page,
        loc: &Locator,
        need: Need,
        until: Instant,
    ) -> Result<String, String> {
        let mut last = format!("no element matches {}", loc.describe());
        loop {
            match self.resolve(page, loc) {
                Ok(found) if found.len() > 1 => {
                    return Err(format!(
                        "{} elements match {}; name it more exactly, such as by its accessible name or `test_id`",
                        found.len(),
                        loc.describe()
                    ))
                }
                Ok(found) if found.len() == 1 => {
                    let object = &found[0];
                    match self.on(page, object, READY, &[json!(need.name())]) {
                        Ok(Value::String(problem)) if !problem.is_empty() => {
                            last = format!("{} {problem}", loc.describe());
                        }
                        Ok(_) => return Ok(object.clone()),
                        Err(e) => last = e,
                    }
                }
                Ok(_) => last = format!("no element matches {}", loc.describe()),
                // The page may be between documents.
                Err(e) => last = e,
            }
            if Instant::now() >= until {
                return Err(last);
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn wait_load(&self, page: &Page, before: u64, until: Instant) -> Result<(), String> {
        while self.loads(page) <= before {
            if Instant::now() >= until {
                return Err("the page did not finish loading".into());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        Ok(())
    }

    fn mouse_click(&self, page: &Page, object: &str) -> Result<(), String> {
        let point = self.on(page, object, CENTER, &[])?;
        let (x, y) = (
            point[0].as_f64().unwrap_or(0.0),
            point[1].as_f64().unwrap_or(0.0),
        );
        let s = Some(page.session.as_str());
        self.call(
            s,
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseMoved", "x": x, "y": y }),
        )?;
        self.call(
            s,
            "Input.dispatchMouseEvent",
            json!({ "type": "mousePressed", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        )?;
        self.call(
            s,
            "Input.dispatchMouseEvent",
            json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "clickCount": 1 }),
        )?;
        Ok(())
    }

    fn key(&self, page: &Page, key: &str) -> Result<(), String> {
        let mut modifiers = 0;
        let mut name = key;
        let parts: Vec<&str> = key.split('+').collect();
        if parts.len() > 1 && !key.ends_with('+') {
            for m in &parts[..parts.len() - 1] {
                modifiers |= match *m {
                    "Alt" => 1,
                    "Control" | "Ctrl" => 2,
                    "Meta" => 4,
                    "Shift" => 8,
                    other => {
                        return Err(format!(
                            "`{other}` is not a modifier key: expected Alt, Control, Meta or Shift"
                        ))
                    }
                };
            }
            name = parts[parts.len() - 1];
        }
        let (code, vk, text): (String, i64, Option<String>) = match name {
            "Enter" => ("Enter".into(), 13, Some("\r".into())),
            "Tab" => ("Tab".into(), 9, None),
            "Escape" => ("Escape".into(), 27, None),
            "Backspace" => ("Backspace".into(), 8, None),
            "Delete" => ("Delete".into(), 46, None),
            "ArrowUp" => ("ArrowUp".into(), 38, None),
            "ArrowDown" => ("ArrowDown".into(), 40, None),
            "ArrowLeft" => ("ArrowLeft".into(), 37, None),
            "ArrowRight" => ("ArrowRight".into(), 39, None),
            "Home" => ("Home".into(), 36, None),
            "End" => ("End".into(), 35, None),
            "PageUp" => ("PageUp".into(), 33, None),
            "PageDown" => ("PageDown".into(), 34, None),
            " " | "Space" => ("Space".into(), 32, Some(" ".into())),
            k if k.chars().count() == 1 => {
                let c = k.chars().next().unwrap();
                let upper = c.to_ascii_uppercase();
                let code = if c.is_ascii_alphabetic() {
                    format!("Key{upper}")
                } else if c.is_ascii_digit() {
                    format!("Digit{c}")
                } else {
                    String::new()
                };
                // Only letters and digits have a key code of their own.
                let vk = if c.is_ascii_alphanumeric() { upper as i64 } else { 0 };
                (code, vk, Some(k.to_string()))
            }
            other => return Err(format!("`{other}` is not a key kiln knows: use a character, or Enter, Tab, Escape, Backspace, Delete, Space, Home, End, PageUp, PageDown or an arrow key")),
        };
        let key_name = if name == "Space" { " " } else { name };
        let text = if modifiers & (1 | 2 | 4) != 0 {
            None
        } else {
            text
        };
        let mut down = json!({
            "type": if text.is_some() { "keyDown" } else { "rawKeyDown" },
            "key": key_name,
            "code": code,
            "windowsVirtualKeyCode": vk,
            "modifiers": modifiers,
        });
        if let Some(t) = &text {
            down["text"] = json!(t);
            down["unmodifiedText"] = json!(t);
        }
        let s = Some(page.session.as_str());
        self.call(s, "Input.dispatchKeyEvent", down)?;
        self.call(
            s,
            "Input.dispatchKeyEvent",
            json!({ "type": "keyUp", "key": key_name, "code": code, "windowsVirtualKeyCode": vk, "modifiers": modifiers }),
        )?;
        Ok(())
    }

    /// Does what a test asked of a page.
    pub fn act(
        &self,
        page: &Page,
        op: &str,
        r: &Value,
        deadline: Instant,
        results: &Path,
    ) -> Result<String, String> {
        let s = |k: &str| r[k].as_str().unwrap_or("").to_string();
        let until = deadline.min(Instant::now() + ACTION_TIMEOUT);
        let loc = Locator {
            by: s("by"),
            value: s("value"),
            name: s("name"),
        };
        let session = Some(page.session.as_str());
        match op {
            "goto" => {
                let path = s("path");
                let url = if path.starts_with("http://") || path.starts_with("https://") {
                    if !path.starts_with(&page.base) {
                        return Err(format!(
                            "{path} is not the app under test; browser tests visit only {}",
                            page.base
                        ));
                    }
                    path
                } else {
                    format!("{}/{}", page.base, path.trim_start_matches('/'))
                };
                let before = self.loads(page);
                let r = self.call(session, "Page.navigate", json!({ "url": url }))?;
                if let Some(e) = r["errorText"].as_str().filter(|e| !e.is_empty()) {
                    return Err(format!("cannot open {url}: {e}"));
                }
                self.wait_load(page, before, until)?;
                Ok(String::new())
            }
            "reload" => {
                let before = self.loads(page);
                self.call(session, "Page.reload", json!({}))?;
                self.wait_load(page, before, until)?;
                Ok(String::new())
            }
            "path" => self.retrying(
                page,
                "location.pathname + location.search + location.hash",
                until,
            ),
            "title" => self.retrying(page, "document.title", until),
            "page_text" => {
                self.retrying(page, "document.body ? document.body.innerText : ''", until)
            }
            "wait_path" => {
                let want = s("part");
                loop {
                    let now = self.retrying(page, "location.pathname", until)?;
                    if now == want {
                        return Ok(now);
                    }
                    if Instant::now() >= until {
                        return Err(format!("the page never went to {want}; it is at {now}"));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            "press" => self.key(page, &s("key")).map(|_| String::new()),
            "type" => {
                for c in s("text").chars() {
                    if c.is_ascii() && !c.is_ascii_control() {
                        self.key(page, &c.to_string())?;
                    } else {
                        self.call(
                            session,
                            "Input.insertText",
                            json!({ "text": c.to_string() }),
                        )?;
                    }
                }
                Ok(String::new())
            }
            "focused" => self.focused(page),
            "errors" => Ok(json!(self.errors(page)).to_string()),
            "viewport" => {
                self.viewport(
                    page,
                    s("width").parse().unwrap_or(1280),
                    s("height").parse().unwrap_or(800),
                )?;
                Ok(String::new())
            }
            "screenshot" => self.screenshot(page, results, &s("name")),
            "click" => {
                let el = self.element(page, &loc, Need::Click, until)?;
                self.mouse_click(page, &el)?;
                Ok(String::new())
            }
            "fill" => {
                let el = self.element(page, &loc, Need::Edit, until)?;
                self.on(page, &el, CLEAR, &[])?;
                let text = s("text");
                if !text.is_empty() {
                    self.call(session, "Input.insertText", json!({ "text": text }))?;
                }
                Ok(String::new())
            }
            "select" => {
                let el = self.element(page, &loc, Need::Edit, until)?;
                match self.on(page, &el, SELECT, &[json!(s("text"))])? {
                    Value::String(problem) if !problem.is_empty() => Err(problem),
                    _ => Ok(String::new()),
                }
            }
            "check" => {
                let want = s("checked") == "true";
                let el = self.element(page, &loc, Need::Click, until)?;
                if self.on(page, &el, "function() { return !!this.checked }", &[])? != json!(want) {
                    self.mouse_click(page, &el)?;
                }
                if self.on(page, &el, "function() { return !!this.checked }", &[])? != json!(want) {
                    return Err(format!("clicking {} did not change it", loc.describe()));
                }
                Ok(String::new())
            }
            "focus" => {
                let el = self.element(page, &loc, Need::Visible, until)?;
                self.on(page, &el, "function() { this.focus() }", &[])?;
                Ok(String::new())
            }
            "press_on" => {
                let el = self.element(page, &loc, Need::Visible, until)?;
                self.on(page, &el, "function() { this.focus() }", &[])?;
                self.key(page, &s("key")).map(|_| String::new())
            }
            "text" => {
                let el = self.element(page, &loc, Need::Present, until)?;
                Ok(text_of(self.on(
                    page,
                    &el,
                    "function() { return (this.innerText ?? this.textContent ?? '').trim() }",
                    &[],
                )?))
            }
            "value" => {
                let el = self.element(page, &loc, Need::Present, until)?;
                Ok(text_of(self.on(
                    page,
                    &el,
                    "function() { return this.value ?? '' }",
                    &[],
                )?))
            }
            "attribute" => {
                let el = self.element(page, &loc, Need::Present, until)?;
                Ok(self
                    .on(
                        page,
                        &el,
                        "function(n) { return this.getAttribute(n) }",
                        &[json!(s("attribute"))],
                    )?
                    .to_string())
            }
            "style" => {
                let el = self.element(page, &loc, Need::Present, until)?;
                Ok(text_of(self.on(
                    page,
                    &el,
                    "function(p) { return getComputedStyle(this).getPropertyValue(p) }",
                    &[json!(s("property"))],
                )?))
            }
            "count" => Ok(self.settled(page, &loc, until)?.len().to_string()),
            "visible" => {
                let found = self.settled(page, &loc, until)?;
                for el in found {
                    if self.on(page, &el, READY, &[json!("visible")])? == json!("") {
                        return Ok("true".into());
                    }
                }
                Ok("false".into())
            }
            "is_focused" => {
                let el = self.element(page, &loc, Need::Present, until)?;
                Ok(self
                    .on(
                        page,
                        &el,
                        "function() { return document.activeElement === this }",
                        &[],
                    )?
                    .to_string())
            }
            "is_checked" => {
                let el = self.element(page, &loc, Need::Present, until)?;
                Ok(self
                    .on(page, &el, "function() { return !!this.checked }", &[])?
                    .to_string())
            }
            "wait" => self
                .element(page, &loc, Need::Visible, until)
                .map(|_| String::new()),
            "expect_text" => {
                let want = s("text");
                let mut seen = String::new();
                loop {
                    if let Ok(el) = self.element(page, &loc, Need::Present, Instant::now()) {
                        if let Ok(v) = self.on(page, &el, "function() { return (this.innerText ?? this.textContent ?? '').trim() }", &[]) {
                            seen = text_of(v);
                            if seen.contains(&want) {
                                return Ok(seen);
                            }
                        }
                    }
                    if Instant::now() >= until {
                        return Err(format!(
                            "{} never showed {want:?}; it shows {seen:?}",
                            loc.describe()
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            }
            "expect_gone" => loop {
                let mut shown = 0;
                for el in self.resolve(page, &loc).unwrap_or_default() {
                    if self.on(page, &el, READY, &[json!("visible")]).ok() == Some(json!("")) {
                        shown += 1;
                    }
                }
                if shown == 0 {
                    return Ok(String::new());
                }
                if Instant::now() >= until {
                    return Err(format!("{} is still shown", loc.describe()));
                }
                std::thread::sleep(Duration::from_millis(50));
            },
            other => Err(format!("the runner does not know `{other}`")),
        }
    }

    /// Evaluates an expression for text, again while the page changes
    /// documents.
    fn retrying(&self, page: &Page, expression: &str, until: Instant) -> Result<String, String> {
        loop {
            match self.eval(page, expression) {
                Ok(v) => return Ok(text_of(v)),
                Err(e) if Instant::now() >= until => return Err(e),
                Err(_) => std::thread::sleep(Duration::from_millis(50)),
            }
        }
    }

    fn focused(&self, page: &Page) -> Result<String, String> {
        let r = self.call(
            Some(&page.session),
            "Runtime.evaluate",
            json!({ "expression": "document.activeElement && document.activeElement !== document.body ? document.activeElement : null" }),
        )?;
        let Some(object) = r["result"]["objectId"].as_str() else {
            return Ok(String::new());
        };
        let tree = self.call(
            Some(&page.session),
            "Accessibility.getPartialAXTree",
            json!({ "objectId": object, "fetchRelatives": false }),
        )?;
        let node = &tree["nodes"][0];
        let role = node["role"]["value"].as_str().unwrap_or("");
        let name = node["name"]["value"].as_str().unwrap_or("");
        Ok(if name.is_empty() {
            role.to_string()
        } else {
            format!("{role} {name:?}")
        })
    }
}

static COUNT: AtomicU64 = AtomicU64::new(0);

fn text_of(v: Value) -> String {
    match v {
        Value::String(s) => s,
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn exception_text(e: &Value) -> String {
    e["exception"]["description"]
        .as_str()
        .or(e["text"].as_str())
        .unwrap_or("a script error")
        .lines()
        .next()
        .unwrap_or("")
        .to_string()
}

struct Locator {
    by: String,
    value: String,
    name: String,
}

impl Locator {
    fn describe(&self) -> String {
        match self.by.as_str() {
            "role" if self.name.is_empty() => format!("role {}", self.value),
            "role" => format!("{} {:?}", self.value, self.name),
            "label" => format!("label {:?}", self.value),
            "text" => format!("text {:?}", self.value),
            "testid" => format!("test id {:?}", self.value),
            _ => format!("selector {:?}", self.value),
        }
    }
}

#[derive(Clone, Copy)]
enum Need {
    Present,
    Visible,
    Click,
    Edit,
}

impl Need {
    fn name(self) -> &'static str {
        match self {
            Need::Present => "present",
            Need::Visible => "visible",
            Need::Click => "click",
            Need::Edit => "edit",
        }
    }
}

/// Finds elements by label, text, test id or CSS.
const FIND: &str = r##"function(by, value) {
  const norm = (s) => (s || "").replace(/\s+/g, " ").trim();
  if (by === "css") return Array.from(document.querySelectorAll(value));
  if (by === "testid") return Array.from(document.querySelectorAll(`[data-testid="${CSS.escape(value)}"]`));
  if (by === "label") {
    const out = new Set();
    for (const l of document.querySelectorAll("label")) {
      const copy = l.cloneNode(true);
      copy.querySelectorAll("input, textarea, select, [aria-hidden=true]").forEach((c) => c.remove());
      if (norm(copy.textContent) === value && l.control) out.add(l.control);
    }
    for (const el of document.querySelectorAll("[aria-label]")) {
      if (norm(el.getAttribute("aria-label")) === value) out.add(el);
    }
    for (const el of document.querySelectorAll("[aria-labelledby]")) {
      const text = el.getAttribute("aria-labelledby").split(/\s+/)
        .map((id) => norm(document.getElementById(id)?.textContent)).join(" ");
      if (text === value) out.add(el);
    }
    return Array.from(out);
  }
  const all = Array.from(document.body ? document.body.querySelectorAll("*") : [])
    .filter((el) => !["SCRIPT", "STYLE", "TEMPLATE"].includes(el.tagName) && norm(el.textContent).includes(value));
  return all.filter((el) => !Array.from(el.children).some((c) => all.includes(c)));
}"##;

/// Says why an element is not ready for an action, or `""` when it is.
const READY: &str = r##"function(need) {
  if (!this.isConnected) return "is gone from the page";
  if (need === "present") return "";
  const r = this.getBoundingClientRect();
  const s = getComputedStyle(this);
  if (r.width === 0 || r.height === 0 || s.visibility === "hidden" || s.display === "none") return "is not visible";
  if (need === "visible") return "";
  if (this.disabled || this.closest("fieldset[disabled]")) return "is disabled";
  if (need === "edit" && this.readOnly) return "is read-only";
  const island = this.closest("kiln-island");
  if (island && !island.hasAttribute("data-k-ready") && !island.hasAttribute("data-k-error")) {
    this.scrollIntoView({ block: "center", inline: "center" });
    return "is in an ember that has not started yet";
  }
  this.scrollIntoView({ block: "center", inline: "center" });
  if (need === "click") {
    const b = this.getBoundingClientRect();
    const hit = document.elementFromPoint(b.left + b.width / 2, b.top + b.height / 2);
    const label = hit && hit.closest("label");
    if (hit && !this.contains(hit) && !(label && label.control === this)) {
      return "is covered by " + hit.tagName.toLowerCase() + (hit.id ? "#" + hit.id : "");
    }
  }
  return "";
}"##;

const CENTER: &str = r##"function() {
  const b = this.getBoundingClientRect();
  return [b.left + b.width / 2, b.top + b.height / 2];
}"##;

/// Focuses a field and empties it, as selecting its text and deleting it does.
const CLEAR: &str = r##"function() {
  this.focus();
  if ("value" in this && this.value !== "") {
    const proto = Object.getPrototypeOf(this);
    const setter = Object.getOwnPropertyDescriptor(proto, "value")?.set;
    setter ? setter.call(this, "") : (this.value = "");
    this.dispatchEvent(new Event("input", { bubbles: true }));
  } else if (this.isContentEditable) {
    this.textContent = "";
  }
}"##;

const SELECT: &str = r##"function(choice) {
  if (this.tagName !== "SELECT") return "it is not a select";
  const option = Array.from(this.options).find((o) => o.value === choice || o.label.trim() === choice);
  if (!option) return "no option has the value or text " + JSON.stringify(choice);
  this.focus();
  this.value = option.value;
  this.dispatchEvent(new Event("input", { bubbles: true }));
  this.dispatchEvent(new Event("change", { bubbles: true }));
  return "";
}"##;

/// Moves messages between the tests and the browser, and records what
/// pages do.
fn pump(
    mut socket: WebSocket<TcpStream>,
    out: mpsc::Receiver<String>,
    pending: Pending,
    events: Events,
) {
    loop {
        loop {
            match out.try_recv() {
                Ok(text) => {
                    if socket.send(Message::text(text)).is_err() {
                        return;
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                let Ok(v) = serde_json::from_str::<Value>(text.as_str()) else {
                    continue;
                };
                if let Some(id) = v["id"].as_u64() {
                    if let Some(tx) = pending.lock().unwrap().remove(&id) {
                        let _ = tx.send(v);
                    }
                    continue;
                }
                record(&v, &events);
            }
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) => {}
            Err(_) => {
                // The browser went away: every waiting call fails.
                pending.lock().unwrap().clear();
                return;
            }
        }
    }
}

fn record(v: &Value, events: &Events) {
    let Some(session) = v["sessionId"].as_str() else {
        return;
    };
    let mut events = events.lock().unwrap();
    let Some(page) = events.get_mut(session) else {
        return;
    };
    let p = &v["params"];
    match v["method"].as_str().unwrap_or("") {
        "Page.loadEventFired" => page.loads += 1,
        "Runtime.exceptionThrown" => page.errors.push(format!(
            "uncaught: {}",
            exception_text(&p["exceptionDetails"])
        )),
        "Runtime.consoleAPICalled" if p["type"] == "error" => {
            let text: Vec<String> = p["args"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .iter()
                .map(|a| {
                    a["value"]
                        .as_str()
                        .map(str::to_string)
                        .or_else(|| a["description"].as_str().map(str::to_string))
                        .unwrap_or_default()
                })
                .collect();
            page.errors
                .push(format!("console.error: {}", text.join(" ")));
        }
        // A response with an error status is the app's answer, which a
        // test checks itself; a request that failed is recorded below.
        "Log.entryAdded" if p["entry"]["level"] == "error" && p["entry"]["source"] != "network" => {
            page.errors.push(format!(
                "{}: {}",
                p["entry"]["source"].as_str().unwrap_or("log"),
                p["entry"]["text"].as_str().unwrap_or("")
            ))
        }
        "Network.loadingFailed" if p["canceled"] != true => page.errors.push(format!(
            "request failed: {}",
            p["errorText"].as_str().unwrap_or("")
        )),
        _ => {}
    }
}
