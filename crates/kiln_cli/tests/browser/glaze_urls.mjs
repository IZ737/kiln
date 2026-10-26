// Check the browser's URL parsing, including HTML entity decoding.
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { readFileSync } from "node:fs";

const { chromium } = createRequire(import.meta.url)("playwright");
const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  await page.setContent(readFileSync(process.argv[2], "utf8"));
  const links = await page.locator("a").evaluateAll((anchors) =>
    anchors.map((a) => ({ text: a.textContent, href: a.getAttribute("href"), protocol: a.protocol })),
  );
  for (const link of links) {
    if (link.text === "static") {
      assert.equal(link.protocol, "javascript:");
    } else {
      assert.notEqual(link.protocol, "javascript:", link.text);
    }
  }
  for (const name of ["hex", "decimal", "named", "controls", "colon", "expression", "multiple entities", "leading control"]) {
    assert.equal(links.find((a) => a.text === name)?.href, "about:invalid#kiln-unsafe-url", name);
  }
  assert.equal(links.find((a) => a.text === "relative")?.href, "/posts/42");
  assert.equal(links.find((a) => a.text === "encoded relative")?.href, "/posts/42");
  assert.equal(links.find((a) => a.text === "encoded https")?.href, "https://example.com/42");
  assert.equal(links.find((a) => a.text === "query")?.href, "?q=42&page=1");
  assert.equal(links.find((a) => a.text === "dynamic entity")?.href, "java&#x73;cript:alert(1)");
  assert.equal(links.find((a) => a.text === "decode once")?.href, "&#x6a;avacript:alert(1)");
  let dialogs = 0;
  page.on("dialog", async (dialog) => { dialogs++; await dialog.dismiss(); });
  await page.getByText("hex", { exact: true }).click();
  assert.equal(dialogs, 0, "the sanitized link must not execute JavaScript");
} finally {
  await browser.close();
}
