//! The part of the Kiln runtime written in Rust. It is a static library
//! linked into every Kiln program next to the C runtime (`kiln.c`), and it
//! exports `k_*` functions with the C ABI, so generated code and the C
//! runtime call them as they call any other runtime function. The plan for
//! what moves here, and why, is `docs/runtime-rust.md`.
//!
//! Code here follows three rules:
//!
//! - **The collector does not see Rust's heap.** Boehm scans stacks and
//!   static data, not memory from Rust's allocator, so a Kiln value is never
//!   kept only in a `Vec`, `Box` or other heap memory of Rust's.
//! - **No Kiln panic jumps over Rust frames.** A panic in Kiln code
//!   `longjmp`s to the trap the C runtime set, so Rust code that calls back
//!   into Kiln goes through a C function that sets the trap below it.
//! - **Rust panics abort.** The library is built with `panic = "abort"`; a
//!   Rust panic is a bug in the runtime and never unwinds into C or
//!   generated code.

mod json;
mod server;
mod value;
mod web;

pub use json::{k_json_encode, k_rt_json_decode};
pub use server::k_rt_web_serve;
pub use web::{k_rt_cookie, k_rt_path_decode, k_rt_query, k_web_reason};
