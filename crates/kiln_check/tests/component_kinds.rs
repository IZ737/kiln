//! The file boundary is explicit even when a template has no state/events.
use kiln_diagnostics::codes;

fn check(files: &[(&str, &str)]) -> kiln_check::AppDiagnostics {
    let parsed: Vec<_> = files
        .iter()
        .map(|(name, src)| {
            let parsed = kiln_syntax::glaze::parse_component(src, name);
            assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
            parsed.glaze
        })
        .collect();
    let inputs: Vec<_> = files
        .iter()
        .zip(&parsed)
        .map(|((name, _), ast)| (*name, ast))
        .collect();
    kiln_check::check_app(&[], &inputs)
}

#[test]
fn reactive_glazes_have_precise_actionable_errors() {
    let src = "{% state n = 0 %}\n<button on:click={ n += 1 }>{n}</button>";
    let result = check(&[("counter.glaze", src)]);
    let errors = &result.glazes[0];
    assert_eq!(errors.len(), 2, "{errors:#?}");
    for error in errors {
        assert_eq!(error.code, codes::EMBER_REQUIRED);
        assert!(error.help.as_ref().unwrap().contains(".ember"));
    }
    assert_eq!(
        &src[errors[1].span.start as usize..errors[1].span.end as usize],
        "on:click"
    );
    assert!(check(&[("counter.ember", src)]).glazes[0].is_empty());
}

#[test]
fn static_embers_keep_browser_contracts_and_resolve_by_tag() {
    let result = check(&[
        ("page.glaze", "<Counter client:visible />"),
        ("counter.ember", "<p>static</p>"),
    ]);
    assert!(
        result.glazes.iter().all(Vec::is_empty),
        "{:?}",
        result.glazes
    );
    let result = check(&[
        ("page.glaze", "<Counter client:visible />"),
        ("counter.glaze", "<p>static</p>"),
    ]);
    assert_eq!(result.glazes[0][0].code, codes::EMBER_REQUIRED);
    // Without state or events it must still enforce serialization and server isolation.
    let result = check(&[("counter.ember", "{% props f: fn() -> Int %}<p>static</p>")]);
    assert_eq!(result.glazes[0][0].code, codes::NOT_SERIALIZABLE);
}

#[test]
fn duplicate_component_names_are_rejected_in_either_order() {
    let mut files = [
        ("counter.glaze", "<p>server</p>"),
        ("counter.ember", "<p>client</p>"),
    ];
    for _ in 0..2 {
        for errors in check(&files).glazes {
            assert_eq!(errors[0].code, codes::AMBIGUOUS_COMPONENT);
            assert!(errors[0].message.contains("counter.glaze"));
            assert!(errors[0].message.contains("counter.ember"));
        }
        files.reverse();
    }
}

#[test]
fn layouts_remain_server_side() {
    let result = check(&[
        ("counter.ember", "{% layout \"base.glaze\" %}<p>client</p>"),
        ("base.glaze", "{% slot %}"),
    ]);
    assert_eq!(result.glazes[0][0].code, codes::EMBER_LAYOUT);
    let result = check(&[
        ("page.glaze", "{% layout \"base.ember\" %}<p>server</p>"),
        ("base.ember", "{% slot %}"),
    ]);
    assert_eq!(result.glazes[0][0].code, codes::EMBER_LAYOUT);
}

#[test]
fn recursion_is_detected_across_component_kinds() {
    let result = check(&[("one.glaze", "<Two />"), ("two.ember", "<One />")]);
    for errors in result.glazes {
        assert_eq!(errors[0].code, codes::ENDLESS_GLAZE);
    }
}

#[test]
fn missing_components_report_both_supported_extensions() {
    let result = check(&[("page.glaze", "<Missing />")]);
    let error = &result.glazes[0][0];
    assert_eq!(error.code, codes::UNKNOWN_GLAZE);
    assert!(error.message.contains("missing.glaze"));
    assert!(error.message.contains(".ember"));
}
