//! What Wares may do, and what a project allows them.
//!
//! A Ware declares its capabilities in its manifest. kiln checks the
//! declaration against the standard modules the Ware's source imports
//! (and calls to the prelude's `env`): using `fs` without declaring
//! `filesystem` is an error. This check reads imports; it is not a sandbox,
//! and a Ware's code runs with the program's full rights.
//!
//! A project's `[policy]` limits the publishers, sources, capabilities and
//! trust of every Ware in its graph, including indirect ones.

use super::lock::{Lock, Source};
use super::manifest::{Policy, TRUST};
use super::name::WareName;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// The capability each standard module needs.
pub fn capability_of(module: &str) -> Option<&'static str> {
    Some(match module {
        "fs" => "filesystem",
        "web" => "network",
        "sql" | "orm" | "auth" | "admin" => "database",
        "os" => "environment",
        "js" => "foreign",
        m if m.starts_with("npm.") => "foreign",
        _ => return None,
    })
}

/// The capabilities a Ware's source uses, each with where it is first used.
pub fn used(dir: &Path) -> Result<BTreeMap<String, String>, String> {
    let files = super::archive::read_tree(dir, false)?;
    let mut out = BTreeMap::new();
    for (path, text) in &files {
        if !path.ends_with(".kiln") {
            continue;
        }
        let parsed = kiln_syntax::parse(text);
        for item in &parsed.module.items {
            if let kiln_syntax::ast::Item::Import(import) = item {
                for p in &import.paths {
                    let name = p
                        .iter()
                        .map(|i| i.name.as_str())
                        .collect::<Vec<_>>()
                        .join(".");
                    if let Some(cap) = capability_of(&name) {
                        out.entry(cap.to_string())
                            .or_insert_with(|| format!("{path} imports `{name}`"));
                    }
                }
            }
        }
        if calls_env(text) {
            out.entry("environment".into())
                .or_insert_with(|| format!("{path} calls `env`"));
        }
    }
    Ok(out)
}

/// Whether source calls the prelude's `env(...)`, as a word not after `.`.
fn calls_env(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut from = 0;
    while let Some(i) = text[from..].find("env(") {
        let at = from + i;
        let before = if at == 0 { b' ' } else { bytes[at - 1] };
        if !(before.is_ascii_alphanumeric() || before == b'_' || before == b'.') {
            let line_start = text[..at].rfind('\n').map_or(0, |n| n + 1);
            if !text[line_start..at].contains("//") {
                return true;
            }
        }
        from = at + 4;
    }
    false
}

/// Every Ware that uses a capability it does not declare.
pub fn check_declared(
    name: &WareName,
    declared: &BTreeSet<String>,
    dir: &Path,
) -> Result<(), String> {
    let used = used(dir)?;
    let missing: Vec<String> = used
        .iter()
        .filter(|(cap, _)| !declared.contains(*cap))
        .map(|(cap, why)| format!("`{cap}` ({why})"))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{name} uses capabilities its kiln.toml does not declare: {}; add them to `capabilities` in its [package]",
            missing.join(", ")
        ))
    }
}

/// Checks the whole graph against a project's policy.
pub fn check(policy: &Policy, lock: &Lock) -> Result<(), String> {
    let mut problems = Vec::new();
    for (name, w) in &lock.wares {
        if let Some(sources) = &policy.sources {
            if !sources.contains(w.source.kind()) {
                problems.push(format!(
                    "{name} comes from {}, and the policy allows sources: {}",
                    w.source.kind(),
                    join(sources)
                ));
            }
        }
        if let Some(publishers) = &policy.publishers {
            if !matches!(w.source, Source::Path(_)) && !publishers.contains(&name.publisher) {
                problems.push(format!(
                    "{name}: the policy allows publishers: {}",
                    join(publishers)
                ));
            }
        }
        if let Some(allowed) = &policy.capabilities {
            let extra: Vec<&String> = w.capabilities.difference(allowed).collect();
            if !extra.is_empty() {
                problems.push(format!(
                    "{name} {} declares {}, which the policy does not allow",
                    w.version,
                    extra
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
        if let Some(min) = &policy.trust {
            if !matches!(w.source, Source::Path(_)) {
                let tier = w.trust.as_deref().unwrap_or("unverified");
                let rank = |t: &str| TRUST.iter().position(|x| *x == t).unwrap_or(0);
                if rank(tier) < rank(min) {
                    problems.push(format!(
                        "{name} {} is {tier}; the policy requires {min}",
                        w.version
                    ));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the dependency policy in kiln.toml refuses:\n  {}",
            problems.join("\n  ")
        ))
    }
}

fn join(set: &BTreeSet<String>) -> String {
    set.iter().cloned().collect::<Vec<_>>().join(", ")
}

/// What changes between two lockfiles, for a person to read before the
/// new one is written. `escalations` are capabilities a Ware gains, or that
/// a new Ware brings.
#[derive(Default)]
pub struct Changes {
    pub lines: Vec<String>,
    pub escalations: Vec<String>,
}

pub fn changes(old: Option<&Lock>, new: &Lock) -> Changes {
    let empty = Lock::default();
    let old = old.unwrap_or(&empty);
    let mut c = Changes::default();
    let caps = |s: &BTreeSet<String>| {
        if s.is_empty() {
            "no capabilities".to_string()
        } else {
            s.iter().cloned().collect::<Vec<_>>().join(", ")
        }
    };
    for (name, w) in &new.wares {
        let trust = w
            .trust
            .as_deref()
            .map(|t| format!(", {t}"))
            .unwrap_or_default();
        match old.wares.get(name) {
            None => {
                c.lines.push(format!(
                    "+ {name} {} from {}{trust} ({})",
                    w.version,
                    w.source,
                    caps(&w.capabilities)
                ));
                if !w.capabilities.is_empty() {
                    c.escalations
                        .push(format!("{name} needs {}", caps(&w.capabilities)));
                }
            }
            Some(o) => {
                if o.version != w.version || o.source != w.source {
                    c.lines.push(format!(
                        "~ {name} {} -> {} from {}{trust}",
                        o.version, w.version, w.source
                    ));
                }
                let gained: BTreeSet<String> = w
                    .capabilities
                    .difference(&o.capabilities)
                    .cloned()
                    .collect();
                if !gained.is_empty() {
                    c.lines
                        .push(format!("! {name} {} gains {}", w.version, caps(&gained)));
                    c.escalations
                        .push(format!("{name} gains {}", caps(&gained)));
                }
                if o.trust != w.trust && w.trust.is_some() {
                    c.lines
                        .push(format!("  {name} is now {}", w.trust.as_deref().unwrap()));
                }
            }
        }
    }
    for (name, o) in &old.wares {
        if !new.wares.contains_key(name) {
            c.lines.push(format!("- {name} {}", o.version));
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_calls() {
        assert!(calls_env("let x = env(\"A\")"));
        assert!(!calls_env("let x = os.env(\"A\")"));
        assert!(!calls_env("// env(\"A\")"));
        assert!(!calls_env("fn myenv() {}\nmyenv()"));
    }
}
