import { twice } from "./util.js";
export function shout(text) { return text.toUpperCase() + "!"; }
export function double(n) { return twice(n); }
export async function later(ms) {
  await new Promise((done) => setTimeout(done, ms ?? 10));
  return "done";
}
export const version = "1.0.0";
export function fail() { throw new Error("nope"); }
export function each(f) { f(); }
