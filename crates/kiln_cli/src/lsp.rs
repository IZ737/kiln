//! `kiln lsp`: a language server over standard input and output.
//!
//! It reports the same diagnostics as `kiln check` while you type, and turns
//! each diagnostic's suggestion into a quick fix. A file is checked with the
//! other `.kiln` and `.glaze` files in its directory, using the editor's
//! unsaved text for any that are open.

use kiln_diagnostics::{codes, Diagnostic};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

struct Document {
    text: String,
    /// The diagnostics last published for this document.
    diagnostics: Vec<Diagnostic>,
}

struct Server {
    docs: HashMap<String, Document>,
    shutdown: bool,
}

/// Runs the server until the client sends `exit`. Returns the process exit
/// code.
pub fn run() -> i32 {
    let stdin = std::io::stdin();
    let mut input = stdin.lock();
    let mut server = Server {
        docs: HashMap::new(),
        shutdown: false,
    };
    while let Some(message) = read_message(&mut input) {
        let method = message["method"].as_str().unwrap_or_default().to_string();
        let id = message.get("id").cloned();
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        if method == "exit" {
            return if server.shutdown { 0 } else { 1 };
        }
        let result = server.handle(&method, &params);
        if let Some(id) = id {
            let response = match result {
                Some(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
                None => json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32601, "message": format!("unknown method `{method}`")},
                }),
            };
            write_message(&response);
        }
    }
    1
}

impl Server {
    /// Handles a request or notification, returning the result of a request
    /// it knows.
    fn handle(&mut self, method: &str, params: &Value) -> Option<Value> {
        let uri = params["textDocument"]["uri"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        match method {
            "initialize" => Some(json!({
                "capabilities": {
                    "textDocumentSync": {"openClose": true, "change": 1, "save": true},
                    "codeActionProvider": {"codeActionKinds": ["quickfix"]},
                },
                "serverInfo": {"name": "kiln", "version": env!("CARGO_PKG_VERSION")},
            })),
            "shutdown" => {
                self.shutdown = true;
                Some(Value::Null)
            }
            "textDocument/didOpen" => {
                let text = params["textDocument"]["text"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                self.docs.insert(
                    uri.clone(),
                    Document {
                        text,
                        diagnostics: Vec::new(),
                    },
                );
                self.check(&uri);
                None
            }
            "textDocument/didChange" => {
                // Full sync: the last change holds the whole text.
                let changes = params["contentChanges"].as_array();
                let text = changes
                    .and_then(|c| c.last())
                    .and_then(|c| c["text"].as_str());
                if let (Some(text), Some(doc)) = (text, self.docs.get_mut(&uri)) {
                    doc.text = text.to_string();
                }
                self.check(&uri);
                None
            }
            "textDocument/didSave" => {
                self.check(&uri);
                None
            }
            "textDocument/didClose" => {
                self.docs.remove(&uri);
                publish(&uri, &[], "");
                None
            }
            "textDocument/codeAction" => Some(self.code_actions(&uri, params)),
            _ => None,
        }
    }

    /// Checks a document with its directory and publishes diagnostics for
    /// every open document there.
    fn check(&mut self, uri: &str) {
        let Some(path) = uri_to_path(uri) else {
            // An unsaved buffer: check it on its own.
            if let Some(doc) = self.docs.get_mut(uri) {
                let name = PathBuf::from(if uri.ends_with(".glaze") {
                    "untitled.glaze"
                } else {
                    "untitled.kiln"
                });
                let results = crate::check_sources(vec![(name, doc.text.clone())]);
                doc.diagnostics = results.into_iter().next().map(|r| r.1).unwrap_or_default();
                publish(uri, &doc.diagnostics, &doc.text);
            }
            return;
        };
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        // Open documents in this directory, by path.
        let open: HashMap<PathBuf, String> = self
            .docs
            .keys()
            .filter_map(|u| uri_to_path(u).map(|p| (p, u.clone())))
            .filter(|(p, _)| p.parent() == Some(dir.as_path()))
            .collect();
        let mut texts: Vec<(PathBuf, String)> = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let source = p.extension().is_some_and(|e| e == "kiln") || crate::is_glaze(&p);
                if !source || open.contains_key(&p) || !p.is_file() {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&p) {
                    texts.push((p, text));
                }
            }
        }
        for (p, u) in &open {
            texts.push((p.clone(), self.docs[u].text.clone()));
        }
        let names: Vec<PathBuf> = texts.iter().map(|(p, _)| p.clone()).collect();
        let results = crate::check_sources(texts);
        for (p, (_, diagnostics)) in names.iter().zip(results) {
            if let Some(u) = open.get(p) {
                let doc = self.docs.get_mut(u).unwrap();
                doc.diagnostics = diagnostics;
                publish(u, &doc.diagnostics, &doc.text);
            }
        }
    }

    fn code_actions(&self, uri: &str, params: &Value) -> Value {
        let Some(doc) = self.docs.get(uri) else {
            return json!([]);
        };
        let start = offset_of(&doc.text, &params["range"]["start"]);
        let end = offset_of(&doc.text, &params["range"]["end"]);
        let mut actions = Vec::new();
        for diag in &doc.diagnostics {
            let Some(s) = &diag.suggestion else { continue };
            let (ds, de) = (diag.span.start as usize, diag.span.end as usize);
            let (ss, se) = (s.span.start as usize, s.span.end as usize);
            // Offer the fix when the cursor or selection touches the error
            // or the text the fix replaces.
            let touches = |a: usize, b: usize| a <= end && start <= b;
            if !touches(ds, de) && !touches(ss, se) {
                continue;
            }
            let edit = json!({"range": range(&doc.text, ss, se), "newText": s.replacement});
            actions.push(json!({
                "title": s.message,
                "kind": "quickfix",
                "isPreferred": true,
                "diagnostics": [lsp_diagnostic(diag, &doc.text)],
                "edit": {"changes": {uri: [edit]}},
            }));
        }
        Value::Array(actions)
    }
}

fn publish(uri: &str, diagnostics: &[Diagnostic], text: &str) {
    let diagnostics: Vec<Value> = diagnostics
        .iter()
        .map(|d| lsp_diagnostic(d, text))
        .collect();
    write_message(&json!({
        "jsonrpc": "2.0",
        "method": "textDocument/publishDiagnostics",
        "params": {"uri": uri, "diagnostics": diagnostics},
    }));
}

fn lsp_diagnostic(d: &Diagnostic, text: &str) -> Value {
    let mut message = d.message.clone();
    if let Some(help) = &d.help {
        message.push_str("\nhelp: ");
        message.push_str(help);
    }
    if let Some(entry) = codes::lookup(d.code) {
        message.push_str(&format!(
            "\n{}: {} (`kiln explain {}`)",
            d.code, entry.title, d.code
        ));
    }
    json!({
        "range": range(text, d.span.start as usize, d.span.end as usize),
        "severity": 1,
        "code": d.code,
        "source": "kiln",
        "message": message,
    })
}

/// An LSP range for a byte range of `text`.
fn range(text: &str, start: usize, end: usize) -> Value {
    json!({"start": position(text, start), "end": position(text, end)})
}

/// An LSP position (zero-based line, UTF-16 column) for a byte offset.
fn position(text: &str, offset: usize) -> Value {
    let offset = offset.min(text.len());
    let mut line = 0;
    let mut line_start = 0;
    for (i, c) in text.char_indices() {
        if i >= offset {
            break;
        }
        if c == '\n' {
            line += 1;
            line_start = i + 1;
        }
    }
    let character: usize = text[line_start..]
        .char_indices()
        .take_while(|(i, _)| line_start + i < offset)
        .map(|(_, c)| c.len_utf16())
        .sum();
    json!({"line": line, "character": character})
}

/// The byte offset of an LSP position.
fn offset_of(text: &str, pos: &Value) -> usize {
    let line = pos["line"].as_u64().unwrap_or(0) as usize;
    let character = pos["character"].as_u64().unwrap_or(0) as usize;
    let mut offset = 0;
    for (n, l) in text.split_inclusive('\n').enumerate() {
        if n == line {
            let mut units = 0;
            for (i, c) in l.char_indices() {
                if units >= character || c == '\n' {
                    return offset + i;
                }
                units += c.len_utf16();
            }
            return offset + l.trim_end_matches('\n').len();
        }
        offset += l.len();
    }
    text.len()
}

/// The path of a `file://` URI.
fn uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let bytes = rest.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&rest[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    let mut path = String::from_utf8(out).ok()?;
    // `file:///C:/x` on Windows.
    if path.len() > 2 && path.as_bytes()[2] == b':' {
        path.remove(0);
    }
    Some(PathBuf::from(path))
}

fn read_message(input: &mut impl BufRead) -> Option<Value> {
    let mut length = None;
    loop {
        let mut line = String::new();
        if input.read_line(&mut line).ok()? == 0 {
            return None;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(n) = line.strip_prefix("Content-Length:") {
            length = n.trim().parse::<usize>().ok();
        }
    }
    let mut body = vec![0; length?];
    input.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

fn write_message(message: &Value) {
    let body = message.to_string();
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_utf16_units() {
        let text = "let a = 1\nlet é = \"😀x\"\n";
        let x = text.find('x').unwrap();
        assert_eq!(position(text, x), json!({"line": 1, "character": 11}));
        assert_eq!(offset_of(text, &json!({"line": 1, "character": 11})), x);
        assert_eq!(position(text, 0), json!({"line": 0, "character": 0}));
    }

    #[test]
    fn decodes_file_uris() {
        assert_eq!(
            uri_to_path("file:///home/a%20b/main.kiln"),
            Some(PathBuf::from("/home/a b/main.kiln"))
        );
        assert_eq!(uri_to_path("untitled:Untitled-1"), None);
    }
}
