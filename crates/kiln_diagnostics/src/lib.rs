//! Source locations, error codes and diagnostic rendering.
//!
//! Every error the compiler reports is a [`Diagnostic`] with a stable code
//! (for example `E0101`). Diagnostics render two ways: a human format with
//! the offending source line underlined, and a JSON format for editors and
//! coding agents (`kiln check --json`).

use serde::Serialize;
use std::fmt::Write as _;

pub mod codes;

/// A half-open byte range `start..end` into a source file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Span {
    pub start: u32,
    pub end: u32,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Span {
            start: start as u32,
            end: end as u32,
        }
    }

    /// The smallest span covering both `self` and `other`.
    pub fn to(self, other: Span) -> Span {
        Span {
            start: self.start.min(other.start),
            end: self.end.max(other.end),
        }
    }

    pub fn len(self) -> usize {
        (self.end - self.start) as usize
    }

    pub fn is_empty(self) -> bool {
        self.start == self.end
    }
}

/// A 1-based line and column, counted in characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct LineCol {
    pub line: usize,
    pub column: usize,
}

/// A source file held in memory, with an index of line starts.
#[derive(Clone, Debug)]
pub struct SourceFile {
    pub name: String,
    pub text: String,
    line_starts: Vec<usize>,
}

impl SourceFile {
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Self {
        let text = text.into();
        let line_starts = std::iter::once(0)
            .chain(text.match_indices('\n').map(|(i, _)| i + 1))
            .collect();
        SourceFile {
            name: name.into(),
            text,
            line_starts,
        }
    }

    pub fn line_col(&self, offset: u32) -> LineCol {
        let offset = (offset as usize).min(self.text.len());
        let line = match self.line_starts.binary_search(&offset) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let column = self.text[self.line_starts[line]..offset].chars().count() + 1;
        LineCol {
            line: line + 1,
            column,
        }
    }

    /// The text of a 1-based line, without its trailing newline.
    pub fn line_text(&self, line: usize) -> &str {
        let start = self.line_starts[line - 1];
        let end = self
            .line_starts
            .get(line)
            .map(|&next| next - 1)
            .unwrap_or(self.text.len());
        self.text[start..end].trim_end_matches('\r')
    }

    pub fn slice(&self, span: Span) -> &str {
        &self.text[span.start as usize..span.end as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
}

/// A machine-applicable fix: replace `span` with `replacement`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Suggestion {
    pub message: String,
    pub span: Span,
    pub replacement: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Diagnostic {
    pub code: &'static str,
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    pub help: Option<String>,
    pub suggestion: Option<Suggestion>,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>, span: Span) -> Self {
        Diagnostic {
            code,
            severity: Severity::Error,
            message: message.into(),
            span,
            help: None,
            suggestion: None,
        }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_suggestion(
        mut self,
        message: impl Into<String>,
        span: Span,
        replacement: impl Into<String>,
    ) -> Self {
        self.suggestion = Some(Suggestion {
            message: message.into(),
            span,
            replacement: replacement.into(),
        });
        self
    }

    /// Render for a terminal:
    ///
    /// ```text
    /// error[E0101]: expected `)`, found `{`
    ///  --> main.kiln:3:14
    ///   |
    /// 3 | fn main( {
    ///   |          ^
    ///   = help: ...
    /// ```
    pub fn render(&self, file: &SourceFile) -> String {
        let mut out = String::new();
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        let start = file.line_col(self.span.start);
        let _ = writeln!(out, "{severity}[{}]: {}", self.code, self.message);
        let gutter = start.line.to_string().len();
        let pad = " ".repeat(gutter);
        let _ = writeln!(
            out,
            "{pad}--> {}:{}:{}",
            file.name, start.line, start.column
        );
        let _ = writeln!(out, "{pad} |");
        let line = file.line_text(start.line);
        let _ = writeln!(out, "{} | {}", start.line, line);
        let end = file.line_col(self.span.end);
        let width = if end.line == start.line {
            (end.column - start.column).max(1)
        } else {
            (line.chars().count() + 1)
                .saturating_sub(start.column)
                .max(1)
        };
        let _ = writeln!(
            out,
            "{pad} | {}{}",
            " ".repeat(start.column - 1),
            "^".repeat(width)
        );
        if let Some(help) = &self.help {
            let _ = writeln!(out, "{pad} = help: {help}");
        }
        if let Some(s) = &self.suggestion {
            let _ = writeln!(out, "{pad} = fix: {}", s.message);
        }
        out
    }

    pub fn to_json(&self, file: &SourceFile) -> serde_json::Value {
        let explanation = codes::lookup(self.code).map(|c| c.title);
        serde_json::json!({
            "code": self.code,
            "severity": self.severity,
            "message": self.message,
            "file": file.name,
            "start": file.line_col(self.span.start),
            "end": file.line_col(self.span.end),
            "span": self.span,
            "help": self.help,
            "suggestion": self.suggestion,
            "explanation": explanation,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_col_counts_from_one() {
        let file = SourceFile::new("a.kiln", "let a = 1\nlet b = 2\n");
        assert_eq!(file.line_col(0), LineCol { line: 1, column: 1 });
        assert_eq!(file.line_col(10), LineCol { line: 2, column: 1 });
        assert_eq!(file.line_col(14), LineCol { line: 2, column: 5 });
        assert_eq!(file.line_text(2), "let b = 2");
    }

    #[test]
    fn renders_caret_under_span() {
        let file = SourceFile::new("main.kiln", "fn main( {\n}\n");
        let diag = Diagnostic::error(
            codes::EXPECTED_TOKEN,
            "expected `)`, found `{`",
            Span::new(9, 10),
        )
        .with_help("close the parameter list");
        let text = diag.render(&file);
        assert_eq!(
            text,
            "error[E0101]: expected `)`, found `{`\n --> main.kiln:1:10\n  |\n1 | fn main( {\n  |          ^\n  = help: close the parameter list\n"
        );
    }

    #[test]
    fn json_includes_location_and_explanation() {
        let file = SourceFile::new("main.kiln", "let = 1\n");
        let diag = Diagnostic::error(codes::EXPECTED_TOKEN, "expected a name", Span::new(4, 5));
        let json = diag.to_json(&file);
        assert_eq!(json["code"], "E0101");
        assert_eq!(json["start"]["column"], 5);
        assert!(json["explanation"].is_string());
    }
}
