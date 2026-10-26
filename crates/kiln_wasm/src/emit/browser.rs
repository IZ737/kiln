//! The modules that exist only in the browser: `ember`, `browser` and
//! `js`, and the bindings of npm packages.

use super::*;

pub(super) fn lib_call(g: &mut Gen, module: &str, name: &str, args: &[Expr], ty: &Ty, span: Span) {
    let _ = ty;
    for a in args {
        g.discard(a);
    }
    g.unsupported(span, format!("`{module}.{name}`"));
}
