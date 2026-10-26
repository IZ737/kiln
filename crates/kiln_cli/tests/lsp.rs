//! Drives `kiln lsp` the way an editor does.

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

struct Client {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
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
            next_id: 100,
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

    /// Sends a request and returns its result, skipping notifications.
    fn request(&mut self, method: &str, params: Value) -> Value {
        self.next_id += 1;
        let id = self.next_id;
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let message = self.receive();
            if message["id"] == id {
                assert!(message.get("error").is_none(), "{method} failed: {message}");
                return message["result"].clone();
            }
        }
    }

    fn notify(&mut self, method: &str, params: Value) {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}));
    }

    fn open(&mut self, uri: &str, text: &str) {
        let language = if uri.ends_with(".ember") {
            "ember"
        } else if uri.ends_with(".glaze") {
            "glaze"
        } else {
            "kiln"
        };
        self.notify(
            "textDocument/didOpen",
            json!({"textDocument": {"uri": uri, "languageId": language, "version": 1, "text": text}}),
        );
    }

    fn change(&mut self, uri: &str, text: &str) {
        self.notify(
            "textDocument/didChange",
            json!({"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": text}]}),
        );
    }

    /// Sends a request about the place in `text` marked with `|`, after
    /// opening or updating the document with `text` minus the mark.
    fn at(&mut self, method: &str, uri: &str, marked: &str) -> Value {
        let (text, position) = unmark(marked);
        self.change(uri, &text);
        self.request(
            method,
            json!({"textDocument": {"uri": uri}, "position": position}),
        )
    }

    fn finish(mut self) {
        self.request("shutdown", Value::Null);
        self.notify("exit", Value::Null);
        assert!(self.child.wait().unwrap().success());
    }
}

/// The text without its `|` mark, and the mark's LSP position.
fn unmark(marked: &str) -> (String, Value) {
    let i = marked.find('|').expect("no | in the text");
    let before = &marked[..i];
    let line = before.matches('\n').count();
    let line_start = before.rfind('\n').map_or(0, |n| n + 1);
    let character: usize = before[line_start..].chars().map(char::len_utf16).sum();
    let text = format!("{before}{}", &marked[i + 1..]);
    (text, json!({"line": line, "character": character}))
}

fn labels(completion: &Value) -> Vec<String> {
    let mut labels: Vec<String> = completion["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["label"].as_str().unwrap().to_string())
        .collect();
    labels.sort();
    labels
}

fn hover_text(hover: &Value) -> String {
    hover["contents"]["value"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// A fresh directory for one test's program.
fn program_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-lsp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn file_uri(path: &std::path::Path) -> String {
    format!("file://{}", path.display())
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

const MAIN: &str = "import web

// A blog post.
struct Post {
    id: Int
    title: String
}

impl Post {
    // The post's address.
    fn slug(self) -> String { self.title.lower() }
}

enum Role {
    Admin
    Viewer(since: Int)
}

fn main() {
    let post = Post(id: 1, title: \"Hi\")
    print(post.title)
    print(helper(post))
    let role = Role.Admin
}
";

const OTHER: &str = "// Doubles a post's id.
fn helper(p: Post) -> Int { p.id * 2 }
";

/// Starts a server on a program with `main.kiln` and `other.kiln`, with
/// `main.kiln` open.
fn blog(name: &str) -> (Client, String, String, std::path::PathBuf) {
    let dir = program_dir(name);
    std::fs::write(dir.join("main.kiln"), MAIN).unwrap();
    std::fs::write(dir.join("other.kiln"), OTHER).unwrap();
    let main = file_uri(&dir.join("main.kiln"));
    let other = file_uri(&dir.join("other.kiln"));
    let mut client = Client::start();
    let init = client.request("initialize", json!({"rootUri": file_uri(&dir)}));
    let caps = &init["capabilities"];
    for cap in [
        "hoverProvider",
        "definitionProvider",
        "documentSymbolProvider",
        "workspaceSymbolProvider",
    ] {
        assert_eq!(caps[cap], true, "{cap}");
    }
    assert_eq!(
        caps["completionProvider"]["triggerCharacters"],
        json!(["."])
    );
    client.open(&main, MAIN);
    (client, main, other, dir)
}

/// `MAIN` with a `|` after the first `needle`, moved on by `skip` bytes.
fn mark(needle: &str, skip: usize) -> String {
    let i = MAIN.find(needle).unwrap() + skip;
    format!("{}|{}", &MAIN[..i], &MAIN[i..])
}

#[test]
fn hover_shows_types_signatures_and_doc_comments() {
    let (mut client, main, _, _) = blog("hover");
    let hover = |client: &mut Client, needle: &str, skip: usize| {
        hover_text(&client.at("textDocument/hover", &main, &mark(needle, skip)))
    };
    assert_eq!(
        hover(&mut client, "print(post", 7),
        "```kiln\npost: Post\n```"
    );
    assert_eq!(
        hover(&mut client, "let post", 5),
        "```kiln\npost: Post\n```"
    );
    assert_eq!(
        hover(&mut client, "helper(post", 2),
        "```kiln\nfn helper(p: Post) -> Int\n```\n\nDoubles a post's id."
    );
    assert_eq!(
        hover(&mut client, "post.title)", 6),
        "```kiln\nPost.title: String\n```"
    );
    assert_eq!(
        hover(&mut client, "fn slug", 4),
        "```kiln\nfn Post.slug(self) -> String\n```\n\nThe post's address."
    );
    assert!(hover(&mut client, "Post(id", 1).starts_with("```kiln\nstruct Post {\n    id: Int\n"));
    assert_eq!(
        hover(&mut client, "Role.Admin", 6),
        "```kiln\nRole.Admin\n```"
    );
    // The standard library's doc comments come along.
    assert!(hover(&mut client, "web", 1).contains("import web"));
    assert!(hover(&mut client, "lower()", 1).contains("fn String.lower(self) -> String"));
    // Nothing to say about a keyword.
    assert_eq!(
        client.at("textDocument/hover", &main, &mark("let role", 1)),
        Value::Null
    );
    client.finish();
}

#[test]
fn go_to_definition_within_and_across_files() {
    let (mut client, main, other, _) = blog("definition");
    let mut definition = |needle: &str, skip: usize| {
        client.at("textDocument/definition", &main, &mark(needle, skip))
    };
    let at = |line: u64, character: u64| json!({"line": line, "character": character});
    let local = definition("print(post", 7);
    assert_eq!(local["uri"], main);
    assert_eq!(local["range"]["start"], at(19, 8));
    let helper = definition("helper(post", 1);
    assert_eq!(helper["uri"], other);
    assert_eq!(helper["range"]["start"], at(1, 3));
    assert_eq!(definition("post.title)", 6)["range"]["start"], at(5, 4));
    assert_eq!(definition("Post(id", 0)["range"]["start"], at(3, 7));
    // A named argument of a struct goes to its field.
    assert_eq!(definition("title: \"Hi", 0)["range"]["start"], at(5, 4));
    assert_eq!(definition("Role.Admin", 6)["range"]["start"], at(14, 4));
    // The standard library has no file to go to.
    assert_eq!(definition("lower()", 1), Value::Null);
    client.finish();
}

#[test]
fn completes_in_unsaved_and_unfinished_code() {
    let (mut client, main, _, _) = blog("completion");
    let mut complete = |marked: &str| labels(&client.at("textDocument/completion", &main, marked));
    let body_end = MAIN.rfind('}').unwrap();
    let with = |line: &str| format!("{}{line}\n{}", &MAIN[..body_end], &MAIN[body_end..]);
    // Members after `.`, with the rest of the line still missing.
    assert_eq!(complete(&with("    post.|")), ["id", "slug", "title"]);
    assert_eq!(
        complete(&with("    let x = post.ti|")),
        ["id", "slug", "title"]
    );
    assert!(complete(&with("    post.title.|")).contains(&"upper".to_string()));
    assert!(complete(&with("    web.|")).contains(&"app".to_string()));
    assert_eq!(complete(&with("    Role.|")), ["Admin", "Viewer"]);
    // Names in scope, from this file and the other one.
    let names = complete(&with("    print(po|)"));
    for name in ["post", "role", "helper", "Post", "print", "web", "let"] {
        assert!(names.contains(&name.to_string()), "{name} in {names:?}");
    }
    // A local is only offered after it is bound.
    assert!(!complete(&mark("let post", 0)).contains(&"post".to_string()));
    // Types where a type is written.
    let types = complete(&with("    let y: Po|"));
    assert!(types.contains(&"Post".to_string()) && !types.contains(&"helper".to_string()));
    assert_eq!(
        complete(&with("    let y: web.Re|")),
        ["App", "Request", "Response"]
    );
    // Nothing in comments and strings.
    assert!(complete(&with("    // po|")).is_empty());
    assert!(complete(&with("    let s = \"po|\"")).is_empty());
    client.finish();
}

#[test]
fn offers_imports_for_modules_not_imported() {
    let (mut client, main, _, _) = blog("imports");
    let body_end = MAIN.rfind('}').unwrap();
    let marked = format!("{}    js|\n{}", &MAIN[..body_end], &MAIN[body_end..]);
    let completion = client.at("textDocument/completion", &main, &marked);
    let json = completion["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["label"] == "json")
        .unwrap();
    assert_eq!(json["additionalTextEdits"][0]["newText"], "import json\n");
    client.finish();
}

#[test]
fn lists_document_and_workspace_symbols() {
    let (mut client, main, other, _) = blog("symbols");
    let symbols = client.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": main}}),
    );
    let names: Vec<&str> = symbols
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["Post", "impl Post", "Role", "main"]);
    assert_eq!(symbols[0]["children"][1]["name"], "title");
    assert_eq!(symbols[1]["children"][0]["name"], "slug");
    assert_eq!(
        symbols[0]["selectionRange"]["start"],
        json!({"line": 3, "character": 7})
    );
    // Workspace symbols cover files that are not open.
    let found = client.request("workspace/symbol", json!({"query": "hlpr"}));
    assert_eq!(found[0]["name"], "helper");
    assert_eq!(found[0]["location"]["uri"], other);
    let found = client.request("workspace/symbol", json!({"query": "slug"}));
    assert_eq!(found[0]["containerName"], "Post");
    client.finish();
}

#[test]
fn understands_glazes_next_to_the_code() {
    let (mut client, _, _, dir) = blog("glaze");
    let card = file_uri(&dir.join("card.glaze"));
    let text = "{% props post: Post %}\n<h1>{post.title}</h1>\n";
    client.open(&card, text);
    let marked = "{% props post: Post %}\n<h1>{post.|}</h1>\n";
    assert_eq!(
        labels(&client.at("textDocument/completion", &card, marked)),
        ["id", "slug", "title"]
    );
    let marked = "{% props post: Post %}\n<h1>{post.ti|tle}</h1>\n";
    assert_eq!(
        hover_text(&client.at("textDocument/hover", &card, marked)),
        "```kiln\nPost.title: String\n```"
    );
    // HTML text is not code.
    let marked = "{% props post: Post %}\n<h1>ti|tle</h1>\n";
    assert!(labels(&client.at("textDocument/completion", &card, marked)).is_empty());
    client.finish();
}

#[test]
fn positions_are_utf16_and_unsaved_buffers_work() {
    let mut client = Client::start();
    client.request("initialize", json!({}));
    let uri = "untitled:Untitled-1";
    let text = "fn main() {\n    let café = \"😀\"\n    print(café)\n}\n";
    client.open(uri, text);
    let marked = "fn main() {\n    let café = \"😀\"\n    print(ca|fé)\n}\n";
    let hover = client.at("textDocument/hover", uri, marked);
    assert_eq!(hover_text(&hover), "```kiln\ncafé: String\n```");
    assert_eq!(hover["range"]["end"], json!({"line": 2, "character": 14}));
    let definition = client.at("textDocument/definition", uri, marked);
    assert_eq!(definition["uri"], uri);
    assert_eq!(
        definition["range"]["start"],
        json!({"line": 1, "character": 8})
    );
    client.finish();
}

/// Every prefix of a real program is code being typed. Requests at its end
/// must get answers, not crash the server.
#[test]
fn survives_every_unfinished_prefix() {
    let source = include_str!("../../../examples/blog/main.kiln");
    let mut client = Client::start();
    client.request("initialize", json!({}));
    let uri = "untitled:Untitled-2";
    client.open(uri, "");
    let ends: Vec<usize> = source.char_indices().map(|(i, _)| i).step_by(5).collect();
    for end in ends {
        let marked = format!("{}|", &source[..end]);
        client.at("textDocument/completion", uri, &marked);
        client.at("textDocument/hover", uri, &marked);
        client.at("textDocument/definition", uri, &marked);
    }
    client.finish();
}

/// Checks a symbol as VS Code does: it has a name, and its name lies in it.
fn check_symbol(symbol: &Value, text: &str) {
    let name = symbol["name"].as_str().unwrap();
    assert!(!name.trim().is_empty(), "a symbol with no name in {text:?}");
    let pos = |p: &Value| {
        (
            p["line"].as_u64().unwrap(),
            p["character"].as_u64().unwrap(),
        )
    };
    let (range, selection) = (&symbol["range"], &symbol["selectionRange"]);
    assert!(
        pos(&range["start"]) <= pos(&selection["start"])
            && pos(&selection["end"]) <= pos(&range["end"]),
        "{name}'s name is outside it in {text:?}"
    );
    for child in symbol["children"].as_array().into_iter().flatten() {
        check_symbol(child, text);
    }
}

#[test]
fn symbols_of_unfinished_code_have_names() {
    let dir = program_dir("unfinished-symbols");
    let mut client = Client::start();
    client.request("initialize", json!({}));
    let kiln = file_uri(&dir.join("main.kiln"));
    let glaze = file_uri(&dir.join("post.glaze"));
    client.open(&kiln, "");
    client.open(&glaze, "");
    let mut texts: Vec<(&str, String)> = [
        "fn ",
        "fn (",
        "fn main() {}\nfn ",
        "struct ",
        "struct P {\n    : Int\n}",
        "enum E {\n    ,\n}",
        "enum ",
        "impl ",
        "impl P {\n    fn \n}",
        "const ",
        "const = 1",
    ]
    .iter()
    .map(|t| (kiln.as_str(), t.to_string()))
    .collect();
    texts.extend(
        [
            "{% props %}",
            "{% props : Int %}",
            "{% state = 1 %}",
            "{% props post: %}",
        ]
        .iter()
        .map(|t| (glaze.as_str(), t.to_string())),
    );
    for (uri, source) in [
        (&kiln, include_str!("../../../examples/blog/main.kiln")),
        (&glaze, include_str!("../../../examples/blog/post.glaze")),
    ] {
        for (end, _) in source.char_indices().step_by(3) {
            texts.push((uri.as_str(), source[..end].to_string()));
        }
    }
    for (uri, text) in texts {
        client.change(uri, &text);
        let symbols = client.request(
            "textDocument/documentSymbol",
            json!({"textDocument": {"uri": uri}}),
        );
        for symbol in symbols.as_array().unwrap() {
            check_symbol(symbol, &text);
        }
    }
    client.finish();
}

#[test]
fn completes_and_explains_styles() {
    let dir = program_dir("styles");
    let css = ":root {\n    --space-md: 1rem;\n    --space-lg: 2rem;\n    --breakpoint-wide: 800px;\n}\n.theme-light { --color-accent: #c65a34; }\n.theme-dark { --color-accent: #ffb38a; }\n";
    std::fs::write(dir.join("design.css"), css).unwrap();
    let page = "{% css \"design.css\" %}\n<p ui={{ fg: color.accent }}>hi</p>\n";
    std::fs::write(dir.join("page.glaze"), page).unwrap();
    let main_text = "fn s() -> Ui {\n    return ui({ p: space.md, theme: .light })\n}\n";
    std::fs::write(dir.join("main.kiln"), main_text).unwrap();
    let main = file_uri(&dir.join("main.kiln"));
    let glaze = file_uri(&dir.join("page.glaze"));
    let mut client = Client::start();
    client.request("initialize", json!({"rootUri": file_uri(&dir)}));
    client.open(&main, main_text);
    client.open(&glaze, page);
    let in_style = |style: &str| format!("fn s() -> Ui {{\n    return ui({{ {style} }})\n}}\n");
    let mut complete =
        |style: &str| labels(&client.at("textDocument/completion", &main, &in_style(style)));
    let keys = complete("p: space.md, |");
    for key in ["p", "bg", "hover", "at", "theme"] {
        assert!(keys.contains(&key.to_string()), "{key} in {keys:?}");
    }
    assert_eq!(complete("p: space.|"), ["lg", "md"]);
    assert_eq!(complete("m: |"), [".auto", "space.lg", "space.md"]);
    assert!(complete("weight: .b|").contains(&"bold".to_string()));
    assert_eq!(complete("at: { | }"), ["wide"]);
    assert_eq!(complete("theme: .|"), ["dark", "light"]);
    // A state takes the keys but not another state.
    let hover_keys = complete("hover: { | }");
    assert!(hover_keys.contains(&"bg".to_string()) && !hover_keys.contains(&"focus".to_string()));
    let marked = "{% css \"design.css\" %}\n<p ui={{ fg: color.| }}>hi</p>\n";
    assert_eq!(
        labels(&client.at("textDocument/completion", &glaze, marked)),
        ["accent"]
    );
    // CSS Modules' classes are fields.
    std::fs::write(dir.join("card.module.css"), ".card {}\n.card-title {}\n").unwrap();
    let marked = "{% css \"design.css\" %}\n{% css module card from \"card.module.css\" %}\n<p class={card.|}>hi</p>\n";
    assert_eq!(
        labels(&client.at("textDocument/completion", &glaze, marked)),
        ["card", "card_title"]
    );
    // Hover tells what a key and a token are in CSS.
    let hover = |client: &mut Client, style: &str| {
        hover_text(&client.at("textDocument/hover", &main, &in_style(style)))
    };
    assert!(hover(&mut client, "|p: space.md").contains("padding-top, padding-right"));
    assert!(hover(&mut client, "p: space.m|d").contains("--space-md: 1rem;"));
    // Tokens and themes go to their declarations.
    let mut definition =
        |style: &str| client.at("textDocument/definition", &main, &in_style(style));
    let token = definition("p: space.m|d");
    assert_eq!(token["uri"], file_uri(&dir.join("design.css")));
    assert_eq!(token["range"]["start"], json!({"line": 1, "character": 4}));
    let theme = definition("theme: .li|ght");
    assert_eq!(theme["range"]["start"], json!({"line": 5, "character": 0}));
    client.finish();
}

#[test]
fn ember_files_support_language_features_and_component_navigation() {
    let (mut client, main, _, dir) = blog("ember");
    let counter = file_uri(&dir.join("counter.ember"));
    let page = file_uri(&dir.join("page.glaze"));
    let source = "{% props post: Post %}\n{% state count = 0 %}\n<button on:click={ count += 1 }>{post.title}: {count}</button>\n";
    std::fs::write(dir.join("counter.ember"), source).unwrap();
    client.open(&counter, source);
    let marked = source.replace("post.title", "post.ti|tle");
    assert_eq!(
        hover_text(&client.at("textDocument/hover", &counter, &marked)),
        "```kiln\nPost.title: String\n```"
    );
    let marked = source.replace("post.title", "post.|");
    assert_eq!(
        labels(&client.at("textDocument/completion", &counter, &marked)),
        ["id", "slug", "title"]
    );
    let marked = source.replace("{count}", "{co|unt}");
    let definition = client.at("textDocument/definition", &counter, &marked);
    assert_eq!(definition["uri"], counter);
    assert_eq!(definition["range"]["start"]["line"], 1);
    client.open(&page, "{% props post: Post %}<Counter post={post} />");
    let marked = "{% props post: Post %}<Cou|nter post={post} />";
    let definition = client.at("textDocument/definition", &page, marked);
    assert_eq!(definition["uri"], counter);
    assert!(hover_text(&client.at("textDocument/hover", &page, marked)).contains("counter.ember"));
    let completion = client.at(
        "textDocument/completion",
        &page,
        "{% props post: Post %}<Cou| />",
    );
    assert!(labels(&completion).contains(&"Counter".to_string()));
    // Cross-file type navigation still lands in the Kiln file.
    let definition = client.at(
        "textDocument/definition",
        &counter,
        "{% props post: Po|st %}<p>{post.title}</p>",
    );
    assert_eq!(definition["uri"], main);
    let symbols = client.request(
        "textDocument/documentSymbol",
        json!({"textDocument": {"uri": counter}}),
    );
    assert!(!symbols.as_array().unwrap().is_empty());
    client.finish();
}
