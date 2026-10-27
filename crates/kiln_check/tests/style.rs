use kiln_check::style::Tokens;
use kiln_check::{check_app_with_css, Styling};

fn styling() -> Styling {
    Styling {
        tokens: Tokens::from_properties([
            ("color-accent", "#c65a34"),
            ("color-text", "#222"),
            ("space-md", "1rem"),
            ("breakpoint-tablet", "640px"),
        ]),
        ..Styling::default()
    }
}

/// The error codes and messages for a program, with the tokens above.
fn errors(src: &str) -> Vec<(String, String)> {
    let module = kiln_syntax::parse(src);
    assert!(module.diagnostics.is_empty(), "{:#?}", module.diagnostics);
    let checked = check_app_with_css(&[&module.module], &[], &styling());
    checked.modules[0]
        .iter()
        .map(|d| (d.code.to_string(), d.message.clone()))
        .collect()
}

fn codes(src: &str) -> Vec<String> {
    errors(src).into_iter().map(|e| e.0).collect()
}

#[test]
fn styles_with_tokens_keywords_and_branches_check() {
    let src = "fn s(big: Bool, n: Int) -> Ui {\n    let a = ui({ p: space.md, bg: color.accent, weight: .bold, cols: 2, hover: { fg: color.text }, at: { tablet: { cols: 3 } } })\n    let b: Ui = { text_size: \"1.2rem\", opacity: 0.5, m: if big { .auto } else { 0 }, w: match n { 1 => .full, _ => \"20rem\" } }\n    return ui.merge(a, b)\n}\n\nfn c(s: Ui) -> String {\n    return ui.classes(s)\n}\n";
    assert_eq!(errors(src), []);
}

#[test]
fn wrong_style_values_are_errors() {
    let src = "fn s(n: Int, x: String) -> Ui {\n    return ui({ p: color.accent, bg: \"1rem\", padding: space.md, cols: n, fg: \"${x}\", flex: true, grid: true })\n}\n";
    let found = errors(src);
    assert_eq!(
        found.iter().map(|e| e.0.as_str()).collect::<Vec<_>>(),
        ["E0220"; 6],
        "{found:#?}"
    );
    assert!(found[0].1.contains("spacing"), "{found:#?}");
    assert!(found[2].1.contains("no key `padding`"), "{found:#?}");
}

#[test]
fn unknown_tokens_and_breakpoints_are_errors() {
    assert_eq!(
        codes("fn s() -> Ui {\n    return ui({ bg: color.acent, at: { phone: { p: 0 } } })\n}\n"),
        ["E0221", "E0221"]
    );
}

#[test]
fn a_local_named_like_a_category_is_not_a_token() {
    // `text` is a local here, so `text.len` is not a token.
    assert_eq!(
        codes("fn s(text: String) -> Ui {\n    return ui({ text_size: text.len })\n}\n"),
        ["E0220"]
    );
}

#[test]
fn styles_become_css_rules() {
    let module = kiln_syntax::parse("fn s() -> Ui {\n    return ui({ px: space.md, hover: { fg: color.text }, at: { tablet: { p: 0 } } })\n}\n");
    let checked = check_app_with_css(&[&module.module], &[], &styling());
    let css = kiln_check::style::stylesheet(&checked.ui_rules, &styling().tokens);
    assert!(css.contains("{padding-left:var(--space-md)}"), "{css}");
    assert!(css.contains(":hover{color:var(--color-text)}"), "{css}");
    assert!(css.contains("@media (min-width:640px){"), "{css}");
}

fn themed() -> Tokens {
    Tokens::from_rules([
        (":root", "breakpoint-wide", "800px"),
        (".theme-light", "color-accent", "#c65a34"),
        (".theme-dark-mode", "color-accent", "#ffb38a"),
    ])
}

/// The error codes and messages for a program, with the themes above.
fn theme_errors(src: &str) -> Vec<(String, String)> {
    let module = kiln_syntax::parse(src);
    assert!(module.diagnostics.is_empty(), "{:#?}", module.diagnostics);
    let styling = Styling {
        tokens: themed(),
        ..Styling::default()
    };
    let checked = check_app_with_css(&[&module.module], &[], &styling);
    checked.modules[0]
        .iter()
        .map(|d| (d.code.to_string(), d.message.clone()))
        .collect()
}

#[test]
fn themes_are_chosen_by_name() {
    let src = "fn s() -> Ui {\n    return ui({ theme: .dark_mode, fg: color.accent, at: { wide: { cols: 2 } } })\n}\n";
    assert_eq!(theme_errors(src), []);
    let src = "fn s() -> Ui {\n    return ui({ theme: .dark, hover: { theme: .light } })\n}\n";
    let found = theme_errors(src);
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0].0, "E0221");
    assert!(found[0].1.contains("no theme `dark`"), "{found:?}");
    assert_eq!(found[1].0, "E0220");
    assert!(found[1].1.contains("top of a style"), "{found:?}");
}

#[test]
fn themes_must_set_the_same_tokens_with_fitting_values() {
    assert_eq!(themed().problems(), []);
    let tokens = Tokens::from_rules([
        (":root", "breakpoint-wide", "var(--x)"),
        (":root", "space-md", "red"),
        (".theme-light", "color-accent", "#c65a34"),
        (".theme-light", "color-text", "#222"),
        (".theme-dark", "color-accent", "1rem"),
        (".theme-dark", "breakpoint-wide", "900px"),
    ]);
    let found: Vec<String> = tokens.problems().into_iter().map(|p| p.1).collect();
    let expected = [
        "`--breakpoint-wide: var(--x)` is not a width without `var()`, so it cannot be a breakpoint",
        "`--space-md: red` is not a valid `space` value",
        "`--color-accent: 1rem` is not a valid `color` value",
        "theme `dark` sets `--breakpoint-wide`, but breakpoints are the same in every theme",
        "theme `light` does not set `--breakpoint-wide`, which theme `dark` sets",
        "theme `dark` does not set `--color-text`, which theme `light` sets",
    ];
    assert_eq!(found, expected);
}

#[test]
fn keys_and_words_are_snake_case() {
    let src = "fn s() -> Ui {\n    return ui({ maxW: \"40rem\", direction: .rowReverse, focus_visible: { border_color: color.accent } })\n}\n";
    let module = kiln_syntax::parse(src);
    let checked = check_app_with_css(&[&module.module], &[], &styling());
    let fixes: Vec<String> = checked.modules[0]
        .iter()
        .map(|d| {
            assert_eq!(d.code, "E0220");
            d.suggestion.as_ref().expect("a fix").replacement.clone()
        })
        .collect();
    assert_eq!(fixes, ["max_w", "row_reverse"]);
}
