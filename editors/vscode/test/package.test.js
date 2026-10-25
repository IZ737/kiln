const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const { root } = require("./textmate");

const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const brand = path.join(root, "..", "..", "assets", "brand");

test("every file the manifest names exists", () => {
  const files = [manifest.icon, manifest.main];
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
