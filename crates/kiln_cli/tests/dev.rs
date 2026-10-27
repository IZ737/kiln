//! `kiln dev`: serves a program, rebuilds and restarts it after a change,
//! keeps the last good build when a build fails, and tells pages to reload.
use std::io::{Read, Write};
use std::net::TcpStream;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn get(port: u16, path: &str) -> Option<String> {
    let mut s = TcpStream::connect(("127.0.0.1", port)).ok()?;
    s.set_read_timeout(Some(Duration::from_secs(30))).ok()?;
    write!(s, "GET {path} HTTP/1.1\r\nHost: localhost\r\n\r\n").ok()?;
    let mut out = String::new();
    s.read_to_string(&mut out).ok()?;
    Some(out)
}

fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(120);
    while !ok() {
        assert!(Instant::now() < until, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

const APP: &str = "import web\n\nfn main() ! {\n    let app = web.app()\n    app.get(\"/\", fn(req) {\n        return web.html(\"<!doctype html><title>t</title><body><p>WORD</p></body>\")\n    })\n    try app.listen(\":8080\")\n}\n";

#[test]
fn rebuilds_restarts_and_keeps_the_last_good_build() {
    let dir = std::env::temp_dir().join(format!("kiln-dev-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.kiln"), APP.replace("WORD", "first")).unwrap();
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .args(["dev", "--port", &port.to_string()])
        .arg(&dir)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    eventually("the first build is served", || {
        get(port, "/").is_some_and(|r| r.contains("first"))
    });
    let page = get(port, "/").unwrap();
    assert!(page.contains("/_kiln/dev/version"), "{page}");
    assert!(page.find("kiln dev").unwrap() < page.find("</body>").unwrap());
    let version = get(port, "/_kiln/dev/version").unwrap();
    let version = version.rsplit("\r\n\r\n").next().unwrap().to_string();

    std::fs::write(dir.join("main.kiln"), APP.replace("WORD", "second")).unwrap();
    eventually("the change is served", || {
        get(port, "/").is_some_and(|r| r.contains("second"))
    });
    let next = get(port, &format!("/_kiln/dev/version?v={version}")).unwrap();
    assert!(!next.ends_with(&format!("\r\n\r\n{version}")), "{next}");

    // A broken build leaves the last good one running.
    std::fs::write(dir.join("main.kiln"), "fn main( {\n").unwrap();
    std::thread::sleep(Duration::from_secs(3));
    assert!(get(port, "/").unwrap().contains("second"));

    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&dir);
}
