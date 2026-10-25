//! `kiln upgrade` against a release server on this machine, which serves a
//! "kiln" that is a shell script.
#![cfg(unix)]

use sha2::{Digest, Sha256};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

const NEW_KILN: &str = "#!/bin/sh\necho kiln 9.0.0\n";

/// A copy of kiln to upgrade, since the upgrade replaces it.
fn kiln_copy(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-upgrade-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("kiln");
    std::fs::copy(env!("CARGO_BIN_EXE_kiln"), &exe).unwrap();
    exe
}

/// Serves `releases/latest` (redirecting to v9.0.0) and v9.0.0's archive
/// and checksum for this system, the way GitHub does, and returns the base
/// URL. With `bad_sum`, the checksum is wrong.
fn serve(bad_sum: bool) -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let folder = format!("kiln-v9.0.0-{arch}-{os}");
    let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    let mut header = tar::Header::new_gnu();
    header.set_size(NEW_KILN.len() as u64);
    header.set_mode(0o755);
    header.set_cksum();
    tar.append_data(&mut header, format!("{folder}/kiln"), NEW_KILN.as_bytes())
        .unwrap();
    let archive = tar.into_inner().unwrap().finish().unwrap();
    let mut sum: String = Sha256::digest(&archive)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if bad_sum {
        sum = sum.replace(|c: char| c != '0', "0");
    }
    let sum = format!("{sum}  {folder}.tar.gz\n");

    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/releases", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut line = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            reader.read_line(&mut line).unwrap();
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap() <= 2 {
                    break;
                }
            }
            let path = line.split_whitespace().nth(1).unwrap_or("");
            let (status, location, body): (&str, &str, &[u8]) = match path {
                "/releases/latest" => ("302 Found", "/releases/tag/v9.0.0", b""),
                p if p == format!("/releases/download/v9.0.0/{folder}.tar.gz") => {
                    ("200 OK", "", &archive)
                }
                p if p == format!("/releases/download/v9.0.0/{folder}.tar.gz.sha256") => {
                    ("200 OK", "", sum.as_bytes())
                }
                _ => ("404 Not Found", "", b""),
            };
            let mut head = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n",
                body.len()
            );
            if !location.is_empty() {
                head.push_str(&format!("Location: {location}\r\n"));
            }
            head.push_str("\r\n");
            let _ = stream.write_all(head.as_bytes());
            let _ = stream.write_all(body);
        }
    });
    base
}

fn upgrade(exe: &PathBuf, base: &str, args: &[&str]) -> std::process::Output {
    let home = exe.parent().unwrap().join("home");
    kiln(exe, base, &home, "upgrade", args)
}

fn kiln(
    exe: &PathBuf,
    base: &str,
    home: &PathBuf,
    command: &str,
    args: &[&str],
) -> std::process::Output {
    Command::new(exe)
        .arg(command)
        .args(args)
        .env("KILN_RELEASES_URL", base)
        .env("KILN_HOME", home)
        .env_remove("HTTP_PROXY")
        .env_remove("HTTPS_PROXY")
        .env_remove("http_proxy")
        .env_remove("https_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy")
        .output()
        .unwrap()
}

#[test]
fn upgrade_replaces_kiln_with_the_latest_release() {
    let exe = kiln_copy("latest");
    let base = serve(false);

    let out = upgrade(&exe, &base, &["--check"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("kiln 9.0.0 is available"));
    assert_ne!(
        std::fs::read_to_string(&exe).ok().as_deref(),
        Some(NEW_KILN)
    );

    let out = upgrade(&exe, &base, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read_to_string(&exe).unwrap(), NEW_KILN);
    let version = Command::new(&exe).arg("--version").output().unwrap();
    assert_eq!(String::from_utf8_lossy(&version.stdout), "kiln 9.0.0\n");
    // Nothing is left beside it but the cache of the latest release.
    let dir = exe.parent().unwrap();
    let files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(files.len(), 2, "{files:?}");
    let cache = std::fs::read_to_string(dir.join("home/update-check")).unwrap();
    assert_eq!(cache.lines().nth(1), Some("v9.0.0"));
}

#[test]
fn update_check_records_the_latest_release() {
    let exe = kiln_copy("notice");
    let base = serve(false);
    let home = exe.parent().unwrap().join("home");

    let out = kiln(&exe, &base, &home, "update-check", &[]);
    assert!(out.status.success());
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    let cache = std::fs::read_to_string(home.join("update-check")).unwrap();
    let mut lines = cache.lines();
    assert!(lines.next().unwrap().parse::<u64>().is_ok());
    assert_eq!(lines.next(), Some("v9.0.0"));

    // Without a terminal, commands do not show the notice.
    let out = kiln(&exe, &base, &home, "explain", &["E0101"]);
    assert!(out.status.success());
    assert!(!String::from_utf8_lossy(&out.stderr).contains("is available"));

    // When the server is gone, what was found before stays.
    let out = kiln(
        &exe,
        "http://127.0.0.1:1/releases",
        &home,
        "update-check",
        &[],
    );
    assert!(out.status.success());
    let cache = std::fs::read_to_string(home.join("update-check")).unwrap();
    assert_eq!(cache.lines().nth(1), Some("v9.0.0"));
}

/// Under a pseudo-terminal (from `script`), a command shows the notice
/// after its output.
#[test]
fn commands_in_a_terminal_say_a_newer_kiln_is_out() {
    if Command::new("script").arg("--version").output().is_err() {
        return;
    }
    let exe = kiln_copy("terminal");
    let home = exe.parent().unwrap().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    std::fs::write(home.join("update-check"), format!("{now}\nv9.0.0\n")).unwrap();
    let run = |no_check: &str| {
        let out = Command::new("script")
            .args([
                "-qec",
                &format!("{} explain E0101", exe.display()),
                "/dev/null",
            ])
            .env("KILN_HOME", &home)
            .env("KILN_NO_UPDATE_CHECK", no_check)
            .env_remove("CI")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let shown = run("");
    assert!(shown.contains("note: kiln 9.0.0 is available"), "{shown}");
    assert!(!run("1").contains("is available"));
}

/// What happened with 0.5.3: `kiln upgrade` recorded a check, and a newer
/// release came out an hour or two later. Commands in a terminal start a
/// check in the background, and the command after it says so.
#[test]
fn a_release_after_an_upgrade_is_noticed() {
    if Command::new("script").arg("--version").output().is_err() {
        return;
    }
    let exe = kiln_copy("refresh");
    let base = serve(false);
    let home = exe.parent().unwrap().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let cache = home.join("update-check");
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let current = env!("CARGO_PKG_VERSION");
    std::fs::write(&cache, format!("{}\nv{current}\n", now - 2 * 60 * 60)).unwrap();
    let run = || {
        let out = Command::new("script")
            .args([
                "-qec",
                &format!("{} explain E0101", exe.display()),
                "/dev/null",
            ])
            .env("KILN_HOME", &home)
            .env("KILN_RELEASES_URL", &base)
            .env_remove("KILN_NO_UPDATE_CHECK")
            .env_remove("CI")
            .env_remove("HTTP_PROXY")
            .env_remove("HTTPS_PROXY")
            .env_remove("http_proxy")
            .env_remove("https_proxy")
            .env_remove("ALL_PROXY")
            .env_remove("all_proxy")
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(!run().contains("is available"));
    // The background check outlives the command and records v9.0.0.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !std::fs::read_to_string(&cache)
        .unwrap_or_default()
        .contains("v9.0.0")
    {
        assert!(
            std::time::Instant::now() < deadline,
            "the background check did not record the new release"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let shown = run();
    assert!(shown.contains("note: kiln 9.0.0 is available"), "{shown}");
}

#[test]
fn upgrade_refuses_a_download_with_the_wrong_checksum() {
    let exe = kiln_copy("checksum");
    let before = std::fs::read(&exe).unwrap();
    let out = upgrade(&exe, &serve(true), &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("does not match its checksum"));
    assert!(std::fs::read(&exe).unwrap() == before);
}

#[test]
fn upgrade_reports_a_missing_release() {
    let exe = kiln_copy("missing");
    let out = upgrade(&exe, &serve(false), &["--version", "v8.0.0"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&out.stderr).contains("release v8.0.0 not found"));
}
