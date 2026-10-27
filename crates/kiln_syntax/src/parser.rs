//! A recursive-descent parser with error recovery.
//!
//! The parser never stops at the first error: it records a diagnostic,
//! skips to a point where parsing can resume (the next line, or the next
//! declaration), and carries on, so `kiln check` reports every syntax error
//! in one run.
//!
//! Newlines end statements. Inside `(...)`, `[...]` and record literals
//! they are ignored, so long argument lists can span lines; inside blocks
//! they are significant.

use crate::ast::*;
use crate::token::{StrPart, Token, TokenKind};
use kiln_diagnostics::{codes, Diagnostic, Span};

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
    diags: Vec<Diagnostic>,
    /// A stack of modes: `true` when newlines end statements.
    newline_modes: Vec<bool>,
    /// Set by `parse_stmt` when the statement it returned is a
    /// `let pattern = value else { ... }`, which `parse_block` completes
    /// with the statements after it.
    let_else: bool,
}

/// Binding power of infix operators, lowest first.
const PREC_OR: u8 = 1;
const PREC_AND: u8 = 2;
const PREC_NOT: u8 = 3;
const PREC_COMPARE: u8 = 4;
const PREC_COALESCE: u8 = 5;
const PREC_RANGE: u8 = 6;
const PREC_ADD: u8 = 7;
const PREC_MUL: u8 = 8;

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Parser {
            tokens,
            pos: 0,
            diags: Vec::new(),
            newline_modes: vec![true],
            let_else: false,
        }
    }

    pub fn into_diagnostics(self) -> Vec<Diagnostic> {
        self.diags
    }

    // ----- token access -----

    fn newlines_significant(&self) -> bool {
        *self.newline_modes.last().unwrap_or(&true)
    }

    fn skip_insignificant_newlines(&mut self) {
        if !self.newlines_significant() {
            self.skip_newlines();
        }
    }

    fn skip_newlines(&mut self) {
        while self.tokens[self.pos].kind == TokenKind::Newline {
            self.pos += 1;
        }
    }

    fn current(&mut self) -> &Token {
        self.skip_insignificant_newlines();
        &self.tokens[self.pos]
    }

    pub(crate) fn peek(&mut self) -> &TokenKind {
        &self.current().kind
    }

    pub(crate) fn span(&mut self) -> Span {
        self.current().span
    }

    /// The kind of the `n`th token after the current one, ignoring newlines.
    fn lookahead(&self, n: usize) -> &TokenKind {
        let mut seen = 0;
        let mut i = self.pos;
        loop {
            let token = &self.tokens[i];
            if token.kind == TokenKind::Eof {
                return &token.kind;
            }
            if token.kind != TokenKind::Newline {
                if seen == n {
                    return &token.kind;
                }
                seen += 1;
            }
            i += 1;
        }
    }

    pub(crate) fn previous_span(&self) -> Span {
        self.tokens[self.pos.saturating_sub(1)].span
    }

    pub(crate) fn bump(&mut self) -> Token {
        self.skip_insignificant_newlines();
        let token = self.tokens[self.pos].clone();
        if token.kind != TokenKind::Eof {
            self.pos += 1;
        }
        token
    }

    pub(crate) fn at(&mut self, kind: &TokenKind) -> bool {
        self.peek() == kind
    }

    pub(crate) fn eat(&mut self, kind: &TokenKind) -> bool {
        if self.at(kind) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn at_ident(&mut self, word: &str) -> bool {
        matches!(self.peek(), TokenKind::Ident(name) if name == word)
    }

    pub(crate) fn error(&mut self, diag: Diagnostic) {
        // One error per location: later ones are usually knock-on effects.
        if self
            .diags
            .last()
            .is_some_and(|last| last.span.start == diag.span.start)
        {
            return;
        }
        self.diags.push(diag);
    }

    fn error_expected(&mut self, what: &str) {
        let token = self.current().clone();
        self.error(Diagnostic::error(
            codes::EXPECTED_TOKEN,
            format!("expected {what}, found {}", token.kind),
            token.span,
        ));
    }

    pub(crate) fn expect(&mut self, kind: &TokenKind) -> Option<Span> {
        if self.at(kind) {
            Some(self.bump().span)
        } else {
            self.error_expected(&kind.to_string());
            None
        }
    }

    pub(crate) fn expect_ident(&mut self, what: &str) -> Ident {
        if let TokenKind::Ident(name) = self.peek().clone() {
            let span = self.bump().span;
            Ident { name, span }
        } else {
            let span = self.span();
            self.error_expected(what);
            Ident {
                name: String::new(),
                span,
            }
        }
    }

    /// A field or method name: an identifier, or `where`, which is a
    /// keyword only in `for` loops, so that a query can say
    /// `.where(...)`.
    pub(crate) fn expect_member(&mut self, what: &str) -> Ident {
        if *self.peek() == TokenKind::Where {
            let span = self.bump().span;
            return Ident {
                name: "where".into(),
                span,
            };
        }
        self.expect_ident(what)
    }

    pub(crate) fn with_newlines<T>(
        &mut self,
        significant: bool,
        f: impl FnOnce(&mut Self) -> T,
    ) -> T {
        self.newline_modes.push(significant);
        let result = f(self);
        self.newline_modes.pop();
        result
    }

    /// Parses `open item (, item)* close`, allowing a trailing comma and
    /// newlines anywhere.
    fn delimited<T>(
        &mut self,
        open: TokenKind,
        close: TokenKind,
        mut item: impl FnMut(&mut Self) -> T,
    ) -> Vec<T> {
        let mut items = Vec::new();
        self.with_newlines(false, |p| {
            p.expect(&open);
            while !p.at(&close) && !p.at(&TokenKind::Eof) {
                let before = p.pos;
                items.push(item(p));
                if !p.eat(&TokenKind::Comma) {
                    break;
                }
                if p.pos == before {
                    p.bump();
                }
            }
            p.expect(&close);
        });
        items
    }

    /// Accepts the end of a statement or declaration: a newline, `}` or the
    /// end of the file. A `;` is reported once and skipped.
    fn expect_line_end(&mut self) -> bool {
        match self.peek() {
            TokenKind::Newline => {
                self.bump();
                true
            }
            TokenKind::RBrace | TokenKind::Eof => true,
            TokenKind::Semi => {
                let span = self.bump().span;
                self.error(
                    Diagnostic::error(
                        codes::NO_SEMICOLONS,
                        "semicolons are not used in Kiln",
                        span,
                    )
                    .with_suggestion("remove the `;`", span, ""),
                );
                if self.at(&TokenKind::Newline) {
                    self.bump();
                }
                true
            }
            _ => {
                self.error_expected("end of line");
                false
            }
        }
    }

    /// Skips to the end of the current line, keeping brackets balanced.
    fn recover_to_line_end(&mut self) {
        let mut depth = 0usize;
        loop {
            match self.tokens[self.pos].kind {
                TokenKind::Eof => return,
                TokenKind::Newline if depth == 0 => {
                    self.pos += 1;
                    return;
                }
                TokenKind::RBrace if depth == 0 => return,
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    depth = depth.saturating_sub(1)
                }
                _ => {}
            }
            self.pos += 1;
        }
    }

    /// Skips to the next line that starts a declaration.
    fn recover_to_item(&mut self) {
        if self.tokens[self.pos].kind == TokenKind::Eof {
            return;
        }
        self.pos += 1;
        loop {
            let at_line_start = self.tokens[self.pos - 1].kind == TokenKind::Newline;
            match &self.tokens[self.pos].kind {
                TokenKind::Eof => return,
                TokenKind::Fn
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Impl
                | TokenKind::Import
                | TokenKind::Const
                | TokenKind::Pub
                    if at_line_start =>
                {
                    return
                }
                _ => self.pos += 1,
            }
        }
    }

    // ----- declarations -----

    pub fn parse_module(&mut self) -> Module {
        let mut items = Vec::new();
        loop {
            self.skip_newlines();
            if self.at(&TokenKind::Eof) {
                break;
            }
            match self.parse_item() {
                Some(item) => {
                    items.push(item);
                    if !self.expect_line_end() {
                        self.recover_to_item();
                    }
                }
                None => self.recover_to_item(),
            }
        }
        Module { items }
    }

    fn parse_item(&mut self) -> Option<Item> {
        let start = self.span();
        let is_pub = self.eat(&TokenKind::Pub);
        match self.peek().clone() {
            TokenKind::Import if !is_pub => Some(Item::Import(self.parse_import())),
            TokenKind::Fn => {
                self.bump();
                Some(Item::Fn(self.parse_fn(is_pub, false, start)))
            }
            TokenKind::Ident(word) if word == "server" && *self.lookahead(1) == TokenKind::Fn => {
                self.bump();
                self.bump();
                Some(Item::Fn(self.parse_fn(is_pub, true, start)))
            }
            TokenKind::Ident(word) if matches!(word.as_str(), "function" | "func" | "def") => {
                let span = self.bump().span;
                self.error(
                    Diagnostic::error(
                        codes::WRONG_KEYWORD,
                        format!("`{word}` is not a Kiln keyword; functions are declared with `fn`"),
                        span,
                    )
                    .with_suggestion("use `fn`", span, "fn"),
                );
                Some(Item::Fn(self.parse_fn(is_pub, false, start)))
            }
            TokenKind::Struct => Some(Item::Struct(self.parse_struct(is_pub, start))),
            TokenKind::Enum => Some(Item::Enum(self.parse_enum(is_pub, start))),
            TokenKind::Impl if !is_pub => Some(Item::Impl(self.parse_impl())),
            TokenKind::Const => Some(Item::Const(self.parse_const(is_pub, start))),
            other => {
                let span = self.span();
                let mut diag = Diagnostic::error(
                    codes::EXPECTED_ITEM,
                    format!("expected a declaration, found {other}"),
                    span,
                );
                if matches!(
                    other,
                    TokenKind::Let | TokenKind::Var | TokenKind::Ident(_) | TokenKind::Return
                ) {
                    diag = diag.with_help(
                        "statements belong inside a function such as `fn main() { ... }`",
                    );
                }
                self.error(diag);
                None
            }
        }
    }

    fn parse_import(&mut self) -> Import {
        let start = self.bump().span;
        let mut paths = Vec::new();
        loop {
            let mut path = vec![self.expect_ident("a module name")];
            while self.eat(&TokenKind::Dot) {
                path.push(self.expect_ident("a module name"));
            }
            paths.push(path);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        Import {
            paths,
            span: start.to(self.previous_span()),
        }
    }

    fn parse_generic_params(&mut self) -> Vec<Ident> {
        if self.at(&TokenKind::Lt) {
            self.delimited(TokenKind::Lt, TokenKind::Gt, |p| {
                p.expect_ident("a type parameter")
            })
        } else {
            Vec::new()
        }
    }

    /// Parses a function after its `fn` keyword.
    fn parse_fn(&mut self, is_pub: bool, is_server: bool, start: Span) -> FnDecl {
        let name = self.expect_member("a function name");
        let generics = self.parse_generic_params();
        let params = self.parse_params(true);
        let ret = if self.eat(&TokenKind::Arrow) {
            Some(self.parse_type())
        } else {
            None
        };
        let throws = self.parse_throws();
        let body = self.parse_block();
        FnDecl {
            is_pub,
            is_server,
            name,
            generics,
            params,
            ret,
            throws,
            body,
            span: start.to(self.previous_span()),
        }
    }

    fn parse_throws(&mut self) -> Throws {
        if !self.eat(&TokenKind::Bang) {
            return Throws::Never;
        }
        if matches!(self.peek(), TokenKind::Ident(_)) {
            Throws::Type(Box::new(self.parse_type()))
        } else {
            Throws::Default
        }
    }

    pub(crate) fn parse_params(&mut self, types_required: bool) -> Vec<Param> {
        self.delimited(TokenKind::LParen, TokenKind::RParen, |p| {
            let name = p.expect_ident("a parameter name");
            let ty = if p.eat(&TokenKind::Colon) {
                Some(p.parse_type())
            } else {
                if types_required && name.name != "self" && !name.name.is_empty() {
                    let span = p.span();
                    p.error(
                        Diagnostic::error(
                            codes::EXPECTED_TYPE,
                            format!("parameter `{}` needs a type", name.name),
                            span,
                        )
                        .with_help("function parameters are always typed, as in `id: Int`"),
                    );
                }
                None
            };
            let default = if p.eat(&TokenKind::Eq) {
                Some(p.parse_expr())
            } else {
                None
            };
            Param {
                span: name.span.to(p.previous_span()),
                name,
                ty,
                default,
            }
        })
    }

    /// Parses `{ member \n member ... }` where members are separated by
    /// newlines or commas.
    fn member_list<T>(&mut self, mut member: impl FnMut(&mut Self) -> Option<T>) -> Vec<T> {
        let mut members = Vec::new();
        self.with_newlines(true, |p| {
            p.expect(&TokenKind::LBrace);
            loop {
                p.skip_newlines();
                if p.at(&TokenKind::RBrace) || p.at(&TokenKind::Eof) {
                    break;
                }
                let before = p.pos;
                match member(p) {
                    Some(m) => {
                        members.push(m);
                        if !p.eat(&TokenKind::Comma) && !p.expect_line_end() {
                            p.recover_to_line_end();
                        }
                    }
                    None => p.recover_to_line_end(),
                }
                if p.pos == before {
                    p.bump();
                }
            }
            p.expect(&TokenKind::RBrace);
        });
        members
    }

    fn parse_struct(&mut self, is_pub: bool, start: Span) -> StructDecl {
        self.bump();
        let name = self.expect_ident("a struct name");
        let generics = self.parse_generic_params();
        let fields = self.member_list(|p| {
            let name = p.expect_ident("a field name");
            p.expect(&TokenKind::Colon)?;
            let ty = p.parse_type();
            let default = if p.eat(&TokenKind::Eq) {
                Some(p.parse_expr())
            } else {
                None
            };
            Some(Field {
                span: name.span.to(p.previous_span()),
                name,
                ty,
                default,
            })
        });
        StructDecl {
            is_pub,
            name,
            generics,
            fields,
            span: start.to(self.previous_span()),
        }
    }

    fn parse_enum(&mut self, is_pub: bool, start: Span) -> EnumDecl {
        self.bump();
        let name = self.expect_ident("an enum name");
        let generics = self.parse_generic_params();
        let variants = self.member_list(|p| {
            let name = p.expect_ident("a variant name");
            let fields = if p.at(&TokenKind::LParen) {
                p.delimited(TokenKind::LParen, TokenKind::RParen, |p| {
                    let field_name = if matches!(p.peek(), TokenKind::Ident(_))
                        && *p.lookahead(1) == TokenKind::Colon
                    {
                        let name = p.expect_ident("a field name");
                        p.bump();
                        Some(name)
                    } else {
                        None
                    };
                    VariantField {
                        name: field_name,
                        ty: p.parse_type(),
                    }
                })
            } else {
                Vec::new()
            };
            Some(Variant {
                span: name.span.to(p.previous_span()),
                name,
                fields,
            })
        });
        EnumDecl {
            is_pub,
            name,
            generics,
            variants,
            span: start.to(self.previous_span()),
        }
    }

    fn parse_impl(&mut self) -> ImplDecl {
        let start = self.bump().span;
        let target = self.parse_type();
        let methods = self.member_list(|p| {
            let start = p.span();
            let is_pub = p.eat(&TokenKind::Pub);
            if p.eat(&TokenKind::Fn) {
                Some(p.parse_fn(is_pub, false, start))
            } else {
                p.error_expected("`fn`");
                None
            }
        });
        ImplDecl {
            target,
            methods,
            span: start.to(self.previous_span()),
        }
    }

    fn parse_const(&mut self, is_pub: bool, start: Span) -> ConstDecl {
        self.bump();
        let name = self.expect_ident("a constant name");
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.parse_type())
        } else {
            None
        };
        self.expect(&TokenKind::Eq);
        let value = self.parse_expr();
        ConstDecl {
            is_pub,
            name,
            ty,
            value,
            span: start.to(self.previous_span()),
        }
    }

    // ----- types -----

    pub fn parse_type(&mut self) -> TypeExpr {
        let start = self.span();
        if self.eat(&TokenKind::Fn) {
            let params = self.delimited(TokenKind::LParen, TokenKind::RParen, |p| p.parse_type());
            let ret = if self.eat(&TokenKind::Arrow) {
                Some(Box::new(self.parse_type()))
            } else {
                None
            };
            let throws = self.parse_throws();
            return TypeExpr {
                kind: TypeKind::Fn {
                    params,
                    ret,
                    throws,
                },
                span: start.to(self.previous_span()),
            };
        }
        let kind = if self.at(&TokenKind::LBrace) {
            let fields = self.delimited(TokenKind::LBrace, TokenKind::RBrace, |p| {
                let name = p.expect_ident("a field name");
                p.expect(&TokenKind::Colon);
                (name, p.parse_type())
            });
            TypeKind::Record(fields)
        } else if matches!(self.peek(), TokenKind::Ident(_)) {
            let mut path = vec![self.expect_ident("a type name")];
            while self.at(&TokenKind::Dot) {
                self.bump();
                path.push(self.expect_ident("a type name"));
            }
            let args = if self.at(&TokenKind::Lt) {
                self.delimited(TokenKind::Lt, TokenKind::Gt, |p| p.parse_type())
            } else {
                Vec::new()
            };
            TypeKind::Named { path, args }
        } else {
            let found = self.peek().clone();
            self.error(Diagnostic::error(
                codes::EXPECTED_TYPE,
                format!("expected a type, found {found}"),
                start,
            ));
            return TypeExpr {
                kind: TypeKind::Error,
                span: start,
            };
        };
        let mut ty = TypeExpr {
            kind,
            span: start.to(self.previous_span()),
        };
        while self.at(&TokenKind::Question) {
            let end = self.bump().span;
            ty = TypeExpr {
                span: start.to(end),
                kind: TypeKind::Optional(Box::new(ty)),
            };
        }
        ty
    }

    // ----- statements -----

    pub fn parse_block(&mut self) -> Block {
        let start = self.span();
        let mut stmts = Vec::new();
        let mut let_elses = Vec::new();
        self.with_newlines(true, |p| {
            if p.expect(&TokenKind::LBrace).is_none() {
                return;
            }
            loop {
                p.skip_newlines();
                if p.at(&TokenKind::RBrace) || p.at(&TokenKind::Eof) {
                    break;
                }
                let before = p.pos;
                p.let_else = false;
                let stmt = p.parse_stmt();
                if p.let_else {
                    let_elses.push(stmts.len());
                }
                stmts.push(stmt);
                if !p.expect_line_end() {
                    p.recover_to_line_end();
                }
                if p.pos == before {
                    p.bump();
                }
            }
            p.expect(&TokenKind::RBrace);
        });
        // `let some(x) = value else { leave }` then the rest of the block
        // is `match value { some(x) => { rest }, _ => { leave } }`: the
        // names it binds are in scope for exactly the statements after it.
        for at in let_elses.into_iter().rev() {
            let rest: Vec<Stmt> = stmts.drain(at + 1..).collect();
            if let Some(Stmt::Expr(Expr {
                kind: ExprKind::Match { arms, .. },
                ..
            })) = stmts.last_mut()
            {
                let end = arms[1].body.span.end as usize;
                let rest_span = match rest.last() {
                    Some(last) => Span::new(end, stmt_span(last).end as usize),
                    None => Span::new(end, end),
                };
                arms[0].body = Expr {
                    kind: ExprKind::Block(Block {
                        stmts: rest,
                        span: rest_span,
                    }),
                    span: rest_span,
                };
            }
        }
        Block {
            stmts,
            span: start.to(self.previous_span()),
        }
    }

    /// `let some(x) = value else { ... }`, after `let`: a match whose
    /// first arm `parse_block` fills with the statements that follow.
    fn parse_let_else(&mut self, start: Span) -> Stmt {
        let pattern = self.parse_pattern();
        self.expect(&TokenKind::Eq);
        let value = self.parse_expr();
        let else_span = self.span();
        if self.expect(&TokenKind::Else).is_none() {
            return Stmt::Expr(Expr {
                kind: ExprKind::Error,
                span: start.to(self.previous_span()),
            });
        }
        let otherwise = self.parse_block();
        if !leaves(&otherwise) {
            let span = match otherwise.stmts.last() {
                Some(last) => stmt_span(last),
                None => else_span.to(otherwise.span),
            };
            self.error(
                Diagnostic::error(
                    codes::LET_ELSE_MUST_LEAVE,
                    "the `else` of a `let ... else` must leave: end it with `return`, `throw`, `break`, `continue` or `panic(...)`",
                    span,
                )
                .with_help("when the value does not match, there is nothing to bind the names to, so the code after it cannot run"),
            );
        }
        let span = start.to(self.previous_span());
        self.let_else = true;
        let wildcard = Pattern {
            kind: PatternKind::Wildcard,
            span: else_span,
        };
        let otherwise = Expr {
            span: otherwise.span,
            kind: ExprKind::Block(otherwise),
        };
        let empty = Span::new(span.end as usize, span.end as usize);
        Stmt::Expr(Expr {
            kind: ExprKind::Match {
                scrutinee: Box::new(value),
                arms: vec![
                    MatchArm {
                        span: pattern.span,
                        pattern,
                        guard: None,
                        body: Expr {
                            kind: ExprKind::Block(Block {
                                stmts: Vec::new(),
                                span: empty,
                            }),
                            span: empty,
                        },
                    },
                    MatchArm {
                        span: else_span.to(otherwise.span),
                        pattern: wildcard,
                        guard: None,
                        body: otherwise,
                    },
                ],
            },
            span,
        })
    }

    fn parse_stmt(&mut self) -> Stmt {
        let start = self.span();
        match self.peek().clone() {
            TokenKind::Let
                if matches!(self.lookahead(1), TokenKind::Dot)
                    || (matches!(self.lookahead(1), TokenKind::Ident(_))
                        && matches!(self.lookahead(2), TokenKind::LParen | TokenKind::Dot)) =>
            {
                self.bump();
                self.parse_let_else(start)
            }
            TokenKind::Let | TokenKind::Var => {
                let mut mutable = self.bump().kind == TokenKind::Var;
                if self.at_ident("mut") && matches!(self.lookahead(1), TokenKind::Ident(_)) {
                    let mut_span = self.bump().span;
                    let fix_span = start.to(mut_span);
                    self.error(
                        Diagnostic::error(
                            codes::WRONG_KEYWORD,
                            "`let mut` is not Kiln syntax; mutable variables use `var`",
                            fix_span,
                        )
                        .with_suggestion("use `var`", fix_span, "var"),
                    );
                    mutable = true;
                }
                let name = self.expect_ident("a variable name");
                let ty = if self.eat(&TokenKind::Colon) {
                    Some(self.parse_type())
                } else {
                    None
                };
                self.expect(&TokenKind::Eq);
                let value = self.parse_expr();
                Stmt::Let {
                    mutable,
                    name,
                    ty,
                    value,
                    span: start.to(self.previous_span()),
                }
            }
            TokenKind::Return => {
                self.bump();
                let value = if matches!(
                    self.peek(),
                    TokenKind::Newline | TokenKind::RBrace | TokenKind::Eof | TokenKind::Semi
                ) {
                    None
                } else {
                    Some(self.parse_expr())
                };
                Stmt::Return {
                    value,
                    span: start.to(self.previous_span()),
                }
            }
            TokenKind::Break => Stmt::Break(self.bump().span),
            TokenKind::Continue => Stmt::Continue(self.bump().span),
            TokenKind::For => {
                self.bump();
                let binding = self.expect_ident("a loop variable");
                self.expect(&TokenKind::In);
                let iter = self.parse_expr();
                let filter = if self.eat(&TokenKind::Where) {
                    Some(self.parse_expr())
                } else {
                    None
                };
                let body = self.parse_block();
                Stmt::For {
                    binding,
                    iter,
                    filter,
                    body,
                    span: start.to(self.previous_span()),
                }
            }
            TokenKind::While => {
                self.bump();
                let cond = self.parse_expr();
                let body = self.parse_block();
                Stmt::While {
                    cond,
                    body,
                    span: start.to(self.previous_span()),
                }
            }
            _ => {
                let expr = self.parse_expr();
                let op = match self.peek() {
                    TokenKind::Eq => AssignOp::Set,
                    TokenKind::PlusEq => AssignOp::Add,
                    TokenKind::MinusEq => AssignOp::Sub,
                    TokenKind::StarEq => AssignOp::Mul,
                    TokenKind::SlashEq => AssignOp::Div,
                    _ => return Stmt::Expr(expr),
                };
                self.bump();
                let value = self.parse_expr();
                Stmt::Assign {
                    target: expr,
                    op,
                    value,
                    span: start.to(self.previous_span()),
                }
            }
        }
    }

    // ----- expressions -----

    pub fn parse_expr(&mut self) -> Expr {
        let mut expr = self.parse_binary(PREC_OR);
        while self.at(&TokenKind::Catch) {
            self.bump();
            let binding = if matches!(self.peek(), TokenKind::Ident(_)) {
                Some(self.expect_ident("a name for the error"))
            } else {
                None
            };
            let handler = self.parse_block();
            expr = Expr {
                span: expr.span.to(handler.span),
                kind: ExprKind::Catch {
                    expr: Box::new(expr),
                    binding,
                    handler,
                },
            };
        }
        expr
    }

    fn infix_op(&mut self) -> Option<(BinaryOp, u8)> {
        let span = self.span();
        let (op, prec, foreign) = match self.peek() {
            TokenKind::Or => (BinaryOp::Or, PREC_OR, None),
            TokenKind::And => (BinaryOp::And, PREC_AND, None),
            TokenKind::EqEq => (BinaryOp::Eq, PREC_COMPARE, None),
            TokenKind::NotEq => (BinaryOp::Ne, PREC_COMPARE, None),
            TokenKind::Lt => (BinaryOp::Lt, PREC_COMPARE, None),
            TokenKind::Le => (BinaryOp::Le, PREC_COMPARE, None),
            TokenKind::Gt => (BinaryOp::Gt, PREC_COMPARE, None),
            TokenKind::Ge => (BinaryOp::Ge, PREC_COMPARE, None),
            TokenKind::QuestionQuestion => (BinaryOp::Coalesce, PREC_COALESCE, None),
            TokenKind::DotDot => (BinaryOp::Range, PREC_RANGE, None),
            TokenKind::Plus => (BinaryOp::Add, PREC_ADD, None),
            TokenKind::Minus => (BinaryOp::Sub, PREC_ADD, None),
            TokenKind::Star => (BinaryOp::Mul, PREC_MUL, None),
            TokenKind::Slash => (BinaryOp::Div, PREC_MUL, None),
            TokenKind::Percent => (BinaryOp::Rem, PREC_MUL, None),
            TokenKind::EqEqEq => (
                BinaryOp::Eq,
                PREC_COMPARE,
                Some((codes::NO_STRICT_EQUALITY, "==")),
            ),
            TokenKind::BangEqEq => (
                BinaryOp::Ne,
                PREC_COMPARE,
                Some((codes::NO_STRICT_EQUALITY, "!=")),
            ),
            TokenKind::AmpAmp => (
                BinaryOp::And,
                PREC_AND,
                Some((codes::NO_SYMBOL_LOGIC, "and")),
            ),
            TokenKind::PipePipe => (BinaryOp::Or, PREC_OR, Some((codes::NO_SYMBOL_LOGIC, "or"))),
            _ => return None,
        };
        if let Some((code, fix)) = foreign {
            let found = self.peek().to_string();
            self.error(
                Diagnostic::error(code, format!("{found} is not a Kiln operator"), span)
                    .with_suggestion(format!("use `{fix}`"), span, fix),
            );
        }
        Some((op, prec))
    }

    fn parse_binary(&mut self, min_prec: u8) -> Expr {
        let mut lhs = if self.at(&TokenKind::Not) && min_prec <= PREC_NOT {
            let start = self.bump().span;
            let expr = self.parse_binary(PREC_COMPARE);
            Expr {
                span: start.to(expr.span),
                kind: ExprKind::Unary {
                    op: UnaryOp::Not,
                    expr: Box::new(expr),
                },
            }
        } else {
            self.parse_unary()
        };
        loop {
            if self.at(&TokenKind::Is) && min_prec <= PREC_COMPARE {
                self.bump();
                let pattern = self.parse_pattern();
                lhs = Expr {
                    span: lhs.span.to(pattern.span),
                    kind: ExprKind::Is {
                        expr: Box::new(lhs),
                        pattern: Box::new(pattern),
                    },
                };
                continue;
            }
            let checkpoint = self.diags.len();
            let Some((op, prec)) = self.infix_op() else {
                break;
            };
            if prec < min_prec {
                self.diags.truncate(checkpoint);
                break;
            }
            self.bump();
            // An operator at the end of a line continues onto the next.
            self.skip_newlines();
            let rhs = self.parse_binary(prec + 1);
            lhs = Expr {
                span: lhs.span.to(rhs.span),
                kind: ExprKind::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
            };
        }
        lhs
    }

    fn parse_unary(&mut self) -> Expr {
        enum Prefix {
            Op(UnaryOp),
            Try,
            Throw,
        }
        let start = self.span();
        let prefix = match self.peek() {
            TokenKind::Minus => Prefix::Op(UnaryOp::Neg),
            TokenKind::Bang => {
                self.error(
                    Diagnostic::error(codes::NO_SYMBOL_LOGIC, "`!` is not a Kiln operator", start)
                        .with_suggestion("use `not`", start, "not "),
                );
                Prefix::Op(UnaryOp::Not)
            }
            TokenKind::Try => Prefix::Try,
            TokenKind::Throw => Prefix::Throw,
            _ => {
                let primary = self.parse_primary();
                return self.parse_postfix(primary);
            }
        };
        self.bump();
        let expr = Box::new(self.parse_unary());
        let span = start.to(expr.span);
        let kind = match prefix {
            Prefix::Op(op) => ExprKind::Unary { op, expr },
            Prefix::Try => ExprKind::Try(expr),
            Prefix::Throw => ExprKind::Throw(expr),
        };
        Expr { kind, span }
    }

    /// Whether the next line starts a match arm on a variant, such as
    /// `.Accent => ...` or `.Custom(name) if ok =>`, rather than
    /// continuing a method chain.
    fn at_variant_arm(&self) -> bool {
        // Read raw tokens: the arm's `=>` or `if` must be on the same line.
        let tokens = &self.tokens;
        let mut i = self.pos;
        while tokens[i].kind == TokenKind::Newline {
            i += 1;
        }
        if tokens[i].kind != TokenKind::Dot || !matches!(tokens[i + 1].kind, TokenKind::Ident(_)) {
            return false;
        }
        i += 2;
        if tokens[i].kind == TokenKind::LParen {
            let mut depth = 0;
            loop {
                match tokens[i].kind {
                    TokenKind::LParen => depth += 1,
                    TokenKind::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            i += 1;
                            break;
                        }
                    }
                    TokenKind::Eof => return false,
                    _ => {}
                }
                i += 1;
            }
        }
        matches!(tokens[i].kind, TokenKind::FatArrow | TokenKind::If)
    }

    fn parse_postfix(&mut self, mut expr: Expr) -> Expr {
        loop {
            // A line starting with `.` continues a method chain.
            if self.newlines_significant()
                && self.tokens[self.pos].kind == TokenKind::Newline
                && *self.lookahead(0) == TokenKind::Dot
                && !self.at_variant_arm()
            {
                self.skip_newlines();
            }
            match self.peek() {
                TokenKind::Dot => {
                    self.bump();
                    let name = self.expect_member("a field or method name");
                    expr = Expr {
                        span: expr.span.to(name.span),
                        kind: ExprKind::Field {
                            expr: Box::new(expr),
                            name,
                        },
                    };
                }
                TokenKind::LParen => expr = self.parse_call(expr, Vec::new()),
                TokenKind::Lt => match self.try_type_args() {
                    Some(type_args) => expr = self.parse_call(expr, type_args),
                    None => return expr,
                },
                TokenKind::LBracket => {
                    let mut index = self
                        .delimited(TokenKind::LBracket, TokenKind::RBracket, |p| p.parse_expr());
                    let span = expr.span.to(self.previous_span());
                    let index = if index.len() == 1 {
                        index.pop().unwrap()
                    } else {
                        self.error(Diagnostic::error(
                            codes::EXPECTED_EXPRESSION,
                            "expected exactly one index",
                            span,
                        ));
                        Expr {
                            kind: ExprKind::Error,
                            span,
                        }
                    };
                    expr = Expr {
                        span,
                        kind: ExprKind::Index {
                            expr: Box::new(expr),
                            index: Box::new(index),
                        },
                    };
                }
                _ => return expr,
            }
        }
    }

    /// Tries to read `<T, U>` followed by `(`, as in `db.query<Post>(...)`.
    /// Restores the position when the tokens turn out to be a comparison.
    fn try_type_args(&mut self) -> Option<Vec<TypeExpr>> {
        let (pos, diag_count) = (self.pos, self.diags.len());
        let args = self.delimited(TokenKind::Lt, TokenKind::Gt, |p| p.parse_type());
        if self.diags.len() == diag_count && self.tokens[self.pos].kind == TokenKind::LParen {
            Some(args)
        } else {
            self.pos = pos;
            self.diags.truncate(diag_count);
            None
        }
    }

    fn parse_call(&mut self, callee: Expr, type_args: Vec<TypeExpr>) -> Expr {
        let args = self.delimited(TokenKind::LParen, TokenKind::RParen, |p| {
            let name =
                if matches!(p.peek(), TokenKind::Ident(_)) && *p.lookahead(1) == TokenKind::Colon {
                    let name = p.expect_ident("an argument name");
                    p.bump();
                    Some(name)
                } else {
                    None
                };
            Arg {
                name,
                value: p.parse_spread_or_expr(),
            }
        });
        Expr {
            span: callee.span.to(self.previous_span()),
            kind: ExprKind::Call {
                callee: Box::new(callee),
                type_args,
                args,
            },
        }
    }

    /// `..value` in a struct construction or a record: the fields not
    /// named come from `value`. Anything else is an ordinary expression.
    fn parse_spread_or_expr(&mut self) -> Expr {
        if !self.at(&TokenKind::DotDot) {
            return self.parse_expr();
        }
        let start = self.bump().span;
        let inner = self.parse_expr();
        Expr {
            span: start.to(inner.span),
            kind: ExprKind::Spread(Box::new(inner)),
        }
    }

    fn parse_primary(&mut self) -> Expr {
        let token = self.current().clone();
        let span = token.span;
        let kind = match token.kind {
            TokenKind::Int(text) => {
                self.bump();
                ExprKind::Int(text)
            }
            TokenKind::Float(text) => {
                self.bump();
                ExprKind::Float(text)
            }
            TokenKind::Duration(text) => {
                self.bump();
                ExprKind::Duration(text)
            }
            TokenKind::Str(parts) => {
                self.bump();
                ExprKind::Str(self.string_pieces(parts))
            }
            TokenKind::True => {
                self.bump();
                ExprKind::Bool(true)
            }
            TokenKind::False => {
                self.bump();
                ExprKind::Bool(false)
            }
            TokenKind::None => {
                self.bump();
                ExprKind::None
            }
            TokenKind::Ident(name) if matches!(name.as_str(), "null" | "nil" | "undefined") => {
                self.bump();
                self.error(
                    Diagnostic::error(codes::NO_NULL, format!("Kiln has no `{name}`"), span)
                        .with_help("a value that may be missing has type `T?`; the missing value is `none`")
                        .with_suggestion("use `none`", span, "none"),
                );
                ExprKind::None
            }
            TokenKind::Ident(name) => {
                self.bump();
                ExprKind::Ident(name)
            }
            TokenKind::LParen => self.with_newlines(false, |p| {
                p.bump();
                let inner = p.parse_expr();
                p.expect(&TokenKind::RParen);
                inner.kind
            }),
            TokenKind::LBracket => {
                // `[a, b]` is a list and `[k: v, ...]` a map: the first
                // item decides.
                let mut is_map = None;
                let items = self.delimited(TokenKind::LBracket, TokenKind::RBracket, |p| {
                    let key = p.parse_expr();
                    let map = *is_map.get_or_insert(p.at(&TokenKind::Colon));
                    if map {
                        p.expect(&TokenKind::Colon);
                        let value = p.parse_expr();
                        (key, Some(value))
                    } else {
                        if p.at(&TokenKind::Colon) {
                            let span = p.span();
                            p.error(Diagnostic::error(
                                codes::EXPECTED_TOKEN,
                                "a list's items have no keys; this list started without one",
                                span,
                            ));
                            p.bump();
                            p.parse_expr();
                        }
                        (key, None)
                    }
                });
                if is_map == Some(true) {
                    ExprKind::Map(
                        items
                            .into_iter()
                            .map(|(k, v)| {
                                let v = v.unwrap_or(Expr {
                                    kind: ExprKind::Error,
                                    span: k.span,
                                });
                                (k, v)
                            })
                            .collect(),
                    )
                } else {
                    ExprKind::List(items.into_iter().map(|(k, _)| k).collect())
                }
            }
            TokenKind::LBrace => {
                ExprKind::Record(self.delimited(TokenKind::LBrace, TokenKind::RBrace, |p| {
                    if p.at(&TokenKind::DotDot) {
                        let span = p.span();
                        let value = p.parse_spread_or_expr();
                        return RecordField {
                            name: Ident {
                                name: "..".into(),
                                span,
                            },
                            value: Some(value),
                        };
                    }
                    let name = p.expect_ident("a field name");
                    let value = if p.eat(&TokenKind::Colon) {
                        Some(p.parse_expr())
                    } else {
                        None
                    };
                    RecordField { name, value }
                }))
            }
            TokenKind::Dot if matches!(self.lookahead(1), TokenKind::Ident(_)) => {
                self.bump();
                ExprKind::FieldShorthand(self.expect_ident("a field name"))
            }
            TokenKind::Fn => {
                self.bump();
                let params = self.parse_params(false);
                let ret = if self.eat(&TokenKind::Arrow) {
                    Some(self.parse_type())
                } else {
                    None
                };
                let body = self.parse_block();
                ExprKind::Closure { params, ret, body }
            }
            TokenKind::If => return self.parse_if(),
            TokenKind::Match => return self.parse_match(),
            other => {
                self.error(Diagnostic::error(
                    codes::EXPECTED_EXPRESSION,
                    format!("expected an expression, found {other}"),
                    span,
                ));
                return Expr {
                    kind: ExprKind::Error,
                    span,
                };
            }
        };
        Expr {
            kind,
            span: span.to(self.previous_span()),
        }
    }

    fn string_pieces(&mut self, parts: Vec<StrPart>) -> Vec<StrPiece> {
        parts
            .into_iter()
            .map(|part| match part {
                StrPart::Text(text) => StrPiece::Text(text),
                StrPart::Interp(mut tokens, span) => {
                    tokens.push(Token {
                        kind: TokenKind::Eof,
                        span: Span::new(span.end as usize, span.end as usize),
                    });
                    let mut inner = Parser::new(tokens);
                    inner.newline_modes = vec![false];
                    let expr = inner.parse_expr();
                    if !inner.at(&TokenKind::Eof) {
                        inner.error_expected("`}`");
                    }
                    self.diags.extend(inner.diags);
                    StrPiece::Expr(expr)
                }
            })
            .collect()
    }

    fn parse_if(&mut self) -> Expr {
        let start = self.bump().span;
        let cond = self.parse_expr();
        let then = self.parse_block();
        if self.tokens[self.pos].kind == TokenKind::Newline
            && matches!(self.lookahead(0), TokenKind::Else)
        {
            self.skip_newlines();
        }
        let otherwise = if self.eat(&TokenKind::Else) {
            if self.at(&TokenKind::If) {
                Some(Box::new(self.parse_if()))
            } else {
                let block = self.parse_block();
                Some(Box::new(Expr {
                    span: block.span,
                    kind: ExprKind::Block(block),
                }))
            }
        } else if self.at_ident("elif") {
            let span = self.span();
            self.error(
                Diagnostic::error(codes::WRONG_KEYWORD, "`elif` is not a Kiln keyword", span)
                    .with_suggestion("use `else if`", span, "else if"),
            );
            // Parse the rest as if it read `else if`.
            self.tokens[self.pos].kind = TokenKind::If;
            Some(Box::new(self.parse_if()))
        } else {
            None
        };
        Expr {
            span: start.to(self.previous_span()),
            kind: ExprKind::If {
                cond: Box::new(cond),
                then,
                otherwise,
            },
        }
    }

    fn parse_match(&mut self) -> Expr {
        let start = self.bump().span;
        let scrutinee = self.parse_expr();
        let arms = self.member_list(|p| {
            let pattern = p.parse_pattern();
            let guard = if p.eat(&TokenKind::If) {
                Some(p.parse_expr())
            } else {
                None
            };
            p.expect(&TokenKind::FatArrow)?;
            let body = if p.at(&TokenKind::LBrace) {
                let block = p.parse_block();
                Expr {
                    span: block.span,
                    kind: ExprKind::Block(block),
                }
            } else {
                p.parse_expr()
            };
            Some(MatchArm {
                span: pattern.span.to(body.span),
                pattern,
                guard,
                body,
            })
        });
        Expr {
            span: start.to(self.previous_span()),
            kind: ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
        }
    }

    pub(crate) fn parse_pattern(&mut self) -> Pattern {
        let token = self.current().clone();
        let span = token.span;
        let kind = match token.kind {
            TokenKind::Ident(name) if name == "_" => {
                self.bump();
                PatternKind::Wildcard
            }
            TokenKind::None => {
                self.bump();
                PatternKind::None
            }
            TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::Str(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Minus => PatternKind::Literal(Box::new(self.parse_unary())),
            // `.Accent`: a variant of the matched enum.
            TokenKind::Dot if matches!(self.lookahead(1), TokenKind::Ident(_)) => {
                self.bump();
                let path = vec![self.expect_ident("a variant name")];
                let args = if self.at(&TokenKind::LParen) {
                    self.delimited(TokenKind::LParen, TokenKind::RParen, |p| p.parse_pattern())
                } else {
                    Vec::new()
                };
                PatternKind::Variant { path, args }
            }
            TokenKind::Ident(_) => {
                let mut path = vec![self.expect_ident("a name")];
                while self.at(&TokenKind::Dot) {
                    self.bump();
                    path.push(self.expect_ident("a variant name"));
                }
                if self.at(&TokenKind::LParen) {
                    let args =
                        self.delimited(TokenKind::LParen, TokenKind::RParen, |p| p.parse_pattern());
                    PatternKind::Variant { path, args }
                } else if path.len() == 1 && !path[0].name.starts_with(|c: char| c.is_uppercase()) {
                    PatternKind::Binding(path.pop().unwrap().name)
                } else {
                    PatternKind::Variant {
                        path,
                        args: Vec::new(),
                    }
                }
            }
            other => {
                self.error(Diagnostic::error(
                    codes::EXPECTED_PATTERN,
                    format!("expected a pattern, found {other}"),
                    span,
                ));
                return Pattern {
                    kind: PatternKind::Error,
                    span,
                };
            }
        };
        Pattern {
            kind,
            span: span.to(self.previous_span()),
        }
    }
}

/// Where a statement is in the source.
fn stmt_span(stmt: &Stmt) -> Span {
    match stmt {
        Stmt::Let { span, .. }
        | Stmt::Assign { span, .. }
        | Stmt::Return { span, .. }
        | Stmt::For { span, .. }
        | Stmt::While { span, .. } => *span,
        Stmt::Break(span) | Stmt::Continue(span) => *span,
        Stmt::Expr(e) => e.span,
    }
}

/// Whether a block always leaves the code around it: its last statement
/// returns, breaks, continues, throws or panics.
fn leaves(block: &Block) -> bool {
    match block.stmts.last() {
        Some(Stmt::Return { .. } | Stmt::Break(_) | Stmt::Continue(_)) => true,
        Some(Stmt::Expr(e)) => match &e.kind {
            ExprKind::Throw(_) => true,
            ExprKind::Call { callee, .. } => match &callee.kind {
                ExprKind::Ident(name) => name == "panic",
                ExprKind::Field { expr, name } => {
                    name.name == "exit" && matches!(&expr.kind, ExprKind::Ident(m) if m == "os")
                }
                _ => false,
            },
            ExprKind::Block(b) => leaves(b),
            ExprKind::If {
                then,
                otherwise: Some(otherwise),
                ..
            } => {
                leaves(then)
                    && match &otherwise.kind {
                        ExprKind::Block(b) => leaves(b),
                        ExprKind::If { .. } => leaves(&Block {
                            stmts: vec![Stmt::Expr((**otherwise).clone())],
                            span: otherwise.span,
                        }),
                        _ => false,
                    }
            }
            _ => false,
        },
        _ => false,
    }
}
