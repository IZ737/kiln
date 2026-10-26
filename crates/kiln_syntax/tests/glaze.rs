use kiln_diagnostics::{codes, Diagnostic};
use kiln_syntax::ast::*;
use kiln_syntax::glaze::parse_glaze;

#[track_caller]
fn ok(src: &str) -> Glaze {
    let result = parse_glaze(src);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:#?}",
        result.diagnostics
    );
    result.glaze
}

fn error_codes(src: &str) -> Vec<&'static str> {
    parse_glaze(src)
        .diagnostics
        .iter()
        .map(|d| d.code)
        .collect()
}

/// The diagnostics, checking there is exactly one with `code`.
#[track_caller]
fn one_error(src: &str, code: &str) -> Diagnostic {
    let diags = parse_glaze(src).diagnostics;
    assert_eq!(diags.len(), 1, "{diags:#?}");
    assert_eq!(diags[0].code, code, "{diags:#?}");
    diags.into_iter().next().unwrap()
}

/// Applies a diagnostic's suggested fix to `src`.
fn apply_fix(src: &str, diag: &Diagnostic) -> String {
    let fix = diag.suggestion.as_ref().expect("a suggested fix");
    let mut out = src.to_string();
    out.replace_range(
        fix.span.start as usize..fix.span.end as usize,
        &fix.replacement,
    );
    out
}

#[track_caller]
fn fix_works(src: &str, code: &str) {
    let diag = one_error(src, code);
    ok(&apply_fix(src, &diag));
}

fn element(node: &Node) -> &Element {
    match node {
        Node::Element(e) => e,
        other => panic!("expected an element, got {other:?}"),
    }
}

/// Nodes without whitespace-only text.
fn content(nodes: &[Node]) -> Vec<&Node> {
    nodes
        .iter()
        .filter(|n| !matches!(n, Node::Text { text, .. } if text.trim().is_empty()))
        .collect()
}

#[test]
fn text_values_and_elements() {
    let g = ok("<h1 class=\"title\">Hi {user.name}!</h1>");
    let h1 = element(&g.nodes[0]);
    assert_eq!(h1.name.name, "h1");
    assert!(!h1.is_component());
    assert_eq!(h1.attrs[0].name.name, "class");
    assert!(matches!(&h1.children[0], Node::Text { text, .. } if text == "Hi "));
    assert!(matches!(&h1.children[1], Node::Expr { raw: false, .. }));
    assert!(matches!(&h1.children[2], Node::Text { text, .. } if text == "!"));
}

#[test]
fn raw_values() {
    let g = ok("{raw post.html}");
    assert!(matches!(&g.nodes[0], Node::Expr { raw: true, .. }));
    // A variable named `raw` is an ordinary value.
    let g = ok("{raw}");
    assert!(matches!(&g.nodes[0], Node::Expr { raw: false, .. }));
}

#[test]
fn declarations() {
    let g = ok("{% props post: Post, user: User? = none %}\n{% layout \"base.glaze\" title: post.title %}\n<p>{post.title}</p>\n");
    let props = g.props.unwrap();
    assert_eq!(props.params.len(), 2);
    assert!(props.params[1].default.is_some());
    let layout = g.layout.unwrap();
    assert_eq!(layout.args.len(), 1);
    assert_eq!(layout.args[0].name.as_ref().unwrap().name, "title");
    assert!(!ok("<p>static</p>").is_ember());
}

#[test]
fn template_contents_do_not_declare_embers() {
    let g = ok("{% state n = 0 %}\n<p>{n}</p>");
    assert_eq!(g.states.len(), 1);
    assert!(!g.is_ember());
    let g = ok("<button on:click={\n    count += 1\n    log.info(\"clicked\")\n}>+</button>");
    let button = element(&g.nodes[0]);
    let AttrValue::Handler(block) = &button.attrs[0].value else {
        panic!("expected a handler");
    };
    assert_eq!(block.stmts.len(), 2);
    assert!(!g.is_ember());
}

#[test]
fn attributes() {
    let g = ok("<a href=\"/posts/{post.id}?tab={tab}\" data-x='1' hidden disabled={busy} target=_blank>x</a>");
    let a = element(&g.nodes[0]);
    let kinds: Vec<_> = a
        .attrs
        .iter()
        .map(|at| (at.name.name.as_str(), &at.value))
        .collect();
    let AttrValue::Text(parts) = kinds[0].1 else {
        panic!()
    };
    assert_eq!(parts.len(), 4);
    assert!(matches!(&parts[0], AttrPart::Text(t) if t == "/posts/"));
    assert!(matches!(kinds[2].1, AttrValue::Flag));
    assert!(matches!(kinds[3].1, AttrValue::Expr(_)));
    assert!(matches!(kinds[4].1, AttrValue::Text(p) if p == &[AttrPart::Text("_blank".into())]));
}

#[test]
fn strings_with_quotes_inside_values() {
    ok("<a href=\"{url(\"posts.edit\", id: post.id)}\">Edit</a>");
    ok("<p>{greet(\"}\")}</p>");
}

#[test]
fn components_and_void_elements() {
    let g = ok("<Card post={post} client:visible>\n  <img src=\"/a.png\">\n  <br>\n  <Icon name=\"x\" />\n</Card>");
    let card = element(&g.nodes[0]);
    assert!(card.is_component());
    let children = content(&card.children);
    assert_eq!(children.len(), 3);
}

#[test]
fn script_and_style_are_raw_text() {
    let g = ok("<style>\n  a { color: red; }\n</style>\n<script>if (a < b) { go() }</script>");
    let style = element(&g.nodes[0]);
    assert!(matches!(&style.children[0], Node::Text { text, .. } if text.contains("color: red")));
}

#[test]
fn comments_and_plain_less_than() {
    let g = ok("{# hidden #}<p>1 < 2</p><!-- kept -->");
    let p = element(&g.nodes[0]);
    assert!(matches!(&p.children[0], Node::Text { text, .. } if text == "1 < 2"));
    assert!(matches!(&g.nodes[1], Node::Text { text, .. } if text == "<!-- kept -->"));
}

#[test]
fn if_else_chains() {
    let g = ok("{% if a %}A{% else if b %}B{% else %}C{% end %}");
    let Node::If {
        branches,
        otherwise,
        ..
    } = &g.nodes[0]
    else {
        panic!()
    };
    assert_eq!(branches.len(), 2);
    assert!(otherwise.is_some());
}

#[test]
fn for_with_filter_and_empty() {
    let g = ok("{% for p in posts where p.published %}<li>{p.title}</li>{% empty %}none{% end %}");
    let Node::For { filter, empty, .. } = &g.nodes[0] else {
        panic!()
    };
    assert!(filter.is_some());
    assert!(empty.is_some());
}

#[test]
fn match_and_case() {
    let g = ok("{% match user %}\n  {% case some(u) if u.admin %}Admin\n  {% case some(u) %}{u.name}\n  {% case none %}Guest\n{% end %}");
    let Node::Match { cases, .. } = &g.nodes[0] else {
        panic!()
    };
    assert_eq!(cases.len(), 3);
    assert!(cases[0].guard.is_some());
    assert_eq!(
        error_codes("{% match x %}oops{% case none %}{% end %}"),
        [codes::EXPECTED_TOKEN]
    );
}

#[test]
fn let_and_slot() {
    let g = ok("{% let n = items.len() %}<main>{% slot %}</main>{% slot footer %}");
    assert!(matches!(&g.nodes[0], Node::Let(_)));
    assert!(matches!(&g.nodes[2], Node::Slot { name: Some(n), .. } if n.name == "footer"));
}

#[test]
fn unclosed_elements_and_blocks() {
    fix_works("<ul>\n  <li>one</li>\n", codes::UNCLOSED_GLAZE_TAG);
    fix_works("{% if a %}\n  yes\n", codes::UNCLOSED_GLAZE_TAG);
    // The `{% for %}` is unclosed when `</ul>` arrives; the `<ul>` is fine.
    let src = "<ul>\n  {% for p in posts %}\n    <li>{p}</li>\n</ul>\n";
    let diag = one_error(src, codes::UNCLOSED_GLAZE_TAG);
    assert!(diag.message.contains("for"), "{}", diag.message);
    ok(&apply_fix(src, &diag));
    // An unclosed `<p>` inside a `<div>` is reported once.
    fix_works("<div><p>text</div>", codes::UNCLOSED_GLAZE_TAG);
    fix_works("<style>a {}", codes::UNCLOSED_GLAZE_TAG);
}

#[test]
fn mismatched_and_stray_closing_tags() {
    fix_works("<p>Hello <b>world</i></p>", codes::MISMATCHED_CLOSING_TAG);
    fix_works("<p>Hi</p></div>", codes::STRAY_CLOSING_TAG);
    fix_works("<p>Hi</p>{% end %}", codes::STRAY_CLOSING_TAG);
    fix_works(
        "{% for x in xs %}{x}{% end %}{% else %}",
        codes::STRAY_CLOSING_TAG,
    );
}

#[test]
fn tags_from_other_template_languages() {
    fix_works("{% if a %}A{% endif %}", codes::UNKNOWN_GLAZE_TAG);
    fix_works("{% for x in xs %}{x}{% endfor %}", codes::UNKNOWN_GLAZE_TAG);
    fix_works(
        "{% if a %}A{% elif b %}B{% end %}",
        codes::UNKNOWN_GLAZE_TAG,
    );
    let diag = one_error("{% extends \"base.html\" %}", codes::UNKNOWN_GLAZE_TAG);
    assert!(diag.help.as_deref().unwrap().contains("layout"));
    let diag = one_error("{% include \"card.html\" %}", codes::UNKNOWN_GLAZE_TAG);
    assert!(diag.help.as_deref().unwrap().contains("component"));
}

#[test]
fn values_from_other_template_languages() {
    let src = "<h1>{{ post.title }}</h1>";
    let diag = one_error(src, codes::FOREIGN_TEMPLATE_SYNTAX);
    assert_eq!(apply_fix(src, &diag), "<h1>{post.title}</h1>");
    let src = "<div>{post.html | safe}</div>";
    let diag = one_error(src, codes::FOREIGN_TEMPLATE_SYNTAX);
    assert_eq!(apply_fix(src, &diag), "<div>{raw post.html}</div>");
    one_error(
        "<p><?php echo $name; ?></p>",
        codes::FOREIGN_TEMPLATE_SYNTAX,
    );
    one_error("<p><%= name %></p>", codes::FOREIGN_TEMPLATE_SYNTAX);
}

#[test]
fn react_attributes() {
    fix_works("<p className=\"x\">a</p>", codes::FOREIGN_ATTRIBUTE);
    fix_works("<label htmlFor=\"x\">a</label>", codes::FOREIGN_ATTRIBUTE);
    let src = "<button onClick={save()}>Save</button>";
    let diag = one_error(src, codes::FOREIGN_ATTRIBUTE);
    assert_eq!(
        apply_fix(src, &diag),
        "<button on:click={save()}>Save</button>"
    );
    fix_works(
        "<button @click={save()}>Save</button>",
        codes::FOREIGN_ATTRIBUTE,
    );
}

#[test]
fn misplaced_declarations() {
    one_error(
        "<div>{% props a: Int %}</div>",
        codes::MISPLACED_DECLARATION,
    );
    one_error(
        "{% props a: Int %}{% props b: Int %}",
        codes::MISPLACED_DECLARATION,
    );
    one_error(
        "{% layout \"a.glaze\" %}{% layout \"b.glaze\" %}",
        codes::MISPLACED_DECLARATION,
    );
    one_error(
        "{% if a %}{% state n = 0 %}{% end %}",
        codes::MISPLACED_DECLARATION,
    );
}

#[test]
fn unclosed_code() {
    one_error(
        "<h1>{post.title</h1>\n<p>ok</p>",
        codes::UNCLOSED_GLAZE_CODE,
    );
    one_error("{% if a\n<p>x</p>", codes::UNCLOSED_GLAZE_CODE);
    one_error("{# never closed", codes::UNCLOSED_GLAZE_CODE);
}

#[test]
fn errors_inside_code_use_kiln_codes() {
    assert_eq!(error_codes("<p>{a === b}</p>"), [codes::NO_STRICT_EQUALITY]);
    assert_eq!(error_codes("{% props post %}"), [codes::EXPECTED_TYPE]);
    assert_eq!(error_codes("{% bogus %}"), [codes::UNKNOWN_GLAZE_TAG]);
    assert_eq!(
        error_codes("{% if a b %}x{% end %}"),
        [codes::EXPECTED_TOKEN]
    );
}

#[test]
fn spans_point_into_the_file() {
    let src = "<p>\n  {nope +}\n</p>";
    let diags = parse_glaze(src).diagnostics;
    assert_eq!(diags.len(), 1, "{diags:#?}");
    let at = diags[0].span.start as usize;
    assert_eq!(&src[at..at + 1], "}");
}

#[test]
fn tags_and_comments_are_recorded() {
    let src = "{% props a: Int %}\n{# note #}\n<p>{% if a > 1 %}x{% end %}</p>\n";
    let glaze = ok(src);
    let tags: Vec<&str> = glaze
        .tags
        .iter()
        .map(|s| &src[s.start as usize..s.end as usize])
        .collect();
    assert_eq!(
        tags,
        [
            "{% props a: Int %}",
            "{# note #}",
            "{% if a > 1 %}",
            "{% end %}"
        ]
    );
}

#[test]
fn fill_gives_a_named_slot_its_content() {
    let g = ok("<Card>{% fill title %}<b>Hi</b>{% end %}body</Card>");
    let Node::Element(card) = &g.nodes[0] else {
        panic!("{g:#?}")
    };
    let Node::Fill { name, body, .. } = &card.children[0] else {
        panic!("{card:#?}")
    };
    assert_eq!(name.name, "title");
    assert_eq!(body.len(), 1);
    assert_eq!(
        error_codes("{% fill title %}<b>Hi</b>"),
        [codes::UNCLOSED_GLAZE_TAG]
    );
}

#[test]
fn css_tags_import_stylesheets_and_css_modules() {
    let g = ok("{% css \"site.css\" %}\n{% css module card from \"card.module.css\" %}\n<p></p>");
    assert_eq!(g.css.len(), 2);
    assert!(g.css[0].module.is_none());
    assert_eq!(g.css[1].module.as_ref().unwrap().name, "card");
    // Imports belong at the top level of the glaze.
    assert!(!error_codes("{% if a %}{% css \"x.css\" %}{% end %}").is_empty());
    assert!(!error_codes("{% css module from \"x.css\" %}").is_empty());
}

#[test]
fn source_suffix_declares_kind_without_changing_spans() {
    use kiln_syntax::glaze::parse_component;
    for src in [
        "<p>static</p>",
        "{% state n = 0 %}<button on:click={ n += 1 }>{n}</button>",
    ] {
        let server = parse_component(src, "counter.glaze");
        let client = parse_component(src, "counter.ember");
        assert!(server.diagnostics.is_empty());
        assert!(client.diagnostics.is_empty());
        assert!(!server.glaze.is_ember());
        assert!(client.glaze.is_ember());
        assert_eq!(server.glaze.nodes, client.glaze.nodes);
        assert_eq!(server.glaze.states, client.glaze.states);
    }
}

#[test]
fn qualified_folder_components_keep_the_module_path() {
    let glaze = ok("<posts.search.Result title=\"Found\" />");
    let Node::Element(element) = &glaze.nodes[0] else {
        panic!("expected element")
    };
    assert!(element.is_component());
    assert_eq!(element.name.name, "posts.search.Result");
}
