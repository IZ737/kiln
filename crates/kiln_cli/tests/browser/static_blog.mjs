// Drives the static export of `examples/static_blog`, served by a plain
// file server with no Kiln program running: `node static_blog.mjs <url>`.
// Prints a JSON object with the checks that failed.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const base = process.argv[2];
const failures = [];
const metrics = {};

function check(name, ok, detail = "") {
  if (!ok) failures.push(detail === "" ? name : `${name}: ${detail}`);
}

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  const logs = [];
  const missing = [];
  page.on("console", (m) => {
    if (m.type() === "error") logs.push(m.text());
  });
  page.on("pageerror", (e) => logs.push(`pageerror: ${e.message}`));
  page.on("response", (r) => {
    if (r.status() >= 400 && !r.url().endsWith("/no-such-page")) missing.push(`${r.status()} ${r.url()}`);
  });

  await page.goto(base + "/");
  check("the home page lists the posts", (await page.locator(".posts li").count()) === 2);
  const css = await page.evaluate(() => getComputedStyle(document.querySelector(".brand")).color);
  check("the stylesheet applies", css === "rgb(198, 90, 52)", css);
  const logo = await page.evaluate(() => document.querySelector(".brand img").naturalWidth);
  check("the image loads", logo > 0, String(logo));

  await page.click("text=Glazes & embers");
  await page.waitForURL(/\/posts\/glazes-and-embers\/?$/);
  check("a listed page renders", (await page.locator("h1").textContent()) === "Glazes & embers");
  const start = Date.now();
  await page.waitForSelector("kiln-island[data-k-ready]", { timeout: 10000 });
  metrics.hydrate_ms = Date.now() - start;
  await page.click("button.likes");
  await page.click("button.likes");
  await page.waitForFunction(() => document.querySelector("button.likes").textContent === "Like (2)", null, {
    timeout: 3000,
  }).catch(() => {});
  check("the ember runs with no server", (await page.textContent("button.likes")) === "Like (2)", await page.textContent("button.likes"));

  await page.goto(base + "/about-us");
  await page.waitForURL(/\/about\/?$/, { timeout: 5000 }).catch(() => {});
  check("a redirect page sends the browser on", /\/about\/?$/.test(page.url()), page.url());

  check("no errors in the console", logs.length === 0, logs.join("\n"));
  // The browser logs the 404 below as an error.
  const res = await page.goto(base + "/no-such-page");
  check("a missing page is the 404 page", res.status() === 404 && (await page.locator("h1").textContent()) === "This page is not here");

  const feed = await page.request.get(base + "/feed.xml");
  check("the feed is a file", (await feed.text()).includes("<feed"));

  check("nothing failed to load", missing.length === 0, missing.join(", "));
} finally {
  await browser.close();
}
console.log(JSON.stringify({ failures, metrics }));
