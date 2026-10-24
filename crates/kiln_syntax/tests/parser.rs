use kiln_diagnostics::codes;
use kiln_syntax::ast::*;
use kiln_syntax::parse;

/// Parses `body` inside `fn main() { ... }` and returns its statements.
fn stmts(body: &str) -> Vec<Stmt> {
    let src = format!("fn main() {{\n{body}\n}}\n");
    let result = parse(&src);
    assert!(
        result.diagnostics.is_empty(),
        "unexpected diagnostics: {:#?}",
        result.diagnostics
    );
    match result.module.items.into_iter().next() {
        Some(Item::Fn(f)) => f.body.stmts,
        other => panic!("expected a function, got {other:?}"),
    }
}

fn expr(src: &str) -> Expr {
    match stmts(src).pop() {
        Some(Stmt::Expr(e)) => e,
        other => panic!("expected an expression statement, got {other:?}"),
    }
}

fn error_codes(src: &str) -> Vec<&'static str> {
    parse(src).diagnostics.iter().map(|d| d.code).collect()
}

/// Renders an expression as fully parenthesized text, to test precedence.
fn show(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Int(n) => n.clone(),
        ExprKind::Ident(name) => name.clone(),
        ExprKind::Bool(b) => b.to_string(),
        ExprKind::None => "none".into(),
        ExprKind::Binary { op, lhs, rhs } => format!("({} {op:?} {})", show(lhs), show(rhs)),
        ExprKind::Unary { op, expr } => format!("({op:?} {})", show(expr)),
        ExprKind::Field { expr, name } => format!("{}.{}", show(expr), name.name),
        ExprKind::Call { callee, args, .. } => {
            let args: Vec<_> = args.iter().map(|a| show(&a.value)).collect();
            format!("{}({})", show(callee), args.join(", "))
        }
        ExprKind::Try(inner) => format!("(try {})", show(inner)),
        ExprKind::Throw(inner) => format!("(throw {})", show(inner)),
        ExprKind::Is { expr, .. } => format!("({} is _)", show(expr)),
        other => format!("{other:?}"),
    }
}

#[test]
fn arithmetic_precedence() {
    assert_eq!(show(&expr("1 + 2 * 3 - 4")), "((1 Add (2 Mul 3)) Sub 4)");
}

#[test]
fn logic_uses_words_and_not_binds_looser_than_comparison() {
    assert_eq!(
        show(&expr("not a == b and c or d")),
        "(((Not (a Eq b)) And c) Or d)"
    );
}

#[test]
fn coalesce_with_throw() {
    assert_eq!(
        show(&expr("row ?? throw DbError.NotFound")),
        "(row Coalesce (throw DbError.NotFound))"
    );
}

#[test]
fn generic_call_and_comparison_are_told_apart() {
    let call = expr("db.query<Post>(sql)");
    let ExprKind::Call { type_args, .. } = &call.kind else {
        panic!("expected a call, got {call:?}")
    };
    assert_eq!(type_args.len(), 1);
    assert_eq!(show(&expr("a < b")), "(a Lt b)");
    assert_eq!(show(&expr("a < b and c > d")), "((a Lt b) And (c Gt d))");
}

#[test]
fn try_binds_to_the_whole_call_chain() {
    assert_eq!(show(&expr("try db.one(id)")), "(try db.one(id))");
}

#[test]
fn method_chains_continue_on_lines_starting_with_a_dot() {
    assert_eq!(
        show(&expr("names\n    .filter(f)\n    .map(g)")),
        "names.filter(f).map(g)"
    );
}

#[test]
fn operator_at_line_end_continues_the_expression() {
    assert_eq!(show(&expr("a +\n    b")), "(a Add b)");
}

#[test]
fn newline_ends_a_statement() {
    assert_eq!(stmts("a\n-b").len(), 2);
}

#[test]
fn named_arguments_and_records() {
    let e = expr("web.json(post, status: 201, extra: { post, title: \"x\" })");
    let ExprKind::Call { args, .. } = &e.kind else {
        panic!()
    };
    assert_eq!(args[0].name, None);
    assert_eq!(args[1].name.as_ref().unwrap().name, "status");
    let ExprKind::Record(fields) = &args[2].value.kind else {
        panic!()
    };
    assert!(fields[0].value.is_none(), "`post` is shorthand");
    assert!(fields[1].value.is_some());
}

#[test]
fn string_interpolation_parses_inner_expressions() {
    let e = expr(r#""Hi ${user.name}, you have ${count + 1}""#);
    let ExprKind::Str(pieces) = &e.kind else {
        panic!()
    };
    assert_eq!(pieces.len(), 4);
    let StrPiece::Expr(inner) = &pieces[3] else {
        panic!()
    };
    assert_eq!(show(inner), "(count Add 1)");
}

#[test]
fn match_arms_with_guards_and_variants() {
    let e = expr(
        "match role {\n    Admin => 1\n    Viewer(since) if since.year < 2020 => 2\n    some(x) => { x }\n    _ => 3\n}",
    );
    let ExprKind::Match { arms, .. } = &e.kind else {
        panic!()
    };
    assert_eq!(arms.len(), 4);
    assert!(matches!(&arms[0].pattern.kind, PatternKind::Variant { args, .. } if args.is_empty()));
    assert!(arms[1].guard.is_some());
    assert!(matches!(&arms[2].body.kind, ExprKind::Block(_)));
    assert_eq!(arms[3].pattern.kind, PatternKind::Wildcard);
}

#[test]
fn if_else_if_else() {
    let e = expr("if a {\n    1\n} else if b {\n    2\n} else {\n    3\n}");
    let ExprKind::If { otherwise, .. } = &e.kind else {
        panic!()
    };
    assert!(matches!(
        otherwise.as_ref().unwrap().kind,
        ExprKind::If { .. }
    ));
}

#[test]
fn for_loop_with_where_filter() {
    let s = stmts("for post in posts where post.published {\n    print(post.title)\n}");
    assert!(matches!(
        &s[0],
        Stmt::For {
            filter: Some(_),
            ..
        }
    ));
}

#[test]
fn compound_assignment() {
    let s = stmts("var n = 0\nn += 2");
    assert!(matches!(
        &s[1],
        Stmt::Assign {
            op: AssignOp::Add,
            ..
        }
    ));
}

#[test]
fn declarations() {
    let result = parse(
        "import web, web.router\n\
         pub struct Page<T> {\n    items: List<T>\n    next: Int? = none\n}\n\
         enum Role { Admin, Viewer(since: Date) }\n\
         const LIMIT: Int = 20\n\
         server fn like(id: Int) -> Int ! web.Error {\n    id\n}\n\
         fn main() ! {\n}\n",
    );
    assert!(result.diagnostics.is_empty(), "{:#?}", result.diagnostics);
    let items = &result.module.items;
    assert_eq!(items.len(), 6);
    let Item::Import(import) = &items[0] else {
        panic!()
    };
    assert_eq!(import.paths[1].len(), 2);
    let Item::Struct(page) = &items[1] else {
        panic!()
    };
    assert!(page.is_pub);
    assert!(matches!(page.fields[1].ty.kind, TypeKind::Optional(_)));
    let Item::Fn(like) = &items[4] else { panic!() };
    assert!(like.is_server);
    assert!(matches!(like.throws, Throws::Type(_)));
    let Item::Fn(main) = &items[5] else { panic!() };
    assert_eq!(main.throws, Throws::Default);
}

#[test]
fn foreign_syntax_gets_targeted_errors_with_fixes() {
    let src = "function f() {\n    let mut x = null;\n    if x === 1 && !y {\n    }\n}\n";
    assert_eq!(
        error_codes(src),
        vec![
            codes::WRONG_KEYWORD,
            codes::WRONG_KEYWORD,
            codes::NO_NULL,
            codes::NO_SEMICOLONS,
            codes::NO_STRICT_EQUALITY,
            codes::NO_SYMBOL_LOGIC,
            codes::NO_SYMBOL_LOGIC,
        ]
    );
    assert!(parse(src)
        .diagnostics
        .iter()
        .all(|d| d.suggestion.is_some()));
}

#[test]
fn elif_suggests_else_if() {
    let src = "fn f() {\n    if a {\n    } elif b {\n    }\n}\n";
    assert_eq!(error_codes(src), vec![codes::WRONG_KEYWORD]);
}

#[test]
fn statements_at_top_level_are_explained() {
    let result = parse("print(\"hi\")\n");
    assert_eq!(result.diagnostics[0].code, codes::EXPECTED_ITEM);
    assert!(result.diagnostics[0].help.is_some());
}

#[test]
fn untyped_function_parameter_is_an_error() {
    assert_eq!(error_codes("fn f(a) {\n}\n"), vec![codes::EXPECTED_TYPE]);
}

#[test]
fn recovers_and_reports_errors_in_later_functions() {
    let src = "fn a() {\n    let = 1\n}\n\nfn b() {\n    let x = )\n}\n\nfn c() {\n}\n";
    let result = parse(src);
    assert_eq!(result.diagnostics.len(), 2, "{:#?}", result.diagnostics);
    assert_eq!(result.module.items.len(), 3);
}

#[test]
fn missing_closing_brace_does_not_hang() {
    let result = parse("fn main() {\n    let x = foo(1, 2\n");
    assert!(!result.diagnostics.is_empty());
}
