//! The registry of error codes.
//!
//! Codes are stable once released: a code is never reused for a different
//! error. `kiln explain <code>` prints the explanation and examples, so every
//! entry carries a wrong and a right example.
//!
//! Ranges: `E00xx` lexing, `E01xx` parsing.

pub struct ErrorCode {
    pub code: &'static str,
    pub title: &'static str,
    pub explanation: &'static str,
    pub wrong: &'static str,
    pub right: &'static str,
}

pub const UNEXPECTED_CHAR: &str = "E0001";
pub const UNTERMINATED_STRING: &str = "E0002";
pub const UNTERMINATED_COMMENT: &str = "E0003";
pub const INVALID_ESCAPE: &str = "E0004";
pub const INVALID_NUMBER: &str = "E0005";
pub const UNCLOSED_INTERPOLATION: &str = "E0006";

pub const EXPECTED_TOKEN: &str = "E0101";
pub const EXPECTED_EXPRESSION: &str = "E0102";
pub const EXPECTED_TYPE: &str = "E0103";
pub const EXPECTED_ITEM: &str = "E0104";
pub const EXPECTED_PATTERN: &str = "E0105";
pub const NO_NULL: &str = "E0106";
pub const NO_SEMICOLONS: &str = "E0107";
pub const WRONG_KEYWORD: &str = "E0108";
pub const NO_STRICT_EQUALITY: &str = "E0109";
pub const NO_SYMBOL_LOGIC: &str = "E0110";

pub static ALL: &[ErrorCode] = &[
    ErrorCode {
        code: UNEXPECTED_CHAR,
        title: "unexpected character",
        explanation: "The source contains a character that cannot start any token.",
        wrong: "let price = 5 # dollars",
        right: "let price = 5 // dollars",
    },
    ErrorCode {
        code: UNTERMINATED_STRING,
        title: "unterminated string literal",
        explanation: "A string literal was opened with `\"` but never closed on the same line. Strings cannot span lines; use `\\n` for a line break.",
        wrong: "let name = \"Jake",
        right: "let name = \"Jake\"",
    },
    ErrorCode {
        code: UNTERMINATED_COMMENT,
        title: "unterminated block comment",
        explanation: "A `/*` block comment was never closed with `*/`.",
        wrong: "/* TODO\nfn main() {}",
        right: "/* TODO */\nfn main() {}",
    },
    ErrorCode {
        code: INVALID_ESCAPE,
        title: "invalid escape sequence",
        explanation: "Inside a string, `\\` must be followed by one of `n`, `t`, `r`, `0`, `\\`, `\"` or `$`.",
        wrong: "let path = \"C:\\users\"",
        right: "let path = \"C:\\\\users\"",
    },
    ErrorCode {
        code: INVALID_NUMBER,
        title: "invalid number literal",
        explanation: "A number literal is malformed, for example a trailing `_` or a letter inside the digits.",
        wrong: "let n = 12ab",
        right: "let n = 12",
    },
    ErrorCode {
        code: UNCLOSED_INTERPOLATION,
        title: "unclosed string interpolation",
        explanation: "`${` inside a string starts an interpolated expression, which must be closed with `}` on the same line. Write `\\$` for a literal dollar sign before a brace.",
        wrong: "let s = \"Hi ${name\"",
        right: "let s = \"Hi ${name}\"",
    },
    ErrorCode {
        code: EXPECTED_TOKEN,
        title: "expected a specific token",
        explanation: "The parser needed a particular token here, such as a closing bracket or a name.",
        wrong: "fn add(a: Int, b: Int -> Int { a + b }",
        right: "fn add(a: Int, b: Int) -> Int { a + b }",
    },
    ErrorCode {
        code: EXPECTED_EXPRESSION,
        title: "expected an expression",
        explanation: "A value was needed here, such as a literal, a name, a call or a block.",
        wrong: "let total = ",
        right: "let total = 0",
    },
    ErrorCode {
        code: EXPECTED_TYPE,
        title: "expected a type",
        explanation: "A type was needed here. Types are names such as `Int` or `List<Post>`, optionally followed by `?`.",
        wrong: "fn find(id: ) -> Post? { none }",
        right: "fn find(id: Int) -> Post? { none }",
    },
    ErrorCode {
        code: EXPECTED_ITEM,
        title: "expected a declaration",
        explanation: "At the top level of a file only declarations are allowed: `import`, `fn`, `struct`, `enum`, `impl`, `const` and `server fn`. Statements belong inside a function such as `main`.",
        wrong: "print(\"hello\")",
        right: "fn main() {\n    print(\"hello\")\n}",
    },
    ErrorCode {
        code: EXPECTED_PATTERN,
        title: "expected a pattern",
        explanation: "A `match` arm starts with a pattern: `_`, a literal, a name, or an enum variant such as `some(x)`.",
        wrong: "match x {\n    => 1\n}",
        right: "match x {\n    _ => 1\n}",
    },
    ErrorCode {
        code: NO_NULL,
        title: "Kiln has no null",
        explanation: "There is no `null` or `nil`. A value that may be missing has an optional type `T?`, and the missing value is written `none`.",
        wrong: "let user: User? = null",
        right: "let user: User? = none",
    },
    ErrorCode {
        code: NO_SEMICOLONS,
        title: "semicolons are not used",
        explanation: "Statements end at the end of the line. Remove the `;`. To put two statements on one line, put them on separate lines instead.",
        wrong: "let a = 1;",
        right: "let a = 1",
    },
    ErrorCode {
        code: WRONG_KEYWORD,
        title: "keyword from another language",
        explanation: "This word is a keyword in another language. Kiln uses `fn` for functions, `let` and `var` for variables, and `else if` for chained conditions.",
        wrong: "function add(a: Int, b: Int) -> Int { a + b }",
        right: "fn add(a: Int, b: Int) -> Int { a + b }",
    },
    ErrorCode {
        code: NO_STRICT_EQUALITY,
        title: "`===` and `!==` do not exist",
        explanation: "Kiln never converts types implicitly, so `==` is already strict. Use `==` and `!=`.",
        wrong: "if a === b { }",
        right: "if a == b { }",
    },
    ErrorCode {
        code: NO_SYMBOL_LOGIC,
        title: "use `and`, `or` and `not`",
        explanation: "Logical operators are words: `and`, `or` and `not`. The symbols `&&`, `||` and prefix `!` are not used.",
        wrong: "if a && !b { }",
        right: "if a and not b { }",
    },
];

pub fn lookup(code: &str) -> Option<&'static ErrorCode> {
    let code = code.to_ascii_uppercase();
    ALL.iter().find(|c| c.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_unique_and_well_formed() {
        let mut seen = std::collections::HashSet::new();
        for c in ALL {
            assert!(seen.insert(c.code), "duplicate code {}", c.code);
            assert!(c.code.len() == 5 && c.code.starts_with('E'));
        }
    }

    #[test]
    fn lookup_ignores_case() {
        assert_eq!(lookup("e0106").unwrap().code, NO_NULL);
    }
}
