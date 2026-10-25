//! The runtime that native executables link with, built by `build.rs` and
//! carried inside `kiln`, and the signatures of the functions in it that
//! generated code calls.

#[cfg(kiln_runtime)]
macro_rules! runtime_file {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!(env!("OUT_DIR"), "/", $name)) as &[u8],
        )
    };
}

/// The runtime's files, in link order: musl's start files, the Kiln
/// runtime, the garbage collector and the C library. The generated object
/// goes after `crti.o`.
#[cfg(kiln_runtime)]
pub(crate) const FILES: &[(&str, &[u8])] = &[
    runtime_file!("crt1.o"),
    runtime_file!("crti.o"),
    runtime_file!("native.o"),
    runtime_file!("gc.o"),
    runtime_file!("libc.a"),
    runtime_file!("crtn.o"),
];

#[cfg(not(kiln_runtime))]
pub(crate) const FILES: &[(&str, &[u8])] = &[];

pub(crate) fn unavailable() -> Option<&'static str> {
    #[cfg(kiln_runtime)]
    return None;
    #[cfg(not(kiln_runtime))]
    return Some(include_str!(concat!(env!("OUT_DIR"), "/missing.txt")));
}

/// The C types of runtime parameters and results.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum C {
    /// `int64_t`, a pointer, or a `kv` slot.
    I64,
    F64,
    Bool,
    /// `int`.
    I32,
}

/// The signature of a runtime or C library function: its parameters and
/// result, with `None` for `void`.
pub(crate) fn signature(name: &str) -> Option<(&'static [C], Option<C>)> {
    use C::*;
    const P1: &[C] = &[I64];
    const P2: &[C] = &[I64, I64];
    const P3: &[C] = &[I64, I64, I64];
    let sig: (&'static [C], Option<C>) =
        match name {
            "k_take_error" | "k_time_now" | "k_os_args" | "k_os_cwd" | "k_os_temp_dir"
            | "k_os_pid" | "k_os_platform" | "k_io_read_line" | "k_io_read_all"
            | "k_io_read_lines" => (&[], Some(I64)),
            "k_alloc" | "k_neg" | "k_abs" | "k_int_str" | "k_duration_str" | "k_box"
            | "k_list_first" | "k_list_last" | "k_list_reverse" | "k_list_pop" | "k_hash_int"
            | "k_hash_float" | "k_hash_str" | "k_map_keys" | "k_map_values" | "k_map_entries"
            | "k_str_len" | "k_str_upper" | "k_str_lower" | "k_str_trim" | "k_str_trim_start"
            | "k_str_trim_end" | "k_str_to_int" | "k_str_to_float" | "k_str_chars"
            | "k_str_lines" | "k_env" | "k_time_year" | "k_time_month" | "k_time_day"
            | "k_time_unix" | "k_time_iso" | "k_time_parse" | "k_fs_read" | "k_fs_read_lines"
            | "k_fs_list" | "k_os_env" => (P1, Some(I64)),
            "k_panic" | "k_panic_c" | "k_print" | "k_time_sleep" | "k_fs_mkdir" | "k_fs_remove"
            | "k_os_exit" | "k_io_write" | "k_io_print_err" => (P1, None),
            "k_str_n" | "k_concat" | "k_add" | "k_sub" | "k_mul" | "k_div" | "k_rem" | "k_min"
            | "k_max" | "k_pow" | "k_closure" | "k_list" | "k_list_at" | "k_range"
            | "k_list_get" | "k_list_filter" | "k_list_map" | "k_list_find"
            | "k_list_remove_at" | "k_list_take" | "k_list_skip" | "k_list_concat"
            | "k_list_sort" | "k_list_count" | "k_list_flat_map" | "k_map_new" | "k_map_get"
            | "k_map_at" | "k_map_remove" | "k_str_split" | "k_str_index_of" | "k_str_repeat"
            | "k_time_format" | "k_desc_hash" => (P2, Some(I64)),
            "k_fail" | "k_list_push" | "k_set_add" | "k_fs_write" | "k_fs_append"
            | "k_fs_rename" => (P2, None),
            "k_str_eq" | "k_list_any" | "k_list_all" | "k_map_contains" | "k_set_remove"
            | "k_str_contains" | "k_str_starts_with" | "k_str_ends_with" => (P2, Some(Bool)),
            "k_str_cmp" => (P2, Some(I32)),
            "k_clamp" | "k_list_sort_by" | "k_list_join" | "k_list_index_of" | "k_list_slice"
            | "k_list_fold" | "k_str_replace" | "k_str_slice" | "k_str_pad_start"
            | "k_str_pad_end" => (P3, Some(I64)),
            "k_list_set" | "k_list_insert" | "k_map_set" => (P3, None),
            "k_list_contains" | "k_desc_eq" => (P3, Some(Bool)),
            "k_desc_cmp" => (P3, Some(I32)),
            "k_desc_show" => (&[I64, I64, Bool], Some(I64)),
            "k_enum" => (&[I64, I32, I64], Some(I64)),
            "k_record" => (&[I32, I64], Some(I64)),
            "k_list_extreme" => (&[I64, I64, I32], Some(I64)),
            "k_log" => (&[I64, I64, I32, I64], None),
            "k_float_str" | "k_float_to_int" => (&[F64], Some(I64)),
            "k_float_fixed" => (&[F64, I64], Some(I64)),
            "k_bool_str" => (&[Bool], Some(I64)),
            "k_fs_exists" | "k_fs_is_dir" => (P1, Some(Bool)),
            "round" | "floor" | "ceil" => (&[F64], Some(F64)),
            "fmod" | "fmin" | "fmax" => (&[F64, F64], Some(F64)),
            _ => return None,
        };
    Some(sig)
}
