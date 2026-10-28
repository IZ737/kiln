//! `kiln ui add`: copies components of kiln/ui into a project, so that it
//! owns their source. kiln carries the components, so this works offline.
//! A copy names where it came from and its licence, and an existing file
//! is never overwritten: a component someone changed stays theirs.

use std::path::Path;

/// The kiln/ui version kiln carries.
pub const VERSION: &str = "0.1.0";

const SHARED: (&str, &str) = ("ui.kiln", include_str!("../../../../wares/ui/ui.kiln"));

/// Each component: its name, its file and its source.
const COMPONENTS: &[(&str, &str, &str)] = &[
    (
        "alert",
        "alert.glaze",
        include_str!("../../../../wares/ui/alert.glaze"),
    ),
    (
        "badge",
        "badge.glaze",
        include_str!("../../../../wares/ui/badge.glaze"),
    ),
    (
        "button",
        "button.glaze",
        include_str!("../../../../wares/ui/button.glaze"),
    ),
    (
        "card",
        "card.glaze",
        include_str!("../../../../wares/ui/card.glaze"),
    ),
    (
        "checkbox",
        "checkbox.glaze",
        include_str!("../../../../wares/ui/checkbox.glaze"),
    ),
    (
        "empty_state",
        "empty_state.glaze",
        include_str!("../../../../wares/ui/empty_state.glaze"),
    ),
    (
        "form_errors",
        "form_errors.glaze",
        include_str!("../../../../wares/ui/form_errors.glaze"),
    ),
    (
        "input",
        "input.glaze",
        include_str!("../../../../wares/ui/input.glaze"),
    ),
    (
        "radio",
        "radio.glaze",
        include_str!("../../../../wares/ui/radio.glaze"),
    ),
    (
        "select",
        "select.glaze",
        include_str!("../../../../wares/ui/select.glaze"),
    ),
    (
        "textarea",
        "textarea.glaze",
        include_str!("../../../../wares/ui/textarea.glaze"),
    ),
];

/// The names `kiln ui add` takes.
pub fn names() -> Vec<&'static str> {
    COMPONENTS.iter().map(|c| c.0).collect()
}

/// Copies components (`all` for every one) into `dir/<into>/`.
pub fn add(dir: &Path, into: &str, wanted: &[String]) -> Result<String, String> {
    let part = |s: &str| {
        s.starts_with(|c: char| c.is_ascii_lowercase())
            && s.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    };
    if !into.split('/').all(part) {
        return Err(format!("`{into}` is not a folder name for a module"));
    }
    let mut chosen = Vec::new();
    for w in wanted {
        let w = w.trim().to_lowercase().replace('-', "_");
        if w == "all" {
            chosen.extend(COMPONENTS.iter());
            continue;
        }
        match COMPONENTS.iter().find(|c| c.0 == w) {
            Some(c) => chosen.push(c),
            None => {
                return Err(format!(
                    "kiln/ui has no component `{w}`; it has: {}",
                    names().join(", ")
                ))
            }
        }
    }
    if chosen.is_empty() {
        return Err(format!(
            "name the components to copy, such as `kiln ui add button input`: {}",
            names().join(", ")
        ));
    }
    chosen.sort_by_key(|c| c.0);
    chosen.dedup_by_key(|c| c.0);
    let folder = dir.join(into);
    let mut lines = Vec::new();
    let mut files: Vec<(&str, String)> = chosen
        .iter()
        .map(|(_, file, text)| (*file, glaze_copy(text)))
        .collect();
    files.push((SHARED.0, kiln_copy(SHARED.1)));
    // Check every file before writing any.
    let mut writes = Vec::new();
    for (file, text) in files {
        let path = folder.join(file);
        match std::fs::read_to_string(&path) {
            Ok(existing) if existing == text => {
                lines.push(format!("  {into}/{file} is already this version"))
            }
            Ok(_) if file == SHARED.0 => lines.push(format!(
                "  {into}/{file} exists and is kept; compare it with kiln/ui {VERSION} if a component needs something it lacks"
            )),
            Ok(_) => {
                return Err(format!(
                    "{into}/{file} exists and may hold your changes; kiln never overwrites it. Delete or rename it to copy it again"
                ))
            }
            Err(_) => writes.push((path, text, file)),
        }
    }
    std::fs::create_dir_all(&folder).map_err(|e| format!("{}: {e}", folder.display()))?;
    let mut wrote = Vec::new();
    for (path, text, file) in writes {
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))?;
        wrote.push(format!("  wrote {into}/{file}"));
    }
    wrote.append(&mut lines);
    let lines = wrote;
    let module = into.replace('/', ".");
    Ok(format!(
        "copied from kiln/ui {VERSION} (MIT licence); the files are now yours to change\n{}\nuse them as `<{module}.Button>`, with `import {module}` in Kiln code",
        lines.join("\n")
    ))
}

fn provenance() -> String {
    format!("Copied from kiln/ui {VERSION} (MIT licence, https://github.com/IZ737/kiln/tree/main/wares/ui) by `kiln ui add`. It is yours to change.")
}

fn glaze_copy(text: &str) -> String {
    format!("{{# {} #}}\n{text}", provenance())
}

fn kiln_copy(text: &str) -> String {
    format!("// {}\n{text}", provenance())
}
