const test = require("node:test");
const assert = require("node:assert");
const { findKiln } = require("../src/find");

const linux = (files, extra = {}) => ({
  env: { PATH: "/usr/bin:/opt/kiln/bin" },
  platform: "linux",
  home: "/home/ada",
  exists: (f) => files.includes(f),
  ...extra,
});

test("finds kiln on the PATH first", () => {
  const found = findKiln(linux(["/opt/kiln/bin/kiln", "/home/ada/.kiln/bin/kiln"]));
  assert.strictEqual(found.path, "/opt/kiln/bin/kiln");
});

test("falls back to where the installers put kiln", () => {
  const found = findKiln(linux(["/home/ada/.kiln/bin/kiln"]));
  assert.strictEqual(found.path, "/home/ada/.kiln/bin/kiln");
});

test("the kiln.path setting wins, with ~ for the home folder", () => {
  const found = findKiln(linux(["/home/ada/src/kiln", "/opt/kiln/bin/kiln"], { configured: "~/src/kiln" }));
  assert.strictEqual(found.path, "/home/ada/src/kiln");
  // A setting that names nothing is reported, not silently replaced.
  const missing = findKiln(linux(["/opt/kiln/bin/kiln"], { configured: "/nope/kiln" }));
  assert.strictEqual(missing.path, null);
  assert.deepStrictEqual(missing.tried, ["/nope/kiln"]);
});

test("says where it looked when kiln is nowhere", () => {
  const found = findKiln(linux([]));
  assert.strictEqual(found.path, null);
  assert.deepStrictEqual(found.tried, ["/usr/bin/kiln", "/opt/kiln/bin/kiln", "/home/ada/.kiln/bin/kiln"]);
});

test("looks for kiln.exe on Windows", () => {
  const found = findKiln({
    env: { Path: "C:\\Windows;C:\\Tools" },
    platform: "win32",
    home: "C:\\Users\\ada",
    exists: (f) => f === "C:\\Users\\ada\\.kiln\\bin\\kiln.exe",
  });
  assert.strictEqual(found.path, "C:\\Users\\ada\\.kiln\\bin\\kiln.exe");
  assert.strictEqual(found.tried[0], "C:\\Windows\\kiln.exe");
});
