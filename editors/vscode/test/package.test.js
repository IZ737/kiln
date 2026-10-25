const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const Module = require("node:module");
const { execFileSync } = require("node:child_process");
const { root } = require("./textmate");

const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const brand = path.join(root, "..", "..", "assets", "brand");

test("every file the manifest names exists", () => {
  // `main` is the bundle, which `npm run bundle` makes; the next test
  // checks it.
  const files = [manifest.icon];
  for (const lang of manifest.contributes.languages) {
    files.push(lang.configuration, lang.icon.light, lang.icon.dark);
  }
  for (const grammar of manifest.contributes.grammars) files.push(grammar.path);
  for (const file of files) {
    assert.ok(fs.existsSync(path.join(root, file)), `${file} is missing`);
  }
});

test("the icons are copies of the brand assets", () => {
  const copies = {
    "icons/kiln.png": "logo/png/kiln-symbol-128.png",
    "icons/kiln-source.svg": "icons/svg/kiln-source.svg",
    "icons/kiln-source-dark.svg": "icons/svg/kiln-source-dark.svg",
    "icons/glaze-component.svg": "icons/svg/glaze-component.svg",
    "icons/glaze-component-dark.svg": "icons/svg/glaze-component-dark.svg",
  };
  for (const [copy, original] of Object.entries(copies)) {
    assert.ok(
      fs.readFileSync(path.join(root, copy)).equals(fs.readFileSync(path.join(brand, original))),
      `${copy} differs from assets/brand/${original}; copy it again`,
    );
  }
});

test("language configurations are valid JSON", () => {
  for (const lang of manifest.contributes.languages) {
    JSON.parse(fs.readFileSync(path.join(root, lang.configuration), "utf8"));
  }
});

test("menus only name contributed commands", () => {
  const commands = new Set(manifest.contributes.commands.map((c) => c.command));
  for (const entries of Object.values(manifest.contributes.menus)) {
    for (const entry of entries) {
      assert.ok(commands.has(entry.command), `${entry.command} is not contributed`);
    }
  }
});

test("the problem matcher reads kiln's errors", () => {
  const [first, second] = manifest.contributes.problemMatchers[0].pattern;
  const output = [
    "error[E0106]: Kiln has no `null`",
    " --> /home/ada/blog/main.kiln:2:11",
    "  |",
  ];
  const head = output[0].match(new RegExp(first.regexp));
  assert.deepStrictEqual([head[first.code], head[first.message]], ["E0106", "Kiln has no `null`"]);
  const at = output[1].match(new RegExp(second.regexp));
  assert.deepStrictEqual(
    [at[second.file], at[second.line], at[second.column]],
    ["/home/ada/blog/main.kiln", "2", "11"],
  );
  // Windows paths keep their drive letter.
  const windows = " --> C:\\blog\\main.kiln:2:11".match(new RegExp(second.regexp));
  assert.strictEqual(windows[second.file], "C:\\blog\\main.kiln");
});

test("the extension's version is kiln's", () => {
  const cargo = fs.readFileSync(path.join(root, "..", "..", "Cargo.toml"), "utf8");
  const version = /^version = "(.*)"/m.exec(cargo)[1];
  assert.strictEqual(manifest.version, version, "set the version in package.json to Cargo.toml's");
});

test("the bundle is the extension, with its libraries inside", () => {
  const script = (outfile) =>
    "esbuild src/extension.js --bundle " +
    `--outfile=${outfile} --external:vscode --format=cjs --platform=node --target=node18 --minify`;
  assert.strictEqual(manifest.scripts.bundle, script(manifest.main.replace("./", "")));
  assert.strictEqual(manifest.scripts["vscode:prepublish"], "npm run bundle");
  // Build it the same way elsewhere, and load it with only `vscode` given:
  // anything else it needs must be inside.
  const out = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "kiln-bundle-")), "extension.js");
  require("esbuild").buildSync({
    entryPoints: [path.join(root, "src", "extension.js")],
    bundle: true,
    outfile: out,
    external: ["vscode"],
    format: "cjs",
    platform: "node",
    target: "node18",
    minify: true,
    logLevel: "silent",
  });
  const original = Module._load;
  Module._load = function (request, ...rest) {
    // Each name is a class, as the client library extends some at load.
    if (request === "vscode") return new Proxy({}, { get: (t, k) => (t[k] ??= class {}) });
    assert.ok(request === out || Module.isBuiltin(request), `the bundle requires ${request}`);
    return original.call(this, request, ...rest);
  };
  try {
    const extension = require(out);
    assert.strictEqual(typeof extension.activate, "function");
    assert.strictEqual(typeof extension.deactivate, "function");
  } finally {
    Module._load = original;
  }
});

test("the package holds only what VS Code loads", () => {
  const vsce = require.resolve("@vscode/vsce/vsce");
  const files = execFileSync(process.execPath, [vsce, "ls"], { cwd: root, encoding: "utf8" })
    .split("\n")
    .filter(Boolean);
  for (const file of files) {
    assert.ok(!/^(node_modules|src|test|\.vscode)\//.test(file), `${file} is packaged`);
  }
  assert.ok(files.length < 20, `${files.length} files are packaged`);
});

test("the word pattern finds names as VS Code compiles it", () => {
  // VS Code adds only the `g` flag, so the pattern cannot rely on `u`
  // (`\p{L}` without it matches the letters p and L). Completion filters
  // its list by the word at the cursor, which this pattern finds.
  const config = JSON.parse(fs.readFileSync(path.join(root, "language-configuration.json"), "utf8"));
  const words = (text) => text.match(new RegExp(config.wordPattern, "g"));
  assert.deepStrictEqual(words("print(tes) -> Str"), ["print", "tes", "Str"]);
  assert.deepStrictEqual(words("let café_2 = x.len"), ["let", "café_2", "x", "len"]);
  assert.deepStrictEqual(words("sleep(250ms) + 1_000"), ["sleep", "250ms", "1_000"]);
});
