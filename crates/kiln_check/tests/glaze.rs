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
    let db = try sql.open(env("DATABASE_URL") ?? "sqlite::memory:")
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
    check_named(main, page, "page.glaze")
}

fn check_named(main: &str, page: &str, name: &str) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
    let module = kiln_syntax::parse(main);
    assert!(module.diagnostics.is_empty(), "{:#?}", module.diagnostics);
    let parsed: Vec<_> = [
        ("base.glaze", LAYOUT),
        ("post_card.glaze", CARD),
        (name, page),
    ]
    .iter()
    .map(|(name, src)| {
        let r = kiln_syntax::glaze::parse_component(src, name);
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
    ember_ok(ember);
    // Server functions can fail, so handlers must catch.
    let uncaught = ember.replace(" catch { likes }", "");
    assert_eq!(ember_errors(&uncaught), [codes::MISSING_TRY]);
    // Props cannot be reassigned; state can.
    let assign_prop = ember.replace("likes = like_post", "post = post\n    likes = like_post");
    assert_eq!(ember_errors(&assign_prop), [codes::ASSIGN_TO_IMMUTABLE]);
    let server_only = format!("{ember}\n{{% let n = sql.open(\"x\") catch {{ return }} %}}");
    assert!(ember_errors(&server_only).contains(&codes::SERVER_ONLY));
}

#[test]
fn browser_modules_are_only_for_handlers() {
    let ember = "{% props post: Post, title: String %}
{% state shown = \"\" %}
<button on:click={
    browser.set_title(\"Hi\")
    shown = js.global(\"document\").get(\"title\").as_string() catch err { err.message }
}>{shown}</button>";
    ember_ok(ember);
    // The server renders the ember, and has no browser.
    let outside = format!("{ember}\n<p>{{browser.title()}}</p>");
    assert_eq!(ember_errors(&outside), [codes::BROWSER_ONLY]);
    let main = APP.replace("fn main() ! {", "fn main() ! {\n    browser.log(\"hi\")");
    assert!(check_ember(&main, ember)
        .0
        .iter()
        .any(|d| d.code == codes::UNDEFINED_NAME || d.code == codes::BROWSER_ONLY));
}

#[test]
fn handlers_take_known_modifiers() {
    ember_ok("{% props post: Post, title: String %}\n{% state n = 0 %}\n<form on:submit.prevent.latest={ n = 1 }><button on:click.stop={ n = 2 }>{n}</button></form>");
    let (_, diags) = check_ember(APP, "{% props post: Post, title: String %}\n{% state n = 0 %}\n<button on:click.Prevent={ n = 1 }>{n}</button>");
    assert_eq!(
        diags.iter().map(|d| d.code).collect::<Vec<_>>(),
        [codes::UNKNOWN_MODIFIER]
    );
    assert_eq!(
        diags[0].suggestion.as_ref().map(|s| s.replacement.as_str()),
        Some("prevent")
    );
}

#[test]
fn what_crosses_to_the_browser_is_plain_data() {
    // An ember's state and props travel as JSON.
    assert_eq!(
        ember_errors(
            "{% state f = fn(x: Int) -> Int { x } %}\n<button on:click={ f = f }>x</button>"
        ),
        [codes::NOT_SERIALIZABLE]
    );
    assert_eq!(
        ember_errors("{% props pick: fn() -> Int %}\n{% state n = 0 %}\n<button on:click={ n = 1 }>{n}</button>"),
        [codes::NOT_SERIALIZABLE]
    );
    // Structs of plain data are fine; a handle inside one is not.
    ember_ok("{% props post: Post, title: String %}\n{% state p = post %}\n<button on:click={ p = post }>{p.title}</button>");
    let main = APP.replace(
        "server fn like_post(id: Int) -> Int ! {",
        "server fn run(task: fn() -> Int) -> Int { task() }\nstruct Wrap { req: web.Request }\nserver fn wrap(w: Wrap) {}\nserver fn like_post(id: Int) -> Int ! {",
    );
    let codes: Vec<_> = check_ember(&main, PROPS).0.iter().map(|d| d.code).collect();
    assert_eq!(codes, [codes::NOT_SERIALIZABLE, codes::NOT_SERIALIZABLE]);
}

#[test]
fn values_cannot_go_where_browsers_run_code() {
    for attr in ["onclick", "ONCLICK", "srcdoc", "style"] {
        assert_eq!(
            page_errors(&format!("{PROPS}<p {attr}={{true}}>x</p>")),
            [codes::UNSAFE_ATTRIBUTE]
        );
    }
    page_ok(&format!("{PROPS}<button disabled={{true}}>x</button>"));
    assert_eq!(
        page_errors(&format!(
            "{PROPS}<p style=\"color: {{title}}\" onclick=\"go({{post.id}})\">x</p><iframe srcdoc={{title}}></iframe>"
        )),
        [
            codes::UNSAFE_ATTRIBUTE,
            codes::UNSAFE_ATTRIBUTE,
            codes::UNSAFE_ATTRIBUTE
        ]
    );
    // Numbers cannot change CSS, and written-out attributes are the author's.
    page_ok(&format!(
        "{PROPS}<p style=\"width: {{post.likes}}%\" onclick=\"go()\" class=\"{{title}}\">x</p>"
    ));
}

#[test]
fn the_compiler_learns_what_values_are() {
    let module = kiln_syntax::parse(APP).module;
    let page = kiln_syntax::glaze::parse_glaze(&format!(
        "{PROPS}<p hidden={{post.likes > 1}}>{{post.title}} {{post.likes}} {{2s}}</p>"
    ));
    let layout = kiln_syntax::glaze::parse_glaze(LAYOUT).glaze;
    let card = kiln_syntax::glaze::parse_glaze(CARD).glaze;
    let glazes = [
        ("base.glaze", &layout),
        ("post_card.glaze", &card),
        ("page.glaze", &page.glaze),
    ];
    let result = kiln_check::check_app(&[&module], &glazes);
    let mut kinds: Vec<_> = result.glaze_values[2].iter().collect();
    kinds.sort_by_key(|(span, _)| span.start);
    let kinds: Vec<_> = kinds.into_iter().map(|(_, k)| *k).collect();
    use kiln_check::ValueKind::*;
    assert_eq!(kinds, [Bool, String, Number, Other]);
}

/// Checks glazes on their own, with `APP`, returning each one's error codes.
fn glaze_codes(glazes: &[(&str, &str)]) -> Vec<Vec<&'static str>> {
    let module = kiln_syntax::parse(APP).module;
    let parsed: Vec<_> = glazes
        .iter()
        .map(|(n, src)| {
            let r = kiln_syntax::glaze::parse_component(src, n);
            assert!(r.diagnostics.is_empty(), "{n}: {:#?}", r.diagnostics);
            (*n, r.glaze)
        })
        .collect();
    let refs: Vec<_> = parsed.iter().map(|(n, g)| (*n, g)).collect();
    kiln_check::check_app(&[&module], &refs)
        .glazes
        .iter()
        .map(|d| d.iter().map(|d| d.code).collect())
        .collect()
}

#[test]
fn fills_name_slots_of_a_layout_or_component() {
    let layout = "{% props title: String %}<head>{% slot head %}</head>{% slot %}";
    let card = "<div>{% slot title %}{% slot %}</div>";
    let page = "{% layout \"base.glaze\" title: \"Hi\" %}{% fill head %}<meta>{% end %}<Card>{% fill title %}T{% end %}body</Card>";
    assert_eq!(
        glaze_codes(&[
            ("base.glaze", layout),
            ("card.glaze", card),
            ("page.glaze", page)
        ]),
        [Vec::<&str>::new(), vec![], vec![]]
    );
    let wrong = "{% layout \"base.glaze\" title: \"Hi\" %}{% fill haed %}x{% end %}<Card>{% fill title %}a{% end %}{% fill title %}b{% end %}</Card><div>{% fill head %}x{% end %}</div>";
    assert_eq!(
        glaze_codes(&[
            ("base.glaze", layout),
            ("card.glaze", card),
            ("page.glaze", wrong)
        ])[2],
        [
            codes::MISPLACED_FILL,
            codes::MISPLACED_FILL,
            codes::MISPLACED_FILL
        ]
    );
    // Without a layout, there is nothing to fill.
    assert_eq!(
        glaze_codes(&[("page.glaze", "{% fill head %}x{% end %}")])[0],
        [codes::MISPLACED_FILL]
    );
}

#[test]
fn glazes_cannot_render_themselves_without_end() {
    let codes = glaze_codes(&[
        ("a.glaze", "{% layout \"b.glaze\" %}<p>{% slot %}</p>"),
        ("b.glaze", "<div><A /></div>{% slot %}"),
    ]);
    assert_eq!(
        codes,
        [vec![codes::ENDLESS_GLAZE], vec![codes::ENDLESS_GLAZE]]
    );
    // Inside a block that ends, a component may render itself.
    let tree = "{% props post: Post %}{% for t in post.tags %}<Tree post={post} />{% end %}";
    assert_eq!(glaze_codes(&[("tree.glaze", tree)]), [Vec::<&str>::new()]);
}

fn check_ember(main: &str, src: &str) -> (Vec<Diagnostic>, Vec<Diagnostic>) {
    check_named(&main.replace("page.glaze", "page.ember"), src, "page.ember")
}
fn ember_ok(src: &str) {
    let (main, diags) = check_ember(APP, src);
    assert!(main.is_empty(), "{main:#?}");
    assert!(diags.is_empty(), "{diags:#?}");
}
fn ember_errors(src: &str) -> Vec<&'static str> {
    check_ember(APP, src).1.iter().map(|d| d.code).collect()
}
