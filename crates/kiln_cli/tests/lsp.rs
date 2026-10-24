//! Drives `kiln lsp` the way an editor does.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Client {
    fn start() -> Client {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .arg("lsp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Client {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, message: Value) {
        let body = message.to_string();
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn receive(&mut self) -> Value {
        let mut length = 0;
        loop {
            let mut line = String::new();
            self.stdout.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if let Some(n) = line.strip_prefix("Content-Length:") {
                length = n.trim().parse().unwrap();
            }
        }
        let mut body = vec![0; length];
        self.stdout.read_exact(&mut body).unwrap();
        serde_json::from_slice(&body).unwrap()
    }
}

#[test]
fn reports_diagnostics_and_offers_fixes() {
    let dir = std::env::temp_dir().join(format!("kiln-lsp-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("main.kiln");
    std::fs::write(&path, "fn main() {\n}\n").unwrap();
    let uri = format!("file://{}", path.display());

    let mut client = Client::start();
    client.send(json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
    let init = client.receive();
    assert_eq!(init["id"], 1);
    assert_eq!(
        init["result"]["capabilities"]["textDocumentSync"]["change"],
        1
    );

    // The editor's unsaved text is checked, not the file on disk.
    let text = "fn main() {\n    let x = null\n}\n";
    client.send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didOpen",
        "params": {"textDocument": {"uri": uri, "languageId": "kiln", "version": 1, "text": text}},
    }));
    let published = client.receive();
    assert_eq!(published["method"], "textDocument/publishDiagnostics");
    let diags = published["params"]["diagnostics"].as_array().unwrap();
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0]["code"], "E0106");
    assert_eq!(
        diags[0]["range"]["start"],
        json!({"line": 1, "character": 12})
    );

    client.send(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "textDocument/codeAction",
        "params": {
            "textDocument": {"uri": uri},
            "range": {"start": {"line": 1, "character": 13}, "end": {"line": 1, "character": 13}},
            "context": {"diagnostics": []},
        },
    }));
    let actions = client.receive();
    assert_eq!(actions["id"], 2);
    let edit = &actions["result"][0]["edit"]["changes"][uri.as_str()][0];
    assert_eq!(edit["newText"], "none");
    assert_eq!(edit["range"]["start"], json!({"line": 1, "character": 12}));

    // Fixing the text clears the diagnostics.
    client.send(json!({
        "jsonrpc": "2.0",
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {"uri": uri, "version": 2},
            "contentChanges": [{"text": "fn main() {\n    let x: Int? = none\n}\n"}],
        },
    }));
    let cleared = client.receive();
    assert_eq!(cleared["params"]["diagnostics"], json!([]));

    client.send(json!({"jsonrpc": "2.0", "id": 3, "method": "shutdown"}));
    assert_eq!(client.receive()["id"], 3);
    client.send(json!({"jsonrpc": "2.0", "method": "exit"}));
    assert!(client.child.wait().unwrap().success());
}
