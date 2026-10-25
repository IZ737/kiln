// Activates the extension against a stand-in for the `vscode` module, to
// check what it registers and runs without starting VS Code.
const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const Module = require("node:module");

/// A fake `vscode` that records what the extension does.
function fakeVscode(settings) {
  const log = { commands: {}, errors: [], tasks: [], providers: [], clients: [] };
  class Uri {
    constructor(fsPath) {
      this.scheme = "file";
      this.fsPath = fsPath;
    }
    static file(p) {
      return new Uri(p);
    }
    static parse(s) {
      return new Uri(s);
    }
  }
  const vscode = {
    Uri,
    window: {
      createOutputChannel: () => ({ appendLine() {}, show() {}, dispose() {} }),
      showErrorMessage: (message) => {
        log.errors.push(message);
        return Promise.resolve(undefined);
      },
      activeTextEditor: undefined,
    },
    workspace: {
      getConfiguration: () => ({ get: (key) => settings[key] }),
      getWorkspaceFolder: () => undefined,
      onDidChangeConfiguration: () => ({ dispose() {} }),
      textDocuments: [],
    },
    commands: {
      registerCommand: (name, fn) => {
        log.commands[name] = fn;
        return { dispose() {} };
      },
      executeCommand() {},
    },
    env: { openExternal() {} },
    tasks: {
      registerTaskProvider: (type, provider) => {
        log.providers.push([type, provider]);
        return { dispose() {} };
      },
      executeTask: async (task) => log.tasks.push(task),
    },
    ProcessExecution: class {
      constructor(process, args, options) {
        Object.assign(this, { process, args, options });
      }
    },
    Task: class {
      constructor(definition, scope, name, source, execution, matchers) {
        Object.assign(this, { definition, scope, name, source, execution, matchers });
      }
    },
    TaskScope: { Workspace: 2 },
    TaskGroup: { Build: "build" },
    TaskRevealKind: { Always: 1 },
  };
  const client = {
    LanguageClient: class {
      constructor(id, name, server, options) {
        Object.assign(this, { id, server, options, state: 2 });
        log.clients.push(this);
      }
      async start() {}
      async stop() {
        this.state = 1;
      }
    },
    State: { Stopped: 1, Running: 2 },
  };
  return { vscode, client, log };
}

function load(settings) {
  const fake = fakeVscode(settings);
  const original = Module._load;
  Module._load = function (request, ...rest) {
    if (request === "vscode") return fake.vscode;
    if (request === "vscode-languageclient/node") return fake.client;
    return original.call(this, request, ...rest);
  };
  const file = require.resolve("../src/extension");
  delete require.cache[file];
  try {
    return { extension: require(file), ...fake };
  } finally {
    Module._load = original;
  }
}

const context = () => ({ subscriptions: [] });

test("starts kiln lsp and runs commands as kiln tasks", async () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "kiln-ext-"));
  const kiln = path.join(dir, "kiln");
  fs.writeFileSync(kiln, "");
  const program = path.join(dir, "blog");
  fs.mkdirSync(program);
  fs.writeFileSync(path.join(program, "main.kiln"), "fn main() {}\n");
  const { extension, vscode, log } = load({ path: kiln });
  await extension.activate(context());
  assert.strictEqual(log.clients.length, 1);
  assert.deepStrictEqual(log.clients[0].server, { command: kiln, args: ["lsp"] });
  for (const c of ["kiln.check", "kiln.run", "kiln.build", "kiln.restartServer", "kiln.showOutput"]) {
    assert.ok(log.commands[c], `${c} is registered`);
  }
  await log.commands["kiln.build"](vscode.Uri.file(path.join(program, "main.kiln")));
  const task = log.tasks[0];
  assert.deepStrictEqual(task.execution.args, ["build", program]);
  assert.strictEqual(task.execution.process, kiln);
  assert.strictEqual(task.execution.options.cwd, program);
  assert.deepStrictEqual(task.matchers, ["$kiln"]);
  // `run` output is the program's, not kiln errors.
  await log.commands["kiln.run"](vscode.Uri.file(program));
  assert.deepStrictEqual(log.tasks[1].execution.args, ["run", program]);
  assert.deepStrictEqual(log.tasks[1].matchers, []);
  // A task from tasks.json resolves to the same command.
  const [, provider] = log.providers[0];
  const resolved = provider.resolveTask({
    definition: { type: "kiln", command: "check", program: "blog" },
    scope: { uri: vscode.Uri.file(dir) },
  });
  assert.deepStrictEqual(resolved.execution.args, ["check", program]);
  await extension.deactivate();
});

test("explains how to get kiln when it is missing", async () => {
  const { extension, log } = load({ path: path.join(os.tmpdir(), "no-such-kiln") });
  await extension.activate(context());
  assert.strictEqual(log.clients.length, 0);
  assert.match(log.errors[0], /kiln\.path setting names .*no-such-kiln/);
  // Commands still answer, with what to do.
  await log.commands["kiln.check"]();
  assert.match(log.errors[1], /Open a \.kiln file/);
});
