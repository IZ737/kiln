// Drives `examples/garden`, whose routes come from `app.file_routes`:
// `node garden.mjs <url>` against the running app, or `node garden.mjs
// <url> static` against its `kiln fire --static` export on a plain file
// server. Prints a JSON object with the checks that failed.
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const { chromium } = require("playwright");

const base = process.argv[2];
const live = process.argv[3] !== "static";
const failures = [];
const metrics = {};

function check(name, ok, detail = "") {
  if (!ok) failures.push(detail === "" ? name : `${name}: ${detail}`);
}

const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  const logs = [];
  page.on("console", (m) => {
    if (m.type() === "error" && !m.text().includes("404")) logs.push(m.text());
  });
  page.on("pageerror", (e) => logs.push(`pageerror: ${e.message}`));

  await page.goto(base + "/");
  check("the home page's load gives it the plants", (await page.locator(".plants li").count()) === 3);
  check("a page needs no ember code", (await page.locator("script").count()) === 0);
  await page.click("text=Moss");
  await page.waitForURL(/\/plants\/moss\/?$/);
  check("a [slug] page renders", (await page.locator("h1").textContent()) === "Moss");
  check("its load gives it the plant", (await page.locator(".light").textContent()) === "Likes shade.");
  const start = Date.now();
  await page.waitForSelector("kiln-island[data-k-ready]", { timeout: 10000 });
  metrics.hydrate_ms = Date.now() - start;
  await page.click("button.water");
  await page.click("button.water");
  await page
    .waitForFunction(() => document.querySelector("button.water").textContent === "Watered 2 times", null, {
      timeout: 3000,
    })
    .catch(() => {});
  check("the page's ember hydrates and runs", (await page.textContent("button.water")) === "Watered 2 times", await page.textContent("button.water"));

  await page.click("text=About");
  await page.waitForURL(/\/about\/?$/);
  check("a plain page renders", (await page.locator("h1").textContent()) === "About");

  const missing = await page.goto(base + "/plants/rose");
  check("an unknown plant is the 404 page", missing.status() === 404 && (await page.locator("h1").textContent()) === "Nothing grows here");
  const component = await page.goto(base + "/_components/plant_link");
  check("a component is not a page", component.status() === 404);

  const water = await page.request.post(base + "/api/water");
  if (live) {
    check("the API route answers on a server", water.status() === 200 && (await water.json()).watered === true, String(water.status()));
  } else {
    check("the API route is not in the static site", water.status() !== 200, String(water.status()));
  }
  check("no errors in the console", logs.length === 0, logs.join("\n"));
} finally {
  await browser.close();
}
console.log(JSON.stringify({ failures, metrics }));
