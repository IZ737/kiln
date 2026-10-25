// Runs inside VS Code (see run.js): opens a small Kiln program and checks
// that the extension's features work end to end through kiln lsp.
const assert = require("node:assert");
const path = require("node:path");
const vscode = require("vscode");

/// Waits until `check` returns something truthy.
async function until(what, check, timeout = 30000) {
  const start = Date.now();
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() - start > timeout) throw new Error(`timed out waiting for ${what}`);
    await new Promise((r) => setTimeout(r, 200));
  }
}

function positionOf(doc, needle, skip = 0) {
  return doc.positionAt(doc.getText().indexOf(needle) + skip);
}

async function run() {
  const folder = vscode.workspace.workspaceFolders[0].uri;
  const mainUri = vscode.Uri.joinPath(folder, "main.kiln");
  const doc = await vscode.workspace.openTextDocument(mainUri);
  await vscode.window.showTextDocument(doc);
  assert.strictEqual(doc.languageId, "kiln");
  const glaze = await vscode.workspace.openTextDocument(vscode.Uri.joinPath(folder, "card.glaze"));
  assert.strictEqual(glaze.languageId, "glaze");

  const extension = vscode.extensions.getExtension("iz737.kiln");
  const api = await extension.activate();
  await until("kiln lsp to start", () => api.client() && api.client().isRunning());

  // Hover, with the doc comment from the other file.
  const hovers = await until("a hover", async () => {
    const h = await vscode.commands.executeCommand(
      "vscode.executeHoverProvider",
      mainUri,
      positionOf(doc, "helper(post", 2),
    );
    return h.length > 0 && h;
  });
  const text = hovers[0].contents.map((c) => (typeof c === "string" ? c : c.value)).join("\n");
  assert.match(text, /fn helper\(p: Post\) -> Int/);
  assert.match(text, /Doubles a post's id\./);

  // Go to definition, into the other file.
  const definitions = await vscode.commands.executeCommand(
    "vscode.executeDefinitionProvider",
    mainUri,
    positionOf(doc, "helper(post", 2),
  );
  const target = definitions[0].targetUri ?? definitions[0].uri;
  assert.strictEqual(path.basename(target.fsPath), "other.kiln");

  // The outline.
  const symbols = await vscode.commands.executeCommand("vscode.executeDocumentSymbolProvider", mainUri);
  assert.deepStrictEqual(
    symbols.map((s) => s.name),
    ["Post", "main"],
  );

  // Diagnostics and completion on unsaved, unfinished text.
  const editor = vscode.window.activeTextEditor;
  const end = positionOf(doc, "    print(helper(post))").translate(1, 0);
  await editor.edit((e) => e.insert(end, "    let x = null\n    post.\n"));
  const diagnostics = await until("the null error", () => {
    const d = vscode.languages.getDiagnostics(mainUri);
    return d.find((x) => String(x.code?.value ?? x.code) === "E0106") || false;
  });
  assert.match(diagnostics.message, /null/);
  const list = await vscode.commands.executeCommand(
    "vscode.executeCompletionItemProvider",
    mainUri,
    positionOf(doc, "    post.", 9),
    ".",
  );
  const labels = list.items.map((i) => (typeof i.label === "string" ? i.label : i.label.label));
  assert.ok(labels.includes("title") && labels.includes("id"), labels.join(", "));
  // VS Code filters completions by the word at the cursor, found with the
  // language's word pattern.
  const word = doc.getWordRangeAtPosition(positionOf(doc, "    let x", 9));
  assert.strictEqual(word && doc.getText(word), "x");
  const name = doc.getWordRangeAtPosition(positionOf(doc, "print(helper", 10));
  assert.strictEqual(name && doc.getText(name), "helper");
  await vscode.commands.executeCommand("undo");
  await doc.save();

  // Kiln: Check Program runs `kiln check` as a task that succeeds.
  const finished = new Promise((resolve) => {
    const listener = vscode.tasks.onDidEndTaskProcess((e) => {
      if (e.execution.task.source === "kiln") {
        listener.dispose();
        resolve(e.exitCode);
      }
    });
  });
  await vscode.commands.executeCommand("kiln.check", mainUri);
  assert.strictEqual(await finished, 0);
}

module.exports = { run };
