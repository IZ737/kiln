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
    /// Why the last check could not ask GitHub, if it could not.
    error: Option<String>,
}

/// Says that a newer kiln is out, from what the last check found, and
/// starts a new check when that is old.
pub fn show() {
    if let Some(text) = start() {
        eprintln!("{text}");
    }
}

/// Starts a new check when the last one is old, and gives the notice to
/// print, if there is one. Commands call this before they run, so a
/// command stopped with Ctrl-C, such as `kiln dev` or a server under `kiln
/// run`, still starts the check; they print the notice when they finish,
/// or first when they may never finish.
pub fn start() -> Option<String> {
    use std::io::IsTerminal;
    if disabled(|name| std::env::var(name).ok()) || !std::io::stderr().is_terminal() {
        return None;
    }
    let path = cache_path()?;
    let cache = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| parse(&text));
    if stale(cache.as_ref(), now()) {
        refresh_in_background();
    }
    text(cache?.latest.as_deref()?, current())
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
    match crate::upgrade::latest_release() {
        Ok(latest) => {
            save(Some(&latest));
            ExitCode::SUCCESS
        }
        Err(err) => {
            // Keep what was found before and try again in an hour rather
            // than on every command, but say why, for `kiln upgrade --check`.
            write(&Cache {
                checked: now(),
                latest: old.and_then(|old| old.latest),
                error: Some(err),
            });
            ExitCode::FAILURE
        }
    }
}

/// Records the latest release, as `kiln upgrade` finds it too.
pub fn save(latest: Option<&str>) {
    write(&Cache {
        checked: now(),
        latest: latest.map(str::to_string),
        error: None,
    });
}

fn write(cache: &Cache) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, format(cache));
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
    newer(latest, current).then(|| {
        format!("note: kiln {latest} is available (you have {current}); run `kiln upgrade` to install it")
    })
}

/// Whether version `a` (`0.5.2` or `v0.5.2`) is newer than `b`; false when
/// either is not a version.
pub fn newer(a: &str, b: &str) -> bool {
    let a = version(a.trim_start_matches('v'));
    let b = version(b.trim_start_matches('v'));
    matches!((a, b), (Some(a), Some(b)) if a > b)
}

/// `0.5.2` as numbers to compare, or `None` for anything else.
fn version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.split('.').map(|part| part.parse().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// The cache file holds the time of the check, the tag and, when the
/// check failed, why, one per line.
fn parse(text: &str) -> Option<Cache> {
    let mut lines = text.lines();
    let checked = lines.next()?.trim().parse().ok()?;
    let mut line = || {
        lines
            .next()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_string)
    };
    let latest = line();
    let error = line();
    Some(Cache {
        checked,
        latest,
        error,
    })
}

fn format(cache: &Cache) -> String {
    let mut text = format!(
        "{}\n{}\n",
        cache.checked,
        cache.latest.as_deref().unwrap_or("")
    );
    if let Some(error) = &cache.error {
        text.push_str(&error.replace(['\n', '\r'], " "));
        text.push('\n');
    }
    text
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
    // No console, its own process group so Ctrl-C does not reach it, and
    // out of the terminal's job where that allows it, so a terminal that
    // closes every process it started (as editors' terminals do) does not
    // stop it either.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB);
        if command.spawn().is_ok() {
            return;
        }
        // A job that does not allow breaking away refuses the flag.
        command.creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP);
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
        assert!(newer("v0.16.0", "0.15.0"));
        assert!(!newer("v0.15.0", "0.16.0"));
    }

    #[test]
    fn the_cache_round_trips() {
        let cache = Cache {
            checked: 1_800_000_000,
            latest: Some("v0.5.2".to_string()),
            error: None,
        };
        assert_eq!(parse(&format(&cache)), Some(cache));
        let empty = Cache {
            checked: 5,
            latest: None,
            error: None,
        };
        assert_eq!(parse(&format(&empty)), Some(empty));
        let failed = Cache {
            checked: 5,
            latest: Some("v0.5.2".to_string()),
            error: Some("cannot find the latest release".to_string()),
        };
        assert_eq!(parse(&format(&failed)), Some(failed));
        // A cache written by an older kiln has two lines.
        assert_eq!(parse("5\nv0.5.2\n").unwrap().error, None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("garbage\nv1.0.0\n"), None);
    }

    #[test]
    fn checks_again_after_an_hour() {
        let cache = |checked| Cache {
            checked,
            latest: None,
            error: None,
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
