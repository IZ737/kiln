//! Lowering tests: each program is lowered and its printed IR is checked
//! for the parts that matter.

fn lower(src: &str) -> String {
    let parsed = kiln_syntax::parse(src);
    assert!(parsed.diagnostics.is_empty(), "{:#?}", parsed.diagnostics);
    let checked = kiln_check::check_program(&parsed.module);
    assert!(checked.diagnostics.is_empty(), "{:#?}", checked.diagnostics);
    match kiln_ir::lower(&parsed.module, &checked) {
        Ok(ir) => ir.to_string(),
        Err(d) => panic!("{d:#?}"),
    }
}

fn lower_errors(src: &str) -> Vec<String> {
    let parsed = kiln_syntax::parse(src);
    let checked = kiln_check::check_program(&parsed.module);
    assert!(checked.diagnostics.is_empty(), "{:#?}", checked.diagnostics);
    match kiln_ir::lower(&parsed.module, &checked) {
        Ok(_) => Vec::new(),
        Err(d) => d.into_iter().map(|d| d.code.to_string()).collect(),
    }
}

#[test]
fn print_is_a_shown_string() {
    let ir = lower("fn main() {\n    let n = 2\n    print(\"n is ${n}\")\n}\n");
    assert!(
        ir.contains("print(show(concat(\"n is \", show(n#0))))"),
        "{ir}"
    );
}

#[test]
fn captured_var_lives_in_a_cell() {
    let ir = lower(
        "fn main() {\n    var n = 0\n    let k = 1\n    let add = fn() { n += k }\n    add()\n    print(n)\n}\n",
    );
    assert!(ir.contains("fn main() cells [n#0] {"), "{ir}");
    assert!(ir.contains("closure#2[n#0, k#1]"), "{ir}");
    assert!(
        ir.contains("fn closure#2() captures [n#0, k#1] cells [n#0] {"),
        "{ir}"
    );
}

#[test]
fn defaults_are_filled_in() {
    let ir = lower(
        "fn greet(name: String, greeting: String = \"Hello\") -> String {\n    \"${greeting}, ${name}\"\n}\n\nfn main() {\n    print(greet(\"Ada\"))\n}\n",
    );
    assert!(ir.contains("greet(\"Ada\", \"Hello\")"), "{ir}");
}

#[test]
fn named_arguments_keep_source_order() {
    let ir = lower(
        "fn sub(a: Int, b: Int) -> Int {\n    a - b\n}\n\nfn one() -> Int {\n    1\n}\n\nfn two() -> Int {\n    2\n}\n\nfn main() {\n    print(sub(b: one(), a: two()))\n}\n",
    );
    let calls = "let arg#0: Int = one()\n        let arg#1: Int = two()\n        sub(arg#1, arg#0)";
    assert!(ir.contains(calls), "{ir}");
}

#[test]
fn generic_values_are_coerced() {
    let ir = lower("fn main() {\n    let xs = [1, 2]\n    xs.push(3)\n    print(xs)\n}\n");
    assert!(ir.contains("xs#0.push((3 as T))"), "{ir}");
}

#[test]
fn bare_error_wraps() {
    let ir = lower(
        "enum E {\n    Bad\n}\n\nfn f() -> Int ! E {\n    throw E.Bad\n}\n\nfn g() -> Int ! {\n    try f()\n}\n\nfn h() -> Int ! {\n    throw \"no\"\n}\n\nfn main() {\n    print(g() catch err { 0 })\n    print(h() catch err { 0 })\n}\n",
    );
    assert!(
        ir.contains("fn g() -> Int ! Error {\n    try wrap f()"),
        "{ir}"
    );
    assert!(ir.contains("throw wrap \"no\""), "{ir}");
    assert!(ir.contains("    throw E.Bad"), "{ir}");
}

#[test]
fn missing_main_is_reported() {
    assert_eq!(lower_errors("fn helper() {\n}\n"), ["E0402"]);
}
