const test = require("node:test");
const assert = require("node:assert");
const fs = require("node:fs");
const path = require("node:path");
const { root } = require("./textmate");

const manifest = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const brand = path.join(root, "..", "..", "assets", "brand");

test("every file the manifest names exists", () => {
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
