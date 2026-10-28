//! The folders whose components a source file names by their place in
//! the project: a slash-bearing `web.render("posts/index.glaze", ...)` or
//! `{% layout "layouts/base.glaze" %}` path, or a qualified tag such as
//! `<posts.search.Result />`. Those folders are part of the build like
//! imported ones, so a component never has to sit beside, or be imported
//! by, the code that uses it.

use kiln_syntax::ast::{Expr, ExprKind, Glaze, Module, Node};
use std::path::Path;

/// A component file a source file names, relative to the project root,
/// such as `posts/index.glaze`. For a tag, both `.glaze` and `.ember` are
/// given, since either may be the one.
pub fn in_module(module: &Module) -> Vec<String> {
    let mut out = Vec::new();
    super::routes::each_body(module, &mut |body| {
        super::routes::each_block(body, &mut |e| out.extend(render_path(e)));
    });
    out
}

/// The component files a glaze or ember names: its layout and qualified
/// tags.
pub fn in_glaze(glaze: &Glaze) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(layout) = &glaze.layout {
        out.extend(super::routes::literal(&layout.path).filter(|p| rooted(p)));
    }
    each_element(&glaze.nodes, &mut |name| {
        if let Some((folder, component)) = name.rsplit_once('.') {
            if folder.split('.').all(segment) {
                let file = format!("{}/{}", folder.replace('.', "/"), snake_case(component));
                out.push(format!("{file}.glaze"));
                out.push(format!("{file}.ember"));
            }
        }
    });
    out
}

/// The folder of a component file, when the file is there.
pub fn folder(root: &Path, file: &str) -> Option<std::path::PathBuf> {
    let path = root.join(file);
    let real = super::real(&path);
    // Paths are case-sensitive on every platform.
    let name = real.file_name()?.to_owned();
    let exact = std::fs::read_dir(real.parent()?)
        .ok()?
        .filter_map(Result::ok)
        .any(|e| e.file_name() == name && e.path().is_file());
    exact.then(|| path.parent().map(Path::to_path_buf))?
}

/// `web.render("posts/index.glaze", ...)`: the path, when it has a folder.
fn render_path(e: &Expr) -> Option<String> {
    let ExprKind::Call { callee, args, .. } = &e.kind else {
        return None;
    };
    let ExprKind::Field { name, .. } = &callee.kind else {
        return None;
    };
    if name.name != "render" {
        return None;
    }
    let first = args.first().filter(|a| a.name.is_none())?;
    super::routes::literal(&first.value).filter(|p| rooted(p))
}

/// A project-root-relative component path: with a folder, written with
/// `/`, and staying inside the project.
fn rooted(path: &str) -> bool {
    path.contains('/')
        && !path.starts_with('/')
        && (path.ends_with(".glaze") || path.ends_with(".ember"))
        && path.split('/').all(segment)
}

fn segment(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && !part.starts_with("__")
        && !part.contains(['\\', ':'])
}

fn each_element(nodes: &[Node], f: &mut dyn FnMut(&str)) {
    for node in nodes {
        match node {
            Node::Element(e) => {
                if e.is_component() {
                    f(&e.name.name);
                }
                each_element(&e.children, f);
            }
            Node::If {
                branches,
                otherwise,
                ..
            } => {
                for b in branches {
                    each_element(&b.body, f);
                }
                if let Some(o) = otherwise {
                    each_element(o, f);
                }
            }
            Node::For { body, empty, .. } => {
                each_element(body, f);
                if let Some(e) = empty {
                    each_element(e, f);
                }
            }
            Node::Match { cases, .. } => {
                for c in cases {
                    each_element(&c.body, f);
                }
            }
            Node::Fill { body, .. } => each_element(body, f),
            _ => {}
        }
    }
}

/// `LikeButton` is `like_button`, as component files are named.
fn snake_case(name: &str) -> String {
    let mut out = String::new();
    for (i, c) in name.chars().enumerate() {
        if c.is_ascii_uppercase() {
            if i > 0 {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_rooted_render_paths_layouts_and_qualified_tags() {
        let module = kiln_syntax::parse(
            "import web\nfn page() -> web.Response {\n    let a = web.render(\"local.glaze\", {})\n    return web.render(\"pages/home.glaze\", { a })\n}\n",
        )
        .module;
        assert_eq!(in_module(&module), vec!["pages/home.glaze"]);
        let glaze = kiln_syntax::glaze::parse_glaze(
            "{% layout \"layouts/main.glaze\" %}\n<Local />\n{% if true %}<ui.cards.LikeButton n={1} />{% end %}\n<a href=\"../x\">x</a>\n",
        )
        .glaze;
        assert_eq!(
            in_glaze(&glaze),
            vec![
                "layouts/main.glaze",
                "ui/cards/like_button.glaze",
                "ui/cards/like_button.ember"
            ]
        );
        assert!(!rooted("../secret/x.glaze"));
        assert!(!rooted("/abs/x.glaze"));
        assert!(!rooted("x.glaze"));
    }
}
