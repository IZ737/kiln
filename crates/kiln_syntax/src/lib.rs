//! Lexer, parser and syntax tree for Kiln source files (`.kiln`).
//!
//! ```
//! let result = kiln_syntax::parse("fn main() {\n    print(\"hi\")\n}\n");
//! assert!(result.diagnostics.is_empty());
//! assert_eq!(result.module.items.len(), 1);
//! ```

pub mod ast;
pub mod lexer;
pub mod parser;
pub mod token;

use kiln_diagnostics::Diagnostic;

pub struct ParseResult {
    pub module: ast::Module,
    /// Lexer and parser errors, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses a whole source file. Always returns a module, even when there
/// are errors, so tools can keep working on the parts that did parse.
pub fn parse(src: &str) -> ParseResult {
    let (tokens, mut diagnostics) = lexer::lex(src);
    let mut parser = parser::Parser::new(tokens);
    let module = parser.parse_module();
    diagnostics.extend(parser.into_diagnostics());
    diagnostics.sort_by_key(|d| d.span.start);
    ParseResult {
        module,
        diagnostics,
    }
}
