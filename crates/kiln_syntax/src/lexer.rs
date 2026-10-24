//! Turns source text into tokens.
//!
//! Newlines are significant (they end statements), so the lexer emits
//! [`TokenKind::Newline`], collapsing blank lines and comments into one.
//! String literals are lexed into parts: `"Hi ${name}!"` becomes the text
//! `Hi `, the tokens of `name`, and the text `!`.

use crate::token::{StrPart, Token, TokenKind};
use kiln_diagnostics::{codes, Diagnostic, Span};

pub fn lex(src: &str) -> (Vec<Token>, Vec<Diagnostic>) {
    let mut lexer = Lexer {
        src,
        pos: 0,
        diags: Vec::new(),
    };
    let mut tokens: Vec<Token> = Vec::new();
    loop {
        let token = lexer.next_token();
        let is_eof = token.kind == TokenKind::Eof;
        let redundant_newline = token.kind == TokenKind::Newline
            && tokens
                .last()
                .is_none_or(|prev| prev.kind == TokenKind::Newline);
        if !redundant_newline {
            tokens.push(token);
        }
        if is_eof {
            break;
        }
    }
    (tokens, lexer.diags)
}

/// Lexes code embedded in a glaze, starting at byte `start` of `src` and
/// stopping at the first `}` that closes nothing. Returns the tokens (ending
/// with `Eof` at the `}`), the errors, and the offset just past the `}`, or
/// `None` when the end of the file comes first. Spans are offsets into `src`.
pub fn lex_until_close(src: &str, start: usize) -> (Vec<Token>, Vec<Diagnostic>, Option<usize>) {
    let mut lexer = Lexer {
        src,
        pos: start,
        diags: Vec::new(),
    };
    let mut tokens: Vec<Token> = Vec::new();
    let mut depth = 0usize;
    loop {
        let token = lexer.next_token();
        match token.kind {
            TokenKind::Eof => {
                tokens.push(token);
                return (tokens, lexer.diags, None);
            }
            TokenKind::RBrace if depth == 0 => {
                let end = token.span.end as usize;
                tokens.push(Token {
                    kind: TokenKind::Eof,
                    span: token.span,
                });
                return (tokens, lexer.diags, Some(end));
            }
            TokenKind::LBrace => depth += 1,
            TokenKind::RBrace => depth -= 1,
            TokenKind::Newline if tokens.last().is_none_or(|t| t.kind == TokenKind::Newline) => {
                continue;
            }
            _ => {}
        }
        tokens.push(token);
    }
}

struct Lexer<'a> {
    src: &'a str,
    pos: usize,
    diags: Vec<Diagnostic>,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.pos += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn error(&mut self, code: &'static str, message: impl Into<String>, start: usize) {
        self.diags
            .push(Diagnostic::error(code, message, Span::new(start, self.pos)));
    }

    /// Skips spaces, tabs and comments, but not newlines.
    fn skip_trivia(&mut self) {
        loop {
            match (self.peek(), self.peek_at(1)) {
                (Some(' ' | '\t' | '\r'), _) => {
                    self.pos += 1;
                }
                (Some('/'), Some('/')) => {
                    while !matches!(self.peek(), None | Some('\n')) {
                        self.bump();
                    }
                }
                (Some('/'), Some('*')) => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        match self.bump() {
                            Some('*') if self.eat('/') => break,
                            Some(_) => {}
                            None => {
                                self.error(
                                    codes::UNTERMINATED_COMMENT,
                                    "this block comment is never closed",
                                    start,
                                );
                                break;
                            }
                        }
                    }
                }
                _ => return,
            }
        }
    }

    fn next_token(&mut self) -> Token {
        loop {
            self.skip_trivia();
            let start = self.pos;
            let Some(c) = self.bump() else {
                return Token {
                    kind: TokenKind::Eof,
                    span: Span::new(start, start),
                };
            };
            let kind = match c {
                '\n' => TokenKind::Newline,
                c if c.is_alphabetic() || c == '_' => self.ident(start),
                c if c.is_ascii_digit() => self.number(start),
                '"' => self.string(start),
                _ => match self.punct(c) {
                    Some(kind) => kind,
                    None => {
                        self.error(
                            codes::UNEXPECTED_CHAR,
                            format!("unexpected character `{c}`"),
                            start,
                        );
                        continue;
                    }
                },
            };
            return Token {
                kind,
                span: Span::new(start, self.pos),
            };
        }
    }

    fn ident(&mut self, start: usize) -> TokenKind {
        while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
            self.bump();
        }
        let word = &self.src[start..self.pos];
        TokenKind::keyword(word).unwrap_or_else(|| TokenKind::Ident(word.to_string()))
    }

    fn digits(&mut self) {
        while matches!(self.peek(), Some(c) if c.is_ascii_digit() || c == '_') {
            self.bump();
        }
    }

    fn number(&mut self, start: usize) -> TokenKind {
        self.digits();
        let mut is_float = false;
        if self.peek() == Some('.') && matches!(self.peek_at(1), Some(c) if c.is_ascii_digit()) {
            is_float = true;
            self.bump();
            self.digits();
        }
        if matches!(self.peek(), Some('e' | 'E'))
            && matches!(
                (self.peek_at(1), self.peek_at(2)),
                (Some(d), _) | (Some('+' | '-'), Some(d)) if d.is_ascii_digit()
            )
        {
            is_float = true;
            self.bump();
            if matches!(self.peek(), Some('+' | '-')) {
                self.bump();
            }
            self.digits();
        }
        let digits_end = self.pos;
        if self.src[start..digits_end].ends_with('_') {
            self.error(codes::INVALID_NUMBER, "a number cannot end with `_`", start);
        }
        if matches!(self.peek(), Some(c) if c.is_alphabetic() || c == '_') {
            while matches!(self.peek(), Some(c) if c.is_alphanumeric() || c == '_') {
                self.bump();
            }
            let suffix = &self.src[digits_end..self.pos];
            if !is_float && matches!(suffix, "ms" | "s" | "m" | "h") {
                return TokenKind::Duration(self.src[start..self.pos].to_string());
            }
            self.error(
                codes::INVALID_NUMBER,
                format!("invalid suffix `{suffix}` on a number"),
                start,
            );
            self.diags.last_mut().unwrap().help =
                Some("durations take the units `ms`, `s`, `m` and `h`, as in `250ms`".into());
        }
        let text = self.src[start..digits_end].replace('_', "");
        if is_float {
            TokenKind::Float(text)
        } else {
            TokenKind::Int(text)
        }
    }

    fn string(&mut self, start: usize) -> TokenKind {
        let mut parts = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek() {
                Some('"') => {
                    self.bump();
                    break;
                }
                None | Some('\n') => {
                    self.error(
                        codes::UNTERMINATED_STRING,
                        "this string is never closed",
                        start,
                    );
                    break;
                }
                Some('\\') => {
                    let escape_start = self.pos;
                    self.bump();
                    let escaped = match self.bump() {
                        Some('n') => '\n',
                        Some('t') => '\t',
                        Some('r') => '\r',
                        Some('0') => '\0',
                        Some('\\') => '\\',
                        Some('"') => '"',
                        Some('$') => '$',
                        other => {
                            let shown = other.map(String::from).unwrap_or_default();
                            self.error(
                                codes::INVALID_ESCAPE,
                                format!("unknown escape `\\{shown}`"),
                                escape_start,
                            );
                            continue;
                        }
                    };
                    text.push(escaped);
                }
                Some('$') if self.peek_at(1) == Some('{') => {
                    if !text.is_empty() {
                        parts.push(StrPart::Text(std::mem::take(&mut text)));
                    }
                    let interp_start = self.pos;
                    let diag_count = self.diags.len();
                    self.pos += 2;
                    let mut tokens = Vec::new();
                    let mut depth = 0usize;
                    loop {
                        let token = self.next_token();
                        match token.kind {
                            TokenKind::LBrace => depth += 1,
                            TokenKind::RBrace if depth == 0 => break,
                            TokenKind::RBrace => depth -= 1,
                            TokenKind::Newline | TokenKind::Eof => {
                                // Errors inside the broken interpolation are
                                // knock-on effects of the missing `}`.
                                self.diags.truncate(diag_count);
                                self.pos = token.span.start as usize;
                                self.error(
                                    codes::UNCLOSED_INTERPOLATION,
                                    "`${` is never closed with `}`",
                                    interp_start,
                                );
                                return TokenKind::Str(parts);
                            }
                            _ => {}
                        }
                        tokens.push(token);
                    }
                    parts.push(StrPart::Interp(tokens, Span::new(interp_start, self.pos)));
                }
                Some(c) => {
                    self.bump();
                    text.push(c);
                }
            }
        }
        if !text.is_empty() || parts.is_empty() {
            parts.push(StrPart::Text(text));
        }
        TokenKind::Str(parts)
    }

    fn punct(&mut self, c: char) -> Option<TokenKind> {
        use TokenKind::*;
        Some(match c {
            '(' => LParen,
            ')' => RParen,
            '{' => LBrace,
            '}' => RBrace,
            '[' => LBracket,
            ']' => RBracket,
            ',' => Comma,
            ';' => Semi,
            ':' => Colon,
            '%' => Percent,
            '.' if self.eat('.') => DotDot,
            '.' => Dot,
            '-' if self.eat('>') => Arrow,
            '-' if self.eat('=') => MinusEq,
            '-' => Minus,
            '+' if self.eat('=') => PlusEq,
            '+' => Plus,
            '*' if self.eat('=') => StarEq,
            '*' => Star,
            '/' if self.eat('=') => SlashEq,
            '/' => Slash,
            '=' if self.eat('>') => FatArrow,
            '=' if self.eat('=') => {
                if self.eat('=') {
                    EqEqEq
                } else {
                    EqEq
                }
            }
            '=' => Eq,
            '!' if self.eat('=') => {
                if self.eat('=') {
                    BangEqEq
                } else {
                    NotEq
                }
            }
            '!' => Bang,
            '<' if self.eat('=') => Le,
            '<' => Lt,
            '>' if self.eat('=') => Ge,
            '>' => Gt,
            '?' if self.eat('?') => QuestionQuestion,
            '?' => Question,
            '&' if self.eat('&') => AmpAmp,
            '|' if self.eat('|') => PipePipe,
            _ => return Option::None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use TokenKind::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        let (tokens, diags) = lex(src);
        assert!(diags.is_empty(), "unexpected diagnostics: {diags:?}");
        tokens.into_iter().map(|t| t.kind).collect()
    }

    fn ident(s: &str) -> TokenKind {
        Ident(s.to_string())
    }

    #[test]
    fn keywords_and_identifiers() {
        assert_eq!(
            kinds("let name = posts"),
            vec![Let, ident("name"), Eq, ident("posts"), Eof]
        );
    }

    #[test]
    fn blank_lines_and_comments_collapse_to_one_newline() {
        assert_eq!(
            kinds("a // one\n\n/* two\nthree */\n\nb"),
            vec![ident("a"), Newline, ident("b"), Eof]
        );
    }

    #[test]
    fn numbers_and_durations() {
        assert_eq!(
            kinds("1_000 2.5 1e3 250ms 2s"),
            vec![
                Int("1000".into()),
                Float("2.5".into()),
                Float("1e3".into()),
                Duration("250ms".into()),
                Duration("2s".into()),
                Eof
            ]
        );
    }

    #[test]
    fn range_is_not_a_float() {
        assert_eq!(
            kinds("1..10"),
            vec![Int("1".into()), DotDot, Int("10".into()), Eof]
        );
    }

    #[test]
    fn string_interpolation_splits_into_parts() {
        let (tokens, diags) = lex(r#""Hi ${user.name}!""#);
        assert!(diags.is_empty());
        let Str(parts) = &tokens[0].kind else {
            panic!("expected a string")
        };
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0], StrPart::Text("Hi ".into()));
        let StrPart::Interp(inner, _) = &parts[1] else {
            panic!("expected interpolation")
        };
        let inner: Vec<_> = inner.iter().map(|t| t.kind.clone()).collect();
        assert_eq!(inner, vec![ident("user"), Dot, ident("name")]);
        assert_eq!(parts[2], StrPart::Text("!".into()));
    }

    #[test]
    fn braces_without_dollar_are_plain_text() {
        let (tokens, _) = lex(r#""/posts/{id: Int}""#);
        assert_eq!(
            tokens[0].kind,
            Str(vec![StrPart::Text("/posts/{id: Int}".into())])
        );
    }

    #[test]
    fn foreign_operators_are_lexed_for_better_errors() {
        assert_eq!(
            kinds("a === b && c"),
            vec![ident("a"), EqEqEq, ident("b"), AmpAmp, ident("c"), Eof]
        );
    }

    #[test]
    fn unterminated_string_is_reported() {
        let (_, diags) = lex("let s = \"oops\nlet t = 1");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].code, codes::UNTERMINATED_STRING);
    }

    #[test]
    fn unclosed_interpolation_is_one_error() {
        let (tokens, diags) = lex("let s = \"Hi ${name\"\nlet t = 1");
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, codes::UNCLOSED_INTERPOLATION);
        assert!(tokens.iter().any(|t| t.kind == Ident("t".into())));
    }

    #[test]
    fn bad_suffix_is_reported() {
        let (_, diags) = lex("let n = 12ab");
        assert_eq!(diags[0].code, codes::INVALID_NUMBER);
    }

    #[test]
    fn unexpected_character_is_skipped() {
        let (tokens, diags) = lex("a # b");
        assert_eq!(diags[0].code, codes::UNEXPECTED_CHAR);
        assert_eq!(tokens.len(), 3);
    }
}
