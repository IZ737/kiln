// Drives the app in `stores/` in Chromium: `node stores.mjs <url>`. Two
// unrelated embers share the shop's cart; prints a JSON object with the
// checks that failed.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const base = process.argv[2];
const failures = [];
const metrics = {};

function check(name, ok, detail = "") {
  if (!ok) failures.push(detail === "" ? name : `${name}: ${detail}`);
}

async function eventually(name, f, timeout = 3000) {
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

const text = (page, selector) => page.locator(selector).first().textContent();
const badge = (page) => text(page, "#badge");

async function ready(page) {
  await page.waitForFunction(
    () => [...document.querySelectorAll("kiln-island")].every((el) => el.hasAttribute("data-k-ready")),
  );
}

const browser = await chromium.launch();
try {
  const context = await browser.newContext();
  const page = await context.newPage();
  const logs = [];
  page.on("console", (m) => logs.push(`${m.type()}: ${m.text()}`));
  page.on("pageerror", (e) => logs.push(`pageerror: ${e.message}`));
  await page.goto(base);
  await ready(page);
  check("the server renders the store", (await badge(page)) === "guest: 0 items", await badge(page));

  // ----- two embers, one store -----
  // The badge's text changes once per click, though the handler writes
  // twice (the cart, and its own count).
  await page.evaluate(() => {
    window.badgeChanges = 0;
    new MutationObserver(() => window.badgeChanges++).observe(document.querySelector("#badge"), {
      childList: true,
      characterData: true,
      subtree: true,
    });
  });
  await page.click('button.add[data-item="tea"]');
  await eventually("another ember's change shows in the badge", async () => (await badge(page)) === "guest: 1 items");
  check("one click renders the badge once", (await page.evaluate(() => window.badgeChanges)) === 1,
    String(await page.evaluate(() => window.badgeChanges)));
  check("the button keeps its own count", (await text(page, '.clicks[data-item="tea"]')) === "1");
  await page.click('button.add[data-item="tea"]');
  await page.click('button.add[data-item="cake"]');
  await eventually("deep changes to the cart show", async () => (await badge(page)) === "guest: 3 items");
  check("each button's count is its own", (await text(page, '.clicks[data-item="tea"]')) === "2" &&
    (await text(page, '.clicks[data-item="cake"]')) === "1");

  // Typing in one ember updates another, keeping the focus and the text.
  const note = page.locator("#note");
  await note.click();
  await note.pressSequentially("gift", { delay: 10 });
  await eventually("a field written by one ember shows in another", async () => (await badge(page)) === "guest: 3 items (gift)");
  check("typing keeps the focus", await note.evaluate((el) => document.activeElement === el));
  check("typing keeps the text", (await note.inputValue()) === "gift", await note.inputValue());
  check("the buttons keep their counts", (await text(page, '.clicks[data-item="tea"]')) === "2");

  // ----- client navigation keeps the store -----
  await page.evaluate(() => {
    window.samePage = true;
  });
  await page.click("#to-about");
  await page.waitForSelector("#about");
  await ready(page);
  check("client navigation changes the address", page.url() === `${base}/about`, page.url());
  check("client navigation changes the title", (await page.title()) === "About the shop");
  check("client navigation keeps the document", await page.evaluate(() => window.samePage === true));
  await eventually("the store survives client navigation", async () => (await badge(page)) === "guest: 3 items (gift)");
  await page.goBack();
  await page.waitForSelector("#to-about");
  await ready(page);
  check("going back stays in the document", await page.evaluate(() => window.samePage === true));
  await eventually("the store survives going back", async () => (await badge(page)) === "guest: 3 items (gift)");
  check("the islands of the page are new", (await text(page, '.clicks[data-item="tea"]')) === "0");
  await page.click('button.add[data-item="cake"]');
  await eventually("the new page's embers change the kept store", async () => (await badge(page)) === "guest: 4 items (gift)");

  // A link outside `client:navigate` loads the page, and a reload starts
  // the stores again.
  await page.click("#full-about");
  await page.waitForSelector("#about");
  await ready(page);
  check("an ordinary link loads a new document", await page.evaluate(() => window.samePage !== true));
  check("a full load starts the store again", (await badge(page)) === "guest: 0 items", await badge(page));

  // ----- a store with a key outlives the page -----
  await page.goto(base);
  await ready(page);
  await page.click("#theme");
  await eventually("the preference changes", async () => (await text(page, "#theme")) === "dark");
  await page.click('button.add[data-item="tea"]');
  await eventually("the cart changes", async () => (await badge(page)) === "guest: 1 items");
  await page.reload();
  await ready(page);
  await eventually("a persisted store survives a reload", async () => (await text(page, "#theme")) === "dark");
  check("a store without a key does not", (await badge(page)) === "guest: 0 items", await badge(page));
  check(
    "only the store with a key is kept",
    (await page.evaluate(() => Object.keys(localStorage).join(","))) === "kiln:store:shop.prefs",
    await page.evaluate(() => Object.keys(localStorage).join(",")),
  );
  await page.click("#clear");
  await eventually("replacing a whole store shows", async () => (await badge(page)) === "guest: 0 items");
  check("the page logged no errors", !logs.some((l) => /error|warn/.test(l)), logs.join("\n"));
  await context.close();

  // ----- requests do not share stores -----
  {
    const a = await browser.newPage();
    const b = await browser.newPage();
    const t0 = Date.now();
    await Promise.all([a.goto(`${base}/who?user=ana`), b.goto(`${base}/who?user=bob`)]);
    metrics.overlapping_ms = Date.now() - t0;
    await Promise.all([ready(a), ready(b)]);
    check("request A sees its own store", (await badge(a)) === "ana: 1 items", await badge(a));
    check("request B sees its own store", (await badge(b)) === "bob: 1 items", await badge(b));
    const snapshot = await a.evaluate(() => document.getElementById("kiln-stores").textContent);
    check("a page's snapshot holds only its request's store", !snapshot.includes("bob"), snapshot);
    await a.click('button.add[data-item="tea"]');
    await eventually("hydration starts from the request's store", async () => (await badge(a)) === "ana: 2 items");
    await a.close();
    await b.close();
  }

  // ----- a page without embers -----
  {
    const page = await browser.newPage();
    await page.goto(`${base}/plain`);
    const html = await page.content();
    check("a page without embers has no script or stores", !html.includes("<script"), html);
    await page.close();
  }
} catch (e) {
  failures.push(`the test stopped: ${e.stack ?? e}`);
} finally {
  await browser.close();
}
console.log(JSON.stringify({ failures, metrics }));
