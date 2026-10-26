# Journal

A small site where people sign up, write posts and comment on each
other's: the example of phase 5's databases, sign-in and admin, in
folder modules.

- `shared/` holds the models (`User`, `Post`, `Comment`), the rules every
  form checks, who may read and change a post, and the database.
- `accounts/` is signing up, in and out: Argon2id passwords, sessions in
  the database, and a check that forms come from this site.
- `posts/` is the pages, the forms with their tokens, and
  `comment_box.ember`, which adds a comment through the `server fn`
  `add_comment`. The server gives that function the request, so it knows
  who is commenting; without WebAssembly the same form posts to a route
  that checks the same rules.
- `main.kiln` puts them together, with the admin's pages generated from
  the models for the person `JOURNAL_ADMIN` names.
- `migrations/` holds what `kiln db diff` wrote.

```sh
kiln db migrate --url sqlite:journal.db --path examples/journal
JOURNAL_HTTP=1 JOURNAL_ADMIN=you@example.com kiln run examples/journal
```

`JOURNAL_HTTP=1` lets the session cookie travel over plain HTTP, for a
laptop; leave it out behind HTTPS. `crates/kiln_cli/tests/browser.rs`
drives the app in Chromium.
