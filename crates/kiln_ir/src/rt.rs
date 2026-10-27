//! Standard library functions that the runtime's Rust part (`kiln_rt`)
//! implements with one calling convention, so that both backends build
//! their calls from this table instead of a case each.
//!
//! The function `k_rt_<module>_<name>` takes a type description first when
//! [`RtFn::desc`] says so, then each argument as a `kv` slot (an `Int`, a
//! `Float`'s bits, a `Bool` as 0 or 1, or a pointer), and returns a `kv`,
//! which is 0 for a function with no value. A function that can fail
//! stores its message with `k_rt_fail` and returns 0, and the generated
//! code then fails with a standard `Error`, as for the other library
//! functions.

/// Which type the description passed first describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DescOf {
    /// None is passed.
    None,
    /// The call's type, such as `List<Post>` for `db.query<Post>(...)`.
    Ret,
    /// The type of argument `i`, under its coercion to `Any`.
    Arg(usize),
}

#[derive(Debug)]
pub struct RtFn {
    pub module: &'static str,
    /// The function's name, or `Type.method` with the receiver first.
    pub name: &'static str,
    pub symbol: &'static str,
    /// How many arguments, the receiver's included.
    pub args: usize,
    pub fallible: bool,
    pub desc: DescOf,
}

const fn f(
    module: &'static str,
    name: &'static str,
    symbol: &'static str,
    args: usize,
    fallible: bool,
    desc: DescOf,
) -> RtFn {
    RtFn {
        module,
        name,
        symbol,
        args,
        fallible,
        desc,
    }
}

use DescOf::*;

pub static RT_FNS: &[RtFn] = &[
    f("sql", "open", "k_rt_sql_open", 3, true, None),
    f("sql", "error_kind", "k_rt_sql_error_kind", 1, false, None),
    f("sql", "__value", "k_rt_sql_value", 1, false, Arg(0)),
    f("sql", "Db.query", "k_rt_sql_query", 3, true, Ret),
    f("sql", "Db.one", "k_rt_sql_one", 3, true, Ret),
    f("sql", "Db.scalar", "k_rt_sql_scalar", 3, true, Ret),
    f("sql", "Db.exec", "k_rt_sql_exec", 3, true, None),
    f("sql", "Db.insert", "k_rt_sql_insert", 3, true, None),
    f("sql", "Db.dialect", "k_rt_sql_dialect", 1, false, None),
    f("sql", "Db.close", "k_rt_sql_close", 1, false, None),
    f("sql", "Db.__begin", "k_rt_sql_begin", 1, true, None),
    f("sql", "Db.__commit", "k_rt_sql_commit", 1, true, None),
    f("sql", "Db.__rollback", "k_rt_sql_rollback", 1, false, None),
    f("admin", "site", "k_rt_admin_site", 6, false, None),
    f("auth", "sessions", "k_rt_auth_sessions", 5, true, None),
    f(
        "auth",
        "hash_password",
        "k_rt_auth_hash_password",
        1,
        true,
        None,
    ),
    f(
        "auth",
        "verify_password",
        "k_rt_auth_verify_password",
        2,
        false,
        None,
    ),
    f(
        "web",
        "Response.set_cookie",
        "k_rt_web_set_cookie",
        8,
        false,
        None,
    ),
    f("auth", "token", "k_rt_auth_token", 1, false, None),
    f("auth", "sha256", "k_rt_auth_sha256", 1, false, None),
    f(
        "auth",
        "Sessions.sign_in",
        "k_rt_auth_sign_in",
        4,
        true,
        None,
    ),
    f(
        "auth",
        "Sessions.sign_out",
        "k_rt_auth_sign_out",
        3,
        true,
        None,
    ),
    f(
        "auth",
        "Sessions.sign_out_all",
        "k_rt_auth_sign_out_all",
        2,
        true,
        None,
    ),
    f(
        "auth",
        "Sessions.current",
        "k_rt_auth_current",
        2,
        true,
        None,
    ),
    f(
        "auth",
        "Sessions.require",
        "k_rt_auth_require",
        2,
        true,
        None,
    ),
    f("auth", "Sessions.csrf", "k_rt_auth_csrf", 2, true, None),
    f("auth", "Sessions.check", "k_rt_auth_check", 3, true, None),
    f("auth", "Sessions.sweep", "k_rt_auth_sweep", 1, true, None),
    f("web", "Request.form", "k_rt_web_form", 1, true, Ret),
    f("validate", "check", "k_rt_validate_check", 1, false, None),
    f(
        "validate",
        "problem",
        "k_rt_validate_problem",
        2,
        false,
        None,
    ),
    f("validate", "when", "k_rt_validate_when", 3, false, None),
    f(
        "validate",
        "required",
        "k_rt_validate_required",
        2,
        false,
        None,
    ),
    f("validate", "length", "k_rt_validate_length", 4, false, None),
    f("validate", "range", "k_rt_validate_range", 4, false, None),
    f("validate", "email", "k_rt_validate_email", 2, false, None),
    f("validate", "one_of", "k_rt_validate_one_of", 3, false, None),
    f("validate", "same", "k_rt_validate_same", 3, false, None),
    f(
        "validate",
        "Checked.error",
        "k_rt_validate_error",
        2,
        false,
        None,
    ),
    f(
        "validate",
        "Checked.messages",
        "k_rt_validate_messages",
        1,
        false,
        None,
    ),
    f("orm", "__like", "k_rt_orm_like", 2, false, None),
    f("orm", "__include", "k_rt_orm_include", 7, true, Arg(1)),
    f("orm", "__insert", "k_rt_orm_insert", 5, true, Ret),
];

/// The entry for a library call, if the Rust runtime implements it.
pub fn rt_fn(module: &str, name: &str) -> Option<&'static RtFn> {
    RT_FNS.iter().find(|f| f.module == module && f.name == name)
}

/// The entry with this symbol.
pub fn rt_symbol(symbol: &str) -> Option<&'static RtFn> {
    RT_FNS.iter().find(|f| f.symbol == symbol)
}
