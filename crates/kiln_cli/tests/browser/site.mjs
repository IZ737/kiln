// Drives the documentation site's embers in Chromium: `node site.mjs
// <url>`. Prints a JSON object: the checks that failed, and how long the
// search took.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const base = process.argv[2];
const failures = [];
const metrics = {};

function check(name, ok, detail = "") {
  if (!ok) failures.push(detail === "" ? name : `${name}: ${detail}`);
}

async function eventually(name, f, timeout = 5000) {
  const end = Date.now() + timeout;
  let last;
  while (Date.now() < end) {
    try {
      last = await f();
      if (last === true) return true;
    } catch (e) {
      last = e.message;
    }
    await new Promise((r) => setTimeout(r, 25));
  }
  check(name, false, `still ${JSON.stringify(last)}`);
  return false;
}

const browser = await chromium.launch();
try {
  const context = await browser.newContext();
  await context.grantPermissions(["clipboard-read", "clipboard-write"], { origin: base });
  const page = await context.newPage();
  const logs = [];
  page.on("console", (m) => {
    if (m.type() === "error" || m.type() === "warning") logs.push(m.text());
  });
  page.on("pageerror", (e) => logs.push(e.message));

  // ----- the search box asks the server as you type -----
  await page.goto(`${base}/docs/`);
  await eventually("the search box hydrates", async () =>
    (await page.locator(".search").evaluate((el) => el.closest("kiln-island")?.hasAttribute("data-k-ready"))) === true,
  );
  const started = Date.now();
  await page.locator("#search-q").pressSequentially("glaze", { delay: 20 });
  await eventually("typing shows the pages that match", async () =>
    (await page.locator(".search-hits a").allTextContents()).some((t) => t.includes("Glaze")),
  );
  metrics.search_ms = Date.now() - started;
  check("the input keeps what was typed", (await page.locator("#search-q").inputValue()) === "glaze");
  check("the input keeps focus", await page.locator("#search-q").evaluate((el) => el === document.activeElement));
  await page.locator("#search-q").fill("");
  await page.locator("#search-q").pressSequentially("xyzzyq");
  await eventually("no match says so", async () => (await page.locator(".search-none").count()) === 1);
  await page.locator("#search-q").fill("");
  await page.locator("#search-q").dispatchEvent("input");
  await eventually("an empty box shows nothing", async () => (await page.locator(".search-hits").count()) === 0);

  // ----- the copy buttons copy the install commands -----
  await page.goto(`${base}/install`);
  const button = page.locator(".copy-button").first();
  await button.scrollIntoViewIfNeeded();
  await eventually("the copy button hydrates when it is seen", async () =>
    (await button.evaluate((el) => el.closest("kiln-island")?.hasAttribute("data-k-ready"))) === true,
  );
  await button.click();
  await eventually("the button says it copied", async () => (await button.textContent()) === "Copied");
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  check("the command is on the clipboard", copied.startsWith("curl --proto '=https'"), copied);

  const versioned = page.locator(".copy-button").nth(1);
  await versioned.scrollIntoViewIfNeeded();
  await eventually("the versioned install command hydrates", async () =>
    (await versioned.evaluate((el) => el.closest("kiln-island")?.hasAttribute("data-k-ready"))) === true,
  );
  await versioned.click();
  await eventually("the versioned command keeps shell quotes", async () =>
    (await page.evaluate(() => navigator.clipboard.readText())).endsWith('--version v0.9.0 --dir "$HOME/.kiln"'),
  );

  check("the site logged nothing", logs.length === 0, logs.join("\n"));
} catch (e) {
  failures.push(`the test stopped: ${e.stack}`);
} finally {
  await browser.close();
}
console.log(JSON.stringify({ failures, metrics }));
