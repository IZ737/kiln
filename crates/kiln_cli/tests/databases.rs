//! One program, `tests/databases/main.kiln`, on every database: SQLite in
//! memory always, and PostgreSQL and MySQL when `KILN_POSTGRES_URL` and
//! `KILN_MYSQL_URL` name databases it may change (CI starts both). Each
//! must print `main.out`, with each backend.

use std::path::{Path, PathBuf};
use std::process::Command;

fn program() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/databases")
}

fn backends() -> Vec<&'static str> {
    let mut out = vec!["c"];
    if kiln_native::unavailable(kiln_native::Target::host()).is_none() {
        out.push("native");
    }
    out
}

fn run_on(url: &str) {
    if !url.starts_with("sqlite") {
        // The database is shared with other tests: tables that reference
        // the program's (as the journal's `comments` does) would stop it
        // dropping them.
        let mut db = kiln_dbwire::open(url, None).unwrap();
        for table in ["comments", "posts", "users"] {
            db.exec(&format!("drop table if exists {table}"), &[])
                .unwrap();
        }
    }
    let expected = std::fs::read_to_string(program().join("main.out")).unwrap();
    for backend in backends() {
        let out = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["run", "--backend", backend])
            .arg(program())
            .env("DATABASE_URL", url)
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "{backend} on {url}:\n{stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(stdout, expected, "{backend} on {url}");
    }
}

#[test]
fn sqlite() {
    run_on("sqlite::memory:");
}

#[test]
fn postgres() {
    match std::env::var("KILN_POSTGRES_URL") {
        Ok(url) => run_on(&url),
        Err(_) => eprintln!("skipping: KILN_POSTGRES_URL is not set"),
    }
}

#[test]
fn mysql() {
    match std::env::var("KILN_MYSQL_URL") {
        Ok(url) => run_on(&url),
        Err(_) => eprintln!("skipping: KILN_MYSQL_URL is not set"),
    }
}

/// While one request waits for a slow query, another is answered: the
/// clients wait on Kiln's tasks rather than blocking the program.
#[test]
fn a_slow_query_holds_up_only_its_own_request() {
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::time::{Duration, Instant};
    for var in ["KILN_POSTGRES_URL", "KILN_MYSQL_URL"] {
        let Ok(url) = std::env::var(var) else {
            eprintln!("skipping: {var} is not set");
            continue;
        };
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut server = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .arg("run")
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/database_tasks"))
            .env("DATABASE_URL", &url)
            .env("PORT", port.to_string())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(60);
        while TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "{var}: the server never started");
            std::thread::sleep(Duration::from_millis(50));
        }
        let ask = |path: &str| {
            let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
            c.write_all(
                format!("GET {path} HTTP/1.1\r\nhost: x\r\nconnection: close\r\n\r\n").as_bytes(),
            )
            .unwrap();
            c
        };
        let read = |mut c: TcpStream| {
            let mut out = String::new();
            c.read_to_string(&mut out).unwrap();
            out
        };
        let start = Instant::now();
        let slow = ask("/slow");
        std::thread::sleep(Duration::from_millis(100));
        let fast = read(ask("/fast"));
        let fast_took = start.elapsed();
        assert!(fast.ends_with("fast 2"), "{var}: {fast}");
        assert!(
            fast_took < Duration::from_millis(800),
            "{var}: /fast took {fast_took:?}"
        );
        let slow = read(slow);
        assert!(slow.ends_with("slow"), "{var}: {slow}");
        assert!(start.elapsed() >= Duration::from_millis(900), "{var}");
        let _ = read(ask("/quit"));
        let _ = server.wait();
    }
}
