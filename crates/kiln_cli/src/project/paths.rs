//! Source paths that name files by where they are: `./card.glaze` and
//! `../shared` from the importing file's folder, `@/components/card.glaze`
//! from the project root, and `@ui/card.glaze` from a folder that
//! `[aliases]` in `kiln.toml` names. Every path stays inside the project.

use std::collections::BTreeMap;

/// The project-relative path (`components/card.glaze`) that `spec` names
/// from a file in the folder `from` (project-relative, `""` for the root).
pub fn resolve(
    spec: &str,
    from: &str,
    aliases: &BTreeMap<String, String>,
) -> Result<String, String> {
    if spec.contains('\\') {
        return Err(format!("`{spec}`: write paths with `/`"));
    }
    let (base, rest) = if let Some(rest) = spec.strip_prefix("@/") {
        (String::new(), rest)
    } else if let Some(aliased) = spec.strip_prefix('@') {
        let (name, rest) = aliased.split_once('/').unwrap_or((aliased, ""));
        match aliases.get(name) {
            Some(folder) => (folder.clone(), rest),
            None => {
                let known = if aliases.is_empty() {
                    "kiln.toml has no [aliases]".to_string()
                } else {
                    format!(
                        "the aliases are {}",
                        aliases
                            .keys()
                            .map(|a| format!("`@{a}/`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                return Err(format!(
                    "`{spec}`: there is no alias `@{name}/` ({known}; `@/` is the project root)"
                ));
            }
        }
    } else if spec.starts_with("./") || spec.starts_with("../") {
        (from.to_string(), spec)
    } else {
        return Err(format!(
            "`{spec}`: start the path with `./` or `../` (from this file's folder) or `@/` (from the project root)"
        ));
    };
    let mut parts: Vec<&str> = base.split('/').filter(|p| !p.is_empty()).collect();
    for part in rest.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.pop().is_none() {
                    return Err(format!("`{spec}` leaves the project"));
                }
            }
            p if p.starts_with("__") => {
                return Err(format!("`{spec}`: names starting with `__` are reserved"))
            }
            p => parts.push(p),
        }
    }
    if parts.is_empty() {
        return Err(format!(
            "`{spec}` is the project root, not a file or folder in it"
        ));
    }
    Ok(parts.join("/"))
}

/// [`resolve`] for a file of the module `owner` (`pages.blog`). In a
/// Ware, `@/` is the Ware's root and there are no aliases.
pub fn locate(
    owner: &str,
    spec: &str,
    aliases: &BTreeMap<String, String>,
) -> Result<String, String> {
    let folder = owner.replace('.', "/");
    match super::ware_of(owner) {
        Some(ware) => {
            let prefix = ware.replace('.', "/");
            let from = folder.strip_prefix(&prefix).unwrap_or("");
            let path = resolve(spec, from.trim_start_matches('/'), &BTreeMap::new())?;
            Ok(format!("{prefix}/{path}"))
        }
        None => resolve(spec, &folder, aliases),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_relative_rooted_and_aliased_paths() {
        let aliases = BTreeMap::from([("ui".to_string(), "components".to_string())]);
        let r = |spec: &str, from: &str| resolve(spec, from, &aliases);
        assert_eq!(r("./card.glaze", "pages").unwrap(), "pages/card.glaze");
        assert_eq!(
            r("../components/card.glaze", "pages").unwrap(),
            "components/card.glaze"
        );
        assert_eq!(r("../../shared", "pages/blog").unwrap(), "shared");
        assert_eq!(r("@/shared", "pages/blog").unwrap(), "shared");
        assert_eq!(
            r("@ui/button.glaze", "").unwrap(),
            "components/button.glaze"
        );
        assert!(r("../x", "").unwrap_err().contains("leaves the project"));
        assert!(r("@nope/x", "").unwrap_err().contains("no alias `@nope/`"));
        assert!(r("card.glaze", "").unwrap_err().contains("start the path"));
        assert!(r("@/", "").is_err());
        assert!(r("@/__ware/x", "").is_err());
    }
}
