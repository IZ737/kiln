//! The notice that a newer kiln is out. Commands read what the last check
//! found from `~/.kiln/update-check` (or `$KILN_HOME/update-check`), so
//! they never wait for the network; when that is an hour old, a `kiln
//! update-check` in the background asks GitHub again for the next command.
//!
//! The notice goes to standard error, and only when that is a terminal:
//! never in CI, editors, pipes or `--json` output. `KILN_NO_UPDATE_CHECK=1`
//! turns it off.

use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

/// How often to ask for the latest release, in seconds. An hour, since
/// releases can come out a few times a day, and `kiln upgrade` records a
/// check too: with a day, a release soon after an upgrade went unnoticed
/// until the next day.
const INTERVAL: u64 = 60 * 60;

/// What the last check found.
#[derive(Debug, PartialEq, Eq)]
struct Cache {
    /// When it ran, in seconds since 1970.
    checked: u64,
    /// The latest release's tag, such as `v0.5.2`, if it was found.
    latest: Option<String>,
}

/// Says that a newer kiln is out, from what the last check found, and
/// starts a new check when that is old. Called after a command finishes.
pub fn show() {
    use std::io::IsTerminal;
    if disabled(|name| std::env::var(name).ok()) || !std::io::stderr().is_terminal() {
        return;
    }
    let Some(path) = cache_path() else {
        return;
    };
    let cache = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| parse(&text));
    if stale(cache.as_ref(), now()) {
        refresh_in_background();
    }
    if let Some(text) = cache.and_then(|cache| text(cache.latest.as_deref()?, current())) {
        eprintln!("{text}");
    }
}

/// `kiln update-check`, which `show` starts: asks for the latest release
/// and writes it to the cache. It prints nothing.
pub fn update_check() -> ExitCode {
    let Some(path) = cache_path() else {
        return ExitCode::FAILURE;
    };
    let old = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| parse(&text));
    // When GitHub cannot be reached, keep what was found before, and try
    // again in an hour rather than on every command.
    let latest = crate::upgrade::latest_release()
        .ok()
        .or(old.and_then(|old| old.latest));
    save(latest.as_deref());
    ExitCode::SUCCESS
}

/// Records the latest release, as `kiln upgrade` finds it too.
pub fn save(latest: Option<&str>) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let cache = Cache {
        checked: now(),
        latest: latest.map(str::to_string),
    };
    let _ = std::fs::write(path, format(&cache));
}

/// Whether the environment turns the notice off: `KILN_NO_UPDATE_CHECK`
/// set to anything but `0`, or `CI` set, as CI services do.
fn disabled(var: impl Fn(&str) -> Option<String>) -> bool {
    let off = var("KILN_NO_UPDATE_CHECK").is_some_and(|v| !v.is_empty() && v != "0");
    off || var("CI").is_some_and(|v| !v.is_empty())
}

/// Whether to ask again: never asked, asked an hour ago, or the clock went
/// back.
fn stale(cache: Option<&Cache>, now: u64) -> bool {
    cache.is_none_or(|cache| cache.checked > now || now - cache.checked >= INTERVAL)
}

/// The notice, when the release `latest` is newer than kiln `current`.
fn text(latest: &str, current: &str) -> Option<String> {
    let latest = latest.trim_start_matches('v');
    (version(latest)? > version(current)?).then(|| {
        format!("note: kiln {latest} is available (you have {current}); run `kiln upgrade` to install it")
    })
}

/// `0.5.2` as numbers to compare, or `None` for anything else.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.').map(|part| part.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// The cache file holds the time of the check and the tag, one per line.
fn parse(text: &str) -> Option<Cache> {
    let mut lines = text.lines();
    let checked = lines.next()?.trim().parse().ok()?;
    let latest = lines
        .next()
        .map(str::trim)
        .filter(|tag| !tag.is_empty())
        .map(str::to_string);
    Some(Cache { checked, latest })
}

fn format(cache: &Cache) -> String {
    format!(
        "{}\n{}\n",
        cache.checked,
        cache.latest.as_deref().unwrap_or("")
    )
}

/// `$KILN_HOME/update-check`, where the installers put kiln, or
/// `~/.kiln/update-check`.
fn cache_path() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    let home = var("KILN_HOME").map(PathBuf::from).or_else(|| {
        let user = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        var(user).map(|home| PathBuf::from(home).join(".kiln"))
    })?;
    Some(home.join("update-check"))
}

/// Starts `kiln update-check` and leaves it running after this kiln exits.
fn refresh_in_background() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let mut command = Command::new(exe);
    command
        .arg("update-check")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // Its own process group, so closing the terminal (or Ctrl-C) does not
    // stop it halfway.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let _ = command.spawn();
}

fn current() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_newer_release_gets_a_notice() {
        assert_eq!(
            text("v0.5.2", "0.5.1").as_deref(),
            Some(
                "note: kiln 0.5.2 is available (you have 0.5.1); run `kiln upgrade` to install it"
            )
        );
        assert!(text("v0.10.0", "0.9.3").is_some());
        assert!(text("v1.0.0", "0.99.99").is_some());
        assert!(text("v0.5.1", "0.5.1").is_none());
        assert!(text("v0.5.0", "0.5.1").is_none());
        assert!(text("v0.6.0-rc1", "0.5.1").is_none());
        assert!(text("nonsense", "0.5.1").is_none());
    }

    #[test]
    fn the_cache_round_trips() {
        let cache = Cache {
            checked: 1_800_000_000,
            latest: Some("v0.5.2".to_string()),
        };
        assert_eq!(parse(&format(&cache)), Some(cache));
        let empty = Cache {
            checked: 5,
            latest: None,
        };
        assert_eq!(parse(&format(&empty)), Some(empty));
        assert_eq!(parse(""), None);
        assert_eq!(parse("garbage\nv1.0.0\n"), None);
    }

    #[test]
    fn checks_again_after_an_hour() {
        let cache = |checked| Cache {
            checked,
            latest: None,
        };
        let now = 1_800_000_000;
        assert!(stale(None, now));
        assert!(!stale(Some(&cache(now - 60)), now));
        assert!(stale(Some(&cache(now - INTERVAL)), now));
        // A release an hour or two after `kiln upgrade` recorded a check
        // is noticed the same day.
        assert!(stale(Some(&cache(now - 2 * 60 * 60)), now));
        assert!(stale(Some(&cache(now + 60)), now));
    }

    #[test]
    fn the_environment_can_turn_it_off() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert!(!disabled(env(&[])));
        assert!(disabled(env(&[("KILN_NO_UPDATE_CHECK", "1")])));
        assert!(!disabled(env(&[("KILN_NO_UPDATE_CHECK", "0")])));
        assert!(!disabled(env(&[("KILN_NO_UPDATE_CHECK", "")])));
        assert!(disabled(env(&[("CI", "true")])));
    }
}
