//! `json.encode` and `json.decode` in the browser.
//!
//! Each type gets an encoder, which writes a value (as a slot) as JSON
//! text, and a decoder, which reads a value of the type from what
//! `JSON.parse` gave. Both follow the native runtime's `json.rs`: the same
//! shapes, the same float digits and the same messages for data of the
//! wrong shape.

use super::*;

const IMPORTS: &[&str] = &[
    "  (import \"k\" \"j_quote\" (func $k_j_quote (param externref) (result externref)))",
    "  (import \"k\" \"j_float\" (func $k_j_float (param f64) (result externref)))",
    "  (import \"k\" \"j_parse\" (func $k_j_parse (param externref) (result externref)))",
    "  (import \"k\" \"j_kind\" (func $k_j_kind (param externref) (result i32)))",
    "  (import \"k\" \"j_bool\" (func $k_j_bool (param externref) (result i32)))",
    "  (import \"k\" \"j_num\" (func $k_j_num (param externref) (result f64)))",
    "  (import \"k\" \"j_int_ok\" (func $k_j_int_ok (param externref) (result i32)))",
    "  (import \"k\" \"j_int\" (func $k_j_int (param externref) (result i64)))",
    "  (import \"k\" \"j_trailing\" (func $k_j_trailing (result externref)))",
    "  (import \"k\" \"j_len\" (func $k_j_len (param externref) (result i32)))",
    "  (import \"k\" \"j_at\" (func $k_j_at (param externref i32) (result externref)))",
    "  (import \"k\" \"j_key\" (func $k_j_key (param externref i32) (result externref)))",
    "  (import \"k\" \"j_val\" (func $k_j_val (param externref i32) (result externref)))",
    "  (import \"k\" \"j_error\" (func $k_j_error (param externref) (result externref)))",
    "  (import \"k\" \"j_skip_err\" (func $k_j_skip_err (param externref) (result externref)))",
    "  (import \"k\" \"j_extra_member\" (func $k_j_extra_member (param externref) (result externref)))",
    "  (import \"k\" \"j_empty_object\" (func $k_j_empty_object (param externref) (result externref)))",
    "  (import \"k\" \"j_mismatch\" (func $k_j_mismatch (param externref externref externref) (result externref)))",
    "  (import \"k\" \"j_index\" (func $k_j_index (param externref i32) (result externref)))",
    "  (import \"k\" \"j_int_key\" (func $k_j_int_key (param externref) (result i32)))",
    "  (import \"k\" \"j_int_key_value\" (func $k_j_int_key_value (param externref) (result i64)))",
];

// What `j_kind` says a value is.
const NULL: i32 = 0;
const BOOL: i32 = 1;
const NUMBER: i32 = 2;
const STRING: i32 = 3;
const ARRAY: i32 = 4;
const OBJECT: i32 = 5;
const MISSING: i32 = 6;
const ERROR: i32 = 9;

fn imports(g: &mut Gen) {
    for imp in IMPORTS {
        g.extra_imports.insert(imp.to_string());
    }
}

/// The text a JSON value of this type starts as, for messages.
fn expected(g: &Gen, ty: &Ty) -> &'static str {
    let p = g.prims;
    match ty {
        _ if g.is_string(ty) => "a string",
        Ty::Adt(id, _) if *id == p.bool_ => "true or false",
        Ty::Adt(id, _) if *id == p.int || *id == p.duration => "an integer",
        Ty::Adt(id, _) if *id == p.float => "a number",
        Ty::Adt(id, _) if *id == p.list || *id == p.set => "an array",
        Ty::Adt(id, _) if *id == p.map => "an object",
        Ty::Record(_) => "an object",
        Ty::Adt(id, _) if !g.p.is_prim(*id) => match g.p.adts[*id].kind {
            AdtKind::Struct(_) => "an object",
            AdtKind::Enum(_) => "a string or an object",
        },
        _ => "a type JSON can hold",
    }
}

/// A `$ShowF` that writes a value of `ty` (a slot) as JSON text.
pub(super) fn encode_fn(g: &mut Gen, ty: &Ty, span: Span) -> String {
    imports(g);
    let (name, fresh) = g.helper(format!("jenc:{ty:?}"), "jenc");
    if !fresh {
        return name;
    }
    let p = g.prims;
    let body = g.in_helper(|g| match ty {
        _ if g.is_string(ty) => {
            g.line("(call $k_j_quote (extern.convert_any (local.get 0)))");
        }
        Ty::Adt(id, _) if *id == p.int || *id == p.duration => {
            g.line("(call $k_int_str (call $k_i64 (local.get 0)))");
        }
        Ty::Adt(id, _) if *id == p.float => {
            g.line("(call $k_j_float (call $k_f64 (local.get 0)))");
        }
        Ty::Adt(id, _) if *id == p.bool_ => {
            g.line("(call $k_bool_str (call $k_i32 (local.get 0)))");
        }
        Ty::Opt(inner) => {
            let f = encode_fn(g, inner, span);
            let null = g.str_const("null");
            g.line("(if (result externref) (ref.is_null (local.get 0))");
            g.line(&format!("  (then {null})"));
            g.line(&format!(
                "  (else (call {f} (struct.get $Opt 0 (ref.cast (ref $Opt) (local.get 0))))))"
            ));
        }
        Ty::Adt(id, args) if *id == p.list || *id == p.set => {
            let f = encode_fn(g, &args[0], span);
            let r = g.funcref(&f);
            let (open, comma, close) = (g.str_const("["), g.str_const(","), g.str_const("]"));
            let items = g.tmp("externref");
            if *id == p.set {
                g.line(&format!("(local.set {items} (call $k_list_join (call $k_map_list (ref.cast (ref null $Map) (local.get 0)) (i32.const 0)) ({comma}) ({r})))"));
            } else {
                g.line(&format!("(local.set {items} (call $k_list_join (ref.cast (ref null $List) (local.get 0)) ({comma}) ({r})))"));
            }
            g.line(&format!(
                "(call $k_concat (call $k_concat ({open}) (local.get {items})) ({close}))"
            ));
        }
        Ty::Adt(id, args) if *id == p.map => {
            let kt = args[0].clone();
            let k = encode_fn(g, &kt, span);
            let v = encode_fn(g, &args[1], span);
            let key_is_string = g.is_string(&kt);
            let (open, comma, colon, close) = (
                g.str_const("{"),
                g.str_const(","),
                g.str_const(":"),
                g.str_const("}"),
            );
            let (l, i, r, e) = (
                g.tmp("(ref null $List)"),
                g.tmp("i32"),
                g.tmp("externref"),
                g.tmp("(ref null $Slots)"),
            );
            g.line(&format!("(local.set {l} (call $k_map_list (ref.cast (ref null $Map) (local.get 0)) (i32.const 2)))"));
            g.line(&format!("(local.set {r} ({open}))"));
            g.line("(block $done (loop $next");
            g.line(&format!("(br_if $done (i32.ge_s (local.get {i}) (call $k_len (local.get {l}))))"));
            g.line(&format!("(if (local.get {i}) (then (local.set {r} (call $k_concat (local.get {r}) ({comma})))))"));
            g.line(&format!("(local.set {e} (ref.cast (ref null $Slots) (call $k_item (local.get {l}) (local.get {i}))))"));
            let key = if key_is_string {
                format!("(call {k} (array.get $Slots (local.get {e}) (i32.const 0)))")
            } else {
                format!("(call $k_j_quote (call {k} (array.get $Slots (local.get {e}) (i32.const 0))))")
            };
            g.line(&format!("(local.set {r} (call $k_concat (local.get {r}) {key}))"));
            g.line(&format!("(local.set {r} (call $k_concat (local.get {r}) ({colon})))"));
            g.line(&format!("(local.set {r} (call $k_concat (local.get {r}) (call {v} (array.get $Slots (local.get {e}) (i32.const 1)))))"));
            g.line(&format!("(local.set {i} (i32.add (local.get {i}) (i32.const 1)))"));
            g.line("(br $next)))");
            g.line(&format!("(call $k_concat (local.get {r}) ({close}))"));
        }
        Ty::Record(fields) => {
            let fields: Vec<(String, String, Ty)> = fields
                .iter()
                .enumerate()
                .map(|(i, (n, t))| {
                    (
                        n.clone(),
                        format!("(array.get $Slots (ref.cast (ref $Slots) (local.get 0)) (i32.const {i}))"),
                        t.clone(),
                    )
                })
                .collect();
            object(g, &fields, span);
        }
        Ty::Adt(id, args) if !g.p.is_prim(*id) => {
            let adt = g.p.adts[*id].clone();
            let map = g.type_map(*id, args);
            match &adt.kind {
                AdtKind::Struct(fields) => {
                    let mut out = Vec::new();
                    for (i, f) in fields.iter().enumerate() {
                        let slot = g.slot_text(&f.ty);
                        let mut access = format!(
                            "(struct.get $S{id} {i} (ref.cast (ref $S{id}) (local.get 0)))"
                        );
                        for op in slot.lines().map(str::trim) {
                            access = format!("({op} {access})");
                        }
                        out.push((f.name.clone(), access, substitute(&f.ty, &map)));
                    }
                    object(g, &out, span);
                }
                AdtKind::Enum(variants) => {
                    let x = g.tmp("(ref null $Enum)");
                    g.line(&format!("(local.set {x} (ref.cast (ref null $Enum) (local.get 0)))"));
                    for (i, var) in variants.iter().enumerate() {
                        g.line(&format!("(if (i32.eq (struct.get $Enum 0 (local.get {x})) (i32.const {i})) (then"));
                        let field = |j: usize| {
                            format!("(array.get $Slots (struct.get $Enum 1 (local.get {x})) (i32.const {j}))")
                        };
                        if var.fields.is_empty() {
                            let c = g.str_const(&quote(&var.name));
                            g.line(&format!("(return ({c}))"));
                        } else {
                            let head = g.str_const(&format!("{{{}:", quote(&var.name)));
                            let close = g.str_const("}");
                            let inner = g.tmp("externref");
                            let named = var.fields.iter().all(|(n, _)| n.is_some());
                            let tys: Vec<Ty> =
                                var.fields.iter().map(|(_, t)| substitute(t, &map)).collect();
                            let saved = std::mem::take(&mut g.f.body);
                            if named {
                                let fs: Vec<(String, String, Ty)> = var
                                    .fields
                                    .iter()
                                    .enumerate()
                                    .map(|(j, (n, _))| {
                                        (n.clone().unwrap_or_default(), field(j), tys[j].clone())
                                    })
                                    .collect();
                                object(g, &fs, span);
                            } else if tys.len() == 1 {
                                let f = encode_fn(g, &tys[0], span);
                                g.line(&format!("(call {f} {})", field(0)));
                            } else {
                                let (open, comma, close) =
                                    (g.str_const("["), g.str_const(","), g.str_const("]"));
                                g.line(&format!("({open})"));
                                for (j, t) in tys.iter().enumerate() {
                                    let f = encode_fn(g, t, span);
                                    if j > 0 {
                                        g.line(&format!("(call $k_concat ({comma}))"));
                                    }
                                    g.line(&format!("(call {f} {})", field(j)));
                                    g.line("call $k_concat");
                                }
                                g.line(&format!("({close})"));
                                g.line("call $k_concat");
                            }
                            let value = std::mem::replace(&mut g.f.body, saved);
                            g.f.body.push_str(&value);
                            g.line(&format!("local.set {inner}"));
                            g.line(&format!("(return (call $k_concat (call $k_concat ({head}) (local.get {inner})) ({close})))"));
                        }
                        g.line("))");
                    }
                    g.line("global.get $s_empty");
                }
            }
        }
        _ => {
            g.diags.push(Diagnostic::error(
                codes::NOT_COMPILED_YET,
                "writing a value of this type as JSON is not supported in the browser yet",
                span,
            ));
            g.line("global.get $s_empty");
        }
    });
    g.emit_helper(&format!("(func {name} (type $ShowF)"), body);
    name
}

/// Pushes `{"a":...,"b":...}` for the fields given as (name, access to the
/// slot, type).
fn object(g: &mut Gen, fields: &[(String, String, Ty)], span: Span) {
    let mut first = true;
    let r = g.tmp("externref");
    let open = g.str_const("{");
    g.line(&format!("(local.set {r} ({open}))"));
    for (name, access, ty) in fields {
        let f = encode_fn(g, ty, span);
        let label = format!("{}{}:", if first { "" } else { "," }, quote(name));
        first = false;
        let l = g.str_const(&label);
        g.line(&format!(
            "(local.set {r} (call $k_concat (local.get {r}) ({l})))"
        ));
        g.line(&format!(
            "(local.set {r} (call $k_concat (local.get {r}) (call {f} {access})))"
        ));
    }
    let close = g.str_const("}");
    g.line(&format!("(call $k_concat (local.get {r}) ({close}))"));
}

/// A string in JSON, as serde_json writes it.
fn quote(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_default()
}

/// A decoder for `ty`: `(func (param externref value) (param externref
/// path) (result anyref))`, which returns the value as a slot, or sets
/// `$lib_error` and returns null.
pub(super) fn decode_fn(g: &mut Gen, ty: &Ty, span: Span) -> String {
    imports(g);
    let (name, fresh) = g.helper(format!("jdec:{ty:?}"), "jdec");
    if !fresh {
        return name;
    }
    let p = g.prims;
    let exp = expected(g, ty);
    let body = g.in_helper(|g| {
        let k = g.tmp("i32");
        g.line(&format!("(local.set {k} (call $k_j_kind (local.get 0)))"));
        g.line(&format!("(if (i32.eq (local.get {k}) (i32.const {ERROR})) (then (global.set $lib_error (call $k_j_error (local.get 0))) (return (ref.null any))))"));
        // Fails with "expected <what> at <path>, found <the value>".
        let mismatch = |g: &mut Gen, what: &str| {
            let w = g.str_const(what);
            format!("(global.set $lib_error (call $k_j_mismatch ({w}) (local.get 1) (local.get 0))) (return (ref.null any))")
        };
        let only = |g: &mut Gen, kind: i32, what: &str| {
            let fail = mismatch(g, what);
            g.line(&format!("(if (i32.ne (local.get {k}) (i32.const {kind})) (then {fail}))"));
        };
        // A part that failed fails the whole value.
        let check = "(if (i32.eqz (ref.is_null (global.get $lib_error))) (then (return (ref.null any))))";
        // An array or object that ends in a syntax error.
        let truncated = "(global.set $lib_error (call $k_j_error (local.get 0))) (if (i32.eqz (ref.is_null (global.get $lib_error))) (then (return (ref.null any))))";
        match ty {
            _ if g.is_string(ty) => {
                only(g, STRING, exp);
                g.line("(any.convert_extern (local.get 0))");
            }
            Ty::Adt(id, _) if *id == p.bool_ => {
                only(g, BOOL, exp);
                g.line("(ref.i31 (call $k_j_bool (local.get 0)))");
            }
            Ty::Adt(id, _) if *id == p.float => {
                only(g, NUMBER, exp);
                g.line("(struct.new $BoxF64 (call $k_j_num (local.get 0)))");
            }
            Ty::Adt(id, _) if *id == p.int || *id == p.duration => {
                only(g, NUMBER, exp);
                let ok = g.tmp("i32");
                let not_int = mismatch(g, exp);
                let too_big = mismatch(g, "an integer that fits in 64 bits");
                g.line(&format!("(local.set {ok} (call $k_j_int_ok (local.get 0)))"));
                g.line(&format!("(if (i32.eq (local.get {ok}) (i32.const 1)) (then {not_int}))"));
                g.line(&format!("(if (i32.eq (local.get {ok}) (i32.const 2)) (then {too_big}))"));
                g.line("(struct.new $BoxI64 (call $k_j_int (local.get 0)))");
            }
            Ty::Opt(inner) => {
                let f = decode_fn(g, inner, span);
                let v = g.tmp("anyref");
                g.line(&format!("(if (i32.or (i32.eq (local.get {k}) (i32.const {NULL})) (i32.eq (local.get {k}) (i32.const {MISSING}))) (then (return (ref.null any))))"));
                g.line(&format!("(local.set {v} (call {f} (local.get 0) (local.get 1)))"));
                g.line(check);
                g.line(&format!("(call $k_box (local.get {v}))"));
            }
            Ty::Adt(id, args) if *id == p.list || *id == p.set => {
                let set = *id == p.set;
                let item = args[0].clone();
                if set && !(g.is_string(&item) || g.wt(&item).as_deref() == Some("i64")) {
                    let fail = mismatch(g, "a set of strings or integers");
                    only(g, ARRAY, exp);
                    g.line(&fail);
                    g.line("ref.null any");
                    return;
                }
                only(g, ARRAY, exp);
                let f = decode_fn(g, &item, span);
                let (n, i, v) = (g.tmp("i32"), g.tmp("i32"), g.tmp("anyref"));
                g.line(&format!("(local.set {n} (call $k_j_len (local.get 0)))"));
                let out = if set {
                    let hash = g.hash_fn(&item, span);
                    let eq = g.eq_fn(&item, span);
                    let (h, q) = (g.funcref(&hash), g.funcref(&eq));
                    let m = g.tmp("(ref null $Map)");
                    g.line(&format!("(local.set {m} (call $k_map_new ({h}) ({q})))"));
                    m
                } else {
                    let l = g.tmp("(ref null $List)");
                    g.line(&format!("(local.set {l} (call $k_list (local.get {n})))"));
                    l
                };
                g.line("(block $done (loop $next");
                g.line(&format!("(br_if $done (i32.ge_s (local.get {i}) (local.get {n})))"));
                g.line(&format!("(local.set {v} (call {f} (call $k_j_at (local.get 0) (local.get {i})) (call $k_j_index (local.get 1) (local.get {i}))))"));
                g.line(check);
                if set {
                    g.line(&format!("(call $k_set_add (local.get {out}) (local.get {v}))"));
                } else {
                    g.line(&format!("(call $k_list_push (local.get {out}) (local.get {v}))"));
                }
                g.line(&format!("(local.set {i} (i32.add (local.get {i}) (i32.const 1)))"));
                g.line("(br $next)))");
                g.line(truncated);
                g.line(&format!("(local.get {out})"));
            }
            Ty::Adt(id, args) if *id == p.map => {
                let kt = args[0].clone();
                let string_key = g.is_string(&kt);
                if !string_key && g.wt(&kt).as_deref() != Some("i64") {
                    let fail = mismatch(g, "a map with `String` or `Int` keys");
                    only(g, OBJECT, exp);
                    g.line(&fail);
                    g.line("ref.null any");
                    return;
                }
                only(g, OBJECT, exp);
                let f = decode_fn(g, &args[1], span);
                let hash = g.hash_fn(&kt, span);
                let eq = g.eq_fn(&kt, span);
                let (h, q) = (g.funcref(&hash), g.funcref(&eq));
                let (m, n, i, key, v) = (
                    g.tmp("(ref null $Map)"),
                    g.tmp("i32"),
                    g.tmp("i32"),
                    g.tmp("externref"),
                    g.tmp("anyref"),
                );
                g.line(&format!("(local.set {m} (call $k_map_new ({h}) ({q})))"));
                g.line(&format!("(local.set {n} (call $k_j_len (local.get 0)))"));
                g.line("(block $done (loop $next");
                g.line(&format!("(br_if $done (i32.ge_s (local.get {i}) (local.get {n})))"));
                g.line(&format!("(local.set {key} (call $k_j_key (local.get 0) (local.get {i})))"));
                let dot = g.str_const(".");
                let path = format!("(call $k_concat (call $k_concat (local.get 1) ({dot})) (local.get {key}))");
                if !string_key {
                    let pre = g.str_const("cannot read JSON: expected an integer key at $");
                    let found = g.str_const(", found ");
                    g.line(&format!("(if (i32.eqz (call $k_j_int_key (local.get {key}))) (then (global.set $lib_error (call $k_concat (call $k_concat (call $k_concat ({pre}) (local.get 1)) ({found})) (call $k_j_quote (local.get {key})))) (return (ref.null any))))"));
                }
                g.line(&format!("(local.set {v} (call {f} (call $k_j_val (local.get 0) (local.get {i})) {path}))"));
                g.line(check);
                let k_slot = if string_key {
                    format!("(any.convert_extern (local.get {key}))")
                } else {
                    format!("(struct.new $BoxI64 (call $k_j_int_key_value (local.get {key})))")
                };
                g.line(&format!("(call $k_map_set (local.get {m}) {k_slot} (local.get {v}))"));
                g.line(&format!("(local.set {i} (i32.add (local.get {i}) (i32.const 1)))"));
                g.line("(br $next)))");
                g.line(truncated);
                g.line(&format!("(local.get {m})"));
            }
            Ty::Record(fields) => {
                only(g, OBJECT, exp);
                let fs: Vec<(String, Ty)> = fields.clone();
                let vals = read_fields(g, &fs, span);
                for v in &vals {
                    g.line(&format!("(local.get {v})"));
                }
                g.line(&format!("array.new_fixed $Slots {}", vals.len()));
            }
            Ty::Adt(id, args) if !g.p.is_prim(*id) => {
                let id = *id;
                let adt = g.p.adts[id].clone();
                let map = g.type_map(id, args);
                match &adt.kind {
                    AdtKind::Struct(fields) => {
                        only(g, OBJECT, exp);
                        if fields.is_empty() {
                            g.line("ref.null $Empty");
                            return;
                        }
                        let fs: Vec<(String, Ty)> = fields
                            .iter()
                            .map(|f| (f.name.clone(), substitute(&f.ty, &map)))
                            .collect();
                        let vals = read_fields(g, &fs, span);
                        for (v, f) in vals.iter().zip(fields) {
                            g.line(&format!("(local.get {v})"));
                            g.unslot(&f.ty);
                        }
                        g.line(&format!("struct.new $S{id}"));
                    }
                    AdtKind::Enum(variants) => {
                        let names: Vec<&str> = variants.iter().map(|v| v.name.as_str()).collect();
                        let names = match names.split_last() {
                            Some((last, rest)) if !rest.is_empty() => {
                                format!("{} or {last}", rest.join(", "))
                            }
                            _ => names.join(""),
                        };
                        let unknown = |g: &mut Gen, found: &str, with: &str| {
                            let pre = g.str_const(&format!("cannot read JSON: expected {names} at $"));
                            let mid = g.str_const(", found ");
                            let post = g.str_const(with);
                            format!("(global.set $lib_error (call $k_concat (call $k_concat (call $k_concat (call $k_concat ({pre}) (local.get 1)) ({mid})) (call $k_j_quote {found})) ({post}))) (return (ref.null any))")
                        };
                        let s = g.tmp("externref");
                        // "Name" for a variant without fields.
                        g.line(&format!("(if (i32.eq (local.get {k}) (i32.const {STRING})) (then"));
                        g.line(&format!("(local.set {s} (local.get 0))"));
                        for (i, var) in variants.iter().enumerate() {
                            if var.fields.is_empty() {
                                let c = g.str_const(&var.name);
                                g.line(&format!("(if (call $k_str_eq (local.get {s}) ({c})) (then (return (struct.new $Enum (i32.const {i}) (array.new_fixed $Slots 0)))))"));
                            }
                        }
                        let fail = unknown(g, &format!("(local.get {s})"), "");
                        g.line(&fail);
                        g.line("))");
                        only(g, OBJECT, exp);
                        // {"Name": fields} for one with them.
                        g.line("(if (i32.eqz (call $k_j_len (local.get 0))) (then (global.set $lib_error (call $k_j_error (local.get 0))) (if (ref.is_null (global.get $lib_error)) (then (global.set $lib_error (call $k_j_empty_object (local.get 0))))) (return (ref.null any))))");
                        g.line(&format!("(local.set {s} (call $k_j_key (local.get 0) (i32.const 0)))"));
                        let inner = g.tmp("externref");
                        let path = g.tmp("externref");
                        let dot = g.str_const(".");
                        g.line(&format!("(local.set {inner} (call $k_j_val (local.get 0) (i32.const 0)))"));
                        g.line(&format!("(local.set {path} (call $k_concat (call $k_concat (local.get 1) ({dot})) (local.get {s})))"));
                        for (i, var) in variants.iter().enumerate() {
                            if var.fields.is_empty() {
                                continue;
                            }
                            let c = g.str_const(&var.name);
                            g.line(&format!("(if (call $k_str_eq (local.get {s}) ({c})) (then"));
                            let tys: Vec<Ty> =
                                var.fields.iter().map(|(_, t)| substitute(t, &map)).collect();
                            let named = var.fields.iter().all(|(n, _)| n.is_some());
                            let vals = if named {
                                let fs: Vec<(String, Ty)> = var
                                    .fields
                                    .iter()
                                    .zip(&tys)
                                    .map(|((n, _), t)| (n.clone().unwrap_or_default(), t.clone()))
                                    .collect();
                                // The fields are read from the inner object.
                                let sub = sub_decoder(g, &fs, span);
                                let r = g.tmp("(ref null $Slots)");
                                g.line(&format!("(local.set {r} (ref.cast (ref null $Slots) (call {sub} (local.get {inner}) (local.get {path}))))"));
                                g.line(check);
                                g.line("(global.set $lib_error (call $k_j_extra_member (local.get 0)))");
                                g.line(check);
                                g.line(&format!("(return (struct.new $Enum (i32.const {i}) (local.get {r})))"));
                                g.line("))");
                                continue;
                            } else if tys.len() == 1 {
                                let f = decode_fn(g, &tys[0], span);
                                let v = g.tmp("anyref");
                                g.line(&format!("(local.set {v} (call {f} (local.get {inner}) (local.get {path})))"));
                                g.line(check);
                                vec![v]
                            } else {
                                let want = format!("{} items", tys.len());
                                let fail_kind = {
                                    let w = g.str_const("an array");
                                    format!("(global.set $lib_error (call $k_j_mismatch ({w}) (local.get {path}) (local.get {inner}))) (return (ref.null any))")
                                };
                                g.line(&format!("(if (i32.ne (call $k_j_kind (local.get {inner})) (i32.const {ARRAY})) (then {fail_kind}))"));
                                let pre = g.str_const(&format!("cannot read JSON: expected {want} at $"));
                                let mid = g.str_const(", found ");
                                let items = g.str_const(" items");
                                let more = g.str_const("more");
                                let n = g.tmp("i32");
                                g.line(&format!("(local.set {n} (call $k_j_len (local.get {inner})))"));
                                let at_path = |j: usize| {
                                    format!("(call $k_j_index (local.get {path}) (i32.const {j}))")
                                };
                                let count = tys.len();
                                g.line(&format!("(if (i32.lt_s (local.get {n}) (i32.const {count})) (then (global.set $lib_error (call $k_concat (call $k_concat (call $k_concat ({pre}) {}) ({mid})) (call $k_concat (call $k_int_str (i64.extend_i32_s (local.get {n}))) ({items})))) (return (ref.null any))))", at_path(0).replace("(i32.const 0)", &format!("(local.get {n})"))));
                                let mut vals = Vec::new();
                                for (j, t) in tys.iter().enumerate() {
                                    let f = decode_fn(g, t, span);
                                    let v = g.tmp("anyref");
                                    let ap = at_path(j);
                                    g.line(&format!("(local.set {v} (call {f} (call $k_j_at (local.get {inner}) (i32.const {j})) {ap}))"));
                                    g.line(check);
                                    vals.push(v);
                                }
                                let ap = at_path(count);
                                g.line(&format!("(if (i32.gt_s (local.get {n}) (i32.const {count})) (then (global.set $lib_error (call $k_concat (call $k_concat (call $k_concat ({pre}) {ap}) ({mid})) ({more}))) (return (ref.null any))))"));
                                vals
                            };
                            g.line("(global.set $lib_error (call $k_j_extra_member (local.get 0)))");
                            g.line(check);
                            let items: String =
                                vals.iter().map(|v| format!(" (local.get {v})")).collect();
                            g.line(&format!("(return (struct.new $Enum (i32.const {i}) (array.new_fixed $Slots {}{items})))", vals.len()));
                            g.line("))");
                        }
                        let fail = unknown(g, &format!("(local.get {s})"), " with fields");
                        g.line(&fail);
                        g.line("ref.null any");
                    }
                }
            }
            _ => {
                g.diags.push(Diagnostic::error(
                    codes::NOT_COMPILED_YET,
                    "reading a value of this type from JSON is not supported in the browser yet",
                    span,
                ));
                g.line("ref.null any");
            }
        }
    });
    g.emit_helper(
        &format!("(func {name} (param externref) (param externref) (result anyref)"),
        body,
    );
    name
}

/// A decoder that reads the named fields of an object into a `$Slots`.
fn sub_decoder(g: &mut Gen, fields: &[(String, Ty)], span: Span) -> String {
    let rec = Ty::Record(fields.to_vec());
    decode_fn(g, &rec, span)
}

/// Reads the fields of the object in local 0 (path in local 1) into new
/// locals, which hold slots, in the order of the text as serde_json does:
/// a missing field is an error unless it is optional, and members the type
/// does not have are skipped.
fn read_fields(g: &mut Gen, fields: &[(String, Ty)], span: Span) -> Vec<String> {
    let decoders: Vec<String> = fields.iter().map(|(_, t)| decode_fn(g, t, span)).collect();
    let vals: Vec<String> = fields.iter().map(|_| g.tmp("anyref")).collect();
    let seen: Vec<String> = fields.iter().map(|_| g.tmp("i32")).collect();
    let (n, i, key, v) = (
        g.tmp("i32"),
        g.tmp("i32"),
        g.tmp("externref"),
        g.tmp("externref"),
    );
    let fail =
        "(if (i32.eqz (ref.is_null (global.get $lib_error))) (then (return (ref.null any))))";
    g.line(&format!("(local.set {n} (call $k_j_len (local.get 0)))"));
    g.line("(block $done (loop $next");
    g.line(&format!(
        "(br_if $done (i32.ge_s (local.get {i}) (local.get {n})))"
    ));
    g.line(&format!(
        "(local.set {key} (call $k_j_key (local.get 0) (local.get {i})))"
    ));
    g.line(&format!(
        "(local.set {v} (call $k_j_val (local.get 0) (local.get {i})))"
    ));
    g.line(&format!(
        "(local.set {i} (i32.add (local.get {i}) (i32.const 1)))"
    ));
    for (j, (name, _)) in fields.iter().enumerate() {
        let c = g.str_const(name);
        let path = g.str_const(&format!(".{name}"));
        let f = &decoders[j];
        g.line(&format!("(if (call $k_str_eq (local.get {key}) ({c})) (then (local.set {} (call {f} (local.get {v}) (call $k_concat (local.get 1) ({path})))) {fail} (local.set {} (i32.const 1)) (br $next)))", vals[j], seen[j]));
    }
    g.line(&format!(
        "(global.set $lib_error (call $k_j_skip_err (local.get {v})))"
    ));
    g.line(fail);
    g.line("(br $next)))");
    g.line("(global.set $lib_error (call $k_j_error (local.get 0)))");
    g.line(fail);
    for (j, (name, ty)) in fields.iter().enumerate() {
        if matches!(ty, Ty::Opt(_)) {
            continue;
        }
        let pre = g.str_const(&format!("cannot read JSON: missing field `{name}` at $"));
        g.line(&format!("(if (i32.eqz (local.get {})) (then (global.set $lib_error (call $k_concat ({pre}) (local.get 1))) (return (ref.null any))))", seen[j]));
    }
    vals
}

/// `json.decode` of the text on the stack into a slot of `ty`, setting
/// `$lib_error` when the text is not JSON or not of that shape.
pub(super) fn decode(g: &mut Gen, ty: &Ty, span: Span) {
    imports(g);
    let f = decode_fn(g, ty, span);
    let v = g.tmp("externref");
    let r = g.tmp("anyref");
    g.line("call $k_j_parse");
    g.line(&format!("local.set {v}"));
    g.line(&format!(
        "(if (i32.eq (call $k_j_kind (local.get {v})) (i32.const 9))"
    ));
    g.line(&format!(
        "  (then (global.set $lib_error (call $k_j_error (local.get {v}))))"
    ));
    g.line(&format!(
        "  (else (local.set {r} (call {f} (local.get {v}) (global.get $s_empty)))"
    ));
    g.line("    (if (ref.is_null (global.get $lib_error)) (then (global.set $lib_error (call $k_j_trailing))))))");
    g.line(&format!("local.get {r}"));
}
