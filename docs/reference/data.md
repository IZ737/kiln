# Databases, models and migrations

Kiln reaches a SQL database two ways, and they share one connection.
[`sql`](#sql) is what the database speaks: a query written out, with its
values sent apart from its text. [`orm`](#orm) is what the program means:
a model is an ordinary struct, and a query on it is Kiln the compiler
turns into SQL while it builds the program. [`kiln db`](#migrations)
writes the migrations the models ask for, and reads a database that
already exists into models.

SQLite needs nothing on the system: `sqlite:` databases are files, or
memory. `postgres://` and `mysql://` URLs are read and reported as not
supported yet, rather than silently behaving like something else.

## `sql`

```kiln
import sql

fn main() ! {
    let db = try sql.open(env("DATABASE_URL"))
    let posts = try db.query<Post>("select * from posts where published = $1", true)
    let one = try db.one<Post>("select * from posts where id = $1", id)
    let many = try db.scalar<Int>("select count(*) from posts")
    let changed = try db.exec("delete from posts where id = $1", id)
}
```

| | |
| --- | --- |
| `sql.open(url, max_connections: 4, busy_timeout: 5s) -> Db !` | A pool of connections, not one socket per query |
| `db.query<T>(sql, ...) -> List<T> !` | Every row as a `T` |
| `db.one<T>(sql, ...) -> T? !` | The first row, or `none` |
| `db.scalar<T>(sql, ...) -> T !` | One value from one row; no row is an error |
| `db.exec(sql, ...) -> Int !` | The number of rows it changed |
| `db.insert<T>(table, value) -> T !` | Stores a value and returns it with the key the database made |
| `db.transaction<T>(fn(tx) -> T !) -> T !` | Commits when the function returns, rolls back when it fails |
| `db.dialect() -> String` | `"sqlite"`, `"postgres"` or `"mysql"` |
| `db.close()` | Closes the pool's connections |
| `sql.error_kind(err) -> String` | What kind of failure an error was (below) |

Values go in `$1`, `$2`, ... whatever the database, and the extra
arguments fill them in order; the driver writes the placeholders each
database wants. **A value is never written into the SQL**, so a query
cannot be changed by what a person types. A table or column name is not a
value: write those out, or map them in a model.

A row is read into the type the call names: a struct or record takes a
column per field by name, and a single type (`Int`, `String`) takes the
first column. A nullable column fits an optional. An `Instant` or a `Date`
is stored as text in UTC (`2026-10-26T21:30:00Z`), and any other value
with a JSON form is stored as its JSON.

`sql.error_kind(err)` tells the kinds apart, so a handler can answer a
duplicate email differently from a database that is not there:
`connection`, `syntax`, `unique`, `foreign_key`, `not_null`, `check`,
`constraint`, `decode`, `busy`, `transaction`, `unsupported` and `other`.
Errors name the table and column the database named, and never the values
that were sent.

```kiln
try db.transaction(fn(tx) ! {
    try tx.exec("update accounts set balance = balance - $1 where id = $2", amount, from)
    try tx.exec("update accounts set balance = balance + $1 where id = $2", amount, to)
})
```

Everything inside a transaction runs on its connection, `orm` queries
included. SQLite takes one writer at a time: a busy database is tried
again until `busy_timeout` is over, and other requests keep running while
it waits.

## `orm`

A model is an ordinary struct. `Post` is stored in the table `posts` (the
name in snake case, in the plural), each field in the column of its name,
and `id` is its key.

```kiln
import orm

struct Post {
    id: Int
    title: String
    summary: String? = none
    published: Bool = false
    author_id: Int
    // Relations are not columns.
    author: Author? = none
    comments: List<Comment> = []
}

let published = try orm.find<Post>(db)
    .where(.published)
    .where(fn(p) { p.title.starts_with(term) })
    .order_by(.id, descending: true)
    .limit(20)
    .all()
```

| | |
| --- | --- |
| `orm.find<T>(db) -> Query<T>` | A query to build on |
| `orm.get<T>(db, key) -> T? !` | The row with that key |
| `orm.insert<T>(db, value) -> T !` | Stores a row and returns it with its new key |
| `orm.update<T>(db, value) -> Int !` | Writes a row back by its key |
| `orm.delete<T>(db, value) -> Int !` | Deletes a row by its key |

| Step | |
| --- | --- |
| `.where(condition)` | Kept rows; several `where`s are `and` |
| `.order_by(.field, descending: false)` | The order, which must be written here, not worked out |
| `.limit(n)`, `.offset(n)` | A page of rows |
| `.include(.relation)` | Loads a relation for every row, in one more query |
| `.all()`, `.first()`, `.count()`, `.delete()` | Runs the query |
| `.sql()` | The SQL it became, to read while writing it |

**The conditions are compiled, not run.** `where` and `order_by` are read
while the program is built: fields, literals, comparisons, `and`, `or`,
`not`, arithmetic, and `contains`, `starts_with` and `ends_with` on text
become SQL. A misspelled field or a condition that cannot become SQL is an
error at `kiln check` (E0232), not a surprise at runtime. Any part that
does not name the model is worked out in Kiln first and sent as a value.
A query must be built and run in one chain, so that what runs is what the
compiler read.

**Relations are loaded, never guessed.** A field `author: Author?` next to
`author_id` is the `Author` whose key is `author_id`, and
`comments: List<Comment>` the comments whose `post_id` is this post's key.
They are not columns: give them defaults, and load them with `include`,
which fetches them for every row of the page in one more query. Reading a
relation that was not included gives what the default says, never a
hidden query.

```kiln
let page = try orm.find<Post>(db).include(.author).include(.comments).limit(20).all()
```

**A table that does not follow the conventions** is mapped once, anywhere
in the program:

```kiln
const posts = orm.model<Post>(
    table: "blog_posts",
    columns: { title: "post_title" },
    generated: [.id],      // the database fills these in
    readonly: [.created],  // never written by insert or update
)
```

A name that cannot be used as it is written — a reserved word, or not in
`[a-z_][a-z0-9_]*` — is an error (E0231) that says to map it, rather than
quoting identifiers differently per database behind the program's back.

## Migrations

`kiln db` keeps the database and the models in step. It never changes a
database while a program builds, runs or starts.

```sh
kiln db status                        # what is applied, pending, or drifted
kiln db diff --name add_post_summary  # writes the next migration
kiln db migrate                       # applies the pending ones
kiln db verify                        # checks the files against the ledger
kiln db pull --out models.kiln        # models from a database that exists
```

The database comes from `--url`, or `DATABASE_URL`. `--path` picks the
program's folder.

`diff` compares the models with `migrations/schema.json`, the snapshot of
what the migrations describe, and writes the difference as SQL in
`migrations/sqlite/0001_add_post_summary.sql` for you to read and commit.
A change that would throw data away — dropping a table or a column — is
refused unless `--allow-destructive` asks for it. A new column that cannot
be null is written with a default, so the rows already there stay valid.

`migrate` applies each file once, in order, inside a transaction, and
writes what it applied to a `kiln_migrations` table with the file's
SHA-256. A migration that has run is history: editing or removing the file
is an error, and so is a new migration numbered before one that has
already been applied. `--dry-run` says what it would do.

`status` and `verify` report what is pending and what the database has
that no migration describes, which is how a change someone made by hand
shows up instead of being quietly overwritten.

`pull` reads a database that already exists and writes ordinary Kiln
models for its tables, with a `<table>_id` field for each foreign key. It
never overwrites a file that is there, and it changes nothing in the
database. `pull --check` reports what the database has that the models do
not.

The `kiln_sessions` table [`auth`](stdlib.md#auth) keeps its sessions in
is the runtime's own, not one of the program's models, so the migrations
do not describe it.
