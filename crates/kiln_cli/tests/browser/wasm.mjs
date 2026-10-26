// Runs programs built with `kiln build --target wasm` in a real browser
// (Chromium, through Playwright) and prints what each wrote, as JSON:
// `node wasm.mjs a.mjs b.mjs ...` prints `[{ out, err, code }, ...]`.
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";

const { chromium } = createRequire(import.meta.url)("playwright");

const browser = await chromium.launch();
const results = [];
try {
  for (const file of process.argv.slice(2)) {
    const page = await browser.newPage();
    const errors = [];
    page.on("pageerror", (e) => errors.push(String(e)));
    await page.goto("about:blank");
    const script = readFileSync(file, "utf8").replace(
      /await runMain\(KILN_MODULE\);\s*$/,
      "window.__kiln = await runCollect(KILN_MODULE);",
    );
    await page.addScriptTag({ type: "module", content: script });
    try {
      await page.waitForFunction(() => window.__kiln !== undefined, null, { timeout: 20000 });
      results.push(await page.evaluate(() => window.__kiln));
    } catch (e) {
      results.push({ out: "", err: errors.join("\n") || String(e), code: -1 });
    }
    await page.close();
  }
} finally {
  await browser.close();
}
console.log(JSON.stringify(results));
