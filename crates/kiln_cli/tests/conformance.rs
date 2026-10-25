//! Runs every program in `conformance/` (one folder each) with `kiln run`,
//! once with each backend (the native one when this kiln has its runtime),
//! and compares its output with the expected files in the folder. When
//! `KILN_WINE` names a Wine, each program is also built for Windows and run
//! with it. The expected files:
//!
//! - `main.out`: the expected standard output.
//! - `main.err`: the expected standard error, if the program fails. The
//!   exit code must then be 101 for a panic and 1 for an error that left
//!   `main`. Without this file, the program must exit with 0 and print
//!   nothing on standard error.
//! - `main.args`: the program's arguments, one per line.
//! - `main.in`: the program's standard input.
//! - `main.code`: the expected exit code, for a program that ends itself
//!   with `os.exit` or writes to standard error and succeeds.
//! - `main.requests`: HTTP requests to send to a server, separated by lines
//!   of `###`. The program gets a free port in `PORT`, the test waits until
//!   it accepts connections, and sends each request on a connection of its
//!   own. A request's lines up to the first empty one are its head, and the
//!   rest is its body; `content-length` is added when there is a body (a
//!   chunked body gets `\r\n` line ends instead), and `connection: close`
//!   unless the request has a `connection` header. The
//!   last request should make the program exit. Its exit code must then be
//!   0 (or `main.code`), and `main.err`, with the port written as `PORT`, is
//!   what it logged on standard error.
//! - `main.responses`: the expected responses, each after a `### ` line
//!   with its request line, without the `date` header.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn conformance_programs_produce_expected_output() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../conformance");
    let mut programs: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    programs.sort();
    assert!(
        programs.len() >= 5,
        "expected programs in {}",
        root.display()
    );
    let mut backends = vec!["c"];
    if kiln_native::unavailable(kiln_native::Target::host()).is_none() {
        backends.push("native");
    }
    if std::env::var_os("KILN_WINE").is_some()
        && kiln_native::unavailable(kiln_native::Target::Windows).is_none()
    {
        backends.push("windows");
    }
    let mut failures = Vec::new();
    for backend in backends {
        for program in &programs {
            run_program(program, backend, &mut failures);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Runs one program and records how its behaviour differs from the
/// expected files.
fn run_program(program: &Path, backend: &str, failures: &mut Vec<String>) {
    let read = |ext: &str| std::fs::read_to_string(program.join("main").with_extension(ext)).ok();
    let args = read("args").unwrap_or_default();
    let mut built_exe = None;
    let mut command = if backend == "windows" {
        // Built with `kiln build --target windows`, then run with Wine.
        let exe = std::env::temp_dir().join(format!(
            "kiln-conformance-{}-{}.exe",
            std::process::id(),
            program.file_name().unwrap().to_string_lossy()
        ));
        let built = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .args(["build", "--target", "windows", "-o"])
            .arg(&exe)
            .arg(program)
            .output()
            .unwrap();
        if !built.status.success() {
            failures.push(format!(
                "{} (windows): does not build:\n{}",
                program.display(),
                String::from_utf8_lossy(&built.stderr)
            ));
            return;
        }
        let mut c = Command::new(std::env::var_os("KILN_WINE").unwrap());
        c.arg(&exe).env("WINEDEBUG", "-all");
        built_exe = Some(exe);
        c
    } else {
        let mut c = Command::new(env!("CARGO_BIN_EXE_kiln"));
        c.arg("run").arg("--backend").arg(backend).arg(program);
        c
    };
    let requests = read("requests");
    let port = requests.as_ref().map(|_| free_port());
    if let Some(port) = port {
        command.env("PORT", port.to_string());
    }
    let mut child = command
        .args(args.lines())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let input = read("in").unwrap_or_default();
    let mut stdin = child.stdin.take().unwrap();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(input.as_bytes());
    });
    let mut responses = None;
    let out = match (&requests, port) {
        (Some(requests), Some(port)) => {
            let mut stdout = child.stdout.take().unwrap();
            let mut stderr = child.stderr.take().unwrap();
            let out_reader = std::thread::spawn(move || {
                let mut v = Vec::new();
                let _ = stdout.read_to_end(&mut v);
                v
            });
            let err_reader = std::thread::spawn(move || {
                let mut v = Vec::new();
                let _ = stderr.read_to_end(&mut v);
                v
            });
            responses = Some(send_requests(port, requests));
            // The last request should have ended the program.
            let deadline = Instant::now() + Duration::from_secs(30);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if Instant::now() > deadline {
                    let _ = child.kill();
                    break child.wait().unwrap();
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            let stderr = String::from_utf8_lossy(&err_reader.join().unwrap())
                .replace(&format!(":{port}"), ":PORT");
            std::process::Output {
                status,
                stdout: out_reader.join().unwrap(),
                stderr: stderr.into_bytes(),
            }
        }
        _ => child.wait_with_output().unwrap(),
    };
    writer.join().unwrap();
    if let Some(exe) = built_exe {
        let _ = std::fs::remove_file(exe);
    }
    let expected_out = read("out").unwrap_or_default();
    let expected_err = read("err");
    let expected_code = match &expected_err {
        _ if read("code").is_some() => read("code").unwrap().trim().parse().unwrap(),
        _ if requests.is_some() => 0,
        None => 0,
        Some(err) if err.lines().last().is_some_and(|l| l.starts_with("panic:")) => 101,
        Some(_) => 1,
    };
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let name = format!(
        "{} ({backend})",
        program.file_name().unwrap().to_string_lossy()
    );
    if stdout != expected_out {
        failures.push(format!(
            "{name}: standard output differs\n--- expected\n{expected_out}--- found\n{stdout}"
        ));
    }
    if let Some(found) = responses {
        let expected = read("responses").unwrap_or_default();
        if found != expected {
            failures.push(format!(
                "{name}: responses differ\n--- expected\n{expected}--- found\n{found}"
            ));
        }
    }
    if stderr != expected_err.clone().unwrap_or_default() {
        failures.push(format!("{name}: standard error differs:\n{stderr}"));
    }
    if out.status.code() != Some(expected_code) {
        failures.push(format!(
            "{name}: exit code {:?}, expected {expected_code}",
            out.status.code()
        ));
    }
}

/// A port that no program is listening on.
fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

/// Sends the requests in a `main.requests` file to the server on `port`,
/// once it accepts connections, and returns the responses as
/// `main.responses` holds them.
fn send_requests(port: u16, requests: &str) -> String {
    let address = format!("127.0.0.1:{port}");
    let deadline = Instant::now() + Duration::from_secs(60);
    while TcpStream::connect(&address).is_err() {
        if Instant::now() > deadline {
            return "the server never accepted a connection\n".into();
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let mut transcript = String::new();
    for block in requests.split("\n###\n") {
        let block = block.trim_start_matches('\n');
        let (head, body) = match block.split_once("\n\n") {
            Some((head, body)) => (head, body.trim_end_matches('\n')),
            None => (block.trim_end_matches('\n'), ""),
        };
        let mut raw = head.replace('\n', "\r\n");
        raw.push_str("\r\n");
        let lower = head.to_ascii_lowercase();
        if !body.is_empty()
            && !lower.contains("\ncontent-length:")
            && !lower.contains("\ntransfer-encoding:")
        {
            raw.push_str(&format!("content-length: {}\r\n", body.len()));
        }
        // One connection per request, so the client reads to the end.
        if !lower.contains("\nconnection:") {
            raw.push_str("connection: close\r\n");
        }
        raw.push_str("\r\n");
        if lower.contains("\ntransfer-encoding: chunked") {
            raw.push_str(&format!("{}\r\n\r\n", body.replace('\n', "\r\n")));
        } else {
            raw.push_str(body);
        }
        let request_line = head.lines().next().unwrap_or("");
        transcript.push_str(&format!("### {request_line}\n"));
        let response = match TcpStream::connect(&address) {
            Ok(mut stream) => {
                stream
                    .set_read_timeout(Some(Duration::from_secs(30)))
                    .unwrap();
                let _ = stream.write_all(raw.as_bytes());
                let mut response = Vec::new();
                let _ = stream.read_to_end(&mut response);
                String::from_utf8_lossy(&response).replace("\r\n", "\n")
            }
            Err(e) => format!("cannot connect: {e}\n"),
        };
        for line in response.split_inclusive('\n') {
            if !line.to_ascii_lowercase().starts_with("date:") {
                transcript.push_str(line);
            }
        }
        if !transcript.ends_with('\n') {
            transcript.push('\n');
        }
    }
    transcript
}
