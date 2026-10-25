//! `kiln upgrade`: replaces this kiln with a release from GitHub. It finds
//! and checks releases the way `install.sh` and `install.ps1` do, so a kiln
//! they installed keeps itself up to date.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

/// Where releases come from. `KILN_RELEASES_URL` replaces it in tests.
const RELEASES: &str = "https://github.com/IZ737/kiln/releases";

/// What `kiln upgrade` was asked to do.
pub struct Options {
    /// The release to install, such as `v0.2.0`; the latest when `None`.
    pub version: Option<String>,
    /// Only say whether there is a newer release.
    pub check: bool,
    /// Install even when this kiln is already that version.
    pub force: bool,
}

pub fn run(options: Options) -> ExitCode {
    match upgrade(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}

fn upgrade(options: Options) -> Result<(), String> {
    let exe = current_exe()?;
    // A Windows upgrade leaves the old kiln.exe behind while it runs.
    let _ = std::fs::remove_file(old_path(&exe));

    let releases = releases_url();
    let releases = releases.as_str();
    let agent = agent(Duration::from_secs(300))?;

    let tag = match options.version {
        Some(version) if version.starts_with('v') => version,
        Some(version) => format!("v{version}"),
        None => {
            let tag = latest(&agent, releases)?;
            crate::notice::save(Some(&tag));
            tag
        }
    };
    let current = env!("CARGO_PKG_VERSION");
    let wanted = tag.trim_start_matches('v');
    if wanted == current && !options.force {
        println!("kiln {current} is up to date");
        return Ok(());
    }
    if options.check {
        println!(
            "kiln {wanted} is available (this is kiln {current}); run `kiln upgrade` to install it"
        );
        return Ok(());
    }

    let (os, arch) = system();
    let name = asset_name(&tag, os, arch);
    let url = format!("{releases}/download/{tag}/{name}");
    println!("downloading kiln {wanted} for {os} {arch}");
    let archive = match get(&agent, &url)? {
        Some(bytes) => bytes,
        None => {
            // Tell a missing release apart from a release without this system.
            return Err(match get(&agent, &format!("{releases}/tag/{tag}")) {
                Ok(Some(_)) => format!(
                    "release {tag} has no prebuilt kiln for {os} {arch}; build it from source: https://github.com/IZ737/kiln#try-it"
                ),
                _ => format!("release {tag} not found; the releases are at {releases}"),
            });
        }
    };
    let sums = get(&agent, &format!("{url}.sha256"))?
        .ok_or_else(|| format!("release {tag} has no checksum file {name}.sha256"))?;
    verify(&archive, &sums)?;
    let binary = extract(&archive, &tag, os, arch)?;
    replace(&exe, &binary)?;
    println!(
        "installed kiln {wanted} at {} (was {current})",
        exe.display()
    );
    Ok(())
}

/// The latest release's tag, for the notice that a newer kiln is out. It
/// gives up sooner than an upgrade.
pub fn latest_release() -> Result<String, String> {
    latest(&agent(Duration::from_secs(15))?, &releases_url())
}

fn releases_url() -> String {
    let releases = std::env::var("KILN_RELEASES_URL").unwrap_or_else(|_| RELEASES.to_string());
    releases.trim_end_matches('/').to_string()
}

/// This executable, with symbolic links followed on Unix so the file
/// itself is replaced. (On Windows that would give a `\\?\` path.)
fn current_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| format!("cannot find this kiln: {err}"))?;
    if cfg!(unix) {
        Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
    } else {
        Ok(exe)
    }
}

fn old_path(exe: &Path) -> PathBuf {
    let mut name = exe.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    exe.with_file_name(name)
}

/// An HTTP client that trusts the system's certificates, so it works behind
/// proxies that a company's own certificate signs, and Mozilla's where the
/// system has none (such as an empty container). It uses the HTTPS_PROXY
/// and NO_PROXY environment variables. Requests give up after `timeout`.
fn agent(timeout: Duration) -> Result<ureq::Agent, String> {
    use ureq::tls::{Certificate, RootCerts, TlsConfig};
    let native = rustls_native_certs::load_native_certs().certs;
    let certs: Vec<Certificate<'static>> = if native.is_empty() {
        webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|der| Certificate::from_der(der.as_ref()).to_owned())
            .collect()
    } else {
        native
            .iter()
            .map(|der| Certificate::from_der(der.as_ref()).to_owned())
            .collect()
    };
    let tls = TlsConfig::builder()
        .root_certs(RootCerts::Specific(Arc::new(certs)))
        .build();
    let config = ureq::Agent::config_builder()
        .tls_config(tls)
        .http_status_as_error(false)
        .user_agent(concat!("kiln/", env!("CARGO_PKG_VERSION")))
        .timeout_global(Some(timeout))
        .build();
    Ok(config.into())
}

/// The body at `url`, or `None` when there is nothing there (404).
/// Redirects, which GitHub sends for every download, are followed here
/// without reading their bodies, which some proxies garble.
fn get(agent: &ureq::Agent, url: &str) -> Result<Option<Vec<u8>>, String> {
    let mut url = url.to_string();
    for _ in 0..10 {
        let mut response = agent
            .get(&url)
            .config()
            .max_redirects(0)
            .build()
            .call()
            .map_err(|err| format!("cannot download {url}: {err}"))?;
        let status = response.status().as_u16();
        if (300..400).contains(&status) {
            url = redirect(&url, response.headers())
                .ok_or_else(|| format!("cannot download {url}: HTTP {status} with no location"))?;
            continue;
        }
        match status {
            200 => {}
            404 => return Ok(None),
            _ => return Err(format!("cannot download {url}: HTTP {status}")),
        }
        let mut body = Vec::new();
        response
            .body_mut()
            .as_reader()
            .read_to_end(&mut body)
            .map_err(|err| format!("cannot download {url}: {err}"))?;
        return Ok(Some(body));
    }
    Err(format!("cannot download {url}: too many redirects"))
}

/// Where a redirect from `from` points.
fn redirect(from: &str, headers: &ureq::http::HeaderMap) -> Option<String> {
    let location = headers.get("location")?.to_str().ok()?;
    if location.starts_with('/') {
        // A path on the same server: keep the scheme and host.
        let host_end = from.find("://").map_or(0, |i| i + 3);
        let path_start = from[host_end..]
            .find('/')
            .map_or(from.len(), |i| host_end + i);
        Some(format!("{}{location}", &from[..path_start]))
    } else {
        Some(location.to_string())
    }
}

/// The latest release's tag, from where `releases/latest` redirects. This
/// needs no API token and has no rate limit.
fn latest(agent: &ureq::Agent, releases: &str) -> Result<String, String> {
    let url = format!("{releases}/latest");
    let response = agent
        .get(&url)
        .config()
        .max_redirects(0)
        .build()
        .call()
        .map_err(|err| format!("cannot find the latest release at {releases}: {err}"))?;
    let location = redirect(&url, response.headers()).unwrap_or_default();
    let tag = location
        .rsplit('/')
        .next()
        .filter(|tag| tag.starts_with('v') && tag[1..].starts_with(|c: char| c.is_ascii_digit()));
    match tag {
        Some(tag) => Ok(tag.to_string()),
        None => Err(format!("cannot find the latest release at {releases}")),
    }
}

/// The names releases use for this system, as in `release.yml`: Rust's
/// own, such as `linux` and `x86_64`.
fn system() -> (&'static str, &'static str) {
    (std::env::consts::OS, std::env::consts::ARCH)
}

/// The release archive for a system: `kiln-v0.3.0-x86_64-linux.tar.gz`, or
/// a `.zip` for Windows.
fn asset_name(tag: &str, os: &str, arch: &str) -> String {
    let ext = if os == "windows" { "zip" } else { "tar.gz" };
    format!("kiln-{tag}-{arch}-{os}.{ext}")
}

/// Checks `archive` against a `sha256sum` line: the hash, then the name.
fn verify(archive: &[u8], sums: &[u8]) -> Result<(), String> {
    let expected = String::from_utf8_lossy(sums)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let actual: String = Sha256::digest(archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if expected == actual {
        Ok(())
    } else {
        Err(format!(
            "the download does not match its checksum (expected {expected}, got {actual}); try again"
        ))
    }
}

/// The kiln executable inside a release archive, which holds a folder named
/// like the archive.
fn extract(archive: &[u8], tag: &str, os: &str, arch: &str) -> Result<Vec<u8>, String> {
    let folder = format!("kiln-{tag}-{arch}-{os}");
    let bad = |err: std::io::Error| format!("cannot unpack the download: {err}");
    let mut binary = Vec::new();
    if os == "windows" {
        let path = format!("{folder}/kiln.exe");
        let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive))
            .map_err(|err| format!("cannot unpack the download: {err}"))?;
        let mut file = zip
            .by_name(&path)
            .map_err(|_| format!("the download has no {path} in it"))?;
        file.read_to_end(&mut binary).map_err(bad)?;
        return Ok(binary);
    }
    let path = format!("{folder}/kiln");
    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(archive));
    for entry in tar.entries().map_err(bad)? {
        let mut entry = entry.map_err(bad)?;
        if entry.path().map_err(bad)?.as_ref() == Path::new(&path) {
            entry.read_to_end(&mut binary).map_err(bad)?;
            return Ok(binary);
        }
    }
    Err(format!("the download has no {path} in it"))
}

/// Puts `binary` in place of `exe`, which is running. The new file is
/// written beside it and checked to run first. On Unix a rename replaces
/// the running file; Windows cannot replace a running executable but can
/// rename it, so the old one moves aside to `kiln.exe.old`, which the next
/// upgrade deletes.
fn replace(exe: &Path, binary: &[u8]) -> Result<(), String> {
    let dir = exe.parent().unwrap_or(Path::new("."));
    let new = dir.join(format!("kiln-upgrade{}", std::env::consts::EXE_SUFFIX));
    let denied = |err: std::io::Error| {
        format!(
            "cannot replace {}: {err}{}",
            exe.display(),
            if err.kind() == std::io::ErrorKind::PermissionDenied {
                "; run kiln upgrade as a user who can change that file"
            } else {
                ""
            }
        )
    };
    std::fs::write(&new, binary).map_err(denied)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&new, std::fs::Permissions::from_mode(0o755));
    }
    let runs = std::process::Command::new(&new)
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success());
    if !runs {
        let _ = std::fs::remove_file(&new);
        return Err("the downloaded kiln does not run on this system".to_string());
    }
    if cfg!(windows) {
        let old = old_path(exe);
        std::fs::rename(exe, &old).map_err(|err| {
            let _ = std::fs::remove_file(&new);
            denied(err)
        })?;
        if let Err(err) = std::fs::rename(&new, exe) {
            let _ = std::fs::rename(&old, exe);
            let _ = std::fs::remove_file(&new);
            return Err(denied(err));
        }
        // Fails while this kiln is running, which is expected.
        let _ = std::fs::remove_file(&old);
    } else {
        std::fs::rename(&new, exe).map_err(|err| {
            let _ = std::fs::remove_file(&new);
            denied(err)
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names_match_the_release_workflow() {
        assert_eq!(
            asset_name("v0.3.0", "linux", "x86_64"),
            "kiln-v0.3.0-x86_64-linux.tar.gz"
        );
        assert_eq!(
            asset_name("v0.3.0", "windows", "x86_64"),
            "kiln-v0.3.0-x86_64-windows.zip"
        );
    }

    #[test]
    fn verify_reads_sha256sum_lines() {
        let sum =
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  hello.tar.gz\n";
        assert!(verify(b"hello", sum.as_bytes()).is_ok());
        assert!(verify(b"hellp", sum.as_bytes())
            .unwrap_err()
            .contains("does not match its checksum"));
    }

    #[test]
    fn extract_finds_kiln_in_both_archives() {
        let mut tar = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "kiln-v1.0.0-x86_64-linux/kiln", &b"elf!"[..])
            .unwrap();
        let tgz = tar.into_inner().unwrap().finish().unwrap();
        assert_eq!(extract(&tgz, "v1.0.0", "linux", "x86_64").unwrap(), b"elf!");
        assert!(extract(&tgz, "v1.0.1", "linux", "x86_64").is_err());

        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file(
            "kiln-v1.0.0-x86_64-windows/kiln.exe",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
        std::io::Write::write_all(&mut zip, b"MZ!!").unwrap();
        let zip = zip.finish().unwrap().into_inner();
        assert_eq!(
            extract(&zip, "v1.0.0", "windows", "x86_64").unwrap(),
            b"MZ!!"
        );
    }
}
