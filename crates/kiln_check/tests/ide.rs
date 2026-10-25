//! What the checker tells the language server.

use kiln_check::ide::{analyze_app, Analysis, Completion, Ref, Target, COMPLETION_MARKER};

fn analyze(src: &str) -> Analysis {
    let module = kiln_syntax::parse(src).module;
    analyze_app(&[&module], &[], Target::Module(0))
}

/// What the name at the first `needle` refers to.
fn refers(src: &str, needle: &str) -> Ref {
    let analysis = analyze(src);
    let offset = src.find(needle).unwrap();
    analysis.reference_at(offset).unwrap().1.clone()
}

const SRC: &str = "import web
struct Post { title: String }
enum Role { Admin }
fn main() {
    let post = Post(title: \"hi\")
    print(post.title.upper())
    let r = Role.Admin
    let app = web.app()
}
";

#[test]
fn records_what_names_refer_to() {
    let analysis = analyze(SRC);
    let post = analysis.global_types["Post"];
    let role = analysis.global_types["Role"];
    let string = analysis.global_types["String"];
    let let_post = SRC.find("post =").unwrap();
    assert_eq!(
        refers(SRC, "post.title"),
        Ref::Local(kiln_diagnostics::Span::new(let_post, let_post + 4))
    );
    assert_eq!(refers(SRC, "title.upper"), Ref::Field(post, "title".into()));
    assert_eq!(refers(SRC, "title: \""), Ref::Field(post, "title".into()));
    assert_eq!(refers(SRC, "upper"), Ref::Method(string, "upper".into()));
    assert_eq!(refers(SRC, "print"), Ref::Fn("print".into()));
    assert_eq!(
        refers(SRC, "Admin\n    let"),
        Ref::Variant(role, "Admin".into())
    );
    assert_eq!(refers(SRC, "Post("), Ref::Type(post));
    assert!(matches!(refers(SRC, "app()"), Ref::ModuleFn(_, name) if name == "app"));
    // Declarations refer to themselves.
    assert_eq!(refers(SRC, "Post {"), Ref::Type(post));
}

#[test]
fn says_what_fits_at_the_marker() {
    let at = |line: &str| {
        let src = SRC.replace("    let app", &format!("    {line}\n    let app"));
        let src = src.replace('|', COMPLETION_MARKER);
        let module = kiln_syntax::parse(&src).module;
        analyze_app(&[&module], &[], Target::Module(0)).completion
    };
    match at("post.|") {
        Some(Completion::Members(ty)) => assert_eq!(analyze(SRC).show(&ty), "Post"),
        other => panic!("{other:?}"),
    }
    match at("print(|)") {
        Some(Completion::Names(locals)) => {
            let names: Vec<&str> = locals.iter().map(|(n, _)| n.as_str()).collect();
            assert_eq!(names, ["r", "post"]);
        }
        other => panic!("{other:?}"),
    }
    assert!(matches!(at("web.|"), Some(Completion::Module(_))));
    assert!(matches!(at("Role.|"), Some(Completion::TypeMembers(_))));
    assert!(matches!(at("let x: |"), Some(Completion::Types)));
    assert!(matches!(
        at("let x: web.|"),
        Some(Completion::ModuleTypes(_))
    ));
    // Without the marker there is nothing to complete.
    assert!(analyze(SRC).completion.is_none());
}

#[test]
fn analyzes_code_with_syntax_errors() {
    let src = "struct Post { title: String }\nfn main() {\n    let post = Post(title: \"x\")\n    post.\n    print(post.title)\n}\n";
    let analysis = analyze(src);
    let offset = src.find("title)").unwrap();
    let post = analysis.global_types["Post"];
    assert_eq!(
        analysis.reference_at(offset).unwrap().1,
        Ref::Field(post, "title".into())
    );
}
