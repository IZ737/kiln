// ----- embers: the browser bridge -----
//
// Takes over the `<kiln-island>`s the server rendered. Each island is an
// ember: its Kiln code, compiled into the page's WebAssembly module, holds
// its state and renders its HTML, and this bridge turns the HTML into
// changes to the page (keeping elements, focus and what the user typed),
// sends events to the island's handlers, and carries its calls of
// `server fn`s. The server adds this script only to pages with an island,
// and the module is fetched only when the first island needs it.
//
// An island's element has `data-k-ember` (its name), `data-k-hydrate`
// (`load`, `idle`, `visible` or `only`), and its props and state as JSON.
// Inside it, `<kiln-slot>` holds HTML its caller rendered, which belongs
// to the caller; `data-k-on-<event>="<n>[:modifier...]"` marks an element
// whose handler number `n` runs on that event; `data-k-key` keeps an
// element's identity in a list.
//
// The program's `store`s live in the module, one set per page: the bridge
// starts them from the JSON the server rendered the page with (in
// `<script id="kiln-stores">`), learns which stores each island reads as it
// renders, and after each event renders again the islands whose stores
// changed. Links inside `data-k-navigate` load the next page into this one,
// keeping the stores; a store with a key is kept in `localStorage` too.

let ember = null;
let loading = null;
const islands = new Map();
let current = null;

/** Whether this browser runs WasmGC: a module with one struct type. */
function wasmGc() {
  try {
    return WebAssembly.validate(
      new Uint8Array([0, 97, 115, 109, 1, 0, 0, 0, 1, 5, 1, 0x5f, 1, 0x7f, 0]),
    );
  } catch {
    return false;
  }
}

function load() {
  if (!loading) {
    loading = (async () => {
      const res = await fetch(KILN_WASM, { credentials: "same-origin" });
      if (!res.ok) throw new Error(`${KILN_WASM} answered ${res.status}`);
      const module =
        WebAssembly.compileStreaming && res.headers.get("content-type") === "application/wasm"
          ? await WebAssembly.compileStreaming(res)
          : await WebAssembly.compile(await res.arrayBuffer());
      await loadPackages();
      const exports = await instantiate(module, { b: emberImports(), n: npmImports() });
      exports.init();
      ember = exports;
      try {
        stores = JSON.parse(exports.__kiln_store_names?.() ?? "[]").map((s) => ({ ...s, json: "" }));
      } catch (err) {
        console.error("kiln: cannot read the stores", err);
      }
      loadStores(document, true);
      return exports;
    })();
  }
  return loading;
}

// ----- islands -----

let observer = null;

/** Takes an island over when its hint says to. */
function schedule(el) {
  if (islands.has(el) || el.kilnScheduled) return;
  el.kilnScheduled = true;
  const hint = el.getAttribute("data-k-hydrate") || "load";
  // An island has no box of its own (it is `display: contents`), so the
  // observer watches its first element.
  const watched = el.firstElementChild;
  if (hint === "visible" && typeof IntersectionObserver === "function" && watched) {
    if (!observer) {
      observer = new IntersectionObserver((entries) => {
        for (const entry of entries) {
          if (entry.isIntersecting) {
            observer.unobserve(entry.target);
            mount(entry.target.kilnIsland);
          }
        }
      });
    }
    watched.kilnIsland = el;
    el.kilnWatched = watched;
    observer.observe(watched);
  } else if (hint === "idle") {
    const idle = window.requestIdleCallback || ((f) => setTimeout(f, 200));
    idle(() => mount(el));
  } else {
    mount(el);
  }
}

async function mount(el) {
  let exports;
  try {
    exports = await load();
  } catch (err) {
    console.error("kiln: cannot load the embers", err);
    el.setAttribute("data-k-error", "cannot load the embers");
    return;
  }
  if (!el.isConnected || islands.has(el)) return;
  const name = el.getAttribute("data-k-ember");
  let id;
  try {
    id = Number(
      exports.__kiln_mount(
        name,
        el.getAttribute("data-k-props") || "{}",
        el.getAttribute("data-k-state") ?? "",
      ),
    );
  } catch (err) {
    fail({ el, name }, err);
    return;
  }
  if (id < 0) {
    console.error(`kiln: there is no ember named "${name}"`);
    return;
  }
  const island = { id, el, name, controllers: new Set(), pending: new Map(), stash: new Map(), dead: false };
  islands.set(el, island);
  // After client navigation, or with a store kept from an earlier visit,
  // the stores can differ from those the server rendered with.
  const rendered =
    el.hasAttribute("data-k-state") && el.getAttribute("data-k-hydrate") !== "only" && !storesMoved;
  render(island, rendered);
  el.setAttribute("data-k-ready", "");
}

/** Stops the islands in `node` (or `node` itself): their calls are
 *  cancelled, and their state is dropped. */
function dispose(node) {
  if (node.nodeType !== 1) return;
  const all = node.nodeName === "KILN-ISLAND" ? [node] : [];
  all.push(...node.querySelectorAll("kiln-island"));
  for (const el of all) {
    if (el.kilnWatched) observer?.unobserve(el.kilnWatched);
    const island = islands.get(el);
    if (!island) continue;
    islands.delete(el);
    island.dead = true;
    for (const c of island.controllers) c.abort("dispose");
    try {
      ember.__kiln_drop(BigInt(island.id));
    } catch {
      // The island already failed.
    }
  }
}

function fail(island, err) {
  const message = err?.kilnMessage ?? String(err?.message ?? err);
  console.error(`kiln: the ember "${island.name}" failed: ${message}`);
  island.el.setAttribute("data-k-error", message);
  island.dead = true;
}

// ----- stores -----

/** The program's stores, in the module's order: `name`, the `persist` key
 *  or null, and `json`, the value as the islands last saw it ("" before
 *  the store starts). */
let stores = [];
/** The stores the island rendering now reads, by number. */
let reading = null;
/** Whether the stores may differ from those the page was rendered with. */
let storesMoved = false;

function storeKey(s) {
  return `kiln:store:${s.persist}`;
}

/** Starts the stores that have not started from the values `doc` was
 *  rendered with. On the first page, a store kept in `localStorage` comes
 *  first; one that no longer decodes (its type changed) is forgotten. */
function loadStores(doc, first) {
  if (!stores.length) return;
  let snapshot = {};
  const el = doc.getElementById("kiln-stores");
  if (el) {
    try {
      snapshot = JSON.parse(el.textContent);
    } catch (err) {
      console.error("kiln: the page's stores do not decode", err);
    }
  }
  stores.forEach((s, i) => {
    try {
      if (first && s.persist) {
        let kept = null;
        try {
          kept = localStorage.getItem(storeKey(s));
        } catch {
          // Storage is off: the store starts as the page says.
        }
        if (kept !== null) {
          if (ember.__kiln_store_load(BigInt(i), kept, 1)) storesMoved = true;
          else localStorage.removeItem(storeKey(s));
        }
      }
      if (Object.hasOwn(snapshot, s.name)) {
        ember.__kiln_store_load(BigInt(i), JSON.stringify(snapshot[s.name]), 0);
      }
      s.json = ember.__kiln_store_json(BigInt(i));
    } catch (err) {
      console.error(`kiln: the store "${s.name}" failed`, err);
    }
  });
}

/** Marks the islands that read a store that changed, and keeps the stores
 *  that have keys. */
function checkStores() {
  stores.forEach((s, i) => {
    let json;
    try {
      json = ember.__kiln_store_json(BigInt(i));
    } catch {
      return;
    }
    if (json === s.json) return;
    s.json = json;
    if (s.persist && json !== "") {
      try {
        localStorage.setItem(storeKey(s), json);
      } catch {
        // Storage is off or full: the store lives as long as the page.
      }
    }
    for (const island of islands.values()) {
      if (island.reads?.has(i)) dirty.add(island);
    }
  });
}

// ----- rendering -----

const dirty = new Set();
let queued = false;

/** Renders an island again before the browser next paints, once however
 *  many of its handlers ran, with the islands whose stores the handlers
 *  changed, in the order they are on the page. */
function invalidate(island) {
  if (island) dirty.add(island);
  if (queued) return;
  queued = true;
  queueMicrotask(() => {
    queued = false;
    checkStores();
    const list = [...dirty].sort((a, b) =>
      a.el.compareDocumentPosition(b.el) & Node.DOCUMENT_POSITION_FOLLOWING ? -1 : 1,
    );
    dirty.clear();
    for (const i of list) {
      if (!i.dead && islands.get(i.el) === i) render(i);
    }
    // A store an island started as it rendered is where the islands
    // that read it saw it.
    stores.forEach((s, i) => {
      if (s.json === "") {
        try {
          s.json = ember.__kiln_store_json(BigInt(i));
        } catch {
          // The island that read it failed.
        }
      }
    });
  });
}

function render(island, check = false) {
  let html;
  reading = island.reads = new Set();
  try {
    html = ember.__kiln_render(BigInt(island.id));
  } catch (err) {
    fail(island, err);
    return;
  } finally {
    reading = null;
  }
  const template = document.createElement("template");
  template.innerHTML = html;
  const changes = { count: 0 };
  morphChildren(island.el, template.content, island, changes);
  if (check && changes.count > 0) {
    console.warn(
      `kiln: the ember "${island.name}" rendered differently in the browser than on the server (${changes.count} changes); the page now shows the browser's HTML`,
    );
  }
  for (const [el, calls] of island.pending) {
    if (calls.size > 0) markPending(el, island);
  }
}

function keyOf(n) {
  return n.nodeType === 1 ? n.getAttribute("data-k-key") : null;
}

function same(a, b) {
  if (a.nodeType !== b.nodeType || a.nodeName !== b.nodeName) return false;
  if (a.nodeName === "KILN-ISLAND") {
    return a.getAttribute("data-k-ember") === b.getAttribute("data-k-ember");
  }
  return true;
}

/** Makes `from`'s children like `to`'s, keeping the elements that match:
 *  by `data-k-key`, or else by position and tag. */
function morphChildren(from, to, island, changes) {
  const before = [...from.childNodes];
  const after = [...to.childNodes];
  const keyed = new Map();
  for (const n of before) {
    const k = keyOf(n);
    if (k !== null) keyed.set(k, n);
  }
  const used = new Set();
  let i = 0;
  let ref = from.firstChild;
  for (const n of after) {
    let match = null;
    const k = keyOf(n);
    if (k !== null) {
      match = keyed.get(k) ?? null;
      if (match && (used.has(match) || !same(match, n))) match = null;
    } else {
      while (i < before.length && (used.has(before[i]) || keyOf(before[i]) !== null)) i++;
      if (i < before.length && same(before[i], n)) match = before[i++];
    }
    if (match) {
      used.add(match);
      if (match === ref) {
        ref = ref.nextSibling;
      } else {
        from.insertBefore(match, ref);
        changes.count++;
      }
      update(match, n, island, changes);
    } else {
      from.insertBefore(n, ref);
      changes.count++;
      adoptSlots(n, island);
      if (n.nodeType === 1) {
        if (n.nodeName === "KILN-ISLAND") schedule(n);
        for (const el of n.querySelectorAll("kiln-island")) schedule(el);
      }
    }
  }
  for (const n of before) {
    if (!used.has(n)) {
      stashSlots(n, island);
      dispose(n);
      n.remove();
      changes.count++;
    }
  }
}

/** Keeps the content of the island's slots in `node`, which is going: the
 *  caller rendered it, and it comes back when the slot does. */
function stashSlots(node, island) {
  if (node.nodeType !== 1) return;
  const slots = node.nodeName === "KILN-SLOT" ? [node] : [...node.querySelectorAll("kiln-slot")];
  for (const slot of slots) {
    if (ownerIsland(slot) !== island.el || !slot.firstChild) continue;
    const kept = document.createDocumentFragment();
    while (slot.firstChild) kept.appendChild(slot.firstChild);
    island.stash.set(slot.getAttribute("data-k-slot"), kept);
  }
}

/** The island whose HTML `node` is part of: the nearest one around it,
 *  except that what is in a `<kiln-slot>` belongs to the island around
 *  the one the slot is in. */
function ownerIsland(node) {
  let skip = 0;
  for (let n = node.parentNode; n; n = n.parentNode) {
    if (n.nodeName === "KILN-SLOT") skip++;
    else if (n.nodeName === "KILN-ISLAND") {
      if (skip === 0) return n;
      skip--;
    }
  }
  return null;
}

/** A new `<kiln-slot>` of `island` takes the content of the old one with
 *  its name, which its caller rendered. */
function adoptSlots(node, island) {
  if (node.nodeType !== 1) return;
  const slots = node.nodeName === "KILN-SLOT" ? [node] : [...node.querySelectorAll("kiln-slot")];
  for (const slot of slots) {
    if (ownerIsland(slot) !== island.el || slot.firstChild) continue;
    const name = slot.getAttribute("data-k-slot");
    const kept = island.stash.get(name);
    if (kept) {
      island.stash.delete(name);
      slot.appendChild(kept);
      continue;
    }
    for (const old of island.el.querySelectorAll("kiln-slot")) {
      if (old !== slot && !node.contains(old) && old.getAttribute("data-k-slot") === name && ownerIsland(old) === island.el) {
        while (old.firstChild) slot.appendChild(old.firstChild);
        break;
      }
    }
  }
}

/** Attributes the bridge sets, which renders leave alone. */
const OWN = new Set(["data-k-ready", "data-k-pending", "aria-busy", "data-k-error"]);

function update(old, n, island, changes) {
  if (old.nodeType !== 1) {
    if (old.nodeValue !== n.nodeValue) {
      old.nodeValue = n.nodeValue;
      changes.count++;
    }
    return;
  }
  // The caller's HTML in a slot is the caller's to render.
  if (old.nodeName === "KILN-SLOT") return;
  if (old.nodeName === "KILN-ISLAND") return updateIsland(old, n, island, changes);
  for (const a of [...n.attributes]) {
    if (old.getAttribute(a.name) !== a.value) {
      old.setAttribute(a.name, a.value);
      if (!a.name.startsWith("data-k-")) changes.count++;
    }
  }
  for (const a of [...old.attributes]) {
    if (!n.hasAttribute(a.name) && !OWN.has(a.name)) {
      old.removeAttribute(a.name);
      if (!a.name.startsWith("data-k-")) changes.count++;
    }
  }
  // What the user typed is in properties, which attributes do not change.
  if (old.nodeName === "INPUT") {
    if (n.hasAttribute("value") && old.value !== n.getAttribute("value")) old.value = n.getAttribute("value");
    if (old.type === "checkbox" || old.type === "radio") old.checked = n.hasAttribute("checked");
  } else if (old.nodeName === "TEXTAREA") {
    if (old.value !== n.value) old.value = n.value;
  } else if (old.nodeName === "OPTION") {
    old.selected = n.hasAttribute("selected");
  }
  morphChildren(old, n, island, changes);
}

/** An ember inside this one: it gets the new props, and the content of
 *  its slots is rendered again, but the rest of it is its own. */
function updateIsland(old, n, island, changes) {
  const props = n.getAttribute("data-k-props") || "{}";
  if (old.getAttribute("data-k-props") !== props) {
    old.setAttribute("data-k-props", props);
    const inner = islands.get(old);
    if (inner && !inner.dead) {
      try {
        ember.__kiln_props(BigInt(inner.id), props);
        invalidate(inner);
      } catch (err) {
        fail(inner, err);
      }
    }
  }
  for (const slot of n.querySelectorAll(":scope > kiln-slot")) {
    const name = slot.getAttribute("data-k-slot");
    for (const target of old.querySelectorAll("kiln-slot")) {
      if (target.getAttribute("data-k-slot") === name && ownerIsland(target) === old) {
        morphChildren(target, slot, island, changes);
        break;
      }
    }
  }
}

// ----- events -----

const BUBBLING = ["click", "dblclick", "input", "change", "submit", "keydown", "keyup", "pointerdown", "pointerup", "focusin", "focusout"];
const DIRECT = ["focus", "blur", "mouseenter", "mouseleave"];

function onEvent(ev) {
  if (!ember) return;
  const attr = `data-k-on-${ev.type}`;
  const path = ev.eventPhase === Event.CAPTURING_PHASE ? [ev.target] : null;
  for (let n = ev.target; n && n.nodeType === 1; n = path ? null : n.parentNode) {
    if (!n.hasAttribute(attr)) continue;
    const [index, ...mods] = n.getAttribute(attr).split(":");
    const owner = ownerIsland(n);
    const island = owner && islands.get(owner);
    if (!island || island.dead) continue;
    if (mods.includes("prevent") || ev.type === "submit") ev.preventDefault();
    fire(island, n, ev, Number(index), mods);
    if (mods.includes("stop")) {
      ev.stopPropagation();
      break;
    }
  }
}

function fields(form) {
  const out = {};
  if (!(form instanceof HTMLFormElement)) return out;
  for (const [k, v] of new FormData(form)) {
    if (typeof v === "string") out[k] = v;
  }
  return out;
}

function fire(island, el, ev, index, mods) {
  const calls = island.pending.get(el);
  const busy = calls?.get(ev.type);
  // While a handler's call is on its way, the handler does not run again,
  // unless it is `.latest`, whose new call replaces the old one.
  if (busy) {
    if (!mods.includes("latest")) return;
    busy.abort("replaced");
  }
  const form = ev.type === "submit" ? el : el.form;
  current = { island, el, type: ev.type };
  try {
    ember.__kiln_fire(
      BigInt(island.id),
      BigInt(index),
      ev.type,
      "value" in el && el.value != null ? String(el.value) : "",
      el.checked ? 1 : 0,
      ev.key ?? "",
      JSON.stringify(fields(form)),
    );
  } catch (err) {
    fail(island, err);
  } finally {
    current = null;
  }
  invalidate(island);
}

/** Marks an element, and its island, as waiting for a call. */
function markPending(el, island) {
  busy(el, true);
  busy(island.el, true);
}

function busy(el, on) {
  if (on) {
    el.setAttribute("data-k-pending", "");
    el.setAttribute("aria-busy", "true");
  } else {
    el.removeAttribute("data-k-pending");
    el.removeAttribute("aria-busy");
  }
}

// ----- calls of server functions -----

const TIMEOUT = 30000;

/** Records an error for the Kiln code, which then fails with it. */
function failWith(err) {
  pendingError = err instanceof Error ? `${err.name}: ${err.message}` : String(err);
}

function describe(v) {
  if (v === undefined || v === null) return String(v);
  if (Array.isArray(v)) return "an array";
  return typeof v === "object" ? "an object" : `a ${typeof v}`;
}

/** The imports of module `b`: the bridge, `browser` and `js`. */
function emberImports() {
  return {
    ember_call: (name, args, done) => call(name, args, done),
    store_read: (i) => {
      if (reading) reading.add(i);
    },
    browser_log: (m) => console.log(m),
    browser_warn: (m) => console.warn(m),
    browser_title: () => document.title,
    browser_set_title: (t) => {
      document.title = t;
    },
    browser_location: () => location.href,
    browser_navigate: (url) => {
      if (isSafeUrl(url)) location.assign(url);
      else console.warn(`kiln: not going to ${JSON.stringify(url)}, which could run script`);
    },
    browser_now: () => performance.now(),
    browser_confirm: (m) => (confirm(m) ? 1 : 0),
    browser_get_item: (k) => {
      try {
        return localStorage.getItem(k);
      } catch {
        return null;
      }
    },
    browser_set_item: (k, v) => {
      try {
        localStorage.setItem(k, v);
      } catch (err) {
        failWith(err);
      }
    },
    browser_remove_item: (k) => {
      try {
        localStorage.removeItem(k);
      } catch {
        // Storage is off: there is nothing to remove.
      }
    },
    browser_element: (selector) => {
      try {
        return document.querySelector(selector);
      } catch {
        return null;
      }
    },
    browser_focus: (selector) => {
      try {
        document.querySelector(selector)?.focus();
      } catch {
        // Not a selector: nothing matches.
      }
    },
    js_global: (name) => globalThis[name],
    js_identity: (v) => v,
    js_float: (n) => n,
    js_bool: (b) => b !== 0,
    js_null: () => null,
    js_get: (v, name) => (v == null ? undefined : v[name]),
    js_set: (v, name, value) => {
      if (v != null) v[name] = value;
    },
    js_call: (v, method, args) => {
      try {
        if (v == null || typeof v[method] !== "function") {
          throw new TypeError(`${describe(v)} has no method ${JSON.stringify(method)}`);
        }
        return v[method](...args);
      } catch (err) {
        failWith(err);
        return undefined;
      }
    },
    js_as_string: (v) => {
      if (typeof v === "string") return v;
      failWith(`expected a string, found ${describe(v)}`);
      return "";
    },
    js_as_float: (v) => {
      if (typeof v === "number") return v;
      failWith(`expected a number, found ${describe(v)}`);
      return 0;
    },
    js_as_int: (v) => {
      if (typeof v === "number" && Number.isInteger(v) && Number.isSafeInteger(v)) return BigInt(v);
      if (typeof v === "bigint" && v >= I64_MIN && v <= I64_MAX) return v;
      failWith(`expected a whole number in Int's range, found ${typeof v === "number" ? v : describe(v)}`);
      return 0n;
    },
    js_as_bool: (v) => {
      if (typeof v === "boolean") return v ? 1 : 0;
      failWith(`expected a boolean, found ${describe(v)}`);
      return 0;
    },
    js_is_nullish: (v) => (v == null ? 1 : 0),
    js_type_of: (v) => (v === null ? "null" : typeof v),
  };
}

function call(name, args, done) {
  const ctx = current;
  if (!ctx) return;
  const { island, el, type } = ctx;
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort("timeout"), TIMEOUT);
  let calls = island.pending.get(el);
  if (!calls) island.pending.set(el, (calls = new Map()));
  calls.set(type, controller);
  island.controllers.add(controller);
  markPending(el, island);
  // Whatever the call is waiting for, cancelling it ends the wait.
  const cancelled = new Promise((_, reject) => {
    controller.signal.addEventListener("abort", () => reject(new Error("cancelled")));
  });
  Promise.race([send(name, args, controller.signal), cancelled])
    .catch(() => {
      const why = controller.signal.reason;
      if (controller.signal.aborted && why !== "timeout") return null;
      if (why === "timeout") return [false, "the call did not finish in time"];
      return [false, "the server cannot be reached"];
    })
    .then((reply) => {
      clearTimeout(timer);
      island.controllers.delete(controller);
      if (calls.get(type) === controller) calls.delete(type);
      if (calls.size === 0) {
        island.pending.delete(el);
        busy(el, false);
      }
      if (island.pending.size === 0) busy(island.el, false);
      // A cancelled call's reply never reaches the island.
      if (!reply || island.dead || islands.get(island.el) !== island) return;
      current = ctx;
      try {
        ember.__kiln_reply(done, reply[0] ? 1 : 0, reply[1]);
      } catch (err) {
        fail(island, err);
      } finally {
        current = null;
      }
      invalidate(island);
    });
}

/** Sends a call: to the server, or to the browser or an npm package.
 *  Gives `[true, '{"ok": result}']` or `[false, message]`. */
async function send(name, args, signal) {
  const local = WAITS[name];
  if (local) return local(JSON.parse(args), signal);
  const dot = name.indexOf(".");
  if (dot > 0 && name.slice(0, dot) in npm) {
    const pkg = name.slice(0, dot);
    try {
      const result = await npmCall(pkg, KILN_NPM[pkg].names[name.slice(dot + 1)], args);
      return [true, JSON.stringify({ ok: result })];
    } catch (err) {
      return [false, err instanceof Error ? `${err.name}: ${err.message}` : String(err)];
    }
  }
  const res = await fetch(`/_kiln/rpc/${name}`, {
    method: "POST",
    headers: {
      "content-type": "application/json",
      accept: "application/json",
      "x-kiln-rpc": KILN_RPC,
    },
    body: args,
    signal,
    credentials: "same-origin",
  });
  const text = await res.text();
  if (res.ok) return [true, text];
  let message = null;
  try {
    message = JSON.parse(text).error;
  } catch {
    // Not JSON: a proxy's page, say.
  }
  return [false, typeof message === "string" ? message : `the server answered ${res.status}`];
}

/** The functions of `browser` that handlers wait for. */
const WAITS = {
  "browser.fetch": async ({ url }, signal) => {
    let res;
    try {
      res = await fetch(url, { signal, credentials: "same-origin" });
    } catch (err) {
      if (signal.aborted) throw err;
      return [false, `cannot fetch ${url}`];
    }
    if (!res.ok) return [false, `${url} answered ${res.status}`];
    return [true, JSON.stringify({ ok: await res.text() })];
  },
  "browser.copy": async ({ text }) => {
    try {
      await navigator.clipboard.writeText(text);
      return [true, '{"ok":true}'];
    } catch (err) {
      return [false, `cannot copy: ${err?.message ?? err}`];
    }
  },
};

// ----- npm packages -----

/** The npm packages the embers use, by their Kiln names, once loaded. */
const npm = {};

async function loadPackages() {
  for (const [name, { url }] of Object.entries(KILN_NPM)) npm[name] = await import(url);
}

/** Calls a package's export `js` with the arguments of `args` (a JSON
 *  object, in order), or reads it when it is not a function. Kiln's
 *  `none` is `undefined`, as JavaScript leaves an argument out. */
function npmCall(pkg, js, args) {
  const v = npm[pkg][js];
  if (typeof v !== "function") return v;
  const values = Object.values(JSON.parse(args)).map((a) => (a === null ? undefined : a));
  return v(...values);
}

/** The imports of module `n`: each package function the embers call,
 *  which takes its arguments as a JSON array and gives its result as
 *  `{"ok": result}`, or fails. */
function npmImports() {
  return {
    call: (pkg, name, args) => {
      try {
        const result = npmCall(pkg, name, args);
        if (result instanceof Promise) {
          failWith(`${pkg}.${name} returned a promise`);
          return "";
        }
        return JSON.stringify({ ok: result ?? null });
      } catch (err) {
        failWith(err);
        return "";
      }
    },
  };
}

// ----- client navigation -----

/** The page a click goes to without loading a new document: a same-origin
 *  link inside `data-k-navigate`, clicked plainly. */
function navigation(ev) {
  if (ev.defaultPrevented || ev.button !== 0 || ev.metaKey || ev.ctrlKey || ev.shiftKey || ev.altKey) return null;
  const a = ev.target instanceof Element ? ev.target.closest("a[href]") : null;
  if (!a || !a.closest("[data-k-navigate]") || a.hasAttribute("download")) return null;
  if (a.target && a.target !== "_self") return null;
  const url = new URL(a.href, location.href);
  if (url.origin !== location.origin || !/^https?:$/.test(url.protocol)) return null;
  // A link to a part of this page is the browser's.
  if (url.hash && url.pathname === location.pathname && url.search === location.search) return null;
  return url;
}

let navigating = null;

/** Shows the page at `url` in this document: the old page's islands stop,
 *  and their calls with them, while the stores stay. Anything but an HTML
 *  page (an error, a download) is loaded the ordinary way. */
async function navigate(url, push) {
  navigating?.abort();
  const controller = (navigating = new AbortController());
  let res, text;
  try {
    res = await fetch(url, {
      credentials: "same-origin",
      signal: controller.signal,
      headers: { accept: "text/html" },
    });
    const kind = res.headers.get("content-type") ?? "";
    if (!res.ok || !kind.startsWith("text/html")) throw new Error(`${url} answered ${res.status}`);
    text = await res.text();
  } catch {
    if (controller.signal.aborted) return;
    location.assign(url.href);
    return;
  }
  if (controller !== navigating) return;
  navigating = null;
  const doc = new DOMParser().parseFromString(text, "text/html");
  if (push) history.pushState({ kiln: true }, "", res.url || url.href);
  dispose(document.body);
  document.title = doc.title;
  for (const el of doc.head.querySelectorAll('link[rel="stylesheet"], style')) {
    if (![...document.head.children].some((old) => old.isEqualNode(el))) {
      document.head.appendChild(document.importNode(el, true));
    }
  }
  document.body.replaceWith(document.importNode(doc.body, true));
  storesMoved = true;
  // The stores that start now start from the new page's values; until the
  // module is loaded, its loading reads them from this document.
  const old = document.getElementById("kiln-stores");
  const next = doc.getElementById("kiln-stores");
  if (next) {
    old?.remove();
    document.head.appendChild(document.importNode(next, true));
  }
  if (ember) loadStores(document, false);
  for (const el of document.querySelectorAll("kiln-island")) schedule(el);
  const target = url.hash && document.getElementById(decodeURIComponent(url.hash.slice(1)));
  if (target) target.scrollIntoView();
  else if (push) scrollTo(0, 0);
}

// ----- start -----

function boot() {
  if (!wasmGc()) {
    // The page stays as the server rendered it: links and forms work.
    document.documentElement.setAttribute("data-k-embers", "unsupported");
    return;
  }
  try {
    const sheet = new CSSStyleSheet();
    sheet.replaceSync("kiln-island,kiln-slot{display:contents}");
    document.adoptedStyleSheets = [...document.adoptedStyleSheets, sheet];
  } catch {
    // Older browsers: the islands are inline elements.
  }
  for (const type of BUBBLING) document.addEventListener(type, onEvent);
  for (const type of DIRECT) document.addEventListener(type, onEvent, true);
  addEventListener("pagehide", () => {
    for (const island of islands.values()) {
      for (const c of island.controllers) c.abort("dispose");
    }
  });
  document.addEventListener("click", (ev) => {
    const url = navigation(ev);
    if (!url) return;
    ev.preventDefault();
    navigate(url, true);
  });
  addEventListener("popstate", (ev) => {
    if (ev.state?.kiln) navigate(new URL(location.href), false);
  });
  if (document.querySelector("[data-k-navigate]") && !history.state) {
    history.replaceState({ kiln: true }, "");
  }
  for (const el of document.querySelectorAll("kiln-island")) schedule(el);
}

boot();
