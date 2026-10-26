// Drives `kiln lsp` the way VS Code does, with no VS Code: run it as
// `node test/lsp-smoke.js path/to/kiln`. CI runs it on Linux and Windows,
// where file URIs look different (`file:///c%3A/...`).
const assert = require("node:assert");
const { spawn } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { pathToFileURL, fileURLToPath } = require("node:url");

const kiln = process.argv[2] && path.resolve(process.argv[2]);
if (!kiln) {
  console.error("usage: node test/lsp-smoke.js path/to/kiln");
  process.exit(2);
}

/// The URI VS Code sends for a path: on Windows the drive letter is lower
/// case and its colon escaped.
function vscodeUri(file) {
  const uri = pathToFileURL(file).href;
  return uri.replace(/^file:\/\/\/([A-Za-z]):/, (_, d) => `file:///${d.toLowerCase()}%3A`);
}

function startServer() {
  const child = spawn(kiln, ["lsp"], { stdio: ["pipe", "pipe", "inherit"] });
  let buffer = Buffer.alloc(0);
  const waiting = new Map();
  child.stdout.on("data", (chunk) => {
    buffer = Buffer.concat([buffer, chunk]);
    for (;;) {
      const end = buffer.indexOf("\r\n\r\n");
      if (end < 0) return;
      const length = Number(/Content-Length: (\d+)/i.exec(buffer.slice(0, end).toString())[1]);
      if (buffer.length < end + 4 + length) return;
      const message = JSON.parse(buffer.slice(end + 4, end + 4 + length).toString());
      buffer = buffer.slice(end + 4 + length);
      if (message.id !== undefined && waiting.has(message.id)) {
        waiting.get(message.id)(message);
        waiting.delete(message.id);
      }
    }
  });
  let nextId = 0;
  const send = (message) => {
    const body = JSON.stringify({ jsonrpc: "2.0", ...message });
    child.stdin.write(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);
  };
  return {
    child,
    notify: (method, params) => send({ method, params }),
    request: (method, params) =>
      new Promise((resolve, reject) => {
        const id = ++nextId;
        const timer = setTimeout(() => reject(new Error(`${method} timed out`)), 20000);
        waiting.set(id, (m) => {
          clearTimeout(timer);
          m.error ? reject(new Error(`${method}: ${m.error.message}`)) : resolve(m.result);
        });
        send({ id, method, params });
      }),
  };
}

const MAIN = `// A blog post.
struct Post {
    id: Int
    title: String
}

fn main() {
    let post = Post(id: 1, title: "Hi")
    print(helper(post))
}
`;
const OTHER = `// Doubles a post's id.
fn helper(p: Post) -> Int { p.id * 2 }
`;

function positionOf(text, needle, skip = 0) {
  const i = text.indexOf(needle) + skip;
  const before = text.slice(0, i);
  return { line: before.split("\n").length - 1, character: i - (before.lastIndexOf("\n") + 1) };
}

async function main() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "kiln-lsp-smoke-"));
  fs.writeFileSync(path.join(dir, "main.kiln"), MAIN);
  fs.writeFileSync(path.join(dir, "other.kiln"), OTHER);
  const main = vscodeUri(path.join(dir, "main.kiln"));
  const server = startServer();
  const init = await server.request("initialize", {
    processId: process.pid,
    rootUri: vscodeUri(dir),
    workspaceFolders: [{ uri: vscodeUri(dir), name: "smoke" }],
    capabilities: {},
  });
  assert.strictEqual(init.capabilities.hoverProvider, true);
  server.notify("initialized", {});
  server.notify("textDocument/didOpen", {
    textDocument: { uri: main, languageId: "kiln", version: 1, text: MAIN },
  });
  const doc = { uri: main };

  const hover = await server.request("textDocument/hover", {
    textDocument: doc,
    position: positionOf(MAIN, "helper(post", 2),
  });
  assert.match(hover.contents.value, /fn helper\(p: Post\) -> Int/);
  assert.match(hover.contents.value, /Doubles a post's id\./);

  const definition = await server.request("textDocument/definition", {
    textDocument: doc,
    position: positionOf(MAIN, "helper(post", 2),
  });
  // Windows' TEMP can contain an 8.3 name (RUNNER~1), while the LSP
  // canonicalizes it to the long path. Use the OS resolver for both.
  assert.strictEqual(
    fs.realpathSync.native(fileURLToPath(definition.uri)),
    fs.realpathSync.native(path.join(dir, "other.kiln")),
  );
  assert.deepStrictEqual(definition.range.start, { line: 1, character: 3 });

  const edited = MAIN.replace("    print(helper(post))", "    print(helper(post))\n    post.");
  server.notify("textDocument/didChange", {
    textDocument: { uri: main, version: 2 },
    contentChanges: [{ text: edited }],
  });
  const completion = await server.request("textDocument/completion", {
    textDocument: doc,
    position: positionOf(edited, "    post.", 9),
  });
  const labels = completion.items.map((i) => i.label).sort();
  assert.deepStrictEqual(labels, ["id", "title"]);

  const symbols = await server.request("workspace/symbol", { query: "helper" });
  assert.strictEqual(symbols[0].name, "helper");
  assert.ok(symbols[0].location.uri.endsWith("other.kiln"));

  // A new unsaved file must share the canonical project identity too.
  const fresh = vscodeUri(path.join(dir, "fresh.kiln"));
  server.notify("textDocument/didOpen", {
    textDocument: {
      uri: fresh, languageId: "kiln", version: 1,
      text: "fn fresh(p: Post) -> Int { p.id }",
    },
  });
  const unsaved = MAIN.replace("helper(post)", "fresh(post)");
  server.notify("textDocument/didChange", {
    textDocument: { uri: main, version: 3 },
    contentChanges: [{ text: unsaved }],
  });
  const freshDefinition = await server.request("textDocument/definition", {
    textDocument: doc,
    position: positionOf(unsaved, "fresh(post", 2),
  });
  assert.strictEqual(freshDefinition.uri, fresh);

  await server.request("shutdown", null);
  server.notify("exit", null);
  const code = await new Promise((resolve) => server.child.on("exit", resolve));
  assert.strictEqual(code, 0);
  console.log("kiln lsp smoke test passed");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
