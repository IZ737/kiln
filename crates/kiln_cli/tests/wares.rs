//! Wares end to end: path and git sources, the lockfile, capabilities,
//! policy, the cache, and publishing to the registry in `registry/`.
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A scratch folder with its own Ware cache and configuration.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "kiln-wares-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }

    fn write(&self, name: &str, text: &str) {
        let file = self.0.join(name);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, text).unwrap();
    }

    fn read(&self, name: &str) -> String {
        std::fs::read_to_string(self.0.join(name)).unwrap_or_default()
    }

    fn kiln(&self, dir: &str, args: &[&str]) -> Output {
        self.kiln_with(dir, args, "")
    }

    fn kiln_with(&self, dir: &str, args: &[&str], stdin: &str) -> Output {
        use std::io::Write;
        let mut child = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(args)
            .current_dir(self.0.join(dir))
            .env("KILN_CACHE_DIR", self.0.join("cache"))
            .env("KILN_CONFIG_DIR", self.0.join("config"))
            .env("KILN_NO_UPDATE_CHECK", "1")
            .env_remove("KILN_REGISTRY")
            .env_remove("KILN_REGISTRY_TOKEN")
            .env_remove("KILN_OFFLINE")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn ok(&self, dir: &str, args: &[&str]) -> String {
        let out = self.kiln(dir, args);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "kiln {args:?} failed:\n{text}");
        text
    }

    fn fails(&self, dir: &str, args: &[&str], expected: &str) {
        let out = self.kiln(dir, args);
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!out.status.success(), "kiln {args:?} succeeded:\n{text}");
        assert!(text.contains(expected), "expected {expected:?} in:\n{text}");
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ware(s: &Scratch, dir: &str, name: &str, version: &str, extra: &str, source: &str) {
    s.write(
        &format!("{dir}/kiln.toml"),
        &format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nkind = \"ware\"\nlicense = \"MIT\"\n{extra}"
        ),
    );
    s.write(&format!("{dir}/lib.kiln"), source);
}

const APP: &str = "import greet\n\nfn main() {\n    print(greet.hello(\"kiln\"))\n}\n";

#[test]
fn path_wares_lock_and_run() {
    let s = Scratch::new();
    ware(
        &s,
        "greet",
        "jake/greet",
        "1.0.0",
        "",
        "pub fn hello(name: String) -> String {\n    return \"hello, ${name}\"\n}\n",
    );
    s.write("app/main.kiln", APP);
    s.ok("app", &["add", "jake/greet", "--path", "../greet", "--as", "greet"]);
    let lock = s.read("app/kiln.lock");
    assert!(lock.contains("source = \"path+../greet\""), "{lock}");
    assert!(s.ok("app", &["run", "."]).contains("hello, kiln"));
    // The lockfile is written the same way twice.
    s.ok("app", &["update"]);
    assert_eq!(lock, s.read("app/kiln.lock"));
    assert!(s.ok("app", &["tree"]).contains("greet: jake/greet 1.0.0"));
    // --locked refuses to change it.
    s.write(
        "app/kiln.toml",
        "[package]\nname = \"app\"\n\n[dependencies]\ngreet = { ware = \"jake/greet\", path = \"../greet\" }\nother = { ware = \"jake/other\", path = \"../greet\" }\n",
    );
    s.fails(
        "app",
        &["--locked", "check", "."],
        "`--locked` does not allow changing it",
    );
    s.ok("app", &["remove", "other"]);
}

#[test]
fn undeclared_capabilities_and_policy() {
    let s = Scratch::new();
    ware(
        &s,
        "greet",
        "jake/greet",
        "1.0.0",
        "",
        "import fs\n\npub fn hello(name: String) -> String {\n    return name\n}\n",
    );
    s.write("app/main.kiln", APP);
    s.write(
        "app/kiln.toml",
        "[package]\nname = \"app\"\n\n[dependencies]\ngreet = { ware = \"jake/greet\", path = \"../greet\" }\n",
    );
    s.fails(
        "app",
        &["check", "."],
        "uses capabilities its kiln.toml does not declare: `filesystem`",
    );
    ware(
        &s,
        "greet",
        "jake/greet",
        "1.0.0",
        "capabilities = [\"filesystem\"]\n",
        "import fs\n\npub fn hello(name: String) -> String {\n    return name\n}\n",
    );
    s.fails("app", &["check", "."], "--allow-capabilities");
    s.ok("app", &["update", "--allow-capabilities"]);
    s.write(
        "app/kiln.toml",
        "[package]\nname = \"app\"\n\n[dependencies]\ngreet = { ware = \"jake/greet\", path = \"../greet\" }\n\n[policy]\ncapabilities = [\"network\"]\n",
    );
    s.fails(
        "app",
        &["check", "."],
        "declares `filesystem`, which the policy does not allow",
    );
}

#[test]
fn version_conflicts_are_explained() {
    let s = Scratch::new();
    ware(&s, "base1", "jake/base", "1.0.0", "", "pub fn one() -> Int {\n    return 1\n}\n");
    ware(&s, "base2", "jake/base", "2.0.0", "", "pub fn one() -> Int {\n    return 2\n}\n");
    ware(
        &s,
        "left",
        "jake/left",
        "1.0.0",
        "\n[dependencies]\nbase = { ware = \"jake/base\", path = \"../base1\" }\n",
        "import base\n\npub fn left() -> Int {\n    return base.one()\n}\n",
    );
    s.write("app/main.kiln", "fn main() {}\n");
    s.write(
        "app/kiln.toml",
        "[package]\nname = \"app\"\n\n[dependencies]\nleft = { ware = \"jake/left\", path = \"../left\" }\nbase = { ware = \"jake/base\", path = \"../base2\" }\n",
    );
    s.fails("app", &["check", "."], "jake/base");
}

#[test]
fn archives_are_canonical_and_the_cache_heals() {
    let s = Scratch::new();
    ware(
        &s,
        "greet",
        "jake/greet",
        "1.0.0",
        "",
        "pub fn hello(name: String) -> String {\n    return \"hello, ${name}\"\n}\n",
    );
    let a = s.ok("greet", &["publish", ".", "--dry-run", "--registry", "http://127.0.0.1:9"]);
    // Files in another order, or with other modification times, hash the same.
    std::thread::sleep(Duration::from_millis(20));
    s.write("greet/lib.kiln", &s.read("greet/lib.kiln"));
    let b = s.ok("greet", &["publish", ".", "--dry-run", "--registry", "http://127.0.0.1:9"]);
    let sum = |t: &str| t.split_whitespace().find(|w| w.starts_with("sha256:")).map(str::to_string);
    assert!(sum(&a).is_some());
    assert_eq!(sum(&a), sum(&b));
    // Files that are not UTF-8 text are refused.
    std::fs::write(s.0.join("greet/logo.bin"), [0xff, 0xfe, 0]).unwrap();
    s.fails(
        "greet",
        &["publish", ".", "--dry-run", "--registry", "http://127.0.0.1:9"],
        "logo.bin",
    );
}

#[test]
fn git_sources_pin_a_commit() {
    if Command::new("git").arg("--version").output().is_err() {
        return;
    }
    let s = Scratch::new();
    ware(
        &s,
        "greet",
        "jake/greet",
        "1.0.0",
        "",
        "pub fn hello(name: String) -> String {\n    return \"hi, ${name}\"\n}\n",
    );
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .args(["-c", "user.email=k@example.com", "-c", "user.name=k"])
            .args(args)
            .current_dir(s.0.join("greet"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    };
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);
    git(&["tag", "v1.0.0"]);
    s.write("app/main.kiln", APP);
    let url = format!("file://{}", s.0.join("greet").display());
    s.ok(
        "app",
        &["add", "jake/greet", "--git", &url, "--tag", "v1.0.0", "--as", "greet"],
    );
    let lock = s.read("app/kiln.lock");
    assert!(lock.contains("source = \"git+file://"), "{lock}");
    assert!(lock.contains('#'), "{lock}");
    assert!(s.ok("app", &["run", "."]).contains("hi, kiln"));
    // Offline, the pinned commit comes from the cache.
    assert!(s.ok("app", &["--offline", "run", "."]).contains("hi, kiln"));
}

/// The registry in `registry/`, on a free port with a fresh database.
struct Registry {
    child: Child,
    url: String,
}

impl Registry {
    fn start(s: &Scratch) -> Registry {
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../registry");
        let db = format!("sqlite:{}", s.0.join("registry.db").display());
        let migrate = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["db", "migrate", "--url", &db, "--path"])
            .arg(&source)
            .env("KILN_CACHE_DIR", s.0.join("cache"))
            .output()
            .unwrap();
        assert!(
            migrate.status.success(),
            "{}",
            String::from_utf8_lossy(&migrate.stderr)
        );
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let child = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["--locked", "run"])
            .arg(&source)
            .env("KILN_CACHE_DIR", s.0.join("cache"))
            .env("DATABASE_URL", &db)
            .env("REGISTRY_HTTP", "1")
            .env("REGISTRY_ADDRESS", format!("127.0.0.1:{port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(120);
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "the registry never started");
            std::thread::sleep(Duration::from_millis(100));
        }
        Registry {
            child,
            url: format!("http://127.0.0.1:{port}"),
        }
    }

    /// Signs up `name` and returns a new publish token.
    fn token(&self, name: &str) -> String {
        let form = format!("name={name}&email={name}%40example.com&password=a+long+enough+password");
        let res = self.http("POST", "/sign-up", "", &form);
        let cookie = between(&res, "set-cookie: ", ";");
        let page = self.http("GET", "/tokens", &cookie, "");
        let csrf = between(&page, "name=\"csrf\" value=\"", "\"");
        let page = self.http("POST", "/tokens", &cookie, &format!("csrf={csrf}&label=tests"));
        between(&page, "data-testid=\"token\">", "<")
    }

    /// One request, as the browser would send it, and the whole response.
    fn http(&self, method: &str, path: &str, cookie: &str, form: &str) -> String {
        use std::io::{Read, Write};
        let host = self.url.trim_start_matches("http://");
        let mut s = std::net::TcpStream::connect(host).unwrap();
        let cookie = if cookie.is_empty() {
            String::new()
        } else {
            format!("Cookie: {cookie}\r\n")
        };
        write!(
            s,
            "{method} {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: {}\r\n{cookie}Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{form}",
            self.url,
            form.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        out
    }
}

impl Drop for Registry {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn between(text: &str, start: &str, end: &str) -> String {
    let from = text.find(start).unwrap_or_else(|| panic!("{start} in {text}")) + start.len();
    let to = text[from..].find(end).unwrap() + from;
    text[from..to].to_string()
}

#[test]
fn publish_add_yank_through_the_registry() {
    let s = Scratch::new();
    let registry = Registry::start(&s);
    let url = registry.url.clone();
    let token = registry.token("jake");
    s.write("app/main.kiln", APP);
    s.kiln_with("app", &["login", "--registry", &url], &format!("{token}\n"));
    let creds = s.read("config/credentials.toml");
    assert!(creds.contains(&url), "{creds}");
    ware(
        &s,
        "greet",
        "jake/greet",
        "1.0.0",
        "",
        "pub fn hello(name: String) -> String {\n    return \"hello, ${name}\"\n}\n",
    );
    s.ok("greet", &["publish", ".", "--registry", &url]);
    // Publishing the same archive again is fine; another archive is not.
    s.ok("greet", &["publish", ".", "--registry", &url]);
    s.write("greet/lib.kiln", "pub fn hello(name: String) -> String {\n    return name\n}\n");
    s.fails("greet", &["publish", ".", "--registry", &url], "already published");
    // Another publisher's names are refused.
    ware(&s, "theirs", "someone/greet", "1.0.0", "", "pub fn x() {}\n");
    s.fails("theirs", &["publish", ".", "--registry", &url], "cannot publish");

    s.ok("app", &["add", "jake/greet", "--registry", &url]);
    assert!(s.ok("app", &["run", "."]).contains("hello, kiln"));
    let lock = s.read("app/kiln.lock");
    assert!(lock.contains(&format!("source = \"registry+{url}\"")), "{lock}");

    // A new version that gains a capability needs approval.
    ware(
        &s,
        "greet",
        "jake/greet",
        "1.1.0",
        "capabilities = [\"environment\"]\n",
        "pub fn hello(name: String) -> String {\n    return \"${env(\"GREETING\")}${name}\"\n}\n",
    );
    s.ok("greet", &["publish", ".", "--registry", &url]);
    s.fails("app", &["update"], "--allow-capabilities");
    assert_eq!(lock, s.read("app/kiln.lock"));
    s.ok("app", &["update", "--allow-capabilities"]);
    assert!(s.read("app/kiln.lock").contains("version = \"1.1.0\""));

    // A yanked release keeps working for lockfiles that name it.
    s.ok("app", &["yank", "jake/greet@1.1.0", "--registry", &url]);
    s.ok("app", &["run", "."]);
    s.ok("app", &["update"]);
    assert!(s.read("app/kiln.lock").contains("version = \"1.0.0\""));

    // Offline, everything comes from the cache.
    drop(registry);
    assert!(s.ok("app", &["--offline", "run", "."]).contains("hello, kiln"));
    assert!(s.ok("app", &["cache", "verify"]).contains("match their checksums"));
}
