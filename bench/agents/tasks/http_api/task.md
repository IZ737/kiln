Build a JSON API for a to-do list in this folder's program. Keep the
to-dos in memory; they do not need to survive a restart.

- `GET /todos`: every to-do, oldest first, as a JSON list of
  `{ "id": 1, "title": "...", "done": false }`. Ids start at 1.
- `POST /todos` with a JSON body `{ "title": "..." }`: adds one and
  answers `201` with it. A title that is empty after trimming spaces is
  refused with `422` and `{ "error": "title is required" }`.
- `GET /todos/{id}`: that to-do, or `404`.
- `POST /todos/{id}/done`: marks it done and answers `200` with it, or `404`.

Add integration tests under `tests/integration/`.
