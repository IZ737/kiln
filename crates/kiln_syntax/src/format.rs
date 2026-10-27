//! `kiln fmt`: one layout for Kiln source.
//!
//! The formatter keeps the program's tokens, comments and line breaks, and
//! decides only the whitespace between them: four spaces of indentation per
//! open bracket, one space around binary operators and after commas and
//! colons, none inside brackets, at most one blank line, no trailing spaces
//! and one newline at the end. Where spacing could change meaning (`<` as a
//! comparison or a type argument, `-` before a number), what was written is
//! kept. The result is lexed again and compared with the original: if the
//! tokens or comments differ, nothing is changed.
//!
//! Glazes are markup, where whitespace is part of the page, so only the code
//! inside `{% ... %}` tags that fit on one line is formatted.

use crate::lexer::lex;
use crate::token::{Token, TokenKind};

/// Formats Kiln source, or says why it cannot.
pub fn format_kiln(src: &str) -> Result<String, String> {
    let (tokens, diags) = lex(src);
    if let Some(d) = diags.first() {
        return Err(format!("it does not lex: {}", d.message));
    }
    let out = layout(src, &tokens, 0);
    check_same(src, &out)?;
    Ok(out)
}

/// The tokens and comments of a text, to compare before and after.
fn essence(src: &str) -> (Vec<&str>, Vec<String>) {
    let (tokens, _) = lex(src);
    let kinds = tokens
        .iter()
        .filter(|t| t.kind != TokenKind::Newline)
        .map(|t| &src[t.span.start as usize..t.span.end as usize])
        .collect();
    let mut comments = Vec::new();
    let mut last = 0;
    for t in tokens.iter().filter(|t| t.kind != TokenKind::Newline) {
        comments.extend(gap_comments(&src[last..t.span.start as usize]));
        last = t.span.end as usize;
    }
    (kinds, comments)
}

fn gap_comments(gap: &str) -> Vec<String> {
    pieces(gap)
        .into_iter()
        .filter_map(|p| match p {
            Piece::Comment(c) => Some(c.trim_end().to_string()),
            _ => None,
        })
        .collect()
}

fn check_same(before: &str, after: &str) -> Result<(), String> {
    if essence(before) != essence(after) {
        return Err("formatting would change its tokens or comments, so it is left as it is; please report this".into());
    }
    let (_, diags) = lex(after);
    if !diags.is_empty() {
        return Err("formatting would make it invalid, so it is left as it is".into());
    }
    Ok(())
}

/// What lies between two tokens.
#[derive(Debug)]
enum Piece<'a> {
    Newline,
    Comment(&'a str),
}

fn pieces(gap: &str) -> Vec<Piece<'_>> {
    let mut out = Vec::new();
    let bytes = gap.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                out.push(Piece::Newline);
                i += 1;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                let end = gap[i..].find('\n').map_or(gap.len(), |e| i + e);
                out.push(Piece::Comment(&gap[i..end]));
                i = end;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = gap[i + 2..].find("*/").map_or(gap.len(), |e| i + 2 + e + 2);
                out.push(Piece::Comment(&gap[i..end]));
                i = end;
            }
            _ => i += 1,
        }
    }
    out
}

fn is_open(k: &TokenKind) -> bool {
    matches!(
        k,
        TokenKind::LParen | TokenKind::LBrace | TokenKind::LBracket
    )
}

fn is_close(k: &TokenKind) -> bool {
    matches!(
        k,
        TokenKind::RParen | TokenKind::RBrace | TokenKind::RBracket
    )
}

fn is_keyword(k: &TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        k,
        And | Break
            | Catch
            | Const
            | Continue
            | Else
            | Enum
            | False
            | Fn
            | For
            | If
            | Impl
            | Import
            | In
            | Is
            | Let
            | Match
            | None
            | Not
            | Or
            | Pub
            | Return
            | Struct
            | Throw
            | True
            | Try
            | Var
            | Where
            | While
    )
}

/// Operators that always have one space on each side.
fn is_spaced_operator(k: &TokenKind) -> bool {
    use TokenKind::*;
    matches!(
        k,
        Eq | EqEq
            | NotEq
            | Le
            | Ge
            | Plus
            | Star
            | Slash
            | Percent
            | QuestionQuestion
            | PlusEq
            | MinusEq
            | StarEq
            | SlashEq
            | Arrow
            | FatArrow
            | And
            | Or
            | EqEqEq
            | BangEqEq
            | AmpAmp
            | PipePipe
    )
}

/// Whether a line that ends with this token goes on to the next line.
fn continues(k: &TokenKind) -> bool {
    use TokenKind::*;
    is_spaced_operator(k) && !matches!(k, Arrow | FatArrow) || matches!(k, Minus)
}

/// The space between two tokens on one line: `None` keeps what is there
/// (as one space, or none).
fn space(a: &TokenKind, b: &TokenKind, had: bool, after_dot: bool) -> bool {
    use TokenKind::*;
    // `x.where(...)`, `fn where(...)`: a keyword there is a method's name.
    if after_dot && matches!(b, LParen) {
        return false;
    }
    match (a, b) {
        (_, Comma) | (_, Semi) => false,
        // `<` and `>` compare, or hold type arguments: keep what is there.
        (Lt | Gt, _) | (_, Lt | Gt) => had,
        (Comma, _) => !is_close(b),
        (LParen | LBracket, _) => false,
        (_, RParen | RBracket) => false,
        (LBrace, RBrace) => false,
        (LBrace, _) | (_, RBrace) | (_, LBrace) => true,
        (Dot | DotDot, _) => false,
        (_, DotDot) => false,
        // `.Variant` after an operator or a comma keeps its space.
        (_, Dot) => had && !matches!(a, Ident(_) | RParen | RBracket | Str(_) | Question),
        (_, Colon) => false,
        (Colon, _) => true,
        // `User?`, `fn f() !`.
        (_, Question) => false,
        (Bang, _) => true,
        (_, Bang) => true,
        (Question, _) => had,
        (x, _) if is_spaced_operator(x) => true,
        (_, y) if is_spaced_operator(y) => true,
        (Fn, LParen) => false,
        (Ident(_), LParen) | (RParen, LParen) | (RBracket, LParen) => had,
        (x, _) if is_keyword(x) => true,
        (_, y) if is_keyword(y) => true,
        _ => had,
    }
}

/// Lays out tokens, with `base` levels of indentation to start from.
fn layout(src: &str, tokens: &[Token], base: usize) -> String {
    let toks: Vec<&Token> = tokens
        .iter()
        .filter(|t| !matches!(t.kind, TokenKind::Newline | TokenKind::Eof))
        .collect();
    let mut out = String::new();
    // The indentation of the line each open bracket is on.
    let mut stack: Vec<usize> = Vec::new();
    let mut line_indent = base;
    let mut line_start = true;
    let mut prev: Option<&Token> = None;
    // Whether the token before `prev` makes it a name: `x.where`, `fn where`.
    let mut before_prev_names = false;
    for t in &toks {
        let start = prev.map_or(0, |p| p.span.end as usize);
        let gap = &src[start..t.span.start as usize];
        let ps = pieces(gap);
        let newlines = ps.iter().filter(|p| matches!(p, Piece::Newline)).count();
        if prev.is_none() {
            // Comments before the first token.
            let mut wrote_comment = false;
            let mut blanks = 0;
            for p in &ps {
                match p {
                    Piece::Comment(c) => {
                        if blanks > 1 && wrote_comment {
                            out.push('\n');
                        }
                        out.push_str(&"    ".repeat(base));
                        out.push_str(c.trim_end());
                        out.push('\n');
                        wrote_comment = true;
                        blanks = 0;
                    }
                    Piece::Newline => blanks += 1,
                }
            }
            if wrote_comment && blanks > 1 {
                out.push('\n');
            }
        } else if let (0, Some(p)) = (newlines, prev) {
            // Same line: a block comment may sit between.
            let comments: Vec<&str> = ps
                .iter()
                .filter_map(|p| match p {
                    Piece::Comment(c) => Some(*c),
                    _ => None,
                })
                .collect();
            if comments.is_empty() {
                if space(&p.kind, &t.kind, !gap.is_empty(), before_prev_names) {
                    out.push(' ');
                }
            } else {
                for c in comments {
                    out.push(' ');
                    out.push_str(c);
                }
                out.push(' ');
            }
        } else if let Some(p) = prev {
            // Line breaks: comments on their own lines, at most one blank line.
            let mut first = true;
            let mut breaks = 0;
            let next_indent = indent_for(&stack, t, base, p);
            for piece in &ps {
                match piece {
                    Piece::Comment(c) => {
                        if first && breaks == 0 {
                            // A comment that ends the line of `prev`.
                            out.push(' ');
                            out.push_str(c.trim_end());
                        } else {
                            if breaks > 1 {
                                out.push('\n');
                            }
                            out.push('\n');
                            out.push_str(&"    ".repeat(next_indent));
                            out.push_str(c.trim_end());
                            breaks = 0;
                        }
                        first = false;
                    }
                    Piece::Newline => {
                        breaks += 1;
                        first = false;
                    }
                }
            }
            let mut blank_pending = breaks > 1;
            // No blank line just inside an opening or before a closing bracket.
            if is_open(&p.kind) || is_close(&t.kind) {
                blank_pending = false;
            }
            out.push('\n');
            if blank_pending {
                out.push('\n');
            }
            line_start = true;
            line_indent = next_indent;
        }
        if line_start {
            out.push_str(&"    ".repeat(line_indent));
            line_start = false;
        }
        out.push_str(&src[t.span.start as usize..t.span.end as usize]);
        if is_open(&t.kind) {
            stack.push(line_indent);
        } else if is_close(&t.kind) {
            stack.pop();
        }
        before_prev_names = prev.is_some_and(|p| matches!(p.kind, TokenKind::Dot | TokenKind::Fn));
        prev = Some(t);
    }
    // Comments after the last token.
    if let Some(p) = prev {
        let gap = &src[p.span.end as usize..];
        let mut breaks = 0;
        let mut first = true;
        for piece in pieces(gap) {
            match piece {
                Piece::Comment(c) => {
                    if first && breaks == 0 {
                        out.push(' ');
                    } else {
                        if breaks > 1 {
                            out.push('\n');
                        }
                        out.push('\n');
                        out.push_str(&"    ".repeat(base));
                    }
                    out.push_str(c.trim_end());
                    breaks = 0;
                    first = false;
                }
                Piece::Newline => {
                    breaks += 1;
                    first = false;
                }
            }
        }
        out.push('\n');
    }
    out
}

/// The indentation of a line that starts with `t`, after `prev`.
fn indent_for(stack: &[usize], t: &Token, base: usize, prev: &Token) -> usize {
    if is_close(&t.kind) {
        // A closing bracket lines up with the line that opened it; a line
        // of several, such as `})`, with the first one's.
        return stack.last().copied().unwrap_or(base);
    }
    let inner = stack.last().map_or(base, |l| l + 1);
    let continued = continues(&prev.kind) || matches!(t.kind, TokenKind::Dot);
    if continued && !is_open(&prev.kind) && !matches!(prev.kind, TokenKind::Comma) {
        inner + 1
    } else {
        inner
    }
}

/// Formats the code in a glaze's one-line `{% ... %}` tags; the markup is
/// left exactly as it is.
pub fn format_glaze(src: &str) -> Result<String, String> {
    let mut out = String::new();
    let mut rest = src;
    while let Some(open) = rest.find("{%") {
        out.push_str(&rest[..open]);
        let after = &rest[open + 2..];
        let Some(close) = after.find("%}") else {
            out.push_str(&rest[open..]);
            rest = "";
            break;
        };
        let inner = &after[..close];
        if inner.contains('\n') || inner.trim().is_empty() {
            out.push_str(&rest[open..open + 2 + close + 2]);
        } else {
            match format_line(inner.trim()) {
                Some(code) => {
                    out.push_str("{% ");
                    out.push_str(&code);
                    out.push_str(" %}");
                }
                None => out.push_str(&rest[open..open + 2 + close + 2]),
            }
        }
        rest = &after[close + 2..];
    }
    out.push_str(rest);
    Ok(out)
}

/// One line of code, formatted, if that keeps its tokens.
fn format_line(code: &str) -> Option<String> {
    let (tokens, diags) = lex(code);
    if !diags.is_empty() {
        return None;
    }
    let laid = layout(code, &tokens, 0);
    let laid = laid.trim_end_matches('\n');
    if laid.contains('\n') || essence(code) != essence(laid) {
        return None;
    }
    Some(laid.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(src: &str) {
        let out = format_kiln(src).unwrap();
        assert_eq!(out, src, "\n--- got ---\n{out}");
    }

    fn becomes(src: &str, expected: &str) {
        let out = format_kiln(src).unwrap();
        assert_eq!(out, expected, "\n--- got ---\n{out}");
        assert_eq!(format_kiln(&out).unwrap(), out, "not idempotent");
    }

    #[test]
    fn indentation_and_spaces() {
        becomes(
            "fn  main( )  {\n  let x=1+2\n\tif x==3 {print( \"yes\" ,x )}\n}",
            "fn main() {\n    let x = 1 + 2\n    if x == 3 { print(\"yes\", x) }\n}\n",
        );
    }

    #[test]
    fn closures_and_chains() {
        same("import web\n\nfn main() ! {\n    let app = web.app()\n    app.get(\"/\", fn(req) {\n        return web.text(\"hi\")\n    })\n    let rows = try orm.find<Post>(db)\n        .where(fn(p) { p.published })\n        .all()\n}\n");
    }

    #[test]
    fn comments_and_blank_lines_survive() {
        becomes(
            "// A program.\n\n\n\nfn main() {   // the start\n\n\n    // say it\n    print(1) /* one */\n\n}\n// end\n",
            "// A program.\n\nfn main() { // the start\n\n    // say it\n    print(1) /* one */\n}\n// end\n",
        );
    }

    #[test]
    fn types_and_optionals() {
        same("pub struct User {\n    name: String\n    boss: User? = none\n    tags: List<String> = []\n}\n\nfn find(id: Int) -> User? ! {\n    return users[id] ?? none\n}\n");
        becomes(
            "fn f(a:Int,b : Int)->Int{\n    return a-b\n}\n",
            "fn f(a: Int, b: Int) -> Int {\n    return a-b\n}\n",
        );
    }

    #[test]
    fn match_arms_and_records() {
        same("fn f(t: Tone) -> String {\n    return match t {\n        Tone.A => \"a\"\n        _ => \"b\"\n    }\n}\n\nfn g() {\n    web.render(\"x.glaze\", { posts: [], title: \"Home\" })\n    x.tone(.Primary)\n}\n");
    }

    #[test]
    fn multiline_calls() {
        same("fn f() {\n    let site = admin.site(\n        db,\n        sessions,\n        fn(who, action, model) { who.roles.contains(\"admin\") },\n        title: \"Admin\",\n    )\n}\n");
    }

    #[test]
    fn glaze_tags() {
        assert_eq!(
            format_glaze("{%props a:Int,b: String%}\n<p>  {a}  </p>\n{% if a>1 %}x{%end%}")
                .unwrap(),
            "{% props a: Int, b: String %}\n<p>  {a}  </p>\n{% if a>1 %}x{% end %}"
        );
    }

    #[test]
    fn every_example_is_stable() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let mut files = Vec::new();
        for dir in [
            "examples",
            "conformance",
            "site",
            "registry",
            "wares",
            "crates/kiln_check/std",
            "crates/kiln_cli/wares",
        ] {
            collect(&root.join(dir), &mut files);
        }
        assert!(files.len() > 50);
        for file in files {
            let src = std::fs::read_to_string(&file).unwrap();
            let once = format_kiln(&src).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            // The repository's own code is kept formatted.
            assert_eq!(
                once,
                src,
                "{} is not formatted: run `kiln fmt`",
                file.display()
            );
            let twice = format_kiln(&once).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
            assert_eq!(once, twice, "{} is not stable", file.display());
            assert_eq!(
                crate::parse(&src).diagnostics.is_empty(),
                crate::parse(&once).diagnostics.is_empty(),
                "{} parses differently",
                file.display()
            );
        }
    }

    fn collect(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect(&p, out);
            } else if p.extension().is_some_and(|x| x == "kiln") {
                out.push(p);
            }
        }
    }
}
