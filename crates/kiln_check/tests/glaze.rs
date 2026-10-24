//! Type-checking glazes together with the `.kiln` files of their app.

use kiln_diagnostics::{codes, Diagnostic};

const APP: &str = r#"
import web, sql

struct Post {
    id: Int
    title: String
    tags: List<String>
    likes: Int = 0
}

enum Role {
    Admin
    Reader
}

server fn like_post(id: Int) -> Int ! {
    let db = try sql.open(env("DATABASE_URL"))
    try db.scalar<Int>("select count(*) from likes where post_id = $1", id)
}

fn main() ! {
    let app = web.app()
    let post = Post(id: 1, title: "Hi", tags: [])
    app.get("/", fn(req) { web.render("page.glaze", { post, title: "Home" }) })
    try app.listen(":8080")
}
"#;

const LAYOUT: &str = "{% props title: String %}\n<html><body>{% slot %}</body></html>\n";
const CARD: &str = "{% props post: Post, wide: Bool = false %}\n<div class=\"card\">{post.title}{% slot %}</div>\n";

/// Checks `page` as `page.glaze` in the app, returning (main, page) diagnostics.
fn check_page(page: &str) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
    check_with(APP, page)
}

fn check_with(main: &str, page: &str) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
    let module = kiln_syntax::parse(main);
    assert!(module.diagnostics.is_empty(), "{:#?}", module.diagnostics);
    let parsed: Vec<_> = [
        ("base.glaze", LAYOUT),
        ("post_card.glaze", CARD),
        ("page.glaze", page),
    ]
    .iter()
    .map(|(name, src)| {
        let r = kiln_syntax::glaze::parse_glaze(src);
        assert!(r.diagnostics.is_empty(), "{name}: {:#?}", r.diagnostics);
        (*name, r.glaze)
    })
    .collect();
    let glazes: Vec<_> = parsed.iter().map(|(n, g)| (*n, g)).collect();
    let mut result = kiln_check::check_app(&[&module.module], &glazes);
    for (i, diags) in result.glazes.iter().enumerate().take(2) {
        assert!(diags.is_empty(), "{}: {diags:#?}", parsed[i].0);
    }
    (result.modules.remove(0), result.glazes.remove(2))
}

#[track_caller]
fn page_ok(page: &str) {
    let (main, diags) = check_page(page);
    assert!(main.is_empty(), "main.kiln: {main:#?}");
    assert!(diags.is_empty(), "page.glaze: {diags:#?}");
}

#[track_caller]
fn page_errors(page: &str) -> Vec<&'static str> {
    check_page(page).1.iter().map(|d| d.code).collect()
}

const PROPS: &str = "{% props post: Post, title: String %}\n";

#[test]
fn a_valid_page() {
    page_ok(&format!(
        "{PROPS}{{% layout \"base.glaze\" title: title %}}
<h1>{{post.title}} ({{post.likes}} likes)</h1>
{{% let count = post.tags.len() %}}
<p class=\"tags {{count}}\">
  {{% for tag in post.tags where tag.len() > 0 %}}<span>{{tag}}</span>{{% empty %}}none{{% end %}}
</p>
{{% if count > 3 %}}many{{% else if count > 0 %}}some{{% else %}}none{{% end %}}
<PostCard post={{post}} wide>extra</PostCard>
"
    ));
}

#[test]
fn values_must_have_a_text_form() {
    let codes = page_errors(&format!("{PROPS}<p>{{post}} {{post.tags}}</p>"));
    assert_eq!(codes, [codes::NOT_DISPLAYABLE, codes::NOT_DISPLAYABLE]);
    assert_eq!(
        page_errors("{% props post: Post, title: String, note: String? = none %}<p>{note}</p>"),
        [codes::NOT_DISPLAYABLE]
    );
    page_ok("{% props post: Post, title: String, note: String? = none %}<p>{note ?? \"\"}</p>{% if note is some(n) %}{n}{% end %}");
    assert_eq!(
        page_errors(&format!("{PROPS}{{raw post.likes}}")),
        [codes::TYPE_MISMATCH]
    );
}

#[test]
fn names_and_fields_are_checked() {
    let (_, diags) = check_page(&format!("{PROPS}<p>{{post.titel}} {{nope}}</p>"));
    let codes: Vec<_> = diags.iter().map(|d| d.code).collect();
    assert_eq!(codes, [codes::NO_SUCH_MEMBER, codes::UNDEFINED_NAME]);
    assert!(diags[0].suggestion.is_some());
}

#[test]
fn conditions_and_loops() {
    assert_eq!(
        page_errors(&format!("{PROPS}{{% if post.title %}}x{{% end %}}")),
        [codes::TYPE_MISMATCH]
    );
    assert_eq!(
        page_errors(&format!("{PROPS}{{% for t in post.likes %}}x{{% end %}}")),
        [codes::TYPE_MISMATCH]
    );
    // Loop variables are scoped to the loop.
    assert_eq!(
        page_errors(&format!(
            "{PROPS}{{% for t in post.tags %}}{{t}}{{% end %}}{{t}}"
        )),
        [codes::UNDEFINED_NAME]
    );
}

#[test]
fn match_is_exhaustive() {
    let page = "{% props post: Post, title: String, role: Role = Role.Reader %}\n{% match role %}{% case Admin %}admin{% end %}";
    assert_eq!(page_errors(page), [codes::NON_EXHAUSTIVE_MATCH]);
    page_ok("{% props post: Post, title: String, role: Role = Role.Reader %}\n{% match role %}{% case Admin %}admin{% case Reader %}reader{% end %}");
}

#[test]
fn components_are_checked_against_their_props() {
    assert_eq!(
        page_errors(&format!("{PROPS}<PostCard />")),
        [codes::BAD_ARGUMENTS]
    );
    let (_, diags) = check_page(&format!("{PROPS}<PostCard pots={{post}} />"));
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].suggestion.as_ref().unwrap().replacement, "post");
    assert_eq!(
        page_errors(&format!("{PROPS}<PostCard post={{title}} />")),
        [codes::TYPE_MISMATCH]
    );
    assert_eq!(
        page_errors(&format!("{PROPS}<PostCard post={{post}} wide=\"yes\" />")),
        [codes::TYPE_MISMATCH]
    );
    assert_eq!(
        page_errors(&format!("{PROPS}<PostCrad post={{post}} />")),
        [codes::UNKNOWN_GLAZE]
    );
    assert_eq!(
        page_errors(&format!("{PROPS}<PostCard post={{post}} client:visble />")),
        [codes::BAD_ARGUMENTS]
    );
}

#[test]
fn layouts_need_their_props_and_a_slot() {
    assert_eq!(
        page_errors(&format!("{PROPS}{{% layout \"base.glaze\" %}}")),
        [codes::BAD_ARGUMENTS]
    );
    assert_eq!(
        page_errors(&format!(
            "{PROPS}{{% layout \"base.glaze\" title: post.likes %}}"
        )),
        [codes::TYPE_MISMATCH]
    );
    assert_eq!(
        page_errors(&format!(
            "{PROPS}{{% layout \"missing.glaze\" title: title %}}"
        )),
        [codes::UNKNOWN_GLAZE]
    );
}

#[test]
fn render_data_must_match_the_props() {
    let page = format!("{PROPS}<p>{{title}}</p>");
    let main_codes = |main: &str| -> Vec<&'static str> {
        check_with(main, &page).0.iter().map(|d| d.code).collect()
    };
    assert!(main_codes(APP).is_empty());
    let missing = APP.replace("{ post, title: \"Home\" }", "{ post }");
    assert_eq!(main_codes(&missing), [codes::BAD_ARGUMENTS]);
    let wrong = APP.replace("title: \"Home\"", "title: 7");
    assert_eq!(main_codes(&wrong), [codes::TYPE_MISMATCH]);
    let unknown = APP.replace("\"page.glaze\"", "\"pages.glaze\"");
    assert_eq!(main_codes(&unknown), [codes::UNKNOWN_GLAZE]);
}

#[test]
fn embers_run_in_the_browser() {
    let ember = "{% props post: Post, title: String %}
{% state likes = post.likes %}
<button on:click={
    likes = like_post(post.id) catch { likes }
}>{likes}</button>";
    page_ok(ember);
    // Server functions can fail, so handlers must catch.
    let uncaught = ember.replace(" catch { likes }", "");
    assert_eq!(page_errors(&uncaught), [codes::MISSING_TRY]);
    // Props cannot be reassigned; state can.
    let assign_prop = ember.replace("likes = like_post", "post = post\n    likes = like_post");
    assert_eq!(page_errors(&assign_prop), [codes::ASSIGN_TO_IMMUTABLE]);
    let server_only = format!("{ember}\n{{% let n = sql.open(\"x\") catch {{ return }} %}}");
    assert!(page_errors(&server_only).contains(&codes::SERVER_ONLY));
}
