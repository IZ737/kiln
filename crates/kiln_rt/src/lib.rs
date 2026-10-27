//! The part of the Kiln runtime written in Rust. It is a static library
//! linked into every Kiln program next to the C runtime (`kiln.c`), and it
//! exports `k_*` functions with the C ABI, so generated code and the C
//! runtime call them as they call any other runtime function. The plan for
//! what moves here, and why, is `docs/plans/runtime-rust.md`.
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

mod auth;
mod base64;
mod form;
mod html;
mod json;
mod markdown;
mod orm;
mod remote;
mod server;
mod sql;
mod timefmt;
mod ui;
mod validate;
mod value;
mod web;

pub use auth::{
    k_rt_admin_site, k_rt_auth_check, k_rt_auth_csrf, k_rt_auth_current, k_rt_auth_hash_password,
    k_rt_auth_require, k_rt_auth_sessions, k_rt_auth_sha256, k_rt_auth_sign_in, k_rt_auth_sign_out,
    k_rt_auth_sign_out_all, k_rt_auth_sweep, k_rt_auth_token, k_rt_auth_verify_password,
};
pub use base64::{k_rt_base64_decode, k_rt_base64_encode};
pub use form::k_rt_web_form;
pub use html::{k_rt_html_escape, k_rt_html_safe_url};
pub use json::{k_json_encode, k_rt_json_decode};
pub use markdown::k_rt_markdown_to_html;
pub use orm::{k_rt_orm_include, k_rt_orm_insert, k_rt_orm_like};
pub use server::k_rt_web_serve;
pub use sql::{
    k_rt_sql_begin, k_rt_sql_close, k_rt_sql_commit, k_rt_sql_dialect, k_rt_sql_error_kind,
    k_rt_sql_exec, k_rt_sql_insert, k_rt_sql_one, k_rt_sql_open, k_rt_sql_query, k_rt_sql_rollback,
    k_rt_sql_scalar, k_rt_sql_value,
};
pub use ui::k_rt_ui_merge;
pub use validate::{
    k_rt_validate_check, k_rt_validate_email, k_rt_validate_error, k_rt_validate_length,
    k_rt_validate_messages, k_rt_validate_one_of, k_rt_validate_problem, k_rt_validate_range,
    k_rt_validate_required, k_rt_validate_same, k_rt_validate_when,
};
pub use web::{k_rt_cookie, k_rt_path_decode, k_rt_query, k_web_reason};
