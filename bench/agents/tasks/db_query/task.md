This program keeps blog posts in a database (`shared/models.kiln`, with
its migrations in `migrations/`). Add a folder module `posts` with

    pub fn published(db: sql.Db, limit: Int) -> List<shared.Post> !

which returns at most `limit` published posts, newest (highest `id`)
first, and a route `GET /posts` that answers with a JSON list of the
titles of the 10 newest published posts, such as `["Second", "First"]`.
Drafts are never shown. Use the database `DATABASE_URL` names.
