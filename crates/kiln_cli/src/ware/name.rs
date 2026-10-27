//! Ware identities: `publisher/name`, each part a lower-case identifier, so
//! a name reads the same on every system and can never pass for another.

use std::fmt;

/// The longest publisher or package part.
pub const MAX_PART: usize = 64;

/// Publishers no account can hold: they would read as the language itself.
const RESERVED_PUBLISHERS: &[&str] = &["std", "core", "builtin", "local_ware"];

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WareName {
    pub publisher: String,
    pub name: String,
}

impl WareName {
    /// Reads `publisher/name`. Only `a-z`, `0-9` and `_` are allowed, the
    /// first character is a letter, and nothing is normalized: `Kiln/UI`
    /// is an error rather than another spelling of `kiln/ui`.
    pub fn parse(text: &str) -> Result<WareName, String> {
        let Some((publisher, name)) = text.split_once('/') else {
            return Err(format!(
                "`{text}` is not a Ware name; Ware names are `publisher/name`, such as `kiln/ui`"
            ));
        };
        for (what, part) in [("publisher", publisher), ("name", name)] {
            check_part(what, part, text)?;
        }
        if RESERVED_PUBLISHERS.contains(&publisher) {
            return Err(format!("the publisher `{publisher}` is reserved"));
        }
        Ok(WareName {
            publisher: publisher.into(),
            name: name.into(),
        })
    }

    /// The module path its source has inside a build: `__ware.kiln.ui`.
    pub fn module_key(&self) -> String {
        format!("__ware.{}.{}", self.publisher, self.name)
    }

    /// The folder its source appears at under a project's root while it is
    /// compiled; nothing is written there.
    pub fn mount(&self) -> String {
        format!("__ware/{}/{}", self.publisher, self.name)
    }
}

impl fmt::Display for WareName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.publisher, self.name)
    }
}

fn check_part(what: &str, part: &str, whole: &str) -> Result<(), String> {
    let ok = !part.is_empty()
        && part.len() <= MAX_PART
        && part.starts_with(|c: char| c.is_ascii_lowercase())
        && part
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !part.contains("__");
    if ok {
        Ok(())
    } else {
        Err(format!(
            "`{whole}`: a Ware {what} is 1 to {MAX_PART} characters of a-z, 0-9 and single `_`, starting with a letter"
        ))
    }
}

/// Whether `alias` can name a dependency in `import`: an identifier that is
/// not a standard module and not reserved.
pub fn check_alias(alias: &str) -> Result<(), String> {
    let ok = alias.starts_with(|c: char| c.is_ascii_lowercase())
        && alias
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && alias.len() <= MAX_PART;
    if !ok {
        return Err(format!(
            "`{alias}` cannot name a dependency: use a lower-case identifier such as `ui`"
        ));
    }
    if kiln_check::std_source(alias).is_some() || ["npm", "test"].contains(&alias) {
        return Err(format!(
            "`{alias}` is a standard module; give the dependency another name with `--as`"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(
            WareName::parse("kiln/ui").unwrap().module_key(),
            "__ware.kiln.ui"
        );
        for bad in [
            "kiln",
            "Kiln/ui",
            "kiln/u-i",
            "kiln/",
            "/ui",
            "kiln/ui/x",
            "std/web",
            "a/b__c",
            "a/1b",
            "a/ü",
        ] {
            assert!(WareName::parse(bad).is_err(), "{bad}");
        }
        assert!(check_alias("web").is_err());
        assert!(check_alias("ui").is_ok());
    }
}
