//! Every program and glaze under `examples/` must type-check. Examples are part of
//! the teaching material for people and coding agents, so they are tested
//! like code.

use std::path::{Path, PathBuf};

fn kiln_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            kiln_files(&path, out);
        } else if path
            .extension()
            .is_some_and(|e| e == "kiln" || e == "glaze" || e == "ember")
        {
            out.push(path);
        }
    }
}

#[test]
fn all_examples_type_check() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut files = Vec::new();
    kiln_files(&root, &mut files);
    assert!(
        files.len() >= 3,
        "expected example files in {}",
        root.display()
    );
    // Each directory is one app.
    let mut dirs: Vec<PathBuf> = files
        .iter()
        .map(|f| f.parent().unwrap().to_path_buf())
        .collect();
    dirs.sort();
    dirs.dedup();
    for dir in dirs {
        let in_dir: Vec<&PathBuf> = files.iter().filter(|f| f.parent() == Some(&dir)).collect();
        let sources: Vec<(PathBuf, String)> = in_dir
            .iter()
            .map(|p| ((*p).clone(), std::fs::read_to_string(p).unwrap()))
            .collect();
        let is_glaze = |p: &Path| p.extension().is_some_and(|e| e == "glaze" || e == "ember");
        let modules: Vec<_> = sources
            .iter()
            .filter(|(p, _)| !is_glaze(p))
            .map(|(p, text)| {
                let r = kiln_syntax::parse(text);
                assert!(
                    r.diagnostics.is_empty(),
                    "{}: {:#?}",
                    p.display(),
                    r.diagnostics
                );
                (p, text, r.module)
            })
            .collect();
        let glazes: Vec<_> = sources
            .iter()
            .filter(|(p, _)| is_glaze(p))
            .map(|(p, text)| {
                let r = kiln_syntax::glaze::parse_component(text, &p.to_string_lossy());
                assert!(
                    r.diagnostics.is_empty(),
                    "{}: {:#?}",
                    p.display(),
                    r.diagnostics
                );
                (p, text, r.glaze)
            })
            .collect();
        let names: Vec<String> = glazes
            .iter()
            .map(|(p, _, _)| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        let result = kiln_check::check_app(
            &modules.iter().map(|(_, _, m)| m).collect::<Vec<_>>(),
            &glazes
                .iter()
                .zip(&names)
                .map(|((_, _, g), n)| (n.as_str(), g))
                .collect::<Vec<_>>(),
        );
        let reports = modules
            .iter()
            .map(|(p, t, _)| (p, t))
            .zip(result.modules)
            .chain(glazes.iter().map(|(p, t, _)| (p, t)).zip(result.glazes));
        for ((path, text), diags) in reports {
            let file =
                kiln_diagnostics::SourceFile::new(path.display().to_string(), (*text).clone());
            let rendered: Vec<_> = diags.iter().map(|d| d.render(&file)).collect();
            assert!(rendered.is_empty(), "{}", rendered.join("\n"));
        }
    }
}
