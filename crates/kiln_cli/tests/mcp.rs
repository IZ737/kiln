//! `kiln mcp`: the Model Context Protocol server agents use. These tests
//! talk to it as an agent would, over standard input and output.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next: u64,
}

impl Server {
    fn start(root: &Path, flags: &[&str]) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_kiln"))
            .arg("mcp")
            .arg("--root")
            .arg(root)
            .args(flags)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap();
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut server = Server {
            child,
            stdin,
            stdout,
            next: 0,
        };
        let init = server.request(
            "initialize",
            json!({ "protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": { "name": "test", "version": "1" } }),
        );
        assert_eq!(init["result"]["serverInfo"]["name"], "kiln");
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        server.notify("notifications/initialized");
        server
    }

    fn notify(&mut self, method: &str) {
        writeln!(
            self.stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "method": method })
        )
        .unwrap();
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next += 1;
        let id = self.next;
        writeln!(
            self.stdin,
            "{}",
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params })
        )
        .unwrap();
        let mut line = String::new();
        self.stdout.read_line(&mut line).unwrap();
        let reply: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(reply["id"], id);
        reply
    }

    /// Calls a tool, and returns its result.
    fn call(&mut self, name: &str, arguments: Value) -> Value {
        let reply = self.request(
            "tools/call",
            json!({ "name": name, "arguments": arguments }),
        );
        reply["result"].clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn project(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("shapes")).unwrap();
    std::fs::write(
        dir.join("main.kiln"),
        "import shapes\n\nfn main() {\n    print(shapes.area(2))\n}\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("shapes/shapes.kiln"),
        "// The area of a square.\npub fn area(side: Int) -> Int {\n    return side * side\n}\n\nfn hidden() -> Int { 1 }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("shapes/test_shapes.kiln"),
        "import test\n\nfn test_area() {\n    test.equal(area(3), 9)\n}\n\nfn test_wrong() {\n    test.equal(area(2), 5)\n}\n",
    )
    .unwrap();
    dir.canonicalize().unwrap()
}

#[test]
fn reads_checks_and_explains_without_running_code() {
    let dir = project("read");
    let mut server = Server::start(&dir, &[]);

    let tools = server.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    for name in [
        "project_graph",
        "check",
        "explain",
        "run_tests",
        "format_preview",
    ] {
        assert!(names.contains(&name), "{names:?}");
    }

    let version = server.call("version", json!({}));
    assert_eq!(version["structuredContent"]["schema"], 1);
    assert_eq!(version["structuredContent"]["allow_exec"], false);

    let graph = server.call("project_graph", json!({}));
    assert_eq!(graph["isError"], false);
    let modules = graph["structuredContent"]["modules"].as_array().unwrap();
    let shapes = modules.iter().find(|m| m["module"] == "shapes").unwrap();
    assert_eq!(shapes["exports"].as_array().unwrap().len(), 1);
    assert_eq!(
        shapes["exports"][0]["signature"],
        "pub fn area(side: Int) -> Int"
    );
    let root = modules.iter().find(|m| m["module"] == "").unwrap();
    assert_eq!(root["imports"], json!(["shapes"]));

    let check = server.call("check", json!({}));
    assert_eq!(check["structuredContent"]["error_count"], 0);

    std::fs::write(
        dir.join("main.kiln"),
        "import shapes\n\nfn main() {\n    let n: String = shapes.area(2)\n    let b: Bool = shapes.area(3)\n}\n",
    )
    .unwrap();
    let check = server.call("check", json!({ "limit": 1 }));
    let data = &check["structuredContent"];
    assert_eq!(data["error_count"], 2, "{check}");
    assert_eq!(data["diagnostics"].as_array().unwrap().len(), 1);
    assert_eq!(data["next_offset"], 1);
    let first = &data["diagnostics"][0];
    assert!(first["code"].as_str().unwrap().starts_with('E'));
    assert_eq!(first["start"]["line"], 4);
    let rest = server.call("check", json!({ "offset": 1 }));
    assert_eq!(
        rest["structuredContent"]["diagnostics"][0]["start"]["line"],
        5
    );
    std::fs::write(dir.join("main.kiln"), "fn main() {\n    print(null)\n}\n").unwrap();
    let null = server.call("check", json!({ "paths": ["main.kiln"] }));
    let null = &null["structuredContent"]["diagnostics"][0];
    assert_eq!(null["code"], "E0106");
    assert_eq!(null["suggestion"]["replacement"], "none", "{null}");

    let explained = server.call("explain", json!({ "code": "E0106" }));
    assert!(explained["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("none"));
    assert_eq!(
        server.call("explain", json!({ "code": "E9999" }))["isError"],
        true
    );

    // Running tests and writing files were not allowed.
    let refused = server.call("run_tests", json!({}));
    assert_eq!(refused["isError"], true);
    assert!(refused["content"][0]["text"]
        .as_str()
        .unwrap()
        .contains("--allow-exec"));
    std::fs::write(dir.join("shapes/messy.kiln"), "fn   messy( )->Int{\n1\n}\n").unwrap();
    assert_eq!(server.call("format_apply", json!({}))["isError"], true);
    let preview = server.call("format_preview", json!({ "path": "shapes/messy.kiln" }));
    let changed = &preview["structuredContent"]["changed"][0];
    assert_eq!(changed["formatted"], "fn messy() -> Int {\n    1\n}\n");
    assert_eq!(
        std::fs::read_to_string(dir.join("shapes/messy.kiln")).unwrap(),
        "fn   messy( )->Int{\n1\n}\n",
        "a preview changes nothing"
    );

    // Nothing outside the root.
    let outside = server.call("project_graph", json!({ "path": "../" }));
    assert_eq!(outside["isError"], true);
    let outside = server.call("check", json!({ "paths": ["/"] }));
    assert_eq!(outside["isError"], true);

    let unknown = server.request("tools/call", json!({ "name": "deploy", "arguments": {} }));
    assert_eq!(unknown["error"]["code"], -32602);
    let unknown = server.request("resources/list", json!({}));
    assert_eq!(unknown["error"]["code"], -32601);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn runs_tests_and_formats_when_allowed() {
    let dir = project("exec");
    let mut server = Server::start(&dir, &["--allow-exec", "--allow-write"]);
    let listed = server.call("list_tests", json!({}));
    assert_eq!(
        listed["structuredContent"]["tests"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let run = server.call("run_tests", json!({}));
    assert_eq!(run["isError"], false, "{run}");
    let report = &run["structuredContent"]["report"];
    assert_eq!(report["summary"]["passed"], 1, "{run}");
    assert_eq!(report["summary"]["failed"], 1, "{run}");
    assert_eq!(run["structuredContent"]["exit_code"], 1);
    let text = run["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("shapes/test_shapes.kiln:8:"), "{text}");

    let only = server.call("run_tests", json!({ "filter": "test_area" }));
    assert_eq!(only["structuredContent"]["exit_code"], 0, "{only}");

    std::fs::write(dir.join("shapes/messy.kiln"), "fn   messy( )->Int{\n1\n}\n").unwrap();
    let applied = server.call("format_apply", json!({ "path": "shapes" }));
    assert_eq!(applied["isError"], false, "{applied}");
    assert_eq!(
        std::fs::read_to_string(dir.join("shapes/messy.kiln")).unwrap(),
        "fn messy() -> Int {\n    1\n}\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
