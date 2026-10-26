// The journal example: two people, posts with per-record permissions,
// the comment ember's typed server call, and hostile calls.
import { createRequire } from "node:module";
const { chromium } = createRequire(import.meta.url)("playwright");
const browser = await chromium.launch();
const base = process.argv[2];
const failures = [];
const check = (ok, message) => { if (!ok) failures.push(message); };
const metrics = {};
try {
  const ada = await browser.newContext();
  const bob = await browser.newContext();
  const errors = [];
  const signUp = async (context, name, email) => {
    const page = await context.newPage();
    page.on("pageerror", e => errors.push(String(e)));
    await page.goto(base + "/sign-up");
    await page.fill("input[name=name]", name);
    await page.fill("input[name=email]", email);
    await page.fill("input[name=password]", "correct horse battery");
    await Promise.all([page.waitForURL(base + "/"), page.click("button[type=submit]")]);
    return page;
  };
  const a = await signUp(ada, "Ada", "ada@example.com");
  check((await a.locator("a[href='/posts/new']").count()) === 1, "a signed-in visitor can write");
  check(!(await a.content()).includes("<script"), "the list of posts loads no script");

  // A post that fails the shared rules comes back with what is wrong.
  await a.click("a[href='/posts/new']");
  await a.click("button[type=submit]");
  check((await a.locator(".problem").allTextContents()).join("|") === "title is required|body is required", "post rules: " + (await a.locator(".problem").allTextContents()));
  await a.fill("input[name=title]", "Hello <world>");
  await a.fill("textarea[name=body]", "First paragraph.\n\nSecond paragraph.");
  await a.check("input[name=published]");
  await Promise.all([a.waitForURL(/\/posts\/1$/), a.click("button[type=submit]")]);
  check((await a.locator("h1").textContent()) === "Hello <world>", "the title is shown as text");
  check((await a.locator("article p").allTextContents()).join("|") === "By Ada|First paragraph.|Second paragraph.|Edit", "paragraphs: " + (await a.locator("article p").allTextContents()).join("|"));
  // A draft only its writer can see.
  await a.goto(base + "/posts/new");
  await a.fill("input[name=title]", "Draft");
  await a.fill("textarea[name=body]", "Not yet.");
  await Promise.all([a.waitForURL(/\/posts\/2$/), a.click("button[type=submit]")]);

  const b = await signUp(bob, "Bob", "bob@example.com");
  check((await b.locator(".posts li").count()) === 1, "drafts are not listed");
  check((await bob.request.get(base + "/posts/2")).status() === 404, "someone else's draft is not found");
  check((await bob.request.get(base + "/posts/1/edit")).status() === 404, "someone else's post cannot be edited");
  check((await bob.request.get(base + "/admin/posts")).status() === 403, "a writer is not an administrator");

  // The ember: a typed call that knows who is calling.
  let call = null;
  b.on("request", r => { if (r.url().includes("/_kiln/rpc/")) call = { url: r.url(), headers: r.headers(), body: r.postData() }; });
  const started = Date.now();
  await b.goto(base + "/posts/1");
  await b.waitForSelector("kiln-island[data-k-ready]");
  metrics.hydrate_ms = Date.now() - started;
  check((await b.locator("#comments li").textContent()) === "No comments yet.", "no comments yet");
  await b.fill("#comment", "Nice post");
  await b.click("text=Add comment");
  await b.waitForFunction(() => document.querySelector("#comments li").textContent === "Bob: Nice post");
  check((await b.inputValue("#comment")) === "", "the box is emptied after a comment");
  check(b.url() === base + "/posts/1", "a comment does not reload the page");
  await b.click("text=Add comment");
  await b.waitForSelector("#problem");
  check((await b.locator("#problem").textContent()).includes("body is required"), "comment rules run on the server: " + await b.locator("#problem").textContent());
  check(call !== null, "the comment went through a server call");

  // The same call, made by others.
  const replay = async (context, change = {}) => {
    const headers = { ...call.headers, origin: base, "sec-fetch-site": "same-origin", ...(change.headers || {}) };
    delete headers["cookie"];
    const r = await context.request.post(call.url, { headers, data: change.body ?? JSON.stringify({ post: 1, body: "Again" }) });
    return r.status();
  };
  const stranger = await browser.newContext();
  { const got = await replay(stranger); check(got === 401, "a call without a session is refused: " + got); }
  { const got = await replay(bob, { headers: { origin: "https://evil.example", "sec-fetch-site": "cross-site" } }); check(got === 403, "a call from another site is refused: " + got); }
  { const got = await replay(bob, { body: JSON.stringify({ post: "one", body: "x" }) }); check(got === 400, "arguments of the wrong type are refused: " + got); }
  { const got = await replay(bob, { body: JSON.stringify({ post: 2, body: "Sneaky" }) }); check(got === 404, "a draft cannot be commented on by someone else: " + got); }
  { const got = await replay(bob, { body: JSON.stringify({ post: 1, body: "x".repeat(501) }) }); check(got === 422, "a comment that is too long is refused: " + got); }
  { const got = await replay(bob); check(got === 200, "the writer's own session can call: " + got); }

  // Without JavaScript the same form posts to the page's route.
  const plain = await browser.newContext({ javaScriptEnabled: false, storageState: await bob.storageState() });
  const p = await plain.newPage();
  await p.goto(base + "/posts/1");
  await p.fill("#comment", "From a plain form");
  await Promise.all([p.waitForURL(base + "/posts/1"), p.click("text=Add comment")]);
  check((await p.locator("#comments li").allTextContents()).includes("Bob: From a plain form"), "the form works without JavaScript");

  // Ada edits her post, and runs the site's admin.
  await a.goto(base + "/posts/1/edit");
  await a.fill("input[name=title]", "Hello, world");
  await Promise.all([a.waitForURL(/\/posts\/1$/), a.click("button[type=submit]")]);
  check((await a.locator("h1").textContent()) === "Hello, world", "the writer can edit");
  await a.goto(base + "/admin/comments");
  check((await a.locator("h1").textContent()) === "Comments (3)", "the admin lists comments: " + await a.locator("h1").textContent());
  await a.goto(base + "/admin/users/2");
  check((await a.locator("form").textContent()).includes("not shown here"), "the admin never shows a password hash");

  // Signing out ends the session for good.
  const before = (await ada.cookies()).find(c => c.name === "kiln_session");
  await a.goto(base + "/");
  await Promise.all([a.waitForURL(base + "/"), a.click("text=Sign out")]);
  check((await a.locator("a[href='/sign-in']").count()) === 1, "signed out");
  const old = await browser.newContext();
  await old.addCookies([{ ...before }]);
  check((await old.request.get(base + "/admin/posts")).status() === 401, "a session that was signed out cannot be used again");
  check(errors.length === 0, errors.join("\n"));
} catch (e) {
  failures.push(String(e.stack || e));
} finally { await browser.close(); }
console.log(JSON.stringify({ failures, metrics }));
