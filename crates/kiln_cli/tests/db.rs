//! `kiln db`: the migrations a program's models ask for, applying them
//! once each, the drift a database shows afterwards, and the models of a
//! database that already exists.

use std::path::{Path, PathBuf};
use std::process::Command;

const APP: &str = r#"import sql, orm

struct Post {
    id: Int
    title: String
    summary: String? = none
    author_id: Int
    author: Author? = none
}

struct Author {
    id: Int
    name: String
}

fn main() ! {
    let db = try sql.open(env("DATABASE_URL"))
    print(try orm.find<Post>(db).include(.author).count())
}
"#;

fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-db-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `kiln db ...` in a program's folder, and returns what it said.
fn db(dir: &Path, args: &[&str]) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_kiln"))
        .arg("db")
        .args(args)
        .arg("--path")
        .arg(dir)
        .arg("--url")
        .arg(format!("sqlite:{}", dir.join("app.db").display()))
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    (out.status.success(), text)
}

#[track_caller]
fn ok(dir: &Path, args: &[&str]) -> String {
    let (success, text) = db(dir, args);
    assert!(success, "`kiln db {}` failed:\n{text}", args.join(" "));
    text
}

#[test]
fn migrations_are_written_once_and_applied_once() {
    let dir = dir("migrate");
    std::fs::write(dir.join("main.kiln"), APP).unwrap();

    let written = ok(&dir, &["diff", "--name", "create_posts"]);
    assert!(written.contains("create table posts"), "{written}");
    let file = dir.join("migrations/sqlite/0001_create_posts.sql");
    let sql = std::fs::read_to_string(&file).unwrap();
    assert!(sql.contains("id integer primary key"), "{sql}");
    assert!(sql.contains("title text not null"), "{sql}");
    assert!(sql.contains("summary text,"), "{sql}");
    // A relation gives the column it is loaded through a foreign key.
    assert!(
        sql.contains("author_id integer not null references authors(id)"),
        "{sql}"
    );
    // Nothing has been applied yet.
    assert!(ok(&dir, &["status"]).contains("0001 create_posts  pending"));
    assert!(!dir.join("app.db").exists() || ok(&dir, &["status"]).contains("pending"));

    // `--dry-run` changes nothing.
    assert!(ok(&dir, &["migrate", "--dry-run"]).contains("would apply"));
    assert!(ok(&dir, &["status"]).contains("pending"));

    assert!(ok(&dir, &["migrate"]).contains("applied 0001_create_posts.sql"));
    let status = ok(&dir, &["status"]);
    assert!(status.contains("0001 create_posts  applied"), "{status}");
    assert!(
        status.contains("the database matches the migrations"),
        "{status}"
    );
    // Applying again does nothing.
    assert!(ok(&dir, &["migrate"]).contains("nothing to apply"));
    // The models have not changed, so there is nothing to write.
    assert!(ok(&dir, &["diff", "--name", "again"]).contains("nothing to write"));

    // A new field is an additive migration, with a default for the rows
    // that are already there.
    let grown = APP.replace(
        "    author_id: Int",
        "    views: Int = 0\n    author_id: Int",
    );
    std::fs::write(dir.join("main.kiln"), &grown).unwrap();
    assert!(ok(&dir, &["diff", "--name", "add_views"]).contains("add posts.views"));
    let sql = std::fs::read_to_string(dir.join("migrations/sqlite/0002_add_views.sql")).unwrap();
    assert!(
        sql.contains("alter table posts add column views integer not null default 0"),
        "{sql}"
    );
    ok(&dir, &["migrate"]);
    assert!(ok(&dir, &["status"]).contains("the database matches the migrations"));

    // Dropping a field would throw data away, so it is refused without
    // being asked for.
    std::fs::write(dir.join("main.kiln"), APP).unwrap();
    let (success, text) = db(&dir, &["diff", "--name", "drop_views"]);
    assert!(!success, "{text}");
    assert!(text.contains("--allow-destructive"), "{text}");
    assert!(ok(
        &dir,
        &["diff", "--name", "drop_views", "--allow-destructive"]
    )
    .contains("drop posts.views"));

    // A migration that has run is history: editing it is an error.
    let file = dir.join("migrations/sqlite/0001_create_posts.sql");
    let edited = format!("{}\n-- changed\n", std::fs::read_to_string(&file).unwrap());
    std::fs::write(&file, edited).unwrap();
    let (success, text) = db(&dir, &["migrate"]);
    assert!(!success, "{text}");
    assert!(text.contains("has changed since it was applied"), "{text}");
    assert!(ok(&dir, &["status"]).contains("the file has changed since"));
}

#[test]
fn drift_in_the_database_is_reported() {
    let dir = dir("drift");
    std::fs::write(dir.join("main.kiln"), APP).unwrap();
    ok(&dir, &["diff", "--name", "create_posts"]);
    ok(&dir, &["migrate"]);
    let conn = rusqlite::Connection::open(dir.join("app.db")).unwrap();
    conn.execute_batch("alter table posts add column extra text")
        .unwrap();
    drop(conn);
    let status = ok(&dir, &["status"]);
    assert!(
        status.contains("the database has posts.extra, which no migration describes"),
        "{status}"
    );
}

#[test]
fn a_database_that_already_exists_becomes_models() {
    let dir = dir("pull");
    let conn = rusqlite::Connection::open(dir.join("app.db")).unwrap();
    conn.execute_batch(
        "create table writers (id integer primary key, name text not null);
         create table stories (
            id integer primary key,
            headline text not null,
            standfirst text,
            words integer not null,
            rating real,
            writer_id integer not null references writers(id));
         insert into writers (name) values ('Ada');",
    )
    .unwrap();
    drop(conn);
    let said = ok(&dir, &["pull"]);
    assert!(said.contains("2 model(s)"), "{said}");
    let source = std::fs::read_to_string(dir.join("models.kiln")).unwrap();
    assert!(source.contains("struct Story {"), "{source}");
    assert!(source.contains("struct Writer {"), "{source}");
    assert!(source.contains("    headline: String\n"), "{source}");
    assert!(
        source.contains("    standfirst: String? = none\n"),
        "{source}"
    );
    assert!(source.contains("    words: Int\n"), "{source}");
    assert!(source.contains("    rating: Float? = none\n"), "{source}");
    assert!(source.contains("    writer_id: Int\n"), "{source}");

    // A second pull never overwrites code that may have been edited.
    let (success, text) = db(&dir, &["pull"]);
    assert!(!success, "{text}");
    assert!(text.contains("never overwrites"), "{text}");

    // The rows are still there: pulling reads and writes nothing.
    let conn = rusqlite::Connection::open(dir.join("app.db")).unwrap();
    let names: i64 = conn
        .query_row("select count(*) from writers", [], |r| r.get(0))
        .unwrap();
    assert_eq!(names, 1);

    // `--check` reports what the database has that the models do not.
    conn.execute_batch("alter table stories add column subject text")
        .unwrap();
    drop(conn);
    let said = ok(&dir, &["pull", "--check"]);
    assert!(said.contains("stories.subject"), "{said}");
}
