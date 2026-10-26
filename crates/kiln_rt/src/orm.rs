//! `orm`: the parts of a typed query that happen while the program runs.
//!
//! The compiler works out the SQL, which columns a row has and which
//! fields link two models, so what is left here is loading a relation for
//! rows that have already been read, and running an insert that comes
//! back with the key the database generated.

use crate::sql::{self, DbError, Kind, SqlValue};
use crate::value::*;

/// `orm.__like(value, mode)`: the `like` pattern that finds `value` as
/// text, with `%` and `_` in it kept as characters.
///
/// # Safety
///
/// `value` is a Kiln string.
#[no_mangle]
pub unsafe extern "C" fn k_rt_orm_like(value: Kv, mode: Kv) -> Kv {
    let text = String::from_utf8_lossy(bytes(value));
    let mut escaped = String::with_capacity(text.len() + 2);
    for c in text.chars() {
        if matches!(c, '%' | '_' | '!') {
            escaped.push('!');
        }
        escaped.push(c);
    }
    let pattern = match mode {
        1 => format!("{escaped}%"),
        2 => format!("%{escaped}"),
        _ => format!("%{escaped}%"),
    };
    string(pattern.as_bytes())
}

/// The slots of a struct value.
unsafe fn fields(v: Kv) -> *mut Kv {
    ptr::<Kv>(v)
}

/// A key as a string, to group rows by it.
fn group_key(v: &SqlValue) -> Option<String> {
    match v {
        SqlValue::Null => None,
        SqlValue::Int(n) => Some(format!("i{n}")),
        SqlValue::Float(f) => Some(format!("f{f}")),
        SqlValue::Bool(b) => Some(format!("b{b}")),
        SqlValue::Text(t) => Some(format!("t{t}")),
        SqlValue::Blob(b) => Some(format!("x{}", String::from_utf8_lossy(b))),
    }
}

/// `orm.__include(db, rows, field, sql, key_field, fk_field, list)`:
/// loads one relation for every row with one more query, and stores it in
/// each row's field.
///
/// # Safety
///
/// `d` describes `rows`, a `List<T>` of the model whose relation this is.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn k_rt_orm_include(
    d: *const KDesc,
    db: Kv,
    rows: Kv,
    field: Kv,
    sql: Kv,
    key_field: Kv,
    fk_field: Kv,
    list: Kv,
) -> Kv {
    let (field, key_field, fk_field) = (field as i64, key_field as i64, fk_field as i64);
    let list = list != 0;
    let parent = (*d).part(0);
    let rel = parent.part(field);
    // The related model: the item of a list, or what an optional holds.
    let child = rel.part(0);
    let l = &*ptr::<KList>(rows);
    if l.len == 0 {
        return 0;
    }
    // The keys to load for, each once, and where they go.
    let mut keys: Vec<SqlValue> = Vec::new();
    let mut per_row: Vec<Option<String>> = Vec::new();
    let key_desc = parent.part(key_field);
    for i in 0..l.len {
        let row = fields(*l.items.add(i as usize));
        let v = sql::to_sql_value(key_desc, *row.add(key_field as usize));
        let k = group_key(&v);
        if let Some(k) = &k {
            if !keys
                .iter()
                .any(|existing| group_key(existing).as_ref() == Some(k))
            {
                keys.push(v);
            }
        }
        per_row.push(k);
    }
    if keys.is_empty() {
        for i in 0..l.len {
            let row = fields(*l.items.add(i as usize));
            *row.add(field as usize) = if list {
                kv(k_list(0, std::ptr::null()))
            } else {
                0
            };
        }
        return 0;
    }
    let marks: Vec<String> = (1..=keys.len()).map(|i| format!("${i}")).collect();
    let text = format!(
        "{}({})",
        String::from_utf8_lossy(bytes(sql)),
        marks.join(", ")
    );
    let loaded = sql::with_conn(db, |c| c.query(&text, &keys)).and_then(|found| {
        let mut children: Vec<(Option<String>, Kv)> = Vec::new();
        for row in &found.rows {
            let v = sql::row_kv(child, &found.columns, row)
                .map_err(|m| DbError::new(Kind::Decode, m))?;
            let f = fields(v);
            let k = group_key(&sql::to_sql_value(
                child.part(fk_field),
                *f.add(fk_field as usize),
            ));
            children.push((k, v));
        }
        Ok(children)
    });
    let children = match loaded {
        Ok(c) => c,
        Err(e) => {
            sql::fail(&e.text());
            return 0;
        }
    };
    for i in 0..l.len {
        let row = fields(*l.items.add(i as usize));
        let want = &per_row[i as usize];
        let mut found = children
            .iter()
            .filter(|(k, _)| k.is_some() && k == want)
            .map(|(_, v)| *v);
        *row.add(field as usize) = if list {
            let out = k_list(0, std::ptr::null());
            for v in found {
                k_list_push(out, v);
            }
            kv(out)
        } else {
            match found.next() {
                Some(v) => kv(k_box(v)),
                None => 0,
            }
        };
    }
    0
}

/// `orm.__insert(db, value, sql, key_field, params)`: stores a row and,
/// when the database generates its key, returns the row with it.
///
/// # Safety
///
/// `d` describes `value`, the model's struct.
#[no_mangle]
pub unsafe extern "C" fn k_rt_orm_insert(
    d: *const KDesc,
    db: Kv,
    value: Kv,
    sql: Kv,
    key_field: Kv,
    params: Kv,
) -> Kv {
    let key_field = key_field as i64;
    let desc = &*d;
    let mut text = String::from_utf8_lossy(bytes(sql)).into_owned();
    // PostgreSQL reports a generated key only when the statement asks for
    // it; the other databases report it by themselves.
    if key_field >= 0 && sql::dialect_of(db) == sql::Dialect::Postgres {
        text.push_str(&format!(
            " returning {}",
            desc.label(key_field).unwrap_or("id")
        ));
    }
    let values = sql::params_of(params);
    let done = match sql::with_conn(db, |c| c.exec(&text, &values)) {
        Ok(done) => done,
        Err(e) => {
            sql::fail(&e.text());
            return 0;
        }
    };
    if key_field < 0 {
        return value;
    }
    let Some(key) = done.key else {
        sql::fail(
            &DbError::new(
                Kind::Unsupported,
                "the database reported no key for the new row",
            )
            .text(),
        );
        return 0;
    };
    // The row with its new key, as a value of its own: `insert` returns
    // the stored row and leaves the one it was given alone.
    let out = slots(desc.n);
    let from = fields(value);
    for i in 0..desc.n {
        *out.add(i as usize) = *from.add(i as usize);
    }
    *out.add(key_field as usize) = key as Kv;
    kv(out)
}
