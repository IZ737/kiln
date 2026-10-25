//! `json`: encoding and decoding by type description, on `serde_json`.
//!
//! Structs and records are objects, lists and sets arrays, maps objects (a
//! key that is not a `String` is written as the JSON of the key, in a
//! string), optionals null when none, and a `Duration` a number of
//! nanoseconds. An enum variant without fields is its name as a string;
//! one with fields is an object with one member, the variant's name,
//! holding an object of the named fields, the one unnamed field, or an
//! array of several.
//!
//! This file is the only one that names `serde_json`. Decoding walks the
//! type's description with a `DeserializeSeed`, building Kiln values as it
//! reads, with no `serde_json::Value` in between; encoding is a `Serialize`
//! over a value and its description.

use crate::value::*;
use serde::de::{self, DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeMap, SerializeSeq, Serializer};
use std::cell::RefCell;
use std::fmt;
use std::io;

// ----- encoding -----

/// `json.encode`: the value as JSON text.
///
/// # Safety
///
/// `d` describes `v`, as the backends write it.
#[no_mangle]
pub unsafe extern "C" fn k_json_encode(d: *const KDesc, v: Kv) -> *mut KStr {
    let text = encode(&*d, v);
    ptr(string(&text))
}

fn encode(d: &KDesc, v: Kv) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ser = serde_json::Serializer::with_formatter(&mut out, Numbers);
    // Every map key is written as a string, so this cannot fail.
    let _ = Val { d, v }.serialize(&mut ser);
    out
}

/// Writes floats as C's `%g` does with the fewest digits that read back as
/// the same number: `0.1`, `1`, `-2.5e-07`, `1e+21`.
struct Numbers;

impl serde_json::ser::Formatter for Numbers {
    fn write_f64<W: ?Sized + io::Write>(&mut self, w: &mut W, f: f64) -> io::Result<()> {
        w.write_all(shortest_g(f).as_bytes())
    }
}

fn shortest_g(f: f64) -> String {
    // `{:e}` gives the shortest digits that read back as `f`.
    let e = format!("{f:e}");
    let (mantissa, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let exp: i32 = exp.parse().unwrap_or(0);
    let neg = mantissa.starts_with('-');
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let p = digits.len() as i32;
    let mut s = String::from(if neg { "-" } else { "" });
    if exp < -4 || exp >= p {
        s.push_str(&digits[..1]);
        if p > 1 {
            s.push('.');
            s.push_str(&digits[1..]);
        }
        s.push_str(&format!(
            "e{}{:02}",
            if exp < 0 { '-' } else { '+' },
            exp.abs()
        ));
    } else if exp >= 0 {
        let point = exp as usize + 1;
        s.push_str(&digits[..point]);
        if point < digits.len() {
            s.push('.');
            s.push_str(&digits[point..]);
        }
    } else {
        s.push_str("0.");
        s.push_str(&"0".repeat((-exp - 1) as usize));
        s.push_str(&digits);
    }
    s
}

fn text(v: Kv) -> String {
    String::from_utf8_lossy(bytes(v)).into_owned()
}

/// A value and its type.
struct Val<'a> {
    d: &'a KDesc,
    v: Kv,
}

/// The fields of a struct, record or variant.
struct Fields<'a> {
    d: &'a KDesc,
    x: *const Kv,
}

/// A variant's unnamed fields.
struct Tuple<'a> {
    d: &'a KDesc,
    x: *const Kv,
}

fn slot(x: *const Kv, i: i64) -> Kv {
    unsafe { *x.add(i as usize) }
}

impl Serialize for Val<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let (d, v) = (self.d, self.v);
        match d.kind {
            D_STR => s.serialize_str(&text(v)),
            D_INT | D_DURATION => s.serialize_i64(v as i64),
            D_FLOAT => s.serialize_f64(f64::from_bits(v)),
            D_BOOL => s.serialize_bool(v != 0),
            D_OPT if v == 0 => s.serialize_none(),
            D_OPT => Val {
                d: d.part(0),
                v: unsafe { (*ptr::<KBox>(v)).v },
            }
            .serialize(s),
            D_LIST => {
                let l = unsafe { &*ptr::<KList>(v) };
                let mut seq = s.serialize_seq(Some(l.len as usize))?;
                for i in 0..l.len {
                    seq.serialize_element(&Val {
                        d: d.part(0),
                        v: slot(l.items, i),
                    })?;
                }
                seq.end()
            }
            D_SET => {
                let m = unsafe { &*ptr::<KMap>(v) };
                let mut seq = s.serialize_seq(Some(m.len as usize))?;
                for i in live(m) {
                    seq.serialize_element(&Val {
                        d: d.part(0),
                        v: slot(m.keys, i),
                    })?;
                }
                seq.end()
            }
            D_MAP => {
                let m = unsafe { &*ptr::<KMap>(v) };
                let (key, val) = (d.part(0), d.part(1));
                let mut map = s.serialize_map(Some(m.len as usize))?;
                for i in live(m) {
                    let k = slot(m.keys, i);
                    let name = if key.kind == D_STR {
                        text(k)
                    } else {
                        String::from_utf8_lossy(&encode(key, k)).into_owned()
                    };
                    map.serialize_entry(
                        &name,
                        &Val {
                            d: val,
                            v: slot(m.vals, i),
                        },
                    )?;
                }
                map.end()
            }
            D_STRUCT | D_RECORD if d.n == 0 => s.serialize_map(Some(0))?.end(),
            D_STRUCT | D_RECORD => Fields { d, x: ptr::<Kv>(v) }.serialize(s),
            D_ENUM => {
                let e = ptr::<KEnum>(v);
                let var = d.part(unsafe { (*e).tag });
                if var.n == 0 {
                    return s.serialize_str(var.name());
                }
                let f = unsafe { (*e).f.as_ptr() };
                let mut map = s.serialize_map(Some(1))?;
                if var.named() {
                    map.serialize_entry(var.name(), &Fields { d: var, x: f })?;
                } else if var.n == 1 {
                    map.serialize_entry(
                        var.name(),
                        &Val {
                            d: var.part(0),
                            v: slot(f, 0),
                        },
                    )?;
                } else {
                    map.serialize_entry(var.name(), &Tuple { d: var, x: f })?;
                }
                map.end()
            }
            _ => s.serialize_unit(),
        }
    }
}

impl Serialize for Fields<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut map = s.serialize_map(Some(self.d.n as usize))?;
        for i in 0..self.d.n {
            let name = self.d.label(i).unwrap_or("");
            map.serialize_entry(
                name,
                &Val {
                    d: self.d.part(i),
                    v: slot(self.x, i),
                },
            )?;
        }
        map.end()
    }
}

impl Serialize for Tuple<'_> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut seq = s.serialize_seq(Some(self.d.n as usize))?;
        for i in 0..self.d.n {
            seq.serialize_element(&Val {
                d: self.d.part(i),
                v: slot(self.x, i),
            })?;
        }
        seq.end()
    }
}

/// The indexes of a map's entries that are in use, in insertion order.
fn live(m: &KMap) -> impl Iterator<Item = i64> + '_ {
    (0..m.used).filter(|&i| unsafe { *m.live.add(i as usize) })
}

// ----- decoding -----

/// `json.decode<T>`: reads `text` as a value of the described type. On
/// failure it returns 0 and stores the reason in `*error`.
///
/// # Safety
///
/// `d` is a type description and `text` a string, as the backends write
/// them, and `error` can be written.
#[no_mangle]
pub unsafe extern "C" fn k_rt_json_decode(
    d: *const KDesc,
    text: *const KStr,
    error: *mut *mut KStr,
) -> Kv {
    let ctx = Ctx::default();
    let mut de = serde_json::Deserializer::from_slice((*text).bytes());
    let read = Seed { ctx: &ctx, d: &*d }
        .deserialize(&mut de)
        .and_then(|v| de.end().map(|()| v));
    match read {
        Ok(v) => v,
        Err(e) => {
            *error = ptr(string(ctx.message(&e).as_bytes()));
            0
        }
    }
}

/// What went wrong, when the reader says so itself rather than through
/// `serde_json`'s message.
enum Problem {
    /// A value of the wrong type or shape, with where it is.
    Data(String),
    /// Malformed text; the position comes from `serde_json`.
    Syntax(&'static str),
}

#[derive(Default)]
struct Ctx {
    /// Where in the value the reader is, such as `.posts[2].title`.
    path: RefCell<String>,
    problem: RefCell<Option<Problem>>,
}

impl Ctx {
    fn fail<E: de::Error>(&self, problem: Problem) -> E {
        self.problem.borrow_mut().get_or_insert(problem);
        E::custom("kiln")
    }

    fn mismatch<E: de::Error>(&self, expected: &str, found: &str) -> E {
        let path = self.path.borrow();
        self.fail(Problem::Data(format!(
            "cannot read JSON: expected {expected} at ${path}, found {found}"
        )))
    }

    /// Runs `f` with `part` added to the path.
    fn at<T>(&self, part: &str, f: impl FnOnce() -> T) -> T {
        let mark = self.path.borrow().len();
        self.path.borrow_mut().push_str(part);
        let r = f();
        self.path.borrow_mut().truncate(mark);
        r
    }

    fn message(&self, e: &serde_json::Error) -> String {
        let what = match self.problem.borrow_mut().take() {
            Some(Problem::Data(message)) => return message,
            Some(Problem::Syntax(what)) => what.to_string(),
            None => syntax(e),
        };
        if e.is_eof() {
            format!("invalid JSON: {what}, but the text ended")
        } else {
            format!(
                "invalid JSON at line {}, column {}: {what}",
                e.line(),
                e.column()
            )
        }
    }
}

/// `serde_json`'s description of malformed text, in Kiln's words.
fn syntax(e: &serde_json::Error) -> String {
    let full = e.to_string();
    let what = full
        .rsplit_once(" at line ")
        .map_or(full.as_str(), |(w, _)| w);
    match what {
        "EOF while parsing a value"
        | "EOF while parsing a list"
        | "EOF while parsing an object"
        | "expected value"
        | "expected ident" => "expected a value",
        "EOF while parsing a string" => "expected the end of the string",
        "expected `:`" => "expected `:` after the member name",
        "key must be a string" => "expected a member name in quotes",
        "trailing comma" => "expected another value after `,`",
        "trailing characters" => "unexpected text after the value",
        "invalid escape" => "unknown escape",
        "invalid number" => "not a valid number",
        "number out of range" => "the number is too large",
        "control character (\\u0000-\\u001F) found while parsing a string" => {
            "control characters must be escaped in strings"
        }
        // serde_json's names for a first half without a second, and a
        // second half alone.
        "unexpected end of hex escape" => "expected the second half of a surrogate pair",
        "lone leading surrogate in hex escape" => "unexpected second half of a surrogate pair",
        "invalid unicode code point" => "invalid UTF-8",
        "recursion limit exceeded" => "values are nested too deeply",
        other => return other.to_string(),
    }
    .to_string()
}

fn expected(d: &KDesc) -> &'static str {
    match d.kind {
        D_STR => "a string",
        D_BOOL => "true or false",
        D_INT | D_DURATION => "an integer",
        D_FLOAT => "a number",
        D_LIST | D_SET => "an array",
        D_MAP | D_STRUCT | D_RECORD => "an object",
        D_ENUM => "a string or an object",
        _ => "a type JSON can hold",
    }
}

/// Reads a value of the type `d`.
#[derive(Clone, Copy)]
struct Seed<'c> {
    ctx: &'c Ctx,
    d: &'c KDesc,
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Kv;

    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<Kv, D::Error> {
        if self.d.kind == D_OPT {
            de.deserialize_option(self)
        } else {
            de.deserialize_any(self)
        }
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Kv;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(expected(self.d))
    }

    fn visit_none<E: de::Error>(self) -> Result<Kv, E> {
        self.visit_unit()
    }

    fn visit_unit<E: de::Error>(self) -> Result<Kv, E> {
        match self.d.kind {
            D_OPT => Ok(0),
            _ => Err(self.ctx.mismatch(expected(self.d), "null")),
        }
    }

    fn visit_some<D: de::Deserializer<'de>>(self, de: D) -> Result<Kv, D::Error> {
        let v = Seed {
            ctx: self.ctx,
            d: self.d.part(0),
        }
        .deserialize(de)?;
        Ok(kv(unsafe { k_box(v) }))
    }

    fn visit_bool<E: de::Error>(self, b: bool) -> Result<Kv, E> {
        match self.d.kind {
            D_BOOL => Ok(b as Kv),
            _ => Err(self.ctx.mismatch(expected(self.d), "a boolean")),
        }
    }

    fn visit_i64<E: de::Error>(self, n: i64) -> Result<Kv, E> {
        match self.d.kind {
            D_INT | D_DURATION => Ok(n as Kv),
            D_FLOAT => Ok((n as f64).to_bits()),
            _ => Err(self.ctx.mismatch(expected(self.d), "a number")),
        }
    }

    fn visit_u64<E: de::Error>(self, n: u64) -> Result<Kv, E> {
        match self.d.kind {
            D_INT | D_DURATION if n <= i64::MAX as u64 => Ok(n),
            D_INT | D_DURATION => Err(self
                .ctx
                .mismatch("an integer that fits in 64 bits", "a number")),
            D_FLOAT => Ok((n as f64).to_bits()),
            _ => Err(self.ctx.mismatch(expected(self.d), "a number")),
        }
    }

    fn visit_f64<E: de::Error>(self, f: f64) -> Result<Kv, E> {
        match self.d.kind {
            D_FLOAT => Ok(f.to_bits()),
            // serde_json reads `-0` as a float, to keep its sign.
            D_INT | D_DURATION if f == 0.0 && f.is_sign_negative() => Ok(0),
            // A whole number too large for an `i64` arrives as a float.
            D_INT | D_DURATION if f.fract() == 0.0 && f.abs() >= 9.2e18 => Err(self
                .ctx
                .mismatch("an integer that fits in 64 bits", "a number")),
            _ => Err(self.ctx.mismatch(expected(self.d), "a number")),
        }
    }

    fn visit_str<E: de::Error>(self, s: &str) -> Result<Kv, E> {
        match self.d.kind {
            D_STR => Ok(string(s.as_bytes())),
            D_ENUM => match variant(self.d, s) {
                Some(i) if self.d.part(i).n == 0 => {
                    Ok(kv(unsafe { k_enum(i, 0, std::ptr::null()) }))
                }
                _ => Err(self.unknown_variant(s, false)),
            },
            _ => Err(self.ctx.mismatch(expected(self.d), "a string")),
        }
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Kv, A::Error> {
        let (ctx, item) = (self.ctx, self.d.part(0));
        let (list, set) = match self.d.kind {
            D_LIST => (unsafe { k_list(0, std::ptr::null()) }, std::ptr::null_mut()),
            D_SET => {
                let set = unsafe { k_json_new_map(item) };
                if set.is_null() {
                    return Err(ctx.mismatch("a set of strings or integers", "an array"));
                }
                (std::ptr::null_mut(), set)
            }
            _ => return Err(ctx.mismatch(expected(self.d), "an array")),
        };
        let mut index = 0;
        loop {
            let next = ctx.at(&format!("[{index}]"), || {
                seq.next_element_seed(Seed { ctx, d: item })
            })?;
            let Some(v) = next else { break };
            unsafe {
                if set.is_null() {
                    k_list_push(list, v);
                } else {
                    k_set_add(set, v);
                }
            }
            index += 1;
        }
        Ok(if set.is_null() { kv(list) } else { kv(set) })
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Kv, A::Error> {
        match self.d.kind {
            D_MAP => self.read_map(map),
            D_STRUCT | D_RECORD => {
                let slots = read_fields(self.ctx, self.d, map)?;
                Ok(if self.d.n == 0 { 0 } else { kv(slots) })
            }
            D_ENUM => self.read_variant(map),
            _ => Err(self.ctx.mismatch(expected(self.d), "an object")),
        }
    }
}

/// The index of the variant called `name`.
fn variant(d: &KDesc, name: &str) -> Option<i64> {
    (0..d.n).find(|&i| d.part(i).name() == name)
}

impl<'c> Seed<'c> {
    fn unknown_variant<E: de::Error>(self, found: &str, fields: bool) -> E {
        let names: Vec<&str> = (0..self.d.n).map(|i| self.d.part(i).name()).collect();
        let names = match names.split_last() {
            Some((last, rest)) if !rest.is_empty() => format!("{} or {last}", rest.join(", ")),
            _ => names.join(""),
        };
        let path = self.ctx.path.borrow();
        let with = if fields { " with fields" } else { "" };
        self.ctx.fail(Problem::Data(format!(
            "cannot read JSON: expected {names} at ${path}, found \"{found}\"{with}"
        )))
    }

    fn read_map<'de, A: MapAccess<'de>>(self, mut map: A) -> Result<Kv, A::Error> {
        let (ctx, key, val) = (self.ctx, self.d.part(0), self.d.part(1));
        let m = unsafe { k_json_new_map(key) };
        if m.is_null() || (key.kind != D_STR && key.kind != D_INT) {
            return Err(ctx.mismatch("a map with `String` or `Int` keys", "an object"));
        }
        while let Some(name) = map.next_key::<String>()? {
            let k = if key.kind == D_STR {
                string(name.as_bytes())
            } else {
                match int_key(&name) {
                    Some(n) => n as Kv,
                    None => {
                        let path = ctx.path.borrow();
                        return Err(ctx.fail(Problem::Data(format!(
                            "cannot read JSON: expected an integer key at ${path}, found \"{name}\""
                        ))));
                    }
                }
            };
            let v = ctx.at(&format!(".{name}"), || {
                map.next_value_seed(Seed { ctx, d: val })
            })?;
            unsafe { k_map_set(m, k, v) };
        }
        Ok(kv(m))
    }

    fn read_variant<'de, A: MapAccess<'de>>(self, mut map: A) -> Result<Kv, A::Error> {
        let ctx = self.ctx;
        let Some(name) = map.next_key::<String>()? else {
            return Err(ctx.fail(Problem::Syntax("expected a member name in quotes")));
        };
        let i = match variant(self.d, &name) {
            Some(i) if self.d.part(i).n > 0 => i,
            _ => return Err(self.unknown_variant(&name, true)),
        };
        let var = self.d.part(i);
        let f = ctx.at(&format!(".{name}"), || {
            if var.named() {
                map.next_value_seed(FieldsSeed { ctx, d: var })
            } else if var.n == 1 {
                let f = slots(1);
                let v = map.next_value_seed(Seed {
                    ctx,
                    d: var.part(0),
                })?;
                unsafe { *f = v };
                Ok(f)
            } else {
                map.next_value_seed(TupleSeed { ctx, d: var })
            }
        })?;
        if map.next_key::<IgnoredAny>()?.is_some() {
            return Err(ctx.fail(Problem::Syntax("expected `}` after the variant")));
        }
        Ok(kv(unsafe { k_enum(i, var.n as i32, f) }))
    }
}

/// An `Int` map key: digits with an optional `-`, in range.
fn int_key(s: &str) -> Option<i64> {
    if s.starts_with('+') {
        return None;
    }
    s.parse().ok()
}

/// Reads an object into the fields of `d`. A field that is missing is an
/// error unless it is optional; members `d` does not have are skipped.
fn read_fields<'de, A: MapAccess<'de>>(
    ctx: &Ctx,
    d: &KDesc,
    mut map: A,
) -> Result<*mut Kv, A::Error> {
    let slots = slots(d.n);
    let mut seen = vec![false; d.n as usize];
    while let Some(name) = map.next_key::<String>()? {
        match (0..d.n).find(|&i| d.label(i) == Some(name.as_str())) {
            Some(i) => {
                let v = ctx.at(&format!(".{name}"), || {
                    map.next_value_seed(Seed { ctx, d: d.part(i) })
                })?;
                unsafe { *slots.add(i as usize) = v };
                seen[i as usize] = true;
            }
            None => {
                map.next_value::<IgnoredAny>()?;
            }
        }
    }
    for i in 0..d.n {
        if seen[i as usize] || d.part(i).kind == D_OPT {
            continue;
        }
        let path = ctx.path.borrow();
        let label = d.label(i).unwrap_or("");
        return Err(ctx.fail(Problem::Data(format!(
            "cannot read JSON: missing field `{label}` at ${path}"
        ))));
    }
    Ok(slots)
}

/// Reads an object into a variant's named fields.
struct FieldsSeed<'c> {
    ctx: &'c Ctx,
    d: &'c KDesc,
}

impl<'de> DeserializeSeed<'de> for FieldsSeed<'_> {
    type Value = *mut Kv;

    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<*mut Kv, D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for FieldsSeed<'_> {
    type Value = *mut Kv;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an object")
    }

    fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<*mut Kv, A::Error> {
        read_fields(self.ctx, self.d, map)
    }

    fn visit_seq<A: SeqAccess<'de>>(self, _: A) -> Result<*mut Kv, A::Error> {
        Err(self.ctx.mismatch("an object", "an array"))
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an object", "a string"))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an object", "a boolean"))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an object", "a number"))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an object", "a number"))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an object", "a number"))
    }

    fn visit_unit<E: de::Error>(self) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an object", "null"))
    }
}

/// Reads an array of exactly a variant's unnamed fields.
struct TupleSeed<'c> {
    ctx: &'c Ctx,
    d: &'c KDesc,
}

impl<'de> DeserializeSeed<'de> for TupleSeed<'_> {
    type Value = *mut Kv;

    fn deserialize<D: de::Deserializer<'de>>(self, de: D) -> Result<*mut Kv, D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for TupleSeed<'_> {
    type Value = *mut Kv;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("an array")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<*mut Kv, A::Error> {
        let (ctx, d) = (self.ctx, self.d);
        let f = slots(d.n);
        for k in 0..d.n {
            let next = ctx.at(&format!("[{k}]"), || {
                seq.next_element_seed(Seed { ctx, d: d.part(k) })
            })?;
            match next {
                Some(v) => unsafe { *f.add(k as usize) = v },
                None => {
                    let found = format!("{k} items");
                    return Err(ctx.mismatch(&format!("{} items", d.n), &found));
                }
            }
        }
        if seq.next_element::<IgnoredAny>()?.is_some() {
            return Err(ctx.mismatch(&format!("{} items", d.n), "more"));
        }
        Ok(f)
    }

    fn visit_map<A: MapAccess<'de>>(self, _: A) -> Result<*mut Kv, A::Error> {
        Err(self.ctx.mismatch("an array", "an object"))
    }

    fn visit_str<E: de::Error>(self, _: &str) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an array", "a string"))
    }

    fn visit_bool<E: de::Error>(self, _: bool) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an array", "a boolean"))
    }

    fn visit_i64<E: de::Error>(self, _: i64) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an array", "a number"))
    }

    fn visit_u64<E: de::Error>(self, _: u64) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an array", "a number"))
    }

    fn visit_f64<E: de::Error>(self, _: f64) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an array", "a number"))
    }

    fn visit_unit<E: de::Error>(self) -> Result<*mut Kv, E> {
        Err(self.ctx.mismatch("an array", "null"))
    }
}

#[cfg(test)]
mod tests {
    use super::shortest_g;

    #[test]
    fn floats_as_c_writes_them() {
        for (f, s) in [
            (0.1, "0.1"),
            (1.0, "1"),
            (4.5, "4.5"),
            (-2.5e-7, "-2.5e-07"),
            (1e21, "1e+21"),
            (100.0, "1e+02"),
            (123.25, "123.25"),
            (0.0001, "0.0001"),
            (0.00001, "1e-05"),
            (-0.0, "-0"),
        ] {
            assert_eq!(shortest_g(f), s, "{f}");
        }
    }
}
