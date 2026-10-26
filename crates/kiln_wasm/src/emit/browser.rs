//! The modules that exist only in the browser: `ember`, `browser` and
//! `js`, and the bindings of npm packages.

use super::*;

pub(super) fn lib_call(g: &mut Gen, module: &str, name: &str, args: &[Expr], ty: &Ty, span: Span) {
    let _ = ty;
    if let ("ember", "call") = (module, name) {
        // `ember.call(name, args, done)`: the bridge sends the call, and
        // calls `done` with the reply through `__kiln_reply`.
        for a in args {
            g.expr(a);
        }
        g.extra_imports.insert(
            "  (import \"k\" \"ember_call\" (func $k_ember_call (param externref externref anyref)))"
                .to_string(),
        );
        g.line("call $k_ember_call");
        return;
    }
    for a in args {
        g.discard(a);
    }
    g.unsupported(span, format!("`{module}.{name}`"));
}
