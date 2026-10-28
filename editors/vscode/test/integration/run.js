// Starts a real VS Code with the extension and runs suite.js in it:
// `KILN_PATH=path/to/kiln npm run test:integration`. It downloads VS Code
// the first time, and needs a display (CI runs it under xvfb-run).
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { runTests } = require("@vscode/test-electron");

async function main() {
  const kiln = process.env.KILN_PATH;
  if (!kiln || !fs.existsSync(kiln)) {
    throw new Error("set KILN_PATH to a kiln executable");
  }
  const workspace = fs.mkdtempSync(path.join(os.tmpdir(), "kiln-vscode-"));
  fs.writeFileSync(
    path.join(workspace, "main.kiln"),
    `// A blog post.
struct Post {
    id: Int
    title: String
}

fn main() {
    let post = Post(id: 1, title: "Hi")
    print(helper(post))
}
`,
  );
  fs.writeFileSync(
    path.join(workspace, "other.kiln"),
    "// Doubles a post's id.\nfn helper(p: Post) -> Int { p.id * 2 }\n",
  );
  fs.writeFileSync(path.join(workspace, "card.glaze"), "{% props post: Post %}\n<h1>{post.title}</h1>\n");
  fs.writeFileSync(path.join(workspace, "counter.ember"), "{% props post: Post %}\n{% state count = 0 %}\n<button on:click={ count += 1 }>{post.title}: {count}</button>\n");
  fs.writeFileSync(path.join(workspace, "page.glaze"), "{% props post: Post %}<Counter post={post} />\n");
  // kiln.toml and kiln.lock, in a folder of their own so the program
  // above stays manifest-free, beside an ordinary TOML file.
  fs.mkdirSync(path.join(workspace, "config"));
  fs.writeFileSync(path.join(workspace, "config", "kiln.toml"), '[package]\nname = "demo"\n');
  fs.writeFileSync(path.join(workspace, "config", "kiln.lock"), "# kiln.lock\nversion = 1\n");
  fs.writeFileSync(path.join(workspace, "config", "other.toml"), "key = 1\n");
  // GUI shells can replace PATH; test the configured executable deterministically.
  fs.mkdirSync(path.join(workspace, ".vscode"));
  fs.writeFileSync(path.join(workspace, ".vscode", "settings.json"), JSON.stringify({ "kiln.path": path.resolve(kiln) }));
  await runTests({
    extensionDevelopmentPath: path.join(__dirname, "..", ".."),
    extensionTestsPath: path.join(__dirname, "suite.js"),
    launchArgs: [workspace, "--disable-extensions", "--skip-welcome", "--skip-release-notes"],
    // kiln is found on the PATH, as for a user who installed it.
    extensionTestsEnv: {
      PATH: `${path.dirname(path.resolve(kiln))}${path.delimiter}${process.env.PATH}`,
    },
  });
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
