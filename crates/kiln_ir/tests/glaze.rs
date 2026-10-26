//! Compiling glazes into Kiln: the generated code parses, checks and
//! lowers, and errors in it point back into the glaze.

use kiln_ir::glaze::{generate, generate_client, App, GlazeInput, GlazeSource};
use kiln_syntax::ast::Glaze;

const APP: &str = "import web\n\nstruct Post {\n    id: Int\n    title: String\n}\n\nfn main() {\n    let res = web.render(\"page.glaze\", { post: Post(id: 1, title: \"Hi\") })\n    print(res.body)\n}\n";

/// Generates the Kiln source for `glazes` in an app with `APP`.
fn compile(glazes: &[(&str, &str)]) -> (String, kiln_ir::glaze::GlazeSource) {
    let module = kiln_syntax::parse(APP).module;
    let parsed: Vec<Glaze> = glazes
        .iter()
        .map(|(_, src)| {
            let r = kiln_syntax::glaze::parse_glaze(src);
            assert!(r.diagnostics.is_empty(), "{:#?}", r.diagnostics);
            r.glaze
        })
        .collect();
    let refs: Vec<(&str, &Glaze)> = glazes.iter().map(|(n, _)| *n).zip(&parsed).collect();
    let checked = kiln_check::check_app(&[&module], &refs);
    for d in checked.glazes.iter().chain(&checked.modules) {
        assert!(d.is_empty(), "{d:#?}");
    }
    let inputs: Vec<GlazeInput> = (0..glazes.len())
        .map(|i| GlazeInput {
            file: glazes[i].0,
            src: glazes[i].1,
            glaze: &parsed[i],
            values: &checked.glaze_values[i],
            scope: None,
            modules: None,
            states: &checked.glaze_states[i],
        })
        .collect();
    let source = generate(&inputs, &App::default()).expect("glazes");
    let combined = format!("{APP}{}", source.text);
    let parsed_all = kiln_syntax::parse(&combined);
    assert!(
        parsed_all.diagnostics.is_empty(),
        "{:#?}\n{}",
        parsed_all.diagnostics,
        source.text
    );
    let program = kiln_check::check_program_with_glazes(&parsed_all.module, &refs);
    assert!(
        program.diagnostics.is_empty(),
        "{:#?}\n{}",
        program.diagnostics,
        source.text
    );
    let ir = kiln_ir::lower(&parsed_all.module, &program).unwrap_or_else(|d| panic!("{d:#?}"));
    (ir.to_string(), source)
}

#[test]
fn a_page_becomes_two_functions() {
    let (ir, source) = compile(&[(
        "page.glaze",
        "{% props post: Post %}\n<h1 id=\"p{post.id}\">{post.title}</h1>\n",
    )]);
    assert!(
        source
            .text
            .contains("fn __glaze_page(__out: List<String>, post: Post)"),
        "{}",
        source.text
    );
    assert!(
        source
            .text
            .contains("fn __render_page(post: Post) -> String"),
        "{}",
        source.text
    );
    assert!(ir.contains("fn __render_page("), "{ir}");
    // `web.render` calls the render function and makes an HTML response.
    assert!(ir.contains("web.html("), "{ir}");
}

#[test]
fn copied_code_maps_back_to_the_glaze() {
    let page = "{% props post: Post %}\n<h1>{post.title.upper()}</h1>\n";
    let (_, source) = compile(&[("page.glaze", page)]);
    let at = source.text.find("post.title.upper()").unwrap();
    let span = kiln_diagnostics::Span::new(at + 5, at + 10);
    let (glaze, mapped) = source.locate(span).unwrap();
    assert_eq!(glaze, 0);
    assert_eq!(&page[mapped.start as usize..mapped.end as usize], "title");
}

/// Compiles an ember for the server and for the browser.
fn ember(page: &str) -> (GlazeSource, GlazeSource) {
    let module = kiln_syntax::parse(APP).module;
    let glaze = kiln_syntax::glaze::parse_glaze(page).glaze;
    let checked = kiln_check::check_app(&[&module], &[("page.glaze", &glaze)]);
    assert!(checked.glazes[0].is_empty(), "{:#?}", checked.glazes[0]);
    let input = [GlazeInput {
        file: "page.glaze",
        src: page,
        glaze: &glaze,
        values: &checked.glaze_values[0],
        scope: None,
        modules: None,
        states: &checked.glaze_states[0],
    }];
    let app = App {
        ember_script: Some("/_kiln/ember.js"),
        ..App::default()
    };
    (
        generate(&input, &app).unwrap(),
        generate_client(&input, &App::default()),
    )
}

#[test]
fn embers_render_as_islands() {
    let (server, client) = ember(
        "{% props post: Post %}\n{% state n = 1 %}\n<button on:click.prevent={ n += 1 }>{n}</button>\n",
    );
    assert!(server.diagnostics.is_empty());
    assert!(
        server
            .text
            .contains("<kiln-island data-k-ember=\\\"page\\\""),
        "{}",
        server.text
    );
    assert!(server.text.contains("json.encode({ post: post })"));
    assert!(server.text.contains("json.encode({ n: n })"));
    // The server renders no handlers; the browser does.
    assert!(!server.text.contains("data-k-on"));
    assert!(client.diagnostics.is_empty());
    assert!(client
        .text
        .contains("fn __ember_page(__props: String, __state: String) -> ember.Island"));
    assert!(
        client.text.contains("data-k-on-click=\\\""),
        "{}",
        client.text
    );
    assert!(client.text.contains(":prevent"));
    assert!(client
        .text
        .contains("let __s = json.decode<{ n: Int }>(__state)"));
}
