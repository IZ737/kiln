//! Every program under `examples/` must parse without errors. Examples are
//! part of the teaching material for people and coding agents, so they are
//! tested like code.

use std::path::{Path, PathBuf};

fn kiln_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            kiln_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "kiln") {
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
        let result = kiln_syntax::parse(&file.text);
        let rendered: Vec<_> = result.diagnostics.iter().map(|d| d.render(&file)).collect();
        assert!(rendered.is_empty(), "{}", rendered.join("\n"));
    }
}
