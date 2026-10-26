//! The modules that exist only in the browser: `ember`, `browser` and
//! `js`. Their functions are imports `ember.js` and `kiln.js` give, from
//! module `b`; a `js.Value` is an `externref`, as a string is.

use super::*;

/// What an import gives back, and how the value becomes a Kiln one.
#[derive(Clone, Copy)]
enum Ret {
    Nothing,
    /// A value of the import's own type: `externref`, `f64`, `i32` or `i64`.
    Plain(&'static str),
    /// An `externref` that is `null` for `none`.
    Optional,
}

/// The import for a function, its parameters' types, what it returns, and
/// whether it can fail (then it leaves an error for `take_error`).
fn import(module: &str, name: &str) -> Option<(&'static str, &'static [&'static str], Ret, bool)> {
    const S: &str = "externref";
    use Ret::*;
    Some(match (module, name) {
        ("browser", "log") => ("browser_log", &[S], Nothing, false),
        ("browser", "warn") => ("browser_warn", &[S], Nothing, false),
        ("browser", "title") => ("browser_title", &[], Plain(S), false),
        ("browser", "set_title") => ("browser_set_title", &[S], Nothing, false),
        ("browser", "location") => ("browser_location", &[], Plain(S), false),
        ("browser", "navigate") => ("browser_navigate", &[S], Nothing, false),
        ("browser", "now") => ("browser_now", &[], Plain("f64"), false),
        ("browser", "confirm") => ("browser_confirm", &[S], Plain("i32"), false),
        ("browser", "get_item") => ("browser_get_item", &[S], Optional, false),
        ("browser", "set_item") => ("browser_set_item", &[S, S], Nothing, true),
        ("browser", "remove_item") => ("browser_remove_item", &[S], Nothing, false),
        ("browser", "element") => ("browser_element", &[S], Optional, false),
        ("browser", "focus") => ("browser_focus", &[S], Nothing, false),
        ("js", "global") => ("js_global", &[S], Plain(S), false),
        ("js", "string") => ("js_identity", &[S], Plain(S), false),
        ("js", "float") => ("js_float", &["f64"], Plain(S), false),
        ("js", "bool") => ("js_bool", &["i32"], Plain(S), false),
        ("js", "null") => ("js_null", &[], Plain(S), false),
        ("js", "Value.get") => ("js_get", &[S, S], Plain(S), false),
        ("js", "Value.set") => ("js_set", &[S, S, S], Nothing, false),
        ("js", "Value.call") => ("js_call", &[S, S, S], Plain(S), true),
        ("js", "Value.as_string") => ("js_as_string", &[S], Plain(S), true),
        ("js", "Value.as_float") => ("js_as_float", &[S], Plain("f64"), true),
        ("js", "Value.as_int") => ("js_as_int", &[S], Plain("i64"), true),
        ("js", "Value.as_bool") => ("js_as_bool", &[S], Plain("i32"), true),
        ("js", "Value.is_nullish") => ("js_is_nullish", &[S], Plain("i32"), false),
        ("js", "Value.type_of") => ("js_type_of", &[S], Plain(S), false),
        _ => return None,
    })
}

/// The functions that wait for the browser, which a handler's code turns
/// into calls through the bridge, as it does calls of `server fn`s.
const WAITING: &[(&str, &str)] = &[("browser", "fetch"), ("browser", "copy")];

pub(super) fn lib_call(g: &mut Gen, module: &str, name: &str, args: &[Expr], ty: &Ty, span: Span) {
    if let ("ember", "call") = (module, name) {
        // `ember.call(name, args, done)`: the bridge sends the call, and
        // calls `done` with the reply through `__kiln_reply`.
        for a in args {
            g.expr(a);
        }
        g.extra_imports.insert(
            "  (import \"b\" \"ember_call\" (func $b_ember_call (param externref externref anyref)))"
                .to_string(),
        );
        g.line("call $b_ember_call");
        return;
    }
    if WAITING.contains(&(module, name)) {
        for a in args {
            g.discard(a);
        }
        g.diags.push(Diagnostic::error(
            codes::SERVER_CALL_PLACE,
            format!("`{module}.{name}` waits for the browser, so only an `on:` handler can call it, as a statement of its own"),
            span,
        )
        .with_help(format!("write it as a statement in the handler: `value = {module}.{name}(...) catch err {{ ... }}`")));
        g.line("unreachable");
        return;
    }
    let Some((import, params, ret, fails)) = import(module, name) else {
        let _ = ty;
        for a in args {
            g.discard(a);
        }
        g.unsupported(span, format!("`{module}.{name}`"));
        return;
    };
    for (i, a) in args.iter().enumerate() {
        g.expr(a);
        // `js.Value.call` takes its arguments as a JavaScript array.
        if (module, name, i) == ("js", "Value.call", 2) {
            g.line("call $k_list_to_js");
        }
    }
    let result = match ret {
        Ret::Nothing => String::new(),
        Ret::Plain(t) => format!(" (result {t})"),
        Ret::Optional => " (result externref)".to_string(),
    };
    let params = if params.is_empty() {
        String::new()
    } else {
        format!(" (param {})", params.join(" "))
    };
    g.extra_imports.insert(format!(
        "  (import \"b\" \"{import}\" (func $b_{import}{params}{result}))"
    ));
    g.line(&format!("call $b_{import}"));
    if let Ret::Optional = ret {
        let t = g.tmp("externref");
        g.line(&format!("local.set {t}"));
        g.line(&format!("local.get {t}"));
        g.line("ref.is_null");
        g.line("if (result (ref null $Opt))");
        g.line("ref.null $Opt");
        g.line("else");
        g.line(&format!("local.get {t}"));
        g.line("any.convert_extern");
        g.line("struct.new $Opt");
        g.line("end");
    }
    if fails {
        g.line("call $k_take_error_js");
        g.line("global.set $lib_error");
        g.lib_failed();
    }
}
