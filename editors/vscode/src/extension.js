// The Kiln extension: starts `kiln lsp` for language features, and runs
// `kiln check`, `kiln run` and `kiln build` as tasks. Everything a language
// server can do lives in `kiln lsp`, so other editors get it too.
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const vscode = require("vscode");
const { LanguageClient, State } = require("vscode-languageclient/node");
const { findKiln } = require("./find");

const INSTALL_URL = "https://github.com/IZ737/kiln#install";
const COMMANDS = ["check", "run", "build"];

/** @type {LanguageClient | undefined} */
let client;
/** @type {vscode.OutputChannel} */
let output;

/// The `kiln` executable to use, or `undefined` after telling the user how
/// to get one.
function kilnPath({ quiet = false } = {}) {
  const configured = vscode.workspace.getConfiguration("kiln").get("path");
  const found = findKiln({
    configured,
    env: process.env,
    platform: process.platform,
    home: os.homedir(),
    exists: (file) => {
      try {
        return fs.statSync(file).isFile();
      } catch {
        return false;
      }
    },
  });
  if (found.path) return found.path;
  output.appendLine(`kiln was not found. Looked in:\n  ${found.tried.join("\n  ")}`);
  if (!quiet) {
    const message = configured
      ? `The kiln.path setting names ${configured}, which does not exist.`
      : "The Kiln extension needs the kiln command, and it is not on your PATH or in ~/.kiln/bin.";
    vscode.window
      .showErrorMessage(message, "Install Kiln", "Set kiln.path")
      .then((choice) => {
        if (choice === "Install Kiln") {
          vscode.env.openExternal(vscode.Uri.parse(INSTALL_URL));
        } else if (choice === "Set kiln.path") {
          vscode.commands.executeCommand("workbench.action.openSettings", "kiln.path");
        }
      });
  }
  return undefined;
}

async function startClient() {
  const command = kilnPath();
  if (!command) return;
  output.appendLine(`Starting ${command} lsp`);
  client = new LanguageClient(
    "kiln",
    "Kiln",
    { command, args: ["lsp"] },
    {
      documentSelector: [
        { scheme: "file", language: "kiln" },
        { scheme: "file", language: "glaze" },
        { scheme: "untitled", language: "kiln" },
        { scheme: "untitled", language: "glaze" },
      ],
      outputChannel: output,
    },
  );
  try {
    await client.start();
  } catch (err) {
    vscode.window.showErrorMessage(`kiln lsp did not start: ${err.message ?? err}`);
  }
}

async function stopClient() {
  if (client && client.state !== State.Stopped) {
    await client.stop();
  }
  client = undefined;
}

async function restartClient() {
  await stopClient();
  await startClient();
}

/// The program folder of a file: a program is all of a folder's `.kiln`
/// files.
function programOf(uri) {
  if (uri && uri.scheme === "file") {
    return fs.statSync(uri.fsPath).isDirectory() ? uri.fsPath : path.dirname(uri.fsPath);
  }
  return undefined;
}

/// A task that runs `kiln <command>` on the program in `dir`, with `kiln`
/// errors shown in the Problems panel.
function kilnTask(command, dir, kiln) {
  const folder = vscode.workspace.getWorkspaceFolder(vscode.Uri.file(dir));
  const definition = { type: "kiln", command, program: dir };
  const execution = new vscode.ProcessExecution(kiln, [command, dir], { cwd: dir });
  const name = `${command} ${folder ? path.relative(folder.uri.fsPath, dir) || "." : dir}`;
  const task = new vscode.Task(
    definition,
    folder ?? vscode.TaskScope.Workspace,
    name,
    "kiln",
    execution,
    command === "run" ? [] : ["$kiln"],
  );
  if (command === "build") task.group = vscode.TaskGroup.Build;
  task.presentationOptions = { reveal: vscode.TaskRevealKind.Always, clear: true };
  return task;
}

/// Runs `kiln <command>` on the program of `uri`, or of the active editor.
async function runCommand(command, uri) {
  const target = uri instanceof vscode.Uri ? uri : vscode.window.activeTextEditor?.document.uri;
  const dir = programOf(target);
  if (!dir) {
    vscode.window.showErrorMessage(`Open a .kiln file to ${command} its program.`);
    return;
  }
  const kiln = kilnPath();
  if (!kiln) return;
  // kiln reads the files from disk, so unsaved changes in the program
  // must be saved first.
  for (const doc of vscode.workspace.textDocuments) {
    if (doc.isDirty && doc.uri.scheme === "file" && path.dirname(doc.uri.fsPath) === dir) {
      await doc.save();
    }
  }
  await vscode.tasks.executeTask(kilnTask(command, dir, kiln));
}

const taskProvider = {
  provideTasks() {
    const dir = programOf(vscode.window.activeTextEditor?.document.uri);
    const kiln = dir && kilnPath({ quiet: true });
    return kiln ? COMMANDS.map((c) => kilnTask(c, dir, kiln)) : [];
  },
  resolveTask(task) {
    const { command, program } = task.definition;
    const kiln = kilnPath();
    if (!COMMANDS.includes(command) || !kiln) return undefined;
    const folder = task.scope && task.scope.uri ? task.scope.uri.fsPath : undefined;
    const dir = program ? path.resolve(folder ?? "", program) : folder;
    return dir ? kilnTask(command, dir, kiln) : undefined;
  },
};

async function activate(context) {
  output = vscode.window.createOutputChannel("Kiln");
  context.subscriptions.push(
    output,
    ...COMMANDS.map((c) => vscode.commands.registerCommand(`kiln.${c}`, (uri) => runCommand(c, uri))),
    vscode.commands.registerCommand("kiln.restartServer", restartClient),
    vscode.commands.registerCommand("kiln.showOutput", () => output.show()),
    vscode.tasks.registerTaskProvider("kiln", taskProvider),
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (e.affectsConfiguration("kiln.path")) restartClient();
    }),
  );
  await startClient();
  // For tests and other extensions: the client, once started.
  return { client: () => client };
}

function deactivate() {
  return stopClient();
}

module.exports = { activate, deactivate, programOf };
