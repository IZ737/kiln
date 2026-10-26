//! The clients against real servers: `KILN_POSTGRES_URL` and
//! `KILN_MYSQL_URL` name a database each may use as it likes (CI starts
//! both). A test whose variable is not set says so and passes.

use kiln_dbwire::{open, Client, Kind, Value};

fn server(var: &str) -> Option<Box<dyn Client>> {
    let Ok(url) = std::env::var(var) else {
        eprintln!("skipping: {var} is not set");
        return None;
    };
    Some(open(&url, None).unwrap_or_else(|e| panic!("cannot open {var}: {e}")))
}

/// What both databases must do alike. `p` writes the n-th placeholder.
fn round_trip(c: &mut dyn Client, p: fn(usize) -> String, key: &str) {
    c.exec("drop table if exists kiln_wire_child", &[]).unwrap();
    c.exec("drop table if exists kiln_wire", &[]).unwrap();
    c.exec(
        &format!("create table kiln_wire (id {key}, name varchar(200) not null unique, score double precision, n bigint, ok boolean not null, data {})",
            if key.contains("auto_increment") { "blob" } else { "bytea" }),
        &[],
    )
    .unwrap();
    c.exec("create table kiln_wire_child (id integer primary key, parent bigint not null, foreign key (parent) references kiln_wire(id))", &[]).unwrap();
    let insert = format!(
        "insert into kiln_wire (name, score, n, ok, data) values ({}, {}, {}, {}, {})",
        p(1),
        p(2),
        p(3),
        p(4),
        p(5)
    );
    let suffix = if key.contains("auto_increment") {
        ""
    } else {
        " returning id"
    };
    let done = c
        .exec(
            &format!("{insert}{suffix}"),
            &[
                Value::Text("Ada ✓ 'quoted' $1".into()),
                Value::Float(1.5),
                Value::Int(9_007_199_254_740_993),
                Value::Bool(true),
                Value::Blob(vec![0, 255, 10]),
            ],
        )
        .unwrap();
    assert_eq!(done.changed, 1);
    assert_eq!(done.key, Some(1));
    c.exec(
        &format!("{insert}{suffix}"),
        &[
            Value::Text("Grace".into()),
            Value::Null,
            Value::Null,
            Value::Bool(false),
            Value::Null,
        ],
    )
    .unwrap();
    let rows = c
        .query(
            &format!("select id, name, score, n, ok, data from kiln_wire where name = {} or id > {} order by id", p(1), p(2)),
            &[Value::Text("Ada ✓ 'quoted' $1".into()), Value::Int(1)],
        )
        .unwrap();
    assert_eq!(rows.columns, ["id", "name", "score", "n", "ok", "data"]);
    assert_eq!(rows.rows.len(), 2);
    let ada = &rows.rows[0];
    assert_eq!(ada[0], Value::Int(1));
    assert_eq!(ada[1], Value::Text("Ada ✓ 'quoted' $1".into()));
    assert_eq!(ada[2], Value::Float(1.5));
    assert_eq!(ada[3], Value::Int(9_007_199_254_740_993));
    assert!(
        matches!(ada[4], Value::Bool(true) | Value::Int(1)),
        "{:?}",
        ada[4]
    );
    assert_eq!(ada[5], Value::Blob(vec![0, 255, 10]));
    let grace = &rows.rows[1];
    assert_eq!(grace[2], Value::Null);
    assert!(matches!(grace[4], Value::Bool(false) | Value::Int(0)));
    // Values are never read as SQL.
    let none = c
        .query(
            &format!("select id from kiln_wire where name = {}", p(1)),
            &[Value::Text("x' or '1'='1".into())],
        )
        .unwrap();
    assert!(none.rows.is_empty());
    // Kinds of failure.
    let e = c
        .exec(
            &insert,
            &[
                Value::Text("Grace".into()),
                Value::Null,
                Value::Null,
                Value::Bool(true),
                Value::Null,
            ],
        )
        .unwrap_err();
    assert_eq!(e.kind, Kind::Unique, "{e:?}");
    assert!(
        !e.message.contains("Grace"),
        "an error repeats a value: {}",
        e.message
    );
    let e = c
        .exec(
            &format!(
                "insert into kiln_wire_child (id, parent) values ({}, {})",
                p(1),
                p(2)
            ),
            &[Value::Int(1), Value::Int(99)],
        )
        .unwrap_err();
    assert_eq!(e.kind, Kind::ForeignKey, "{e:?}");
    let e = c.query("selec 1", &[]).unwrap_err();
    assert_eq!(e.kind, Kind::Syntax, "{e:?}");
    let e = c
        .query("select * from kiln_no_such_table", &[])
        .unwrap_err();
    assert_eq!(e.kind, Kind::Syntax, "{e:?}");
    // The connection is still good after errors, and in a transaction.
    c.exec("begin", &[]).unwrap();
    c.exec(
        &format!("update kiln_wire set n = {} where id = {}", p(1), p(2)),
        &[Value::Int(7), Value::Int(2)],
    )
    .unwrap();
    c.exec("rollback", &[]).unwrap();
    let n = c
        .query("select n from kiln_wire where id = 2", &[])
        .unwrap();
    assert_eq!(n.rows[0][0], Value::Null);
    let changed = c
        .exec(
            &format!("delete from kiln_wire where id > {}", p(1)),
            &[Value::Int(0)],
        )
        .unwrap();
    assert_eq!(changed.changed, 2);
    // Several statements without values, as a migration has.
    c.exec("drop table kiln_wire_child; drop table kiln_wire", &[])
        .unwrap();
}

#[test]
fn postgres() {
    let Some(mut c) = server("KILN_POSTGRES_URL") else {
        return;
    };
    round_trip(c.as_mut(), |n| format!("${n}"), "bigserial primary key");
    let r = c
        .query(
            "select 1::int4, 2.5::numeric, 'x'::text, null::int8, now() is not null",
            &[],
        )
        .unwrap();
    assert_eq!(
        r.rows[0][..4],
        [
            Value::Int(1),
            Value::Float(2.5),
            Value::Text("x".into()),
            Value::Null
        ]
    );
}

#[test]
fn mysql() {
    let Some(mut c) = server("KILN_MYSQL_URL") else {
        return;
    };
    round_trip(
        c.as_mut(),
        |_| "?".to_string(),
        "bigint auto_increment primary key",
    );
    let r = c
        .query(
            "select 1, 2.5, 'x', null, cast(3 as unsigned), now() is not null",
            &[],
        )
        .unwrap();
    assert_eq!(
        r.rows[0][..5],
        [
            Value::Int(1),
            Value::Float(2.5),
            Value::Text("x".into()),
            Value::Null,
            Value::Int(3)
        ]
    );
}

#[test]
fn wrong_passwords_are_connection_errors() {
    for var in ["KILN_POSTGRES_URL", "KILN_MYSQL_URL"] {
        let Ok(url) = std::env::var(var) else {
            continue;
        };
        let (head, rest) = url.split_once("://").unwrap();
        let (_, host) = rest.rsplit_once('@').unwrap();
        let bad = format!("{head}://kiln:wrong-password@{host}");
        match open(&bad, None) {
            Ok(_) => panic!("{var}: a wrong password signed in"),
            Err(e) => assert_eq!(e.kind, Kind::Connection, "{var}: {e:?}"),
        }
    }
}
