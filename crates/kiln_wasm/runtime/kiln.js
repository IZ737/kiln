// The JavaScript half of Kiln's browser runtime. A Kiln module compiled to
// WebAssembly (WasmGC) imports these functions: strings are JavaScript
// strings, worked on here with the semantics of the native runtime
// (kiln.c), and the module's string constants come from its
// `kiln.strings` section. Both `node` (for tests and `--target wasm`) and
// the browser (for embers) load modules through `instantiate`.

const I64_MIN = -(1n << 63n);
const I64_MAX = (1n << 63n) - 1n;
const utf8 = new TextEncoder();

/** A Kiln panic: the program (or the ember) stops. */
export class KilnPanic extends Error {
  constructor(message) {
    super(message);
    this.name = "KilnPanic";
    this.kilnMessage = message;
  }
}

/** Where `print` and logs go: set by the host. */
export const io = {
  out: (s) => console.log(s),
  err: (s) => console.error(s),
};

function panic(message) {
  throw new KilnPanic(String(message));
}

const PANIC_CODES = [
  "",
  "integer overflow",
  "division by zero",
  "a negative exponent",
  "clamp: the low bound is above the high bound",
  "float is out of the Int range",
];

function panicCode(code, a, b) {
  switch (code) {
    case 6:
      panic(`index ${a} is out of range for a list of length ${b}`);
      break;
    case 7:
      panic(`index ${a} is out of range for inserting into a list of length ${b}`);
      break;
    case 8:
      panic("the key is not in the map");
      break;
    case 9:
      panic("fixed: digits must be between 0 and 20");
      break;
    case 10:
      panic("repeat: the count is negative");
      break;
    default:
      panic(PANIC_CODES[code] ?? "panic");
  }
}

// ----- strings -----

/** Code points, as the native runtime counts characters. */
function chars(s) {
  return Array.from(s);
}

function isAscii(s) {
  return !/[^\x00-\x7f]/.test(s);
}

function codeLen(s) {
  if (isAscii(s)) return s.length;
  let n = 0;
  for (const _ of s) n++;
  return n;
}

/** Characters `start` up to `end`, both clamped to the string. */
function slice(s, start, end) {
  if (isAscii(s)) {
    const n = s.length;
    const a = clamp(start, n);
    const b = clamp(end, n);
    return b > a ? s.slice(a, b) : "";
  }
  const cs = chars(s);
  const a = clamp(start, cs.length);
  const b = clamp(end, cs.length);
  return b > a ? cs.slice(a, b).join("") : "";
}

function clamp(i, n) {
  if (i <= 0n) return 0;
  if (i >= BigInt(n)) return n;
  return Number(i);
}

/** Strings ordered by their UTF-8 bytes, which is code point order. */
function strCmp(a, b) {
  if (a === b) return 0;
  const n = Math.min(a.length, b.length);
  for (let i = 0; i < n; i++) {
    const x = a.codePointAt(i);
    const y = b.codePointAt(i);
    if (x !== y) return x < y ? -1 : 1;
    if (x > 0xffff) i++;
  }
  return a.length < b.length ? -1 : a.length > b.length ? 1 : 0;
}

function strHash(s) {
  let h = 0x811c9dc5;
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 0x01000193);
  }
  return h | 0;
}

const SPACE = /^[ \t\n\r]+|[ \t\n\r]+$/g;

function asciiUpper(s) {
  return s.replace(/[a-z]+/g, (m) => m.toUpperCase());
}

function asciiLower(s) {
  return s.replace(/[A-Z]+/g, (m) => m.toLowerCase());
}

function split(s, sep) {
  if (sep === "") return chars(s);
  return s.split(sep);
}

function lines(s) {
  const out = [];
  let i = 0;
  while (i < s.length) {
    let j = s.indexOf("\n", i);
    if (j < 0) j = s.length;
    const e = j > i && s[j - 1] === "\r" ? j - 1 : j;
    out.push(s.slice(i, e));
    i = j + 1;
  }
  return out;
}

function repeat(s, n) {
  if (n < 0n) panicCode(10);
  return s.repeat(Number(n));
}

function pad(s, width, fill, start) {
  const have = BigInt(codeLen(s));
  const step = BigInt(codeLen(fill));
  if (have >= width || step === 0n) return s;
  const need = width - have;
  const p = slice(fill.repeat(Number((need + step - 1n) / step)), 0n, need);
  return start ? p + s : s + p;
}

function indexOf(s, part) {
  const i = s.indexOf(part);
  if (i < 0) return -1n;
  return BigInt(codeLen(s.slice(0, i)));
}

const INT_TEXT = /^[ \t\n\v\f\r]*[+-]?[0-9]+$/;

function toIntOk(s) {
  if (!INT_TEXT.test(s)) return 0;
  const n = BigInt(s.trim());
  return n >= I64_MIN && n <= I64_MAX ? 1 : 0;
}

const FLOAT_TEXT = /^[+-]?([0-9]+\.?[0-9]*|\.[0-9]+)([eE][+-]?[0-9]+)?$/;

function toFloatOk(s) {
  if (!FLOAT_TEXT.test(s)) return 0;
  const f = Number(s);
  if (!Number.isFinite(f)) return 0;
  // strtod reports a number too small to hold as out of range.
  if (f === 0 && /[1-9]/.test(s.split(/[eE]/)[0])) return 0;
  return 1;
}

function quote(s) {
  return '"' + s.replace(/["\\\n\t]/g, (c) => ({ '"': '\\"', "\\": "\\\\", "\n": "\\n", "\t": "\\t" })[c]) + '"';
}

// ----- numbers -----

/** C's `%.<p>g`. */
function fmtG(f, p) {
  if (Number.isNaN(f)) return "nan";
  if (!Number.isFinite(f)) return f < 0 ? "-inf" : "inf";
  if (f === 0) return Object.is(f, -0) ? "-0" : "0";
  const e = f.toExponential(p - 1);
  const [mant, expText] = e.split("e");
  const x = Number(expText);
  if (x < -4 || x >= p) {
    let m = mant;
    if (m.includes(".")) m = m.replace(/\.?0+$/, "");
    const sign = x < 0 ? "-" : "+";
    const ax = Math.abs(x);
    return `${m}e${sign}${ax < 10 ? "0" : ""}${ax}`;
  }
  let fixed = f.toFixed(Math.max(0, p - 1 - x));
  if (fixed.includes(".")) fixed = fixed.replace(/\.?0+$/, "");
  return fixed;
}

function floatStr(f) {
  if (Number.isFinite(f) && Math.floor(f) === f && Math.abs(f) < 1e15) {
    return (Object.is(f, -0) ? "-" : "") + f.toFixed(1);
  }
  for (let p = 1; p <= 17; p++) {
    const s = fmtG(f, p);
    if (Number(s.replace("inf", "Infinity")) === f) return s;
  }
  return fmtG(f, 17);
}

/** C's `%.<digits>f`, rounding the exact binary value half to even. */
function floatFixed(f, digits) {
  if (digits < 0n || digits > 20n) panicCode(9);
  const d = Number(digits);
  if (!Number.isFinite(f)) return fmtG(f, 1);
  if (Math.abs(f) >= 1e21) return floatStr(f);
  const neg = f < 0 || Object.is(f, -0);
  const [m, e] = decompose(Math.abs(f));
  // |f| = m * 2^e exactly; scale by 10^d and round.
  let n;
  const scale = 10n ** BigInt(d);
  if (e >= 0) {
    n = (m << BigInt(e)) * scale;
  } else {
    const num = m * scale;
    const den = 1n << BigInt(-e);
    n = num / den;
    const r = num % den;
    if (2n * r > den || (2n * r === den && n % 2n === 1n)) n += 1n;
  }
  let text = n.toString();
  if (d > 0) {
    text = text.padStart(d + 1, "0");
    text = text.slice(0, text.length - d) + "." + text.slice(text.length - d);
  }
  return (neg ? "-" : "") + text;
}

/** A finite, non-negative double as `m * 2^e` with integer `m`. */
function decompose(f) {
  if (f === 0) return [0n, 0];
  const buf = new DataView(new ArrayBuffer(8));
  buf.setFloat64(0, f);
  const bits = buf.getBigUint64(0);
  const exp = Number((bits >> 52n) & 0x7ffn);
  const frac = bits & ((1n << 52n) - 1n);
  if (exp === 0) return [frac, -1074];
  return [frac | (1n << 52n), exp - 1075];
}

function floatRem(a, b) {
  return a % b;
}

const UNITS = [
  [3600000000000n, "h"],
  [60000000000n, "m"],
  [1000000000n, "s"],
  [1000000n, "ms"],
  [1000n, "us"],
  [1n, "ns"],
];

function durationStr(ns) {
  for (const [unit, name] of UNITS) {
    if (ns !== 0n && ns % unit === 0n) return `${ns / unit}${name}`;
  }
  return "0s";
}

// ----- the standard library -----

function htmlEscape(s) {
  return s.replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#x27;" })[c]);
}

const UNSAFE_URL = "about:invalid#kiln-unsafe-url";

function isSafeUrl(url) {
  const trimmed = url.replace(/^[\x00-\x20]+|[\x00-\x20]+$/g, "");
  let scheme = "";
  for (const c of trimmed) {
    if (c === "\t" || c === "\n" || c === "\r") continue;
    if (c === ":") return ["http", "https", "mailto", "tel"].includes(scheme.toLowerCase());
    if (/[a-zA-Z]/.test(c) || (scheme !== "" && /[0-9+\-.]/.test(c))) scheme += c;
    else return true;
  }
  return true;
}

function uiSlot(c) {
  return c.length === 13 && c[0] === "k" ? c.slice(1, 7) : null;
}

function uiMerge(a, b) {
  if (b === "") return a;
  if (a === "") return b;
  const later = b.split(" ").map(uiSlot).filter((s) => s !== null);
  const keep = a.split(" ").filter((c) => c !== "" && (uiSlot(c) === null || !later.includes(uiSlot(c))));
  return keep.concat(b.split(" ").filter((c) => c !== "")).join(" ");
}

function base64Encode(s) {
  const bytes = utf8.encode(s);
  let bin = "";
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin);
}

const B64 = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/** Standard Base64 with padding, as the native runtime's `base64` crate
 *  reads it; a failure leaves its message for `take_error`. */
function base64Decode(s) {
  const fail = (m) => {
    pendingError = `cannot read Base64: ${m}`;
    return "";
  };
  const bytes = utf8.encode(s);
  let end = bytes.length;
  while (end > 0 && bytes[end - 1] === 61) end--;
  for (let i = 0; i < end; i++) {
    if (B64.indexOf(String.fromCharCode(bytes[i])) < 0) return fail(`Invalid symbol ${bytes[i]}, offset ${i}.`);
  }
  const pads = bytes.length - end;
  if (end % 4 === 1) return fail(`Invalid input length: ${end}`);
  if (pads > 2 || (pads > 0 && (end + pads) % 4 !== 0) || (pads === 0 && end % 4 !== 0)) return fail("Invalid padding");
  const out = [];
  let acc = 0;
  let bits = 0;
  for (let i = 0; i < end; i++) {
    acc = (acc << 6) | B64.indexOf(String.fromCharCode(bytes[i]));
    bits += 6;
    if (bits >= 8) {
      bits -= 8;
      out.push((acc >> bits) & 0xff);
    }
  }
  if (bits > 0 && (acc & ((1 << bits) - 1)) !== 0) {
    return fail(`Invalid last symbol ${bytes[end - 1]}, offset ${end - 1}.`);
  }
  return new TextDecoder().decode(new Uint8Array(out));
}

/** The message of a JavaScript function that failed, for the module. */
let pendingError = null;

// ----- JSON -----

// `json.decode` reads text as serde_json does in the native runtime, so the
// browser gives the same values and the same messages. The reader builds a
// tree in document order: a syntax error becomes a `JErr` where it was
// found (the tree ends there), and the decoders in the module meet it in
// the order serde_json would, after any type mismatch before it.

/** A number, with its text, so integers keep all 64 bits. */
class JNum {
  constructor(n, src) {
    this.n = n;
    this.src = src;
  }
}

class JErr {
  constructor(message) {
    this.message = message;
  }
}

class JArr {
  constructor() {
    this.items = [];
    this.err = null;
  }
}

class JObj {
  constructor() {
    this.keys = [];
    this.vals = [];
    // Where each member starts (the `,` before it), for messages.
    this.at = [];
    this.end = "";
    this.err = null;
  }
}

const MISSING = Symbol("missing");

/** Text after a value: checked once the value is decoded. */
let jsonTrailing = null;

class JsonReader {
  constructor(text) {
    this.text = text;
    this.i = 0;
  }

  /** "line L, column C" of the character at `i` (1-based, in bytes). */
  where(i) {
    const before = this.text.slice(0, i);
    const line = before.split("\n").length;
    const start = before.lastIndexOf("\n") + 1;
    const col = utf8.encode(this.text.slice(start, i)).length + 1;
    return `line ${line}, column ${col}`;
  }

  /** An error at the character `i`, or at the end of the text. */
  error(what, i = this.i) {
    if (i >= this.text.length) {
      return new JErr(`invalid JSON: ${what}, but the text ended`);
    }
    return new JErr(`invalid JSON at ${this.where(i)}: ${what}`);
  }

  space() {
    while (this.i < this.text.length) {
      const c = this.text[this.i];
      if (c === " " || c === "\t" || c === "\n" || c === "\r") this.i++;
      else break;
    }
  }

  /** A value, or a `JErr` where it fails, and sets `this.failed`. Arrays
   *  and objects are read with a stack of their own, so any depth works. */
  value() {
    const t = this.text;
    const stack = [];
    for (;;) {
      this.space();
      let v;
      const c = t[this.i];
      if (c === undefined) {
        v = this.fail(this.error("expected a value"));
      } else if (c === "[") {
        this.i++;
        const a = new JArr();
        this.space();
        if (t[this.i] === "]") {
          this.i++;
          v = a;
        } else {
          stack.push(a);
          continue;
        }
      } else if (c === "{") {
        const o = new JObj();
        const at = this.where(this.i);
        this.i++;
        this.space();
        if (t[this.i] === "}") {
          o.end = this.where(this.i);
          this.i++;
          v = o;
        } else if (this.member(o, at)) {
          stack.push(o);
          continue;
        } else {
          v = o;
        }
      } else {
        v = this.scalar(c);
      }
      // `v` is done: add it to the arrays and objects it is in.
      for (;;) {
        if (stack.length === 0) return v;
        const top = stack[stack.length - 1];
        if (top instanceof JArr) top.items.push(v);
        else top.vals.push(v);
        if (this.failed) {
          v = stack.pop();
          continue;
        }
        this.space();
        const close = top instanceof JArr ? "]" : "}";
        const d = t[this.i];
        if (d === close) {
          if (top instanceof JObj) top.end = this.where(this.i);
          this.i++;
          v = stack.pop();
          continue;
        }
        if (d !== ",") {
          const what = d === undefined ? "expected a value" : `expected \`,\` or \`${close}\``;
          top.err = this.fail(this.error(what));
          v = stack.pop();
          continue;
        }
        const at = this.where(this.i);
        this.i++;
        this.space();
        if (t[this.i] === close) {
          top.err = this.fail(this.error("expected another value after `,`"));
          v = stack.pop();
          continue;
        }
        if (top instanceof JObj && !this.member(top, at)) {
          v = stack.pop();
          continue;
        }
        break;
      }
    }
  }

  /** A member's name and `:`, at the start of `o`'s next member. */
  member(o, at) {
    this.space();
    if (this.i >= this.text.length) {
      o.err = this.fail(this.error("expected a value"));
      return false;
    }
    if (this.text[this.i] !== '"') {
      o.err = this.fail(this.error("expected a member name in quotes"));
      return false;
    }
    const key = this.string();
    if (this.failed) {
      o.err = key;
      return false;
    }
    this.space();
    if (this.text[this.i] !== ":") {
      const what = this.i >= this.text.length ? "expected a value" : "expected `:` after the member name";
      o.err = this.fail(this.error(what));
      return false;
    }
    this.i++;
    o.keys.push(key);
    o.at.push(at);
    return true;
  }

  scalar(c) {
    switch (c) {
      case '"':
        return this.string();
      case "t":
        return this.word("true", true);
      case "f":
        return this.word("false", false);
      case "n":
        return this.word("null", null);
      default:
        if (c === "-" || (c >= "0" && c <= "9")) return this.number();
        return this.fail(this.error("expected a value"));
    }
  }

  fail(err) {
    this.failed = true;
    return err;
  }

  word(w, v) {
    for (let k = 0; k < w.length; k++) {
      if (this.text[this.i] !== w[k]) {
        return this.fail(this.error(k === 0 ? "expected a value" : "expected a value"));
      }
      this.i++;
    }
    return v;
  }

  number() {
    const t = this.text;
    const start = this.i;
    const digit = (c) => c >= "0" && c <= "9";
    if (t[this.i] === "-") this.i++;
    if (this.i >= t.length) return this.fail(this.error("expected a value"));
    if (t[this.i] === "0") {
      this.i++;
      if (digit(t[this.i])) return this.fail(this.error("not a valid number"));
    } else if (digit(t[this.i])) {
      while (digit(t[this.i])) this.i++;
    } else {
      return this.fail(this.error("not a valid number"));
    }
    if (t[this.i] === ".") {
      this.i++;
      if (!digit(t[this.i])) return this.fail(this.error("not a valid number"));
      while (digit(t[this.i])) this.i++;
    }
    if (t[this.i] === "e" || t[this.i] === "E") {
      this.i++;
      if (t[this.i] === "+" || t[this.i] === "-") this.i++;
      if (!digit(t[this.i])) return this.fail(this.error("not a valid number"));
      while (digit(t[this.i])) this.i++;
    }
    const src = t.slice(start, this.i);
    const n = Number(src);
    if (!Number.isFinite(n)) return this.fail(this.error("the number is too large", this.i - 1));
    return new JNum(n, src);
  }

  /** Four hex digits: -1 when the text ends, -2 when one is not. */
  hex4() {
    const h = this.text.slice(this.i, this.i + 4);
    if (h.length < 4) return -1;
    this.i += 4;
    return /^[0-9a-fA-F]{4}$/.test(h) ? parseInt(h, 16) : -2;
  }

  string() {
    const t = this.text;
    this.i++;
    let out = "";
    let run = this.i;
    for (;;) {
      if (this.i >= t.length) return this.fail(this.error("expected the end of the string"));
      const c = t[this.i];
      if (c === '"') {
        out += t.slice(run, this.i);
        this.i++;
        return out;
      }
      if (c < " ") return this.fail(this.error("control characters must be escaped in strings"));
      if (c !== "\\") {
        this.i++;
        continue;
      }
      out += t.slice(run, this.i);
      this.i++;
      const e = t[this.i];
      if (e === undefined) return this.fail(this.error("expected the end of the string"));
      this.i++;
      const simple = { '"': '"', "\\": "\\", "/": "/", b: "\b", f: "\f", n: "\n", r: "\r", t: "\t" }[e];
      if (simple !== undefined) {
        out += simple;
      } else if (e === "u") {
        const n = this.hex4();
        if (n === -1) return this.fail(this.error("expected the end of the string"));
        if (n === -2) return this.fail(this.error("unknown escape", this.i - 1));
        if (n >= 0xdc00 && n <= 0xdfff) {
          return this.fail(this.error("unexpected second half of a surrogate pair", this.i - 1));
        }
        if (n >= 0xd800 && n <= 0xdbff) {
          if (t[this.i] !== "\\" || t[this.i + 1] !== "u") {
            if (this.i >= t.length) return this.fail(this.error("expected the end of the string"));
            return this.fail(this.error("expected the second half of a surrogate pair"));
          }
          this.i += 2;
          const m = this.hex4();
          if (m === -1) return this.fail(this.error("expected the end of the string"));
          if (m === -2) return this.fail(this.error("unknown escape", this.i - 1));
          if (m < 0xdc00 || m > 0xdfff) {
            return this.fail(this.error("expected the second half of a surrogate pair", this.i - 1));
          }
          out += String.fromCharCode(n, m);
        } else {
          out += String.fromCharCode(n);
        }
      } else {
        return this.fail(this.error("unknown escape", this.i - 1));
      }
      run = this.i;
    }
  }

}

function jParse(text) {
  const r = new JsonReader(text);
  r.failed = false;
  jsonTrailing = null;
  const v = r.value();
  if (!r.failed) {
    r.space();
    if (r.i < text.length) jsonTrailing = r.error("unexpected text after the value").message;
  }
  return v;
}

function jKind(v) {
  if (v === MISSING || v === undefined) return 6;
  if (v === null) return 0;
  if (v instanceof JErr) return 9;
  if (v instanceof JNum) return 2;
  if (v instanceof JArr) return 4;
  if (v instanceof JObj) return 5;
  return typeof v === "boolean" ? 1 : 3;
}

/** The first syntax error inside a value the decoder skips. */
function jSkipErr(v) {
  const work = [v];
  while (work.length > 0) {
    const x = work.pop();
    if (x instanceof JErr) return x.message;
    if (x instanceof JArr || x instanceof JObj) {
      if (x.err) work.push(x.err);
      const parts = x instanceof JArr ? x.items : x.vals;
      for (let i = parts.length - 1; i >= 0; i--) work.push(parts[i]);
    }
  }
  return null;
}

const FOUND = ["null", "a boolean", "a number", "a string", "an array", "an object", "nothing"];

/** Whether a number is an `Int`: 0 yes, 1 not a whole number, 2 too large. */
function jIntOk(v) {
  const src = v.src;
  if (/^-?[0-9]+$/.test(src)) {
    const n = BigInt(src);
    if (n >= I64_MIN && n <= I64_MAX) return 0;
    if (n > I64_MAX && n <= (1n << 64n) - 1n) return 2;
    // serde_json reads a larger integer as a float.
    return Math.abs(v.n) >= 9.2e18 ? 2 : 1;
  }
  if (v.n === 0 && Object.is(v.n, -0)) return 0;
  if (Number.isInteger(v.n) && Math.abs(v.n) >= 9.2e18) return 2;
  return 1;
}

function jInt(v) {
  if (/^-?[0-9]+$/.test(v.src)) return BigInt(v.src);
  return 0n;
}

/** A float as C's `%g` with the fewest digits that read back the same. */
function jFloat(f) {
  if (!Number.isFinite(f)) return "null";
  if (f === 0) return Object.is(f, -0) ? "-0" : "0";
  const e = f.toExponential();
  const [mant, expText] = e.split("e");
  const exp = Number(expText);
  const neg = mant.startsWith("-");
  const digits = mant.replace(/[^0-9]/g, "");
  const p = digits.length;
  let s = neg ? "-" : "";
  if (exp < -4 || exp >= p) {
    s += digits[0];
    if (p > 1) s += "." + digits.slice(1);
    const ax = Math.abs(exp);
    s += `e${exp < 0 ? "-" : "+"}${ax < 10 ? "0" : ""}${ax}`;
  } else if (exp >= 0) {
    s += digits.slice(0, exp + 1);
    if (exp + 1 < p) s += "." + digits.slice(exp + 1);
  } else {
    s += "0." + "0".repeat(-exp - 1) + digits;
  }
  return s;
}

function jsonImports() {
  return {
    j_quote: (s) => JSON.stringify(s),
    j_float: jFloat,
    j_parse: jParse,
    j_trailing: () => jsonTrailing,
    j_kind: jKind,
    j_bool: (v) => (v ? 1 : 0),
    j_num: (v) => v.n,
    j_int_ok: jIntOk,
    j_int: jInt,
    j_len: (v) => (v instanceof JArr ? v.items.length : v.keys.length),
    j_at: (a, i) => a.items[i],
    j_key: (o, i) => o.keys[i],
    j_val: (o, i) => o.vals[i],
    j_error: (v) => (v instanceof JErr ? v.message : v.err ? v.err.message : null),
    j_skip_err: jSkipErr,
    j_extra_member: (o) => (o.keys.length > 1 ? `invalid JSON at ${o.at[1]}: expected \`}\` after the variant` : null),
    j_empty_object: (o) => `invalid JSON at ${o.end}: expected a member name in quotes`,
    j_mismatch: (expected, path, v) => `cannot read JSON: expected ${expected} at $${path}, found ${FOUND[jKind(v)] ?? "a value"}`,
    j_index: (path, i) => `${path}[${i}]`,
    j_int_key: (k) => {
      if (!/^-?[0-9]+$/.test(k)) return 0;
      const n = BigInt(k);
      return n >= I64_MIN && n <= I64_MAX ? 1 : 0;
    },
    j_int_key_value: (k) => BigInt(k),
  };
}

// ----- the imports -----

/** The functions every module imports, as module `k`. */
export function kilnImports() {
  return {
    print: (s) => io.out(s),
    eprint: (s) => io.err(s),
    panic,
    panic_code: (c, a, b) => panicCode(c, a, b),
    fail_exit: (message) => {
      throw new KilnExit(message);
    },
    concat: (a, b) => a + b,
    str_eq: (a, b) => (a === b ? 1 : 0),
    str_cmp: strCmp,
    str_hash: strHash,
    str_len: (s) => BigInt(codeLen(s)),
    str_empty: (s) => (s === "" ? 1 : 0),
    str_upper: asciiUpper,
    str_lower: asciiLower,
    str_trim: (s) => s.replace(SPACE, ""),
    str_trim_start: (s) => s.replace(/^[ \t\n\r]+/, ""),
    str_trim_end: (s) => s.replace(/[ \t\n\r]+$/, ""),
    str_contains: (s, p) => (s.includes(p) ? 1 : 0),
    str_starts_with: (s, p) => (s.startsWith(p) ? 1 : 0),
    str_ends_with: (s, p) => (s.endsWith(p) ? 1 : 0),
    str_replace: (s, from, to) => (from === "" ? s : s.split(from).join(to)),
    str_repeat: repeat,
    str_slice: slice,
    str_pad: (s, w, fill, start) => pad(s, w, fill, start !== 0),
    str_index_of: indexOf,
    str_to_int_ok: toIntOk,
    str_to_int: (s) => BigInt(s.trim()),
    str_to_float_ok: toFloatOk,
    str_to_float: (s) => Number(s),
    str_split: split,
    str_chars: chars,
    str_lines: lines,
    str_quote: quote,
    arr_len: (a) => a.length,
    arr_get: (a, i) => a[i],
    int_str: (n) => n.toString(),
    float_str: floatStr,
    float_fixed: floatFixed,
    duration_str: durationStr,
    float_rem: floatRem,
    html_escape: htmlEscape,
    safe_url: (u) => (isSafeUrl(u) ? u : UNSAFE_URL),
    ui_merge: uiMerge,
    base64_encode: base64Encode,
    base64_decode: base64Decode,
    take_error: () => {
      const e = pendingError;
      pendingError = null;
      return e;
    },
    log: (level, message, data) => io.err(`${level} ${message}${data === "" ? "" : " " + data}`),
    ...jsonImports(),
  };
}

/** `main` returned an error: the program ends with it. */
export class KilnExit extends Error {
  constructor(message) {
    super(message);
    this.name = "KilnExit";
    this.kilnMessage = message;
  }
}

const RUNTIME_STRINGS = {
  true: "true",
  false: "false",
  empty: "",
  lbracket: "[",
  rbracket: "]",
  comma: ", ",
  lbrace: "{",
  rbrace: "}",
  colon: ": ",
};

/** Compiles and instantiates a Kiln module. `extra` adds import modules
 *  (the DOM bridge, npm bindings) to the ones every module has. */
export async function instantiate(bytes, extra = {}) {
  const module = bytes instanceof WebAssembly.Module ? bytes : await WebAssembly.compile(bytes);
  const section = WebAssembly.Module.customSections(module, "kiln.strings")[0];
  const list = section ? JSON.parse(new TextDecoder().decode(section)) : [];
  const s = {};
  list.forEach((text, i) => {
    s[i] = text;
  });
  const imports = { k: { ...kilnImports(), ...(extra.k ?? {}) }, r: RUNTIME_STRINGS, s };
  for (const [name, value] of Object.entries(extra)) {
    if (name !== "k") imports[name] = value;
  }
  const instance = await WebAssembly.instantiate(module, imports);
  return instance.exports;
}

/** Bytes from Base64, in Node or a browser. */
export function fromBase64(b64) {
  if (typeof Buffer !== "undefined") return Buffer.from(b64, "base64");
  return Uint8Array.from(atob(b64), (c) => c.charCodeAt(0));
}

/** Runs a program's `main` and collects what it writes: `{ out, err,
 *  code }`, with code 101 for a panic and 1 for an error, as the native
 *  build exits. */
export async function runCollect(bytes) {
  const out = [];
  const err = [];
  io.out = (s) => out.push(s, "\n");
  io.err = (s) => err.push(s, "\n");
  let code = 0;
  try {
    const exports = await instantiate(typeof bytes === "string" ? fromBase64(bytes) : bytes);
    exports.main();
  } catch (e) {
    if (e instanceof KilnPanic) {
      err.push(`panic: ${e.kilnMessage}\n`);
      code = 101;
    } else if (e instanceof KilnExit) {
      err.push(`error: ${e.kilnMessage}\n`);
      code = 1;
    } else if (e instanceof RangeError && /call stack/.test(e.message)) {
      err.push("panic: stack overflow\n");
      code = 101;
    } else {
      throw e;
    }
  }
  return { out: out.join(""), err: err.join(""), code };
}

/** Runs a program's `main` under Node, as the native build would: its
 *  output, and exit code 101 for a panic and 1 for an error. */
export async function runMain(b64) {
  const fs = await import("node:fs");
  let out = [];
  let size = 0;
  const flush = () => {
    if (out.length > 0) fs.writeSync(1, out.join(""));
    out = [];
    size = 0;
  };
  io.out = (s) => {
    out.push(s, "\n");
    size += s.length;
    if (size > 1 << 16) flush();
  };
  io.err = (s) => {
    flush();
    fs.writeSync(2, s + "\n");
  };
  let code = 0;
  try {
    const exports = await instantiate(fromBase64(b64));
    exports.main();
  } catch (e) {
    flush();
    if (e instanceof KilnPanic) {
      fs.writeSync(2, `panic: ${e.kilnMessage}\n`);
      code = 101;
    } else if (e instanceof KilnExit) {
      fs.writeSync(2, `error: ${e.kilnMessage}\n`);
      code = 1;
    } else if (e instanceof RangeError && /call stack/.test(e.message)) {
      fs.writeSync(2, "panic: stack overflow\n");
      code = 101;
    } else {
      throw e;
    }
  }
  flush();
  process.exitCode = code;
}
