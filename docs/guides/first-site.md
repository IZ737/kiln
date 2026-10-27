# Your first Kiln site

This guide goes from an empty Kiln install to a website in several
folders, with accounts, a form only signed-in people can send, components
from a Ware, a theme switch, a reproducible build and tests that drive a
real browser. The finished app is
[`examples/guestbook`](../../examples/guestbook/main.kiln); every file
this guide shows is there in full.

## 1. Install and start

Install kiln with the one-line installer in the [README](../../README.md#install),
then make an app:

```sh
kiln new guestbook
kiln run guestbook        # http://127.0.0.1:8080
kiln test guestbook       # its unit and integration tests
```

`kiln new` writes a manifest (`kiln.toml`), `main.kiln` with the routes,
a page (`home.glaze`), a folder module (`notes/`) and a test of each kind.
Nothing is downloaded. While you work, `kiln dev guestbook` rebuilds and
reloads the page on every save, and keeps the last good build running
when a change does not compile.

## 2. Folders are modules

A folder is a module, and `pub` is what other folders may use. The
guestbook has four:

```text
guestbook/
  kiln.toml
  main.kiln            the app: imports the folders and starts the server
  shared/              models, rules, the database, the layout and the themes
  accounts/            signing up, in and out
  entries/             the book and its form
  tests/integration/   requests to the running app
  tests/browser/       a real browser
```

```kiln
// main.kiln
import web, shared, accounts, entries

fn main() ! {
    let db = try shared.database()
    let sessions = try shared.sessions(db)
    let app = web.app()
    accounts.routes(app, db, sessions)
    entries.routes(app, db, sessions)
    try app.listen("127.0.0.1:8080")
}
```

A folder's glazes see the modules its `.kiln` files import, so
`entries/index.glaze` can take `{% props entries: List<shared.Entry> %}`.
See [Projects and modules](../reference/projects.md).

## 3. A database and accounts

Models are structs. `kiln db diff` writes the SQL that creates their
tables, to read and commit; `kiln db migrate` applies it.

```kiln
// shared/models.kiln
pub struct Account {
    id: Int = 0
    email: String
    name: String
    password: String
}

pub struct Entry {
    id: Int = 0
    account_id: Int
    message: String
    account: Account? = none     // loaded with include(.account)
}
```

```sh
cd guestbook
kiln db diff --name init
kiln db migrate --url sqlite:guestbook.db
```

`auth` keeps sessions in the database and hashes passwords; `validate`
checks forms on the server and says what is wrong in words a person can
read. Signing up is a form, checked, then a session:

```kiln
app.post("/sign-up", fn(req) {
    let form = try req.form<{ name: String, email: String, password: String }>()
    let email = form.email.trim().lower()
    let taken = try orm.find<shared.Account>(db).where(fn(a) { a.email == email }).count()
    let checked = shared.account_problems(form.name.trim(), email, form.password, taken > 0)
    if not checked.valid {
        var res = web.render("sign_up.glaze", { name: form.name, email: email, problems: checked.messages(), dark: shared.dark(req) })
        res.status = 422
        return res
    }
    let hash = try auth.hash_password(form.password)
    let account = try orm.insert(db, shared.Account(email: email, name: form.name.trim(), password: hash))
    return try sessions.sign_in(web.redirect("/", status: 303), "${account.id}")
})
```

The book's form is only for people who are signed in: `sessions.require`
refuses everyone else, and `sessions.check` refuses a form without the
page's CSRF token.

```kiln
app.post("/entries", fn(req) {
    let who = try sessions.require(req)
    let form = try req.form<{ csrf: String, message: String }>()
    try sessions.check(req, form.csrf)
    ...
})
```

[Databases, models and migrations](../reference/data.md) and the `auth`
module in the [standard library](../reference/stdlib.md) explain the rest.

## 4. Components from a Ware

[kiln/ui](../../wares/ui/README.md) is a Ware of accessible form and page
components. Kiln has no public registry yet, so take it from Kiln's
repository, as a path Ware:

```sh
git clone --depth 1 https://github.com/IZ737/kiln ../kiln
kiln add kiln/ui --path ../kiln/wares/ui
```

`kiln add` writes the dependency in `kiln.toml` and the exact source in
`kiln.lock`; commit both. Import it in Kiln code, and its components are
tags in the glazes of that folder:

```glaze
<form method="post" action="/sign-up">
  <ui.Input label="Email" name="email" type="email" value={email} required autocomplete="email" />
  <ui.Input label="Password" name="password" type="password" hint="At least 10 characters." required />
  <ui.Button type="submit">Sign up</ui.Button>
</form>
```

To change a component, own it: `kiln ui add button` copies it into
`components/`, where you change it as you like and use it as
`<components.Button>`. The guestbook's buttons are round that way.
[Wares](../reference/wares.md) covers registries, versions, the lockfile
and the cache, including running the registry in Kiln's repository and
publishing to it.

## 5. A theme switch

kiln/ui takes its colours from design tokens (`--color-accent`,
`--color-surface`, ...). Two themes set them, in a global stylesheet:

```css
.theme-light { --color-accent: #a8452a; --color-page: #f7f3ec; /* ... */ }
.theme-dark  { --color-accent: #f08a62; --color-page: #1f232b; /* ... */ }
```

The layout chooses one with a typed style, from a cookie a small form
sets, and every component follows:

```glaze
<body ui={{ theme: if dark { .dark } else { .light }, bg: color.page, fg: color.text }}>
```

Both themes must set the same tokens; the compiler checks. See
[Glazes](../reference/glazes.md#themes).

## 6. Tests

Unit tests sit beside the code; integration tests start the app with a
database of their own; browser tests drive Chromium:

```kiln
// tests/browser/test_signing.kiln
import test

fn test_signing_the_book() {
    let page = test.page()
    page.goto("/sign-up")
    page.label("Name").fill("Ada")
    page.label("Email").fill("ada@example.com")
    page.label("Password").fill("correct horse battery")
    page.role("button", name: "Sign up").click()
    page.wait_for_path("/")
    page.label("Message").fill("What a lovely site")
    page.role("button", name: "Sign the book").click()
    page.css(".entries li").expect_text("Ada: What a lovely site")
}
```

```sh
kiln test                     # all of them
kiln test --install-browser   # once, if there is no Chrome or Chromium on the machine
```

`kiln.toml`'s `[test] env` gives the app its settings during tests; the
guestbook's says it is served over plain HTTP. See [Testing](../reference/testing.md).

## 7. A build you can repeat

```sh
kiln build --locked --offline -o guestbook-server
```

`--locked` builds with exactly the Wares in `kiln.lock` and fails if
`kiln.toml` asks for anything else; `--offline` reads them only from the
cache. The executable carries its compiled pages and styles; deploy it
with the migrations, and point `DATABASE_URL` at the database.

## Next

- [Kiln syntax](../reference/syntax.md) and [type rules](../reference/types.md)
- [Glazes](../reference/glazes.md): templates, styles, themes and embers
- [Formatting, live reload and agents](tooling.md)
- [The guide for coding agents](agents.md)
