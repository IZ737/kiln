//! The web server's behaviour over time, which `main.requests` files cannot
//! show. Runs server programs in `conformance/` with each backend (and,
//! when `KILN_WINE` names a Wine, built for Windows) and checks that a
//! sleeping handler and a client that sends slowly do not hold up other
//! requests, that the collector keeps a waiting task's values alive, that
//! connections stay open for more requests, sent one after another or all
//! at once, and that the timeout and keep-alive settings hold.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn slow_requests_do_not_hold_up_others() {
    for backend in backends() {
        let server = Server::start(&program("web_tasks"), backend);
        check(&server, backend);
        server.stop();
    }
}

/// `conformance/web_errors` sets a 1 second timeout and turns keep-alive
/// off.
#[test]
fn timeouts_and_keep_alive_follow_the_settings() {
    for backend in backends() {
        let server = Server::start(&program("web_errors"), backend);
        // A client that sends part of a request's head and stops gets 408
        // when the timeout ends, and other requests go on meanwhile.
        let start = Instant::now();
        let mut stalled = server.connect();
        stalled
            .write_all(b"GET /posts/1 HTTP/1.1\r\nhost: x")
            .unwrap();
        let mut c = server.connect();
        c.write_all(b"GET /posts/1 HTTP/1.1\r\n\r\n").unwrap();
        let (status, headers, _) = read_response(&mut c);
        assert_eq!(status, "HTTP/1.1 200 OK", "{backend}");
        // Keep-alive is off, so the server closes even an HTTP/1.1
        // connection that did not ask it to.
        assert!(headers.contains("connection: close"), "{backend}");
        let (status, _, _) = read_response(&mut stalled);
        assert_eq!(status, "HTTP/1.1 408 Request Timeout", "{backend}");
        let took = start.elapsed();
        assert!(
            took >= Duration::from_millis(900) && took < Duration::from_secs(5),
            "{backend}: the stalled client got its answer after {took:?}"
        );
        server.stop();
    }
}

fn program(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance")
        .join(name)
}

fn backends() -> Vec<&'static str> {
    let mut backends = vec!["c"];
    if kiln_native::unavailable(kiln_native::Target::host()).is_none() {
        backends.push("native");
    }
    if std::env::var_os("KILN_WINE").is_some()
        && kiln_native::unavailable(kiln_native::Target::Windows).is_none()
    {
        backends.push("windows");
    }
    backends
}

fn check(server: &Server, backend: &str) {
    // A request whose handler sleeps for a second...
    let start = Instant::now();
    let mut slow = server.connect();
    slow.write_all(b"GET /slow HTTP/1.1\r\n\r\n").unwrap();
    // ...and a client that sends half a request and stops.
    let mut stalled = server.connect();
    stalled
        .write_all(b"POST /echo HTTP/1.1\r\ncontent-le")
        .unwrap();
    std::thread::sleep(Duration::from_millis(100));

    // Meanwhile other requests are answered, on one kept-alive connection,
    // and one of them makes the collector run.
    let mut c = server.connect();
    for path in ["/fast", "/churn", "/fast"] {
        write!(c, "GET {path} HTTP/1.1\r\n\r\n").unwrap();
        let (status, headers, _) = read_response(&mut c);
        assert_eq!(status, "HTTP/1.1 200 OK", "{backend}: {path}");
        assert!(!headers.contains("connection: close"), "{backend}: {path}");
    }
    let fast = start.elapsed();
    assert!(
        fast < Duration::from_millis(900),
        "{backend}: the other requests waited for the slow one ({fast:?})"
    );

    let (status, _, body) = read_response(&mut slow);
    assert_eq!(status, "HTTP/1.1 200 OK", "{backend}");
    assert_eq!(body, "slow: 5000 items, the last is item 4999", "{backend}");
    assert!(start.elapsed() >= Duration::from_secs(1), "{backend}");

    // The stalled client finishes its request and gets its answer.
    stalled.write_all(b"ngth: 5\r\n\r\nfired").unwrap();
    let (status, _, body) = read_response(&mut stalled);
    assert_eq!(status, "HTTP/1.1 200 OK", "{backend}");
    assert_eq!(body, "fired", "{backend}");

    // Requests sent all at once are answered in order.
    let mut pipelined = server.connect();
    pipelined
        .write_all(b"GET /fast HTTP/1.1\r\n\r\nPOST /echo HTTP/1.1\r\ncontent-length: 3\r\n\r\nabcGET /fast HTTP/1.1\r\nconnection: close\r\n\r\n")
        .unwrap();
    let mut bodies = Vec::new();
    for _ in 0..3 {
        bodies.push(read_response(&mut pipelined).2);
    }
    assert_eq!(bodies, ["fast", "abc", "fast"], "{backend}");
    let mut rest = Vec::new();
    pipelined.read_to_end(&mut rest).unwrap();
    assert!(
        rest.is_empty(),
        "{backend}: the connection should be closed"
    );

    // HTTP/1.0 connections close after the response unless asked not to.
    let mut old = server.connect();
    old.write_all(b"GET /fast HTTP/1.0\r\n\r\n").unwrap();
    let (_, headers, _) = read_response(&mut old);
    assert!(headers.contains("connection: close"), "{backend}");
    let mut old = server.connect();
    old.write_all(b"GET /fast HTTP/1.0\r\nconnection: keep-alive\r\n\r\n")
        .unwrap();
    let (_, headers, _) = read_response(&mut old);
    assert!(headers.contains("connection: keep-alive"), "{backend}");
    old.write_all(b"GET /fast HTTP/1.0\r\n\r\n").unwrap();
    assert_eq!(read_response(&mut old).2, "fast", "{backend}");

    // Many connections at once, each sleeping a second, finish together.
    let start = Instant::now();
    let mut many: Vec<_> = (0..20)
        .map(|_| {
            let mut s = server.connect();
            s.write_all(b"GET /slow HTTP/1.1\r\n\r\n").unwrap();
            s
        })
        .collect();
    for s in &mut many {
        assert_eq!(read_response(s).0, "HTTP/1.1 200 OK", "{backend}");
    }
    let took = start.elapsed();
    assert!(
        took < Duration::from_secs(5),
        "{backend}: 20 sleeping requests took {took:?}"
    );
}

struct Server {
    child: Child,
    port: u16,
    exe: Option<PathBuf>,
}

impl Server {
    fn start(program: &Path, backend: &str) -> Server {
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut exe = None;
        let mut command = if backend == "windows" {
            let path = std::env::temp_dir().join(format!(
                "kiln-server-{}-{}.exe",
                std::process::id(),
                program.file_name().unwrap().to_string_lossy()
            ));
            let built = Command::new(env!("CARGO_BIN_EXE_kiln"))
                .args(["build", "--target", "windows", "-o"])
                .arg(&path)
                .arg(program)
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "{}",
                String::from_utf8_lossy(&built.stderr)
            );
            let mut c = Command::new(std::env::var_os("KILN_WINE").unwrap());
            c.arg(&path).env("WINEDEBUG", "-all");
            exe = Some(path);
            c
        } else {
            let mut c = Command::new(env!("CARGO_BIN_EXE_kiln"));
            c.args(["run", "--backend", backend]).arg(program);
            c
        };
        let child = command
            .env("PORT", port.to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let server = Server { child, port, exe };
        let deadline = Instant::now() + Duration::from_secs(60);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(
                Instant::now() < deadline,
                "{backend}: the server never started"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        server
    }

    fn connect(&self) -> TcpStream {
        let s = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        s.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        s
    }

    fn stop(mut self) {
        let mut s = self.connect();
        let _ = s.write_all(b"GET /quit HTTP/1.1\r\n\r\n");
        let deadline = Instant::now() + Duration::from_secs(20);
        while self.child.try_wait().unwrap().is_none() {
            if Instant::now() > deadline {
                let _ = self.child.kill();
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        if let Some(exe) = self.exe.take() {
            let _ = std::fs::remove_file(exe);
        }
    }
}

/// Reads one response: its status line, its headers in lower case, and its
/// body, which must have a `content-length`.
fn read_response(s: &mut TcpStream) -> (String, String, String) {
    let mut head = Vec::new();
    let mut byte = [0u8];
    while !head.ends_with(b"\r\n\r\n") {
        let n = s.read(&mut byte).unwrap();
        assert!(
            n == 1,
            "the connection ended in a response's head: {head:?}"
        );
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).unwrap();
    let (status, headers) = head.split_once("\r\n").unwrap();
    let headers = headers.to_ascii_lowercase();
    let length: usize = headers
        .lines()
        .find_map(|l| l.strip_prefix("content-length: "))
        .expect("content-length")
        .trim()
        .parse()
        .unwrap();
    let mut body = vec![0; length];
    s.read_exact(&mut body).unwrap();
    (
        status.to_string(),
        headers,
        String::from_utf8(body).unwrap(),
    )
}
