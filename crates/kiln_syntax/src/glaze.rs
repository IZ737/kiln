//! Parses glazes: `.glaze` templates and components.
//!
//! A glaze is HTML with code in it:
//!
//! - `{value}` outputs a value, escaped; `{raw value}` outputs it unescaped.
//! - `{% tag ... %}` declares props, state and the layout, or opens and
//!   closes `if`, `for` and `match` blocks. Every block ends with `{% end %}`.
//! - `{# comment #}` is dropped.
//! - Capitalized tags such as `<LikeButton post={post} />` are components.
//!
//! Code inside braces is lexed by [`crate::lexer`] and parsed by
//! [`crate::parser`], so it is exactly the language of `.kiln` files.

use crate::ast::*;
use crate::lexer::lex_until_close;
use crate::parser::Parser;
use crate::token::{Token, TokenKind};
use kiln_diagnostics::{codes, Diagnostic, Span};

pub struct GlazeResult {
    pub glaze: Glaze,
    /// Errors in source order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Parses a whole `.glaze` file. Always returns a glaze, even when there
/// are errors.
pub fn parse_glaze(src: &str) -> GlazeResult {
    let mut p = GlazeParser {
        src,
        pos: 0,
        diags: Vec::new(),
        open: Vec::new(),
        pending: None,
        glaze: Glaze::default(),
    };
    let (nodes, end) = p.parse_nodes();
    debug_assert!(matches!(end, Ending::Eof));
    p.glaze.nodes = nodes;
    p.diags.sort_by_key(|d| d.span.start);
    GlazeResult {
        glaze: p.glaze,
        diagnostics: p.diags,
    }
}

/// Elements that never have children or a closing tag.
const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "source", "track",
    "wbr",
];

/// Elements whose contents are not parsed: no tags and no `{}` values.
const RAW_TEXT_ELEMENTS: &[&str] = &["script", "style"];

/// Something open that must be closed.
#[derive(Clone, Debug, PartialEq)]
enum Open {
    Element(String),
    If,
    For,
    Match,
    Fill,
}

/// What ended a run of nodes.
#[derive(Clone, Debug)]
enum Ending {
    Eof,
    /// `</name>`
    CloseTag(Ident, Span),
    /// `{% end %}`
    EndBlock(Span),
    ElseIf(Expr, Span),
    Else(Span),
    Empty(Span),
    Case(Pattern, Option<Expr>, Span),
}

/// A parsed `{% ... %}` tag.
enum Tag {
    Props(Vec<Param>),
    State(LocalDecl),
    Let(LocalDecl),
    Layout(Layout),
    Slot(Option<Ident>),
    Fill(Ident),
    If(Expr),
    For {
        binding: Ident,
        iter: Expr,
        filter: Option<Expr>,
    },
    Match(Expr),
    /// A tag that ends or divides a block.
    Ending(Ending),
    /// Already reported.
    Error,
}

struct GlazeParser<'a> {
    src: &'a str,
    pos: usize,
    diags: Vec<Diagnostic>,
    open: Vec<Open>,
    /// An ending that belongs to an enclosing element or block, passed up
    /// after an inner one was found unclosed.
    pending: Option<Ending>,
    glaze: Glaze,
}

impl GlazeParser<'_> {
    fn rest(&self) -> &str {
        &self.src[self.pos..]
    }

    fn starts_with(&self, s: &str) -> bool {
        self.rest().starts_with(s)
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn error(&mut self, diag: Diagnostic) {
        self.diags.push(diag);
    }

    // ----- nodes -----

    /// Parses nodes until the end of the file or an ending tag that belongs
    /// to something open.
    fn parse_nodes(&mut self) -> (Vec<Node>, Ending) {
        let mut nodes = Vec::new();
        let mut text = String::new();
        let mut text_start = self.pos;
        macro_rules! flush_text {
            () => {
                if !text.is_empty() {
                    nodes.push(Node::Text {
                        text: std::mem::take(&mut text),
                        span: Span::new(text_start, self.pos),
                    });
                }
            };
        }
        loop {
            if let Some(end) = self.pending.take() {
                flush_text!();
                return (nodes, end);
            }
            let start = self.pos;
            let Some(c) = self.peek() else {
                flush_text!();
                return (nodes, Ending::Eof);
            };
            if self.at_markup() {
                flush_text!();
                if let Some(end) = self.parse_markup(&mut nodes) {
                    return (nodes, end);
                }
                text_start = self.pos;
                continue;
            }
            if text.is_empty() {
                text_start = start;
            }
            self.bump();
            text.push(c);
        }
    }

    /// Parses one piece of markup at `{` or `<`, pushing nodes. Returns an
    /// ending when one is reached. Leaves the position unchanged when the
    /// character is plain text.
    fn parse_markup(&mut self, nodes: &mut Vec<Node>) -> Option<Ending> {
        let start = self.pos;
        if self.starts_with("<!") && !self.starts_with("<!--") {
            // `<!doctype html>`
            let end = self
                .rest()
                .find('>')
                .map_or(self.src.len(), |i| self.pos + i + 1);
            nodes.push(Node::Text {
                text: self.src[self.pos..end].to_string(),
                span: Span::new(self.pos, end),
            });
            self.pos = end;
        } else if self.starts_with("{#") {
            self.skip_comment();
            self.glaze.tags.push(Span::new(start, self.pos));
        } else if self.starts_with("{%") {
            let tag = self.parse_tag();
            self.glaze.tags.push(Span::new(start, self.pos));
            match tag {
                Tag::Ending(end) => return self.ending(end),
                tag => self.place_tag(tag, start, nodes),
            }
        } else if self.starts_with("{") {
            if let Some(node) = self.parse_value() {
                nodes.push(node);
            }
        } else if self.starts_with("</") {
            let name_start = self.pos + 2;
            let name = self.tag_name(name_start);
            if name.is_empty() {
                return None;
            }
            self.pos = name_start + name.len();
            self.skip_whitespace();
            if self.peek() == Some('>') {
                self.bump();
            } else {
                self.error(Diagnostic::error(
                    codes::EXPECTED_TOKEN,
                    "expected `>` to end the closing tag",
                    Span::new(self.pos, self.pos),
                ));
            }
            let name = Ident {
                name,
                span: Span::new(name_start, name_start + self.tag_name(name_start).len()),
            };
            return self.ending(Ending::CloseTag(name, Span::new(start, self.pos)));
        } else if self.starts_with("<!--") {
            let end = self
                .rest()
                .find("-->")
                .map_or(self.src.len(), |i| self.pos + i + 3);
            nodes.push(Node::Text {
                text: self.src[self.pos..end].to_string(),
                span: Span::new(self.pos, end),
            });
            self.pos = end;
        } else if self.starts_with("<?") || self.starts_with("<%") {
            let close = if self.starts_with("<?") { "?>" } else { "%>" };
            let end = self
                .rest()
                .find(close)
                .map_or(self.src.len(), |i| self.pos + i + 2);
            self.error(
                Diagnostic::error(
                    codes::FOREIGN_TEMPLATE_SYNTAX,
                    "PHP and ERB tags are not used in glazes",
                    Span::new(start, end),
                )
                .with_help(
                    "output a value with `{value}`, and use `{% if %}` and `{% for %}` for logic",
                ),
            );
            self.pos = end;
        } else {
            nodes.push(Node::Element(self.parse_element()));
        }
        None
    }

    /// Whether markup starts here, as opposed to text such as `a < b`.
    fn at_markup(&self) -> bool {
        match self.peek() {
            Some('{') => true,
            Some('<') => match self.peek_at(1) {
                Some('/') => !self.tag_name(self.pos + 2).is_empty(),
                Some('!' | '?' | '%') => true,
                Some(c) => c.is_ascii_alphabetic(),
                None => false,
            },
            _ => false,
        }
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.rest().chars().nth(n)
    }

    /// Decides whether an ending belongs to something open. Endings that
    /// close nothing are reported and skipped.
    fn ending(&mut self, end: Ending) -> Option<Ending> {
        let (fits, what) = match &end {
            Ending::Eof => (true, String::new()),
            // A closing tag inside an element is that element's, possibly
            // misspelled.
            Ending::CloseTag(name, _) => (
                matches!(self.open.last(), Some(Open::Element(_)))
                    || self
                        .open
                        .iter()
                        .any(|o| matches!(o, Open::Element(n) if *n == name.name)),
                format!("`</{}>`", name.name),
            ),
            Ending::EndBlock(_) => (
                self.open.iter().any(|o| !matches!(o, Open::Element(_))),
                "`{% end %}`".into(),
            ),
            Ending::ElseIf(..) | Ending::Else(_) => (
                self.open.contains(&Open::If),
                "this `{% else %}`, which is not inside `{% if %}`,".into(),
            ),
            Ending::Empty(_) => (
                self.open.contains(&Open::For),
                "this `{% empty %}`, which is not inside `{% for %}`,".into(),
            ),
            Ending::Case(..) => (
                self.open.contains(&Open::Match),
                "this `{% case %}`, which is not inside `{% match %}`,".into(),
            ),
        };
        if fits {
            return Some(end);
        }
        let span = end_span(&end);
        self.error(
            Diagnostic::error(
                codes::STRAY_CLOSING_TAG,
                format!("{what} closes nothing"),
                span,
            )
            .with_suggestion("remove it", span, ""),
        );
        None
    }

    fn skip_comment(&mut self) {
        let start = self.pos;
        match self.rest().find("#}") {
            Some(i) => self.pos += i + 2,
            None => {
                self.pos = self.src.len();
                self.error(Diagnostic::error(
                    codes::UNCLOSED_GLAZE_CODE,
                    "`{#` comment is never closed with `#}`",
                    Span::new(start, start + 2),
                ));
            }
        }
    }

    /// Lexes code from `from` up to the closing `}`. On failure, reports
    /// `what` as unclosed and skips to the end of the line.
    fn code_until_close(&mut self, start: usize, from: usize, what: &str) -> Option<Vec<Token>> {
        let (tokens, diags, close) = lex_until_close(self.src, from);
        match close {
            Some(end) => {
                self.diags.extend(diags);
                self.pos = end;
                Some(tokens)
            }
            None => {
                // Errors after the missing `}` are knock-on effects.
                self.error(Diagnostic::error(
                    codes::UNCLOSED_GLAZE_CODE,
                    format!("`{what}` is never closed"),
                    Span::new(start, from),
                ));
                // Resume at the next line, or for a value at the next tag,
                // which is usually where the `}` was forgotten.
                let stops: &[char] = if what == "{" { &['\n', '<'] } else { &['\n'] };
                self.pos = self.src[from..]
                    .find(stops)
                    .map_or(self.src.len(), |i| from + i);
                None
            }
        }
    }

    /// `{value}` or `{raw value}`, at `{`.
    fn parse_value(&mut self) -> Option<Node> {
        let start = self.pos;
        let double = self.starts_with("{{");
        let from = start + if double { 2 } else { 1 };
        let mut tokens = self.code_until_close(start, from, "{")?;
        let mut span = Span::new(start, self.pos);
        if double {
            let inner_end = self.pos - 1;
            if self.peek() == Some('}') {
                self.bump();
            }
            span = Span::new(start, self.pos);
            let inner = self.src[from..inner_end].trim();
            self.error(
                Diagnostic::error(
                    codes::FOREIGN_TEMPLATE_SYNTAX,
                    "glazes output values with single braces",
                    span,
                )
                .with_suggestion(
                    format!("write `{{{inner}}}`"),
                    span,
                    format!("{{{inner}}}"),
                ),
            );
        }
        let inner = &self.src[from..self.pos - if double { 2 } else { 1 }];
        if let Some(filter) = foreign_filter(inner) {
            // `{x | safe}`: the lexer's errors about `|` are knock-on effects.
            self.diags
                .retain(|d| !(d.span.start as usize >= from && (d.span.end as usize) <= self.pos));
            let mut diag = Diagnostic::error(
                codes::FOREIGN_TEMPLATE_SYNTAX,
                format!("glazes have no `| {filter}` filters"),
                span,
            );
            let value = inner[..inner.rfind('|').unwrap()].trim();
            if matches!(filter, "safe" | "raw") {
                diag = diag.with_suggestion(
                    format!("write `{{raw {value}}}` to output it unescaped"),
                    span,
                    format!("{{raw {value}}}"),
                );
            } else {
                diag = diag.with_help("call a method instead, as in `{name.upper()}`");
            }
            self.error(diag);
            return None;
        }
        let raw = matches!(&tokens[0].kind, TokenKind::Ident(w) if w == "raw") && tokens.len() > 2;
        if raw {
            tokens.remove(0);
        }
        let expr = self.parse_code(tokens, |p| p.parse_expr());
        Some(Node::Expr { expr, raw, span })
    }

    /// Runs the Kiln parser on embedded tokens, requiring it to use them all.
    fn parse_code<T>(&mut self, tokens: Vec<Token>, f: impl FnOnce(&mut Parser) -> T) -> T {
        let mut parser = Parser::new(tokens);
        let result = parser.with_newlines(false, |p| {
            let result = f(p);
            if !p.at(&TokenKind::Eof) {
                let found = p.peek().to_string();
                let span = p.span();
                p.error(Diagnostic::error(
                    codes::EXPECTED_TOKEN,
                    format!("expected the end of the value, found {found}"),
                    span,
                ));
            }
            result
        });
        self.diags.extend(parser.into_diagnostics());
        result
    }

    // ----- {% tags %} -----

    fn parse_tag(&mut self) -> Tag {
        let start = self.pos;
        let Some(mut tokens) = self.code_until_close(start, start + 2, "{%") else {
            return Tag::Error;
        };
        let span = Span::new(start, self.pos);
        // The tag must end with `%}`: drop the `%` before the closing brace.
        let n = tokens.len();
        let percent_ok = n >= 2
            && tokens[n - 2].kind == TokenKind::Percent
            && tokens[n - 2].span.end == tokens[n - 1].span.start;
        if percent_ok {
            tokens.remove(n - 2);
        } else {
            self.error(Diagnostic::error(
                codes::EXPECTED_TOKEN,
                "expected `%}` to end the tag",
                Span::new(self.pos - 1, self.pos),
            ));
        }
        if tokens.len() == 1 {
            self.error(Diagnostic::error(
                codes::UNKNOWN_GLAZE_TAG,
                "this tag is empty",
                span,
            ));
            return Tag::Error;
        }
        let head = tokens.remove(0);
        let word = match &head.kind {
            TokenKind::Ident(w) => w.clone(),
            TokenKind::If => "if".into(),
            TokenKind::Else => "else".into(),
            TokenKind::For => "for".into(),
            TokenKind::Match => "match".into(),
            TokenKind::Let => "let".into(),
            other => other.to_string().trim_matches('`').to_string(),
        };
        let only = |p: &mut Self, tokens: &[Token], tag: Tag| {
            if tokens.len() > 1 {
                p.error(Diagnostic::error(
                    codes::EXPECTED_TOKEN,
                    format!("`{{% {word} %}}` takes nothing else"),
                    tokens[0].span.to(tokens[tokens.len() - 2].span),
                ));
            }
            tag
        };
        match word.as_str() {
            "props" => {
                let tokens = wrap(tokens, head.span, TokenKind::LParen, TokenKind::RParen);
                Tag::Props(self.parse_code(tokens, |p| p.parse_params(true)))
            }
            "state" | "let" => {
                let decl = self.parse_code(tokens, |p| {
                    let name = p.expect_ident("a name");
                    let ty = p.eat(&TokenKind::Colon).then(|| p.parse_type());
                    p.expect(&TokenKind::Eq);
                    let value = p.parse_expr();
                    LocalDecl {
                        name,
                        ty,
                        value,
                        span,
                    }
                });
                if word == "state" {
                    Tag::State(decl)
                } else {
                    Tag::Let(decl)
                }
            }
            "layout" => self.parse_code(tokens, |p| {
                let path = p.parse_expr();
                let mut args = Vec::new();
                while !p.at(&TokenKind::Eof) {
                    let before = p.span();
                    let name = p.expect_ident("an argument name");
                    p.expect(&TokenKind::Colon);
                    let value = p.parse_expr();
                    args.push(Arg {
                        name: Some(name),
                        value,
                    });
                    p.eat(&TokenKind::Comma);
                    if p.span() == before {
                        p.bump();
                    }
                }
                Tag::Layout(Layout { path, args, span })
            }),
            "slot" => {
                let name = self.parse_code(tokens, |p| {
                    matches!(p.peek(), TokenKind::Ident(_)).then(|| p.expect_ident("a slot name"))
                });
                Tag::Slot(name)
            }
            "fill" => Tag::Fill(self.parse_code(tokens, |p| p.expect_ident("a slot name"))),
            "if" => Tag::If(self.parse_code(tokens, |p| p.parse_expr())),
            "else" if tokens[0].kind == TokenKind::If => {
                tokens.remove(0);
                Tag::Ending(Ending::ElseIf(
                    self.parse_code(tokens, |p| p.parse_expr()),
                    span,
                ))
            }
            "else" => only(self, &tokens, Tag::Ending(Ending::Else(span))),
            "for" => self.parse_code(tokens, |p| {
                let binding = p.expect_ident("a loop variable");
                p.expect(&TokenKind::In);
                let iter = p.parse_expr();
                let filter = p.eat(&TokenKind::Where).then(|| p.parse_expr());
                Tag::For {
                    binding,
                    iter,
                    filter,
                }
            }),
            "empty" => only(self, &tokens, Tag::Ending(Ending::Empty(span))),
            "match" => Tag::Match(self.parse_code(tokens, |p| p.parse_expr())),
            "case" => {
                let (pattern, guard) = self.parse_code(tokens, |p| {
                    let pattern = p.parse_pattern();
                    let guard = p.eat(&TokenKind::If).then(|| p.parse_expr());
                    (pattern, guard)
                });
                Tag::Ending(Ending::Case(pattern, guard, span))
            }
            "end" => only(self, &tokens, Tag::Ending(Ending::EndBlock(span))),
            _ => self.foreign_tag(&word, tokens, span),
        }
    }

    /// Tags from other template languages, with the glaze way to write them.
    fn foreign_tag(&mut self, word: &str, tokens: Vec<Token>, span: Span) -> Tag {
        let lower = word.to_ascii_lowercase();
        if lower.starts_with("end") {
            self.error(
                Diagnostic::error(
                    codes::UNKNOWN_GLAZE_TAG,
                    format!("glazes close every block with `{{% end %}}`, not `{{% {word} %}}`"),
                    span,
                )
                .with_suggestion("write `{% end %}`", span, "{% end %}"),
            );
            return Tag::Ending(Ending::EndBlock(span));
        }
        if matches!(lower.as_str(), "elif" | "elseif" | "elsif") {
            let cond_span = tokens[0].span.to(tokens[tokens.len() - 1].span);
            let cond = self.src[cond_span.start as usize..cond_span.end as usize]
                .trim_end_matches('%')
                .trim()
                .to_string();
            let cond_expr = self.parse_code(tokens, |p| p.parse_expr());
            self.error(
                Diagnostic::error(
                    codes::UNKNOWN_GLAZE_TAG,
                    format!("write `else if`, not `{word}`"),
                    span,
                )
                .with_suggestion(
                    "write `else if`",
                    span,
                    format!("{{% else if {cond} %}}"),
                ),
            );
            return Tag::Ending(Ending::ElseIf(cond_expr, span));
        }
        let help = match lower.as_str() {
            "extends" => "use `{% layout \"base.glaze\" %}` to wrap this glaze in a layout",
            "include" | "render" | "partial" => {
                "use a component as a tag: `<PostCard post={post} />` renders post_card.glaze"
            }
            "block" | "yield" | "section" => {
                "a layout marks where the page goes with `{% slot %}`, and a page fills a named slot with `{% fill name %}`"
            }
            "set" | "assign" | "var" => "use `{% let name = value %}`",
            "each" | "foreach" => "use `{% for item in items %}`",
            "unless" => "use `{% if not condition %}`",
            "switch" | "when" => "use `{% match value %}` with `{% case pattern %}`",
            _ => "the tags are `props`, `state`, `layout`, `slot`, `fill`, `let`, `if`, `else`, `for`, `empty`, `match`, `case` and `end`",
        };
        self.error(
            Diagnostic::error(
                codes::UNKNOWN_GLAZE_TAG,
                format!("unknown tag `{{% {word} %}}`"),
                span,
            )
            .with_help(help),
        );
        Tag::Error
    }

    /// Adds a non-ending tag to `nodes`, parsing a block's body if it opens
    /// one.
    fn place_tag(&mut self, tag: Tag, start: usize, nodes: &mut Vec<Node>) {
        let span = Span::new(start, self.pos);
        let top_level = self.open.is_empty();
        let misplaced = |p: &mut Self, what: &str| {
            p.error(Diagnostic::error(
                codes::MISPLACED_DECLARATION,
                format!("`{{% {what} %}}` must be at the top level, outside elements and blocks"),
                span,
            ));
        };
        match tag {
            Tag::Props(params) => {
                if !top_level {
                    misplaced(self, "props");
                } else if self.glaze.props.is_some() {
                    self.error(Diagnostic::error(
                        codes::MISPLACED_DECLARATION,
                        "a glaze has one `{% props %}`; list every prop in it",
                        span,
                    ));
                } else {
                    self.glaze.props = Some(Props { params, span });
                }
            }
            Tag::State(decl) => {
                if top_level {
                    self.glaze.states.push(decl);
                } else {
                    misplaced(self, "state");
                }
            }
            Tag::Layout(layout) => {
                if !top_level {
                    misplaced(self, "layout");
                } else if self.glaze.layout.is_some() {
                    self.error(Diagnostic::error(
                        codes::MISPLACED_DECLARATION,
                        "a glaze has at most one `{% layout %}`",
                        span,
                    ));
                } else {
                    self.glaze.layout = Some(layout);
                }
            }
            Tag::Let(decl) => nodes.push(Node::Let(decl)),
            Tag::Slot(name) => nodes.push(Node::Slot { name, span }),
            Tag::Fill(name) => {
                let (body, end) = self.block_body(Open::Fill);
                let span = match end {
                    Ending::EndBlock(s) => span.to(s),
                    other => {
                        self.unclosed("`{% fill %}`".into(), span, "{% end %}", other);
                        span
                    }
                };
                nodes.push(Node::Fill { name, body, span });
            }
            Tag::If(cond) => nodes.push(self.parse_if(cond, span)),
            Tag::For {
                binding,
                iter,
                filter,
            } => nodes.push(self.parse_for(binding, iter, filter, span)),
            Tag::Match(expr) => nodes.push(self.parse_match(expr, span)),
            Tag::Ending(_) | Tag::Error => {}
        }
    }

    /// Parses a block body, with `open` pushed while it is parsed.
    fn block_body(&mut self, open: Open) -> (Vec<Node>, Ending) {
        self.open.push(open);
        let result = self.parse_nodes();
        self.open.pop();
        result
    }

    /// Reports `what`, opened at `span`, as never closed. An ending that
    /// belongs further out is passed up.
    fn unclosed(&mut self, what: String, span: Span, fix: &str, end: Ending) {
        let mut diag = Diagnostic::error(
            codes::UNCLOSED_GLAZE_TAG,
            format!("{what} is never closed"),
            span,
        );
        let at = match &end {
            Ending::Eof => self.src.len(),
            other => end_span(other).start as usize,
        };
        diag = diag.with_suggestion(format!("add `{fix}`"), Span::new(at, at), fix.to_string());
        self.error(diag);
        if !matches!(end, Ending::Eof) {
            self.pending = Some(end);
        }
    }

    fn parse_if(&mut self, cond: Expr, span: Span) -> Node {
        let mut branches = Vec::new();
        let mut otherwise: Option<Vec<Node>> = None;
        let mut cond = Some(cond);
        loop {
            let (body, end) = self.block_body(Open::If);
            match cond.take() {
                Some(c) => branches.push(Branch { cond: c, body }),
                None => otherwise.get_or_insert_with(Vec::new).extend(body),
            }
            match end {
                Ending::ElseIf(_, s) | Ending::Else(s) if otherwise.is_some() => {
                    self.error(Diagnostic::error(
                        codes::STRAY_CLOSING_TAG,
                        "`{% else %}` must be the last branch of `{% if %}`",
                        s,
                    ));
                }
                Ending::ElseIf(c, _) => cond = Some(c),
                Ending::Else(_) => otherwise = Some(Vec::new()),
                Ending::EndBlock(s) => return self.if_node(branches, otherwise, span.to(s)),
                other => {
                    self.unclosed("`{% if %}`".into(), span, "{% end %}", other);
                    return self.if_node(branches, otherwise, span);
                }
            }
        }
    }

    fn if_node(&self, branches: Vec<Branch>, otherwise: Option<Vec<Node>>, span: Span) -> Node {
        Node::If {
            branches,
            otherwise,
            span,
        }
    }

    fn parse_for(&mut self, binding: Ident, iter: Expr, filter: Option<Expr>, span: Span) -> Node {
        let (body, end) = self.block_body(Open::For);
        let (empty, end) = match end {
            Ending::Empty(_) => {
                let (nodes, end) = self.block_body(Open::For);
                (Some(nodes), end)
            }
            other => (None, other),
        };
        let span = match end {
            Ending::EndBlock(s) => span.to(s),
            other => {
                self.unclosed("`{% for %}`".into(), span, "{% end %}", other);
                span
            }
        };
        Node::For {
            binding,
            iter,
            filter,
            body,
            empty,
            span,
        }
    }

    fn parse_match(&mut self, expr: Expr, span: Span) -> Node {
        let mut cases = Vec::new();
        let (before, mut end) = self.block_body(Open::Match);
        if let Some(text) = before.iter().find(|n| !is_blank(n)) {
            self.error(Diagnostic::error(
                codes::EXPECTED_TOKEN,
                "expected `{% case pattern %}` before any content in `{% match %}`",
                node_span(text),
            ));
        }
        loop {
            match end {
                Ending::Case(pattern, guard, case_span) => {
                    let (body, next) = self.block_body(Open::Match);
                    cases.push(Case {
                        pattern,
                        guard,
                        body,
                        span: case_span,
                    });
                    end = next;
                }
                Ending::EndBlock(s) => {
                    return Node::Match {
                        expr,
                        cases,
                        span: span.to(s),
                    }
                }
                other => {
                    self.unclosed("`{% match %}`".into(), span, "{% end %}", other);
                    return Node::Match { expr, cases, span };
                }
            }
        }
    }

    // ----- elements -----

    /// The tag name starting at byte `at`, or an empty string.
    fn tag_name(&self, at: usize) -> String {
        self.src[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
            .collect()
    }

    fn parse_element(&mut self) -> Element {
        let start = self.pos;
        let name_start = start + 1;
        let name = self.tag_name(name_start);
        self.pos = name_start + name.len();
        let name = Ident {
            span: Span::new(name_start, self.pos),
            name,
        };
        let mut attrs = Vec::new();
        let mut self_closing = false;
        loop {
            self.skip_whitespace();
            match self.peek() {
                None => {
                    self.error(Diagnostic::error(
                        codes::EXPECTED_TOKEN,
                        format!("expected `>` to end the `<{}>` tag", name.name),
                        name.span,
                    ));
                    break;
                }
                Some('>') => {
                    self.bump();
                    break;
                }
                Some('/') if self.peek_at(1) == Some('>') => {
                    self.pos += 2;
                    self_closing = true;
                    break;
                }
                Some(c) if is_attr_name_char(c) => {
                    if let Some(attr) = self.parse_attr() {
                        attrs.push(attr);
                    }
                }
                Some(c) => {
                    let at = self.pos;
                    self.bump();
                    self.error(Diagnostic::error(
                        codes::EXPECTED_TOKEN,
                        format!("unexpected `{c}` in the `<{}>` tag", name.name),
                        Span::new(at, self.pos),
                    ));
                }
            }
        }
        let open_span = Span::new(start, self.pos);
        let lower = name.name.to_ascii_lowercase();
        if self_closing || VOID_ELEMENTS.contains(&lower.as_str()) {
            return Element {
                name,
                attrs,
                children: Vec::new(),
                span: open_span,
            };
        }
        if RAW_TEXT_ELEMENTS.contains(&lower.as_str()) {
            return self.raw_text_element(name, attrs, open_span);
        }
        self.open.push(Open::Element(name.name.clone()));
        let (children, end) = self.parse_nodes();
        self.open.pop();
        let span = match end {
            Ending::CloseTag(close, s) if close.name == name.name => open_span.to(s),
            Ending::CloseTag(close, s)
                if !self
                    .open
                    .iter()
                    .any(|o| matches!(o, Open::Element(n) if *n == close.name)) =>
            {
                // Only `parse_nodes` passes up endings that fit something
                // open, so this closing tag was meant for this element.
                self.error(
                    Diagnostic::error(
                        codes::MISMATCHED_CLOSING_TAG,
                        format!("`</{}>` does not close `<{}>`", close.name, name.name),
                        s,
                    )
                    .with_suggestion(
                        format!("write `</{}>`", name.name),
                        close.span,
                        name.name.clone(),
                    ),
                );
                open_span.to(s)
            }
            other => {
                let fix = format!("</{}>", name.name);
                self.unclosed(format!("`<{}>`", name.name), open_span, &fix, other);
                open_span
            }
        };
        Element {
            name,
            attrs,
            children,
            span,
        }
    }

    fn raw_text_element(&mut self, name: Ident, attrs: Vec<Attr>, open_span: Span) -> Element {
        let close = format!("</{}", name.name.to_ascii_lowercase());
        let body_start = self.pos;
        let lower_rest = self.rest().to_ascii_lowercase();
        let (body_end, span) = match lower_rest.find(&close) {
            Some(i) => {
                let body_end = self.pos + i;
                let end = self.src[body_end..]
                    .find('>')
                    .map_or(self.src.len(), |j| body_end + j + 1);
                self.pos = end;
                (body_end, open_span.to(Span::new(end, end)))
            }
            None => {
                self.pos = self.src.len();
                let fix = format!("</{}>", name.name);
                self.unclosed(format!("`<{}>`", name.name), open_span, &fix, Ending::Eof);
                (self.src.len(), open_span)
            }
        };
        let children = if body_start < body_end {
            vec![Node::Text {
                text: self.src[body_start..body_end].to_string(),
                span: Span::new(body_start, body_end),
            }]
        } else {
            Vec::new()
        };
        Element {
            name,
            attrs,
            children,
            span,
        }
    }

    fn parse_attr(&mut self) -> Option<Attr> {
        let start = self.pos;
        while self.peek().is_some_and(is_attr_name_char) {
            self.bump();
        }
        let mut name = Ident {
            name: self.src[start..self.pos].to_string(),
            span: Span::new(start, self.pos),
        };
        if let Some(fixed) = foreign_attr_name(&name.name) {
            self.error(
                Diagnostic::error(
                    codes::FOREIGN_ATTRIBUTE,
                    format!("write `{fixed}`, not `{}`", name.name),
                    name.span,
                )
                .with_suggestion(
                    format!("write `{fixed}`"),
                    name.span,
                    fixed.clone(),
                ),
            );
            name.name = fixed;
        }
        let save = self.pos;
        self.skip_whitespace();
        if self.peek() != Some('=') {
            self.pos = save;
            return Some(Attr {
                span: name.span,
                name,
                value: AttrValue::Flag,
            });
        }
        self.bump();
        self.skip_whitespace();
        let value = match self.peek() {
            Some(q @ ('"' | '\'')) => {
                self.bump();
                AttrValue::Text(self.attr_text(q))
            }
            Some('{') => {
                let open = self.pos;
                let tokens = self.code_until_close(open, open + 1, "{")?;
                if name.name.starts_with("on:") {
                    let close = Span::new(self.pos - 1, self.pos);
                    let mut tokens = tokens;
                    tokens.pop();
                    tokens.insert(
                        0,
                        Token {
                            kind: TokenKind::LBrace,
                            span: Span::new(open, open + 1),
                        },
                    );
                    tokens.push(Token {
                        kind: TokenKind::RBrace,
                        span: close,
                    });
                    tokens.push(Token {
                        kind: TokenKind::Eof,
                        span: close,
                    });
                    let mut parser = Parser::new(tokens);
                    let block = parser.parse_block();
                    self.diags.extend(parser.into_diagnostics());
                    AttrValue::Handler(block)
                } else {
                    AttrValue::Expr(self.parse_code(tokens, |p| p.parse_expr()))
                }
            }
            _ => {
                let value_start = self.pos;
                while self
                    .peek()
                    .is_some_and(|c| !c.is_whitespace() && c != '>' && !self.starts_with("/>"))
                {
                    self.bump();
                }
                AttrValue::Text(vec![AttrPart::Text(
                    self.src[value_start..self.pos].to_string(),
                )])
            }
        };
        Some(Attr {
            span: Span::new(start, self.pos),
            name,
            value,
        })
    }

    /// The text of a quoted attribute value, after the opening quote, with
    /// `{value}` parts.
    fn attr_text(&mut self, quote: char) -> Vec<AttrPart> {
        let start = self.pos - 1;
        let mut parts = Vec::new();
        let mut text = String::new();
        loop {
            match self.peek() {
                None => {
                    self.error(Diagnostic::error(
                        codes::UNTERMINATED_STRING,
                        "this attribute value is never closed",
                        Span::new(start, start + 1),
                    ));
                    break;
                }
                Some(c) if c == quote => {
                    self.bump();
                    break;
                }
                Some('{') => {
                    if !text.is_empty() {
                        parts.push(AttrPart::Text(std::mem::take(&mut text)));
                    }
                    let open = self.pos;
                    match self.code_until_close(open, open + 1, "{") {
                        Some(tokens) => {
                            parts.push(AttrPart::Expr(self.parse_code(tokens, |p| p.parse_expr())))
                        }
                        None => break,
                    }
                }
                Some(c) => {
                    self.bump();
                    text.push(c);
                }
            }
        }
        if !text.is_empty() {
            parts.push(AttrPart::Text(text));
        }
        parts
    }
}

fn is_attr_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '@' | '.')
}

/// The glaze attribute for a React-style one: `className` is `class`,
/// `onClick` is `on:click`.
fn foreign_attr_name(name: &str) -> Option<String> {
    match name {
        "className" => return Some("class".into()),
        "htmlFor" => return Some("for".into()),
        _ => {}
    }
    if let Some(event) = name.strip_prefix('@') {
        return Some(format!("on:{event}"));
    }
    let event = name.strip_prefix("on")?;
    event
        .starts_with(|c: char| c.is_ascii_uppercase())
        .then(|| format!("on:{}", event.to_ascii_lowercase()))
}

/// `safe` in `{x | safe}`, when the value ends in a filter.
fn foreign_filter(inner: &str) -> Option<&str> {
    let (value, filter) = inner.rsplit_once('|')?;
    let filter = filter.trim();
    let is_word = !filter.is_empty()
        && filter
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_');
    // `a || b` is a boolean habit, handled by the expression parser.
    (is_word && !value.ends_with('|')).then_some(filter)
}

/// Wraps tag tokens (which end with `Eof`) in `open ... close`.
fn wrap(mut tokens: Vec<Token>, span: Span, open: TokenKind, close: TokenKind) -> Vec<Token> {
    let eof = tokens.pop().expect("tokens end with Eof");
    tokens.insert(0, Token { kind: open, span });
    tokens.push(Token {
        kind: close,
        span: eof.span,
    });
    tokens.push(eof);
    tokens
}

fn end_span(end: &Ending) -> Span {
    match end {
        Ending::Eof => Span::default(),
        Ending::CloseTag(_, s)
        | Ending::EndBlock(s)
        | Ending::ElseIf(_, s)
        | Ending::Else(s)
        | Ending::Empty(s)
        | Ending::Case(_, _, s) => *s,
    }
}

fn is_blank(node: &Node) -> bool {
    matches!(node, Node::Text { text, .. } if text.trim().is_empty())
}

fn node_span(node: &Node) -> Span {
    match node {
        Node::Text { span, .. }
        | Node::Expr { span, .. }
        | Node::If { span, .. }
        | Node::For { span, .. }
        | Node::Match { span, .. }
        | Node::Slot { span, .. }
        | Node::Fill { span, .. } => *span,
        Node::Element(e) => e.span,
        Node::Let(d) => d.span,
    }
}
