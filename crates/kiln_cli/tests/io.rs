//! Check interactive output while stdin is still open and no input is available.
use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

struct Running(Child);

impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn flush_makes_prompt_visible_before_input() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let dir = std::env::temp_dir()
        .join("work")
        .join(format!("kiln-io-flush-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut backends = vec!["c"];
    if kiln_native::unavailable(kiln_native::Target::host()).is_none() {
        backends.push("native");
    }
    if std::env::var_os("KILN_WINE").is_some()
        && kiln_native::unavailable(kiln_native::Target::Windows).is_none()
    {
        backends.push("windows");
    }
    for backend in backends {
        let exe = dir.join(format!("prompt-{backend}.exe"));
        let mut build = Command::new(env!("CARGO_BIN_EXE_kiln"));
        build.arg("build");
        if backend == "windows" {
            build.args(["--target", "windows"]);
        } else {
            build.args(["--backend", backend]);
        }
        let out = build
            .arg(root.join("conformance/io_flush"))
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{backend}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut run = if backend == "windows" {
            let mut command = Command::new(std::env::var_os("KILN_WINE").unwrap());
            command.arg(&exe);
            command
        } else {
            Command::new(&exe)
        };
        let mut child = Running(
            run.stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut stdout = child.0.stdout.take().unwrap();
        let (send, recv) = mpsc::channel();
        let reader = std::thread::spawn(move || loop {
            let mut byte = [0];
            match stdout.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => {
                    if send.send(byte[0]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        });
        // Nothing has been sent to stdin. Exit-time flushing cannot pass this check.
        for byte in b"> " {
            assert_eq!(
                recv.recv_timeout(Duration::from_secs(10)).unwrap(),
                *byte,
                "{backend}"
            );
        }
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "{backend}: exited before input"
        );
        child.0.stdin.take().unwrap().write_all(b"hello\n").unwrap();
        for byte in b"hello" {
            assert_eq!(
                recv.recv_timeout(Duration::from_secs(10)).unwrap(),
                *byte,
                "{backend}"
            );
        }
        assert_eq!(
            recv.recv_timeout(Duration::from_secs(10)),
            Err(mpsc::RecvTimeoutError::Disconnected),
            "{backend}: unexpected extra output or process did not close stdout"
        );
        assert!(child.0.wait().unwrap().success(), "{backend}");
        reader.join().unwrap();
    }
    std::fs::remove_dir_all(dir).unwrap();
}
