//! The client side of the Ware registry protocol, version 1. A registry
//! serves JSON metadata and archives over HTTP; everything it sends is
//! checked as untrusted input. Plain HTTP is allowed only to this machine.
//!
//! | Request | |
//! | --- | --- |
//! | `GET /v1/wares/{publisher}/{name}` | Every release: version, checksum, dependencies, capabilities, yanked, trust |
//! | `GET /v1/wares/{publisher}/{name}/{version}/archive` | The release's archive (`archive.rs`) |
//! | `PUT /v1/wares/{publisher}/{name}/{version}` | Publish, with `Authorization: Bearer <token>` |
//! | `POST /v1/wares/{publisher}/{name}/{version}/yank` | Yank or restore, with `{"yanked": bool}` |

use super::manifest::{self, DepSource, Dependency};
use super::name::WareName;
use semver::Version;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::PathBuf;
use std::time::Duration;

const MAX_INDEX: u64 = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct Release {
    pub version: Version,
    pub checksum: String,
    pub dependencies: BTreeMap<String, Dependency>,
    pub capabilities: BTreeSet<String>,
    pub yanked: bool,
    pub trust: String,
}

fn host_of(url: &str) -> Option<(&str, &str)> {
    let (scheme, rest) = url.split_once("://")?;
    let authority = rest.split('/').next()?;
    let host = if let Some(v6) = authority.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        authority.rsplit_once(':').map_or(authority, |(h, _)| h)
    };
    Some((scheme, host))
}

pub fn is_loopback(url: &str) -> bool {
    host_of(url).is_some_and(|(_, h)| h == "127.0.0.1" || h == "localhost" || h == "::1")
}

/// A registry URL: HTTPS, or HTTP to this machine. `KILN_INSECURE_REGISTRY=1`
/// allows plain HTTP elsewhere, for a development network, with a warning.
pub fn check_url(url: &str) -> Result<(), String> {
    let (scheme, host) = host_of(url).ok_or_else(|| format!("`{url}` is not a URL"))?;
    if host.is_empty() || url.contains('@') || url.contains(['?', '#']) {
        return Err(format!("`{url}` is not a registry URL"));
    }
    match scheme {
        "https" => Ok(()),
        "http" if is_loopback(url) => Ok(()),
        "http" if std::env::var("KILN_INSECURE_REGISTRY").is_ok_and(|v| v == "1") => {
            eprintln!("warning: {url} is plain HTTP; anyone on the network can change what it serves");
            Ok(())
        }
        "http" => Err(format!(
            "`{url}` is plain HTTP to another machine; use HTTPS (plain HTTP is only for 127.0.0.1 and localhost)"
        )),
        _ => Err(format!("`{url}`: a registry URL starts with https://")),
    }
}

fn agent(url: &str) -> Result<ureq::Agent, String> {
    if is_loopback(url) {
        // A local registry is reached directly, never through a proxy.
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .proxy(None)
            .user_agent(concat!("kiln/", env!("CARGO_PKG_VERSION")))
            .timeout_global(Some(Duration::from_secs(60)))
            .build();
        return Ok(config.into());
    }
    crate::upgrade::agent(Duration::from_secs(120))
}

fn read_body(
    mut response: ureq::http::Response<ureq::Body>,
    limit: u64,
    url: &str,
) -> Result<String, String> {
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(limit + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("cannot read {url}: {e}"))?;
    if body.len() as u64 > limit {
        return Err(format!("{url} sent more than {limit} bytes"));
    }
    String::from_utf8(body).map_err(|_| format!("{url} sent text that is not UTF-8"))
}

fn error_of(status: u16, body: &str) -> String {
    let message = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(String::from))
        .unwrap_or_else(|| body.chars().take(200).collect());
    format!("HTTP {status}: {message}")
}

pub fn index(registry: &str, name: &WareName) -> Result<Vec<Release>, String> {
    let url = format!("{registry}/v1/wares/{}/{}", name.publisher, name.name);
    let response = agent(registry)?
        .get(&url)
        .call()
        .map_err(|e| format!("cannot reach the registry at {registry}: {e}"))?;
    let status = response.status().as_u16();
    let body = read_body(response, MAX_INDEX, &url)?;
    if status == 404 {
        return Err(format!("{registry} has no Ware {name}"));
    }
    if status != 200 {
        return Err(format!("{url}: {}", error_of(status, &body)));
    }
    parse_index(&body, name, registry)
}

/// Reads an index, refusing anything malformed rather than guessing.
pub fn parse_index(body: &str, name: &WareName, registry: &str) -> Result<Vec<Release>, String> {
    let bad = |why: &str| format!("{registry} sent a bad index for {name}: {why}");
    let value: Value = serde_json::from_str(body).map_err(|e| bad(&e.to_string()))?;
    if value.get("name").and_then(Value::as_str) != Some(&name.to_string()) {
        return Err(bad("it is for another Ware"));
    }
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    for r in value
        .get("releases")
        .and_then(Value::as_array)
        .ok_or_else(|| bad("no releases"))?
    {
        let s = |k: &str| r.get(k).and_then(Value::as_str);
        let version =
            Version::parse(s("version").ok_or_else(|| bad("a release without a version"))?)
                .map_err(|e| bad(&e.to_string()))?;
        if !seen.insert(version.clone()) {
            return Err(bad(&format!("{version} is listed twice")));
        }
        let checksum = s("checksum")
            .ok_or_else(|| bad("a release without a checksum"))?
            .to_string();
        super::cache::ware_dir(&checksum).map_err(|e| bad(&e))?;
        let mut dependencies = BTreeMap::new();
        if let Some(deps) = r.get("dependencies").and_then(Value::as_object) {
            for (alias, d) in deps {
                super::name::check_alias(alias).map_err(|e| bad(&e))?;
                let ware = d
                    .get("ware")
                    .and_then(Value::as_str)
                    .ok_or_else(|| bad("a dependency without a Ware"))?;
                let req = d.get("version").and_then(Value::as_str).unwrap_or("*");
                let dep_registry = d.get("registry").and_then(Value::as_str).map(String::from);
                if let Some(r) = &dep_registry {
                    check_url(r).map_err(|e| bad(&e))?;
                }
                dependencies.insert(
                    alias.clone(),
                    Dependency {
                        name: WareName::parse(ware).map_err(|e| bad(&e))?,
                        source: DepSource::Registry {
                            req: manifest::parse_req(req).map_err(|e| bad(&e))?,
                            registry: dep_registry,
                        },
                    },
                );
            }
        }
        let known: Vec<&str> = manifest::CAPABILITIES.iter().map(|(n, _)| *n).collect();
        let mut capabilities = BTreeSet::new();
        for c in r
            .get("capabilities")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let c = c
                .as_str()
                .filter(|c| known.contains(c))
                .ok_or_else(|| bad("an unknown capability"))?;
            capabilities.insert(c.to_string());
        }
        let trust = s("trust").unwrap_or("unverified");
        if !manifest::TRUST.contains(&trust) {
            return Err(bad(&format!("unknown trust tier `{trust}`")));
        }
        out.push(Release {
            version,
            checksum,
            dependencies,
            capabilities,
            yanked: r.get("yanked").and_then(Value::as_bool).unwrap_or(false),
            trust: trust.into(),
        });
    }
    Ok(out)
}

pub fn archive(registry: &str, name: &WareName, version: &Version) -> Result<String, String> {
    let url = format!(
        "{registry}/v1/wares/{}/{}/{version}/archive",
        name.publisher, name.name
    );
    let response = agent(registry)?
        .get(&url)
        .call()
        .map_err(|e| format!("cannot reach the registry at {registry}: {e}"))?;
    let status = response.status().as_u16();
    let body = read_body(response, super::archive::MAX_ARCHIVE as u64, &url)?;
    if status != 200 {
        return Err(format!("{url}: {}", error_of(status, &body)));
    }
    Ok(body)
}

/// Sends a release. The registry checks the token's publisher and the
/// checksum, and refuses a different archive for a version it has.
pub fn publish(
    registry: &str,
    token: &str,
    name: &WareName,
    version: &Version,
    body: &Value,
) -> Result<String, String> {
    let url = format!(
        "{registry}/v1/wares/{}/{}/{version}",
        name.publisher, name.name
    );
    let response = agent(registry)?
        .put(&url)
        .header("authorization", &format!("Bearer {token}"))
        .header("content-type", "application/json")
        .send(body.to_string().as_bytes())
        .map_err(|e| format!("cannot reach the registry at {registry}: {e}"))?;
    let status = response.status().as_u16();
    let text = read_body(response, MAX_INDEX, &url)?;
    match status {
        200 => Ok(format!(
            "{name} {version} was already published with this checksum"
        )),
        201 => Ok(format!("published {name} {version} to {registry}")),
        _ => Err(format!(
            "cannot publish {name} {version}: {}",
            error_of(status, &text)
        )),
    }
}

pub fn yank(
    registry: &str,
    token: &str,
    name: &WareName,
    version: &Version,
    yanked: bool,
) -> Result<(), String> {
    let url = format!(
        "{registry}/v1/wares/{}/{}/{version}/yank",
        name.publisher, name.name
    );
    let response = agent(registry)?
        .post(&url)
        .header("authorization", &format!("Bearer {token}"))
        .header("content-type", "application/json")
        .send(
            serde_json::json!({ "yanked": yanked })
                .to_string()
                .as_bytes(),
        )
        .map_err(|e| format!("cannot reach the registry at {registry}: {e}"))?;
    let status = response.status().as_u16();
    let text = read_body(response, MAX_INDEX, &url)?;
    if status == 200 {
        Ok(())
    } else {
        Err(error_of(status, &text))
    }
}

/// Where kiln keeps settings for the user: `KILN_CONFIG_DIR`, or
/// `%APPDATA%\kiln`, or `$XDG_CONFIG_HOME/kiln`, or `~/.config/kiln`.
pub fn config_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("KILN_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    if cfg!(windows) {
        if let Some(app) = std::env::var_os("APPDATA") {
            return PathBuf::from(app).join("kiln");
        }
    }
    if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME").filter(|d| !d.is_empty()) {
        return PathBuf::from(xdg).join("kiln");
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    home.join(".config").join("kiln")
}

/// The publish token for a registry: `KILN_REGISTRY_TOKEN`, else the one
/// `kiln login` saved. Tokens are never written to manifests or lockfiles.
pub fn token(registry: &str) -> Option<String> {
    if let Ok(t) = std::env::var("KILN_REGISTRY_TOKEN") {
        if !t.is_empty() {
            return Some(t);
        }
    }
    let text = std::fs::read_to_string(config_dir().join("credentials.toml")).ok()?;
    let table: toml::Table = toml::from_str(&text).ok()?;
    table
        .get("registries")?
        .get(registry)?
        .get("token")?
        .as_str()
        .map(String::from)
}

/// Saves a token in `credentials.toml`, readable only by the user on
/// Unix. Windows keeps it in the user's own profile folder.
pub fn save_token(registry: &str, token: &str) -> Result<PathBuf, String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("cannot write {}: {e}", dir.display()))?;
    let path = dir.join("credentials.toml");
    let mut table: toml::Table = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| toml::from_str(&t).ok())
        .unwrap_or_default();
    let registries = table
        .entry("registries")
        .or_insert_with(|| toml::Value::Table(Default::default()));
    if let Some(r) = registries.as_table_mut() {
        let mut entry = toml::Table::new();
        entry.insert("token".into(), toml::Value::String(token.into()));
        r.insert(registry.into(), toml::Value::Table(entry));
    }
    let text = format!(
        "# Registry tokens saved by `kiln login`. Keep this file private.\n{}",
        toml::to_string(&table).map_err(|e| e.to_string())?
    );
    let tmp = path.with_extension("tmp");
    {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        use std::io::Write;
        let mut file = options
            .open(&tmp)
            .map_err(|e| format!("cannot write {}: {e}", tmp.display()))?;
        file.write_all(text.as_bytes()).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, &path).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert!(check_url("http://127.0.0.1:8081").is_ok());
        assert!(check_url("http://localhost:8081").is_ok());
        assert!(check_url("https://wares.example").is_ok());
        assert!(check_url("http://wares.example").is_err());
        assert!(check_url("http://user:pw@127.0.0.1").is_err());
        assert!(check_url("ftp://x").is_err());
    }

    #[test]
    fn untrusted_index() {
        let name = WareName::parse("kiln/ui").unwrap();
        let sum = format!("sha256:{}", "b".repeat(64));
        let ok = format!(
            r#"{{"name":"kiln/ui","releases":[{{"version":"0.1.0","checksum":"{sum}","dependencies":{{"icons":{{"ware":"kiln/icons","version":"^1"}}}},"capabilities":[],"yanked":false,"trust":"verified-publisher"}}]}}"#
        );
        let r = parse_index(&ok, &name, "http://127.0.0.1:1").unwrap();
        assert_eq!(r[0].dependencies["icons"].name.to_string(), "kiln/icons");
        assert!(parse_index(&ok.replace("kiln/ui", "evil/ui"), &name, "x").is_err());
        assert!(parse_index(&ok.replace(&sum, "sha256:zz"), &name, "x").is_err());
        assert!(parse_index(&ok.replace("verified-publisher", "audited"), &name, "x").is_err());
        assert!(parse_index(
            &ok.replace("\"capabilities\":[]", "\"capabilities\":[\"root\"]"),
            &name,
            "x"
        )
        .is_err());
    }
}
