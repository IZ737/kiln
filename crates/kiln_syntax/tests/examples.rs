//! Every program and glaze under `examples/` must parse without errors. Examples are
//! part of the teaching material for people and coding agents, so they are
//! tested like code.

use std::path::{Path, PathBuf};

fn kiln_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            kiln_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "kiln" || e == "glaze")
        {
            out.push(path);
        }
    }
}

#[test]
fn all_examples_parse() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    kiln_files(&root, &mut files);
    assert!(
        files.len() >= 3,
        "expected example files in {}",
        root.display()
    );
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let file = kiln_diagnostics::SourceFile::new(path.display().to_string(), text);
        let diagnostics = if path.extension().is_some_and(|e| e == "glaze") {
            kiln_syntax::glaze::parse_glaze(&file.text).diagnostics
        } else {
            kiln_syntax::parse(&file.text).diagnostics
        };
        let rendered: Vec<_> = diagnostics.iter().map(|d| d.render(&file)).collect();
        assert!(rendered.is_empty(), "{}", rendered.join("\n"));
    }
}

/// Every prefix of every example is a realistic half-typed file: the
/// parsers must recover from it without panicking or looping.
#[test]
fn truncated_examples_do_not_crash() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    kiln_files(&root, &mut files);
    for path in files {
        let text = std::fs::read_to_string(&path).unwrap();
        let glaze = path.extension().is_some_and(|e| e == "glaze");
        for (i, _) in text.char_indices() {
            if glaze {
                kiln_syntax::glaze::parse_glaze(&text[..i]);
            } else {
                kiln_syntax::parse(&text[..i]);
            }
        }
    }
}
