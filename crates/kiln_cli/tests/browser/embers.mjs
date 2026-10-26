// Drives the app in `embers/` in Chromium: `node embers.mjs <url>`. Prints
// a JSON object: the checks that failed, and how big and fast the embers
// were.
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

const browser = await chromium.launch();
try {
  // ----- an ordinary page loads no ember code -----
  {
    const page = await browser.newPage();
    const requests = [];
    page.on("request", (r) => requests.push(r.url()));
    await page.goto(`${base}/plain`);
    const html = await page.content();
    check("a page without embers has no script", !html.includes("<script"), html);
    check(
      "a page without embers fetches no ember files",
      !requests.some((u) => u.includes("/_kiln/ember")),
      requests.join(" "),
    );
    await page.close();
  }

  // ----- the page with embers -----
  const page = await browser.newPage();
  const logs = [];
  // The browser logs the failed calls the test makes on purpose.
  page.on("console", (m) => {
    if (!m.text().startsWith("Failed to load resource")) logs.push(`${m.type()}: ${m.text()}`);
  });
  page.on("pageerror", (e) => logs.push(`pageerror: ${e.message}`));
  page.on("dialog", (d) => {
    logs.push(`dialog: ${d.message()}`);
    d.dismiss();
  });
  const wasm = [];
  page.on("response", async (r) => {
    if (r.url().endsWith(".wasm")) {
      wasm.push({ type: r.headers()["content-type"], size: (await r.body()).length });
    }
  });
  const started = Date.now();
  await page.goto(base);
  await page.waitForSelector("#first[data-k-ready], kiln-island[data-k-ember=counter][data-k-ready]");
  metrics.hydrate_ms = Date.now() - started;

  // Server-rendered state, before and after the islands hydrate.
  check("the first counter starts at 0", (await text(page, "#first .count")) === "0");
  check("the second counter starts at its prop", (await text(page, "#second .count")) === "7");
  check("static content is untouched", (await text(page, "#static")) === "Static text.");
  check("the title shows as text", (await page.title()).includes("</script>"));

  // Independent state: clicking one counter leaves the other alone.
  await page.click("#first .inc");
  await page.click("#first .inc");
  await eventually("the first counter counts clicks", async () => (await text(page, "#first .count")) === "2");
  check("the second counter keeps its own count", (await text(page, "#second .count")) === "7");
  await page.click("#second .inc");
  await eventually("the second counter counts", async () => (await text(page, "#second .count")) === "8");
  check("the first counter is still 2", (await text(page, "#first .count")) === "2");

  // A typed call to a `server fn`.
  await page.click("#first .add");
  await eventually("a server fn call returns its result", async () => (await text(page, "#first .count")) === "12");

  // client:idle and client:only.
  await eventually("client:idle hydrates", async () =>
    page.evaluate(() => document.querySelector("#idle")?.closest("kiln-island")?.hasAttribute("data-k-ready")),
  );
  await eventually("client:only renders in the browser", async () => (await text(page, "#only .count")) === "3");

  // client:visible waits until the island is scrolled to.
  const far = page.locator("kiln-island:has(#far)");
  check("client:visible waits", !(await far.evaluate((el) => el.hasAttribute("data-k-ready"))));
  await page.locator("#far").scrollIntoViewIfNeeded();
  await eventually("client:visible hydrates once visible", async () =>
    far.evaluate((el) => el.hasAttribute("data-k-ready")),
  );
  await page.click("#far .inc");
  await eventually("the far counter counts", async () => (await text(page, "#far .count")) === "1");
  await page.evaluate(() => scrollTo(0, 0));

  // A list: typing keeps focus, Enter adds, keys keep identity.
  const draft = page.locator("#todos .draft");
  await draft.click();
  await draft.pressSequentially("hello", { delay: 10 });
  check("typing keeps the text", (await draft.inputValue()) === "hello", await draft.inputValue());
  check(
    "typing keeps the focus",
    await draft.evaluate((el) => document.activeElement === el),
  );
  await draft.press("Enter");
  await eventually("Enter adds an item", async () => (await text(page, "#todos .size")) === "3");
  check("Enter clears the draft", (await draft.inputValue()) === "");
  check("the draft keeps the focus", await draft.evaluate((el) => document.activeElement === el));
  await page.locator("#todos li", { hasText: "b" }).locator(".note").fill("note on b");
  await page.click("#todos .reverse");
  await eventually("Reverse reorders the list", async () =>
    (await page.locator("#todos li span").allTextContents()).join(",") === "hello,b,a",
  );
  check(
    "a keyed item keeps what was typed in it",
    (await page.locator("#todos li", { hasText: "b" }).locator(".note").inputValue()) === "note on b",
  );
  await page.locator("#todos li", { hasText: "hello" }).locator(".remove").click();
  await eventually("x removes an item", async () => (await text(page, "#todos .size")) === "2");

  // A form: native validation first, then a typed call with an error.
  await page.click("#signup button");
  check("a required field stops the submit", (await text(page, "#signup output")) === "");
  await page.fill("#signup [name=name]", "Ada");
  await page.fill("#signup [name=email]", "ada");
  await page.click("#signup button");
  await eventually("a server error comes back as the error", async () =>
    (await text(page, "#signup output")) === "Error: the email needs an @",
  );
  await page.fill("#signup [name=email]", "ada@example.com");
  await page.click("#signup button");
  await eventually("the form gets the result", async () => (await text(page, "#signup output")) === "Welcome, Ada");
  check("the submit stays on the page", page.url().replace(/\/$/, "") === base.replace(/\/$/, ""), page.url());

  // Pending state, and a double click sends one call.
  await page.dblclick("#slow .go");
  await eventually("the handler shows it is busy", async () => (await page.locator("#slow .busy").count()) === 1);
  check(
    "the busy element is marked",
    (await page.locator("#slow .go").getAttribute("aria-busy")) === "true",
  );
  await eventually("the slow call finishes", async () => (await text(page, "#slow .value")) === "1", 5000);
  check("busy goes away", (await page.locator("#slow .busy").count()) === 0);
  check("aria-busy goes away", (await page.locator("#slow .go").getAttribute("aria-busy")) === null);
  const calls = await page.evaluate(async () => {
    const script = await (await fetch(document.querySelector("script[src*=ember]").src)).text();
    const version = /KILN_RPC = "([^"]*)"/.exec(script)[1];
    const res = await fetch("/_kiln/rpc/slow_calls", {
      method: "POST",
      headers: { "content-type": "application/json", "x-kiln-rpc": version },
      body: "{}",
    });
    return (await res.json()).ok;
  });
  check("a double click calls the server once", calls === 1, String(calls));

  // `.latest`: a newer call replaces an older one, whose late reply is
  // dropped.
  const q = page.locator("#search .q");
  await q.fill("ab");
  await page.waitForTimeout(50);
  await q.fill("abcd");
  await eventually("the latest query wins", async () => (await text(page, "#search .result")) === "ABCD");
  await page.waitForTimeout(600);
  check("an older reply does not overwrite a newer one", (await text(page, "#search .result")) === "ABCD", await text(page, "#search .result"));

  // An optimistic update rolls back when the server says no.
  await page.click("#optimistic .bump");
  await eventually("an accepted update stays", async () => (await text(page, "#optimistic .n")) === "1");
  await page.click("#optimistic .bump");
  await eventually("a refused update rolls back", async () =>
    (await text(page, "#optimistic .n")) === "1" && (await text(page, "#optimistic .error")) === "2 is even",
  );

  // A nested ember: the outer one re-renders around the inner one, which
  // keeps its state; the slot comes back with its content.
  await page.click("#inner .inc");
  await eventually("the inner counter counts", async () => (await text(page, "#inner .count")) === "6");
  await page.click("#panel .toggle");
  await eventually("the panel closes", async () => (await page.locator("#panel .body").count()) === 0);
  await page.click("#panel .toggle");
  await eventually("the panel opens with its slot", async () => (await page.locator("#panel .slotted").count()) === 1);
  check("the inner counter keeps its state", (await text(page, "#inner .count")) === "6", await text(page, "#inner .count"));
  await page.click("#inner .inc");
  await eventually("the inner counter still counts", async () => (await text(page, "#inner .count")) === "7");

  // An ember inside an ember's own HTML: removed, it is disposed; added
  // again, it starts afresh.
  await page.click("#child .inc");
  await eventually("the child counts", async () => (await text(page, "#child .count")) === "101");
  await page.click("#host .toggle");
  await eventually("the child goes", async () => (await page.locator("#child").count()) === 0);
  await page.click("#host .toggle");
  await eventually("the child comes back afresh", async () => (await text(page, "#child .count")) === "100");
  await page.click("#child .inc");
  await eventually("the new child counts", async () => (await text(page, "#child .count")) === "101");

  // Typed browser APIs, and `js.Value` with checked conversions.
  await page.click("#tools .title");
  await eventually("browser.title reads the title", async () => (await text(page, "#tools .title-out")) === "Changed");
  check("browser.set_title sets it", (await page.title()) === "Changed");
  await page.click("#tools .store");
  await eventually("local storage keeps a value", async () => (await text(page, "#tools .stored")) === "kept");
  await page.click("#tools .js");
  await eventually("js.Value reads a property", async () => (await text(page, "#tools .heading")) === "Changed");
  check(
    "a bad conversion fails safely",
    (await text(page, "#tools .bad")) === "expected a number, found an object",
    await text(page, "#tools .bad"),
  );
  check(
    "a method call works, and a missing method is an error",
    (await text(page, "#tools .upper")) === 'ABC TypeError: an object has no method "noSuchMethod"',
    await text(page, "#tools .upper"),
  );
  await page.click("#tools .fetch");
  await eventually("browser.fetch waits for the text", async () => (await text(page, "#tools .fetched")).includes("No embers here."));
  await page.click("#tools .fetch-missing");
  await eventually("a failed fetch is an error", async () => (await text(page, "#tools .fetched")) === "/missing answered 404");

  check("the page logged nothing", logs.length === 0, logs.join("\n"));
  check("the module is served as application/wasm", wasm.every((w) => w.type === "application/wasm"), JSON.stringify(wasm));
  check("the module is fetched once", wasm.length === 1, JSON.stringify(wasm));
  metrics.wasm_bytes = wasm[0]?.size ?? 0;
  await page.close();

  // ----- hostile calls, from another origin's page -----
  {
    const other = await browser.newPage();
    await other.goto("about:blank");
    const status = await other.evaluate(async (url) => {
      try {
        const res = await fetch(`${url}/_kiln/rpc/add`, {
          method: "POST",
          headers: { "content-type": "application/json", "x-kiln-rpc": "x" },
          body: '{"a":1,"b":2}',
        });
        return res.status;
      } catch {
        return "blocked";
      }
    }, base);
    check("another site cannot call a server fn", status === "blocked" || status >= 400, String(status));
    await other.close();
  }

  // ----- without JavaScript, the page and its form still work -----
  {
    const context = await browser.newContext({ javaScriptEnabled: false });
    const page = await context.newPage();
    await page.goto(base);
    check("without JavaScript the counters render", (await text(page, "#second .count")) === "7");
    await page.fill("#signup [name=name]", "Bo");
    await page.click("#signup button");
    await page.waitForSelector("#fallback");
    check("without JavaScript the form posts", (await text(page, "#fallback")).includes("name=Bo"));
    await context.close();
  }

  // ----- a browser without WasmGC keeps the server's page -----
  {
    const page = await browser.newPage();
    await page.addInitScript(() => {
      WebAssembly.validate = () => false;
    });
    await page.goto(base);
    await page.waitForFunction(() => document.documentElement.getAttribute("data-k-embers") === "unsupported");
    await page.click("#first .inc");
    await page.waitForTimeout(100);
    check("without WasmGC the counter shows the server's state", (await text(page, "#first .count")) === "0");
    check(
      "without WasmGC no island hydrates",
      (await page.locator("kiln-island[data-k-ready]").count()) === 0,
    );
    await page.close();
  }

  // ----- many clicks render in batches -----
  {
    const page = await browser.newPage();
    await page.goto(base);
    await page.waitForSelector("kiln-island[data-k-ready] #first");
    const t0 = Date.now();
    for (let i = 0; i < 100; i++) await page.click("#first .inc");
    metrics.click_ms = (Date.now() - t0) / 100;
    await eventually("a hundred clicks count", async () => (await text(page, "#first .count")) === "100");
    await page.close();
  }
} catch (e) {
  failures.push(`the test stopped: ${e.stack ?? e}`);
} finally {
  await browser.close();
}
console.log(JSON.stringify({ failures, metrics }));
