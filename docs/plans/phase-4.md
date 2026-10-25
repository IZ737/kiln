# Phase 4 — Data, models, forms, authentication and admin

**Status:** Planned; design and implementation contract, **not documentation of currently implemented APIs**. Phase 3 must provide compiled Glazes and server-side styling. The current `sql` and `auth` standard-library files are checkable placeholders; `Request.form`, `App.mount`, `orm`, `form`, `validate`, `session` and `admin` need implementation.

**Goal:** A developer can take **an existing SQL database or a fresh SQLite file**, obtain typed Kiln models, query and mutate data safely, handle validated forms, sign users in, enforce permissions, and mount a customizable **secure SSR admin site**—all from one Kiln application without adding a web framework, Node-based build chain or a third-party ORM.

**End-to-end completion gate:** Build and run a realistic database-backed application with working sign-in, sessions, role/record permissions, validation, migrations, generated CRUD admin, and custom Glazes. It must work as a native standalone Kiln executable on supported Linux and Windows. SQLite needs no separate database service; PostgreSQL and MySQL use external database servers. Both C and native compiler backends remain tested where supported.

This plan refines the existing [design overview](../design.md#orm), [product vision](../vision.md#data-forms-and-authentication), [language rules](../reference/types.md), [standard library placeholders](../../crates/kiln_check/std/) and [Phase 3 Glaze plan](phase-3.md). The [roadmap](../../README.md#roadmap) remains the source of phase sequencing.

**Reading rule:** All snippets below are **target API illustrations**, not executable examples of today's Kiln. Exact signatures and compiler lowering may change during implementation; public documentation only claims an API after code, tests and conformance land.

## 1. Non-negotiable design decisions

1. **Ordinary Kiln structs remain models.** No separate model-definition language, schema file, compulsory decorators or required base classes. Inference works by convention; explicit mappings solve exceptions.
2. **SQLite first, but not SQLite only.** Ship SQLite end to end before expanding the same user-facing abstraction to PostgreSQL and MySQL in Phase 4. Never silently pretend a dialect-specific feature works everywhere.
3. **Two complementary database APIs.** Explicit, parameterized `sql` is always available; `orm` supplies a genuinely typed, compiler-understood query surface. Users may mix both in one transaction.
4. **No surprising queries.** Relationships are typed, but fetching a row does not accidentally issue N extra requests. Loading relations is explicit; queries, limits and projections remain inspectable.
5. **No runtime reflection dependency.** The compiler derives schemas, encoders, decoders, query plans and admin descriptions from checked program types; the runtime handles data, IO and crypto. No general reflection engine or magic interpreter.
6. **Import is a first-class adoption path.** `kiln db pull` examines an existing database and creates editable models/mapping metadata. Re-import detects drift and proposes a reviewable update, never overwrites user code silently.
7. **Migrations are reviewable artifacts.** `kiln db diff` proposes changes; `kiln db migrate` applies explicit versions with checksums and locking. Neither starting a production server nor deploying an executable runs destructive DDL automatically.
8. **One type can inform the full data path.** Share field types and constraints between database persistence, form decoding, validation, admin input metadata and Glaze error display; allow separate input structs when an API should not expose every database field.
9. **Authorization belongs on the server.** Hiding a button never grants security. Admin routes, reads, writes, actions and object selection enforce policies before performing work.
10. **Secure defaults beat easy-to-misconfigure convenience.** Parameterized SQL, bounded input, signed opaque session IDs, protected cookies, Argon2id, CSRF protection, constant-time comparison where relevant, safe redirects, and explicit production secrets.
11. **Use maintained native implementations.** Rust libraries/drivers in the runtime are pragmatic starting points, not immovable architecture. Keep drivers, password hashing and session stores replaceable without changing Kiln application APIs.
12. **Respect phase boundaries.** Phase 4 is useful with normal HTTP requests, forms and server-rendered Glazes. No Embers/WasmGC, required Ware registry, `kiln.toml`, general module system or final public `kiln test` command is necessary.

## 2. Intended developer experience

### A. A model is a struct

```kiln
import sql, orm, web, auth, admin

struct Post {
    id: Int
    title: String
    body: String
    published: Bool = false
    author_id: Int
}

fn main() ! {
    let db = try sql.open(env("DATABASE_URL") ?? "sqlite:./data/app.db")
    let app = web.app()

    app.get("/posts", fn(req) {
        let posts = try orm.find<Post>(db)
            .where(.published)
            .order_by(.id)
            .limit(20)
            .all()
        web.render("posts.glaze", { posts })
    })

    // An explicit permission policy is required before mounting admin.
    let guard = auth.admin_policy(db)
    app.mount("/admin", admin.site(db, models: [Post], authorize: guard))
    try app.listen("127.0.0.1:8080")
}
```

`Post` as a model type, `.where(.published)` query lowering, `App.mount` and `admin_policy` are **target syntax/API**, not current features. The single type-list form `models: [Post]` is the intended convenient compiler-recognized admin syntax from the existing design; it is **not** a claim that types are ordinary runtime values. If compiler implementation benefits from typed descriptors such as `orm.model<Post>()`, those may be the underlying public form; preserve the one-line default experience.

### B. A user-submitted form

```kiln
import web, sql, orm, form, validate, auth

struct PostInput {
    title: String
    body: String
    published: Bool = false
}

fn create_post(req: web.Request, db: sql.Db) -> web.Response ! {
    let actor = try auth.require(req)
    try auth.authorize(actor, "post:create")

    // Parsing produces typed data or a structured bad-input result.
    let input = try req.form<PostInput>()
    let checked = validate.check(input, {
        title: [validate.required(), validate.length(min: 3, max: 120)],
        body: [validate.required()]
    })
    if not checked.valid {
        return web.render("posts/new.glaze", {
            values: input, errors: checked.errors
        }, status: 422)
    }
    let post = try orm.insert<Post>(db, {
        title: input.title,
        body: input.body,
        published: input.published,
        author_id: actor.id
    })
    web.redirect("/posts/${post.id}", status: 303)
}
```

Illustrative only: exact validation descriptors, checked-result shape, render-status argument, auth signatures and insert syntax are to be settled before those milestones. Form input is **not assumed trusted because it decoded successfully**.

### C. Starting with somebody else's database

```text
DATABASE_URL=sqlite:./legacy.db kiln db pull --out models.kiln
kiln check .
DATABASE_URL=sqlite:./legacy.db kiln db status
DATABASE_URL=sqlite:./legacy.db kiln db diff --name add_post_summary
# Review the generated dialect-specific SQL and any warnings.
DATABASE_URL=sqlite:./legacy.db kiln db migrate
```

No mandatory manifest. Credentials may come from environment variables or a CLI argument, but tools redact them in logging and diagnostics.

## 3. Ownership, layers and boundaries

| Layer | Responsibilities | Must not do |
| --- | --- | --- |
| Parser/type checker | Schema derivation from structs, field/type validation, query-expression analysis, form/validation typing, generated-Glaze diagnostics | Open databases during ordinary `kiln check`; execute user SQL while compiling |
| IR/codegen | Typed row codecs, static query descriptors, parameter binding, model metadata, admin route/Glaze generation | Emit ad-hoc SQL through interpolated user strings |
| `kiln_rt` / drivers | Database IO, pooling, rows, transactions, encryption/hash verification, cookies/session persistence | Interpret arbitrary user-defined type declarations at runtime |
| CLI | Introspection, migration generation/status/apply, drift reporting, safety prompts | Auto-apply unreviewed DDL during `build`, `run` or deployment |
| Kiln stdlib | Composable typed `sql`, `orm`, `form`, `validate`, `session`, `auth`, `admin` APIs | Require users to adopt the ORM/admin for simple SQL or HTTP apps |

The runtime should link or initialize only the database/auth functionality a program reaches, compatible with later standard-library dead-code elimination and replacement of implementation libraries. Keep driver-specific detail behind a stable error and capability surface.

**Current source compatibility:** `crates/kiln_check/std/sql.kiln` already sketches `sql.open`, `Db.query<T>`, `one<T>`, `scalar<T>`, `exec` and `insert<T>`; `auth.kiln` sketches `User`, `current`, `require`, `hash_password`, `verify_password`; `web.kiln` sketches `Request.form<T>`. Evolve these deliberately. Breaking changes are acceptable before 1.0 but need changelog entries and conformance updates.

## 4. SQL: driver and connection foundation

### Connection schemes and support

| Database | Target DSN | Initial profile |
| --- | --- | --- |
| SQLite | `sqlite:./data/app.db`, `sqlite::memory:` | No external service, file and memory, transactions, optional WAL/busy timeout |
| PostgreSQL | `postgres://user:pass@host:5432/app` | TLS-aware connections, bounded pool, transactions and prepared parameters |
| MySQL | `mysql://user:pass@host:3306/app` | TLS-aware connections, bounded pool, transactions and prepared parameters |

URLs are examples of the proposed contract; credential encoding and URL parser behavior must be specified and tested. Avoid logging plaintext DSNs. Explicit TLS validation is mandatory in production remote-DB configurations; never silently weaken certificate verification.

### Minimum `sql` API

- `sql.open(url, options: ...)` returns a reusable `Db` handle, not an open socket per query.
- `db.query<T>(sql, args...)` decodes a list of typed rows; `db.one<T>` returns `T?`; `db.scalar<T>` decodes exactly one scalar or reports a deliberate cardinality error; `db.exec` returns affected-row count where the driver can supply it.
- Use **positional `$1`, `$2`, ...** in Kiln source across all backends, with safe dialect translation/binding under the hood. Placeholders are never replaced using string concatenation. Identifier/table names cannot be bound values.
- Literal SQL with known row types gets compile-time compatibility checks when a pinned local schema snapshot is available. Otherwise validate typed decoding at runtime with useful column/type diagnostics. Never require a live database during compilation.
- Expose explicit `DbError` classes: connection/timeout, syntax, constraint (unique/FK/not-null/check), decode, transaction conflict, unsupported dialect/operation, cancellation and migration drift. Preserve safe database error metadata, redact secrets and avoid emitting SQL parameter values by default.
- Support nullable columns ↔ Kiln optionals, integer bounds, `String`, `Bool`, `Float`, `Date`/time forms where representable, and typed row records. Preserve unsupported binary/decimal/UUID/JSON-native types losslessly or report unsupported mapping; **never silently round money/decimal values through Float**. If a dedicated `Decimal` or `Bytes` type is introduced, require both backends, documentation and conformance.
- Disallow automatic implicit conversions that could truncate identifiers, dates or numbers; use explicit conversion helpers.

### Transactions and concurrency

Target shape:

```kiln
try db.transaction(fn(tx) ! {
    let account = try tx.one<Account>("select * from accounts where id = $1", id)
    try tx.exec("update accounts set balance = balance - $1 where id = $2", amount, id)
    try tx.exec("insert into transfers (account_id, amount) values ($1, $2)", id, amount)
})
```

- Closure receives a transaction-scoped handle supporting both `sql` and `orm`.
- Commit on success; roll back on error, panic/cancellation or dropped scope; no silent partial commit.
- Connection acquisition and database operations must not block Kiln's HTTP task scheduler indefinitely. Use bounded pools/driver IO workers appropriately.
- SQLite: document one-writer behavior, locking, busy timeout and WAL; never claim server DB concurrency is identical. Postgres/MySQL: sensible pool bounds and idle/connection timeouts. Instrument query latency/errors without logging sensitive parameters.
- First pass may restrict nested transactions; explicit savepoint support is an extension, not a hidden magic nesting model.

## 5. ORM: schema metadata and typed queries

### Model conventions

- `Post` → `posts`; `author_id` → `author_id`; `id: Int` is the default primary key.
- Field defaults are application-level defaults unless a DB default is explicitly represented. They are **not** automatically equivalent to SQL `DEFAULT`.
- `String?` ↔ nullable column; required fields stay non-null unless import reveals a mismatch.
- Define deterministic casing, pluralization, reserved-word quoting and collision rules. Names that cannot be inferred unambiguously require explicit mapping.
- Explicit per-model mapping can override table/column names, key (including non-`Int` keys), uniqueness, indexed fields, server-generated defaults, relation foreign keys and read-only/imported columns.
- Mapping metadata is checked and represented by compiler-recognized typed descriptors, not a second mapping DSL or runtime-only string dictionary.
- Relationships can be expressed as typed `User` / `List<Comment>` model relationships, per the design vision, backed by explicit mapped foreign keys. Relation navigation **does not trigger transparent lazy IO**: access is valid only after an explicit preload/join, or through an explicit relation query. The compiler should diagnose unclear unloaded relation access where possible.
- Generated/imported structs are editable Kiln source. Persistence metadata and relation declarations must remain separately inspectable.

### Typed queries

Target experience:

```kiln
let posts = try orm.find<Post>(db)
    .where(.published)
    .where(fn(p) { p.author_id == author_id })
    .order_by(.id)
    .limit(20)
    .all()
let one = try orm.find<Post>(db).where(fn(p) { p.id == id }).first()
```

- `where`, comparisons, `and`/`or`/`not`, `order_by`, `limit`, `offset`, explicit `count`, `first`, `all`, and primary-key `get` are the first query operators. Pagination must be bounded on admin routes.
- Typed field-shorthand syntax (e.g. `.published`) may reuse existing closure shorthand; query expressions are a **restricted compiler-recognized subset**, not arbitrary closures executed on every fetched row. Unsupported expressions produce a compile diagnostic.
- Captured runtime values become bound SQL parameters, not injected SQL syntax. Query values have checked field types. Misspellings and nonsensical comparisons fail during compilation.
- Explicit projection to typed records and join/preload plans follow once single-table query and CRUD semantics are sound; required before completing the full ORM milestone.
- Relationships must avoid N+1 queries by providing explicit batched `include`/preload with deterministic limits. Never issue hidden work during field access.
- Explain/inspect generated SQL in a development mode so users can investigate performance and correctness.
- Direct raw SQL and ORM calls can share `Db` and transaction handles without conversions.

### Mutation

- `insert`, `update`, `delete`, `upsert` (where supported) and atomic conditional updates.
- Specify generated-key retrieval by dialect; never assume `RETURNING` works everywhere.
- Mutating models requires explicit writable columns; protect generated/readonly keys and guard optimistic conflict/version fields when configured.
- ORM does **not** automatically authorize a caller; that remains the auth/admin/application layer.
- No implicit cascade delete in application code. Database-declared cascades must be discoverable and shown in diff/admin deletion confirmation.

## 6. Database-first schema import: `kiln db pull`

This is a central adoption feature, not an afterthought.

1. Connect and inspect tables, views, columns, nullability, primary/foreign keys, unique constraints, indexes, server defaults and dialect-specific enum/check information.
2. Produce ordinary editable `.kiln` model declarations and a **deterministic sidecar schema snapshot/mapping** sufficient to preserve names and types the Kiln structs alone cannot represent. Include source dialect and a sanitized schema fingerprint, **not credentials**.
3. Identify relationships, including join tables, without inventing a false `belongs_to` for ambiguous keys; report manual resolution requirements.
4. Generate valid names for otherwise invalid/reserved Kiln identifiers while recording the original SQL identifier; show collisions clearly.
5. On a second pull, diff the remote schema, last-known snapshot and edited source; propose changes and highlight conflicts. **Do not overwrite edited code silently**.
6. Views and tables without a supported primary key remain queryable/read-only where practical. Unsupported constraints, triggers, generated columns, custom types, partitions, stored routines and exotic indexes are reported rather than dropped from the developer's mental model.
7. No schema mutation occurs during pull. A successful pull cannot break the source database.

Proposed CLI:

```text
kiln db pull [--url URL] [--out models.kiln] [--schema NAME]
kiln db pull --check          # non-mutating drift / re-import report
```

Environment `DATABASE_URL` is the default. Missing parameters should yield actionable diagnostics, not prompts in CI.

## 7. Migrations: `kiln db diff` and `kiln db migrate`

### Workflow

1. Compiler derives desired schema from selected model types/mapping metadata, and CLI reads the prior **versioned snapshot** (not an arbitrary guessed production baseline).
2. `kiln db diff --name ...` emits a proposed, dialect-specific SQL migration plus metadata: stable revision ID, parent revision, checksum, dependencies, affected objects and warnings for destructive operations.
3. Developer reviews SQL and any data backfill instructions and commits migrations in source control.
4. `kiln db migrate` checks migration ledger/checksums, takes a per-database migration lock, verifies expected ancestry, then applies outstanding versions in order.
5. `kiln db status` reports applied/pending versions and drift; `kiln db verify` checks snapshots and migration checksums; `--dry-run` prints operations without changing the database.
6. If the live schema diverges from the expected migration state, **stop** and require reconciliation. Never infer that an undocumented production change is safe to overwrite.

Use paths such as `migrations/sqlite/`, `migrations/postgres/` and `migrations/mysql/` for distinct DDL dialects, within a normal app folder. A single model may describe portable features, but migrations are generated/reviewed per target database.

### Destructive changes and dialect realities

- Renames are ambiguous: require an explicit rename decision rather than silently implementing as DROP+ADD. Warn and require an interactive confirmation/explicit CI flag for drop, type narrowing, nullability tightening and cascade changes.
- Include backups/data-loss guidance for destructive operations. SQL down migrations may be authored where safe, but **automatic rollback of arbitrary production DDL is not promised**.
- SQLite table rebuilds need special review, transactional safeguards and foreign-key checks.
- Postgres transactional DDL is used where available; MySQL DDL often auto-commits; compensate with explicit state/recovery instructions rather than pretending transactions protect every migration.
- Migrations do not run automatically on server start, deployment, build or schema import.

## 8. Forms and validation

### Parsing and binding

- Implement `req.form<T>()` for `application/x-www-form-urlencoded` first, then multipart form data for bounded text fields and explicitly configured uploads. Malformed encoding, unexpected content type, duplicate scalar names, excessive fields/bytes and type-conversion errors have clear structured outcomes.
- Distinguish **decode failure** (invalid transport/input shape, generally 400) from **validation failure** (well-formed fields that violate app rules, generally 422). Users may re-render a Glaze with field errors and preserved safe input.
- Typed structs provide field names, optionals, booleans, basic input kinds and defaults. Passwords, permission fields and generated IDs must not automatically become user-writable just because they appear on a persisted model.
- `validate` exposes type-checked rules (required, length, bounds, format, choices, matches/cross-field, and custom ordinary Kiln predicates) and structured errors keyed by typed field. Model uniqueness checks may require DB access; race-free uniqueness remains a database constraint.
- Validate on the server on **every write**. Client HTML constraints and future Embers are conveniences, never trusted security checks.
- For SSR, keep form rendering in Glazes and reusable components; provide field-value/error helpers and generated admin field components, not an entirely new template language.
- File uploads: safe names, content-type skepticism, bounded size/count, temporary-file cleanup, no automatic public serving and explicit destination policy. Streaming uploads and external storage integration can be later enhancements if the first safe bounded path suffices.

### Form security

- State-changing cookie-authenticated requests require CSRF verification; generated admin/forms include tokens automatically, custom form helpers expose tokens clearly.
- No CSRF exemptions for JSON just because a request says it is JSON; base policy on authentication mechanism and actual cross-origin attack surface.
- Escape reflected field values and errors in Glazes; avoid reflecting raw submitted HTML, headers or SQL errors into trusted markup.
- Form-bound types should support opt-in allowlists to prevent mass assignment, not blanket `save(req.form<Model>())`.

## 9. Sessions, identity and authorization

### API boundary

- `session`: create, rotate, read, invalidate and expire opaque sessions; server-side storage supports DB-backed persistence, with in-memory storage clearly marked local/dev-only.
- `auth`: password hashing/verification, signed-in identity, login/logout, current/require, permission enforcement, auth middleware/hooks and provider adapters.
- The existing no-argument `auth.current()` / `auth.require()` stubs may be redesigned as **request-explicit** `auth.current(req)` / `auth.require(req)` (or an equally explicit checked request context). Avoid ambient global identity when multiple HTTP tasks run concurrently.
- Application `User` ORM models remain owned by the application; `auth.Identity` or an adapter maps IDs/roles/claims to user records. Never impose a single fixed `auth.User` database schema on pre-existing systems.
- Use ordinary Kiln functions for authorization policies: route/action/model/record checks. Offer `auth.require_role`/permission helpers as conveniences, not a fixed RBAC-only model.
- Failed permissions yield 403 (or deliberately 404 for sensitive object existence), missing login normally 401/redirect as selected by the route. No leakage of protected fields in serialized results.

### Secure defaults

- Passwords: maintained **Argon2id** implementation with calibrated parameters and upgrade-on-login policy. Use cryptographically secure random values; constant-time verification APIs; generic login failures; throttle attempts and make account enumeration difficult.
- Sessions: **opaque, unpredictable identifiers** stored server side (or hashed identifiers in DB), rotated at login/privilege changes, invalidated on logout, bounded by idle and absolute expiration. Prevent session fixation and stale privilege elevation.
- Cookies: `HttpOnly`, appropriate `Secure` in HTTPS environments, `SameSite=Lax` default unless a deliberate cross-site flow requires otherwise, explicit `Path` and prefix constraints, no secrets in URLs. Account for trustworthy reverse-proxy TLS configuration; never infer HTTPS from spoofable untrusted forwarded headers.
- CSRF: generated forms/admin include tokens; verify before mutations, including logout and privileged account changes. Enforce appropriate Origin/Referer checks as defense in depth.
- Authorization: deny by default for admin. Distinguish read/list/create/update/delete and optional per-object policy. Check again at mutation time to avoid TOCTOU where applicable.
- Redirects: do not accept untrusted external `next` URLs without an allowlist. Login/logout endpoints should not enable open redirects.
- Secrets/keys: environment or explicit secret source, secure defaults in local development, no hardcoded shared production secret. If signing keys are used for tokens, support versioned rotation; never store passwords in reversible form.
- Security logs: record authentication and privileged changes without exposing passwords, tokens, cookies, database credentials or personal form contents.

### Integrations

Local/password login with session-backed SSR is required for the Phase 4 completion gate. Design a provider boundary for OIDC/OAuth 2 and API tokens/JWT, but **do not require every identity protocol** to ship before the integrated ORM/admin experience. External-provider features must get their own threat model and protocol conformance before being called supported. Avoid homegrown crypto or identity protocols.

## 10. Generated, customizable admin

Target convention from the established design:

```kiln
app.mount("/admin", admin.site(db, models: [Post, User, Comment], authorize: policy))
```

### Required default UX

- SSR list/search/filter/sort/pagination and create/detail/edit/delete pages for supported writable models.
- Readable relation selectors, input widgets generated from typed fields and declared validation rules, useful empty/loading/error states (server-side), and accessible keyboard/focus/labels.
- Clearly show destructive confirmation, validation errors, uniqueness/FK constraint failures and optimistic conflicts.
- Allow admin operators to inspect safe audit events and understand errors without seeing sensitive SQL/stack traces.
- No user-supplied runtime template interpreter: admin is generated/compiled with the same Glaze and styling pipeline, with safe value escaping.

### Customization in layers

1. **Model registration:** title/label, columns, search fields, filters, sort, pagination, editable/read-only fields, validation and per-action visibility.
2. **Field components:** override input/display widget with typed Glaze components and validation metadata.
3. **Page overrides:** use app-owned Glazes at conventional paths such as `admin/post/edit.glaze`; checked props and source maps apply exactly as to ordinary Glazes.
4. **Custom actions:** named, typed server functions with explicit permissions, CSRF, idempotence/destructive confirmation policies.
5. **Theming:** Phase 3 CSS/Ui/Finishes; no mandatory third-party component Ware (the flagship `kiln/ui` Ware belongs to the Phase 6 ecosystem).

### Permission and safety rules

- `admin.site` **requires an explicit policy**; in dev, an opt-in safe local-only bootstrap flow may create the initial privileged account, but production refuses an unauthenticated mount.
- Authorization applies to list queries and individual reads, then again to writes/actions; filter query results by accessible scope rather than merely hiding inaccessible rows in templates.
- Password hashes, reset tokens, session IDs, secrets, internal audit metadata, server-only config and generated keys are omitted/read-only by default.
- Use POST for destructive actions, validate CSRF, avoid GET mutations, use safe redirects and anti-clickjacking/security headers.
- Admin handles large tables with bounded server-side pagination, indexed search warnings and no unbounded `select all` by default.

## 11. Compiler, diagnostics and LSP

- Derive row codecs, model metadata, query expression lowering, form decoders and admin descriptors at compile time. Do not add a new generated source language or a compulsory reflection runtime.
- Typed query errors point to **Kiln source spans** for unknown field, wrong comparison type, unsupported expression, unknown relation, dialect-only operator or unsafe SQL identifier use.
- Add actionable error codes for unknown model/table mapping, invalid relation/key, incompatible form field/rule, unavailable driver, migration mismatch and admin authorization misconfiguration. Document codes in the diagnostics reference as implemented.
- LSP supports completion/hover/definition for ORM model fields and query shorthands, validation rules, mapped columns, generated form/admin props and admin Glaze override files; diagnostics should work without a live DB.
- Avoid cross-compiler/backend divergence: derived codecs and queries should flow from shared typed IR; no handwritten parallel semantics in C vs native codegen.

## 12. Reliability, operations and performance contracts

- **Bounded resource use:** connection pools, request body size, upload size, statement timeout, result/pagination caps and sensible defaults. Expose tuning only when needed.
- **Cancellation:** handler cancellation and timeouts must release DB resources; transactional scopes must not leak locks.
- **Storage:** SQLite files must have explicit path semantics, backup and WAL/checkpoint guidance. Running a binary does not recreate/delete an existing DB unexpectedly.
- **Observability:** stable SQL/query error classification, timings, pool metrics and safe debug SQL preview; redact secrets and bind values unless developer explicitly opts in within a safe environment.
- **Portability:** native Linux/Windows builds, cross-platform path/encoding tests and representative C backend cases. A standalone program bundles client libraries where possible; remote database servers remain external.
- **Performance:** benchmarks include query throughput/latency, row decoding, ORM query generation, transactions, rendered admin pages and app binary size. Record baseline measurements; optimize after semantics and correctness are established.
- **Replaceability:** treat chosen database drivers, session stores and crypto crates as replaceable private runtime implementations while retaining conformance suites as behavioural contracts.

## 13. PR-sized implementation steps and review gates

Each step must include tests and docs, and must not expand into Phase 5 Ember browser reactivity or Phase 6 developer tooling.

### Step 1 — SQLite SQL foundation

- Implement `sql.open`, typed `query/one/scalar/exec`, placeholders, row encoders/decoders, optionals, error categories, file/memory databases, pool/connection lifecycle.
- Add `db.transaction` with commit/rollback, timeouts, constraints and cancellation cleanup.
- **Gate:** a real SQLite-backed Kiln app performs safe CRUD and transactions; edge cases and failure paths pass on native, C and supported Windows target.

### Step 2 — Struct-based ORM + typed query lowering

- Schema/model metadata from structs, conventions and explicit overrides; generated row codecs.
- Typed filtering/order/limits/count/get/CRUD, constrained query expression lowering, parameterized captures, explicit relations/preloading and readable SQL inspection.
- **Gate:** a related multi-table model round-trips, invalid field/type queries fail during `kiln check`, no hidden N+1 and raw SQL interop works inside a transaction.

### Step 3 — Schema import and migration tooling (SQLite)

- `db pull` introspection/import with deterministic snapshots and drift reports.
- `db diff/status/verify/migrate`, reviewed SQL migrations, checksum ledger, destructive-operation safeguards, SQLite-specific rebuild checks.
- **Gate:** import a pre-existing SQLite database, preserve its data, generate/edit models, make an additive migration, and detect a manual schema drift or migration checksum modification without overwriting anything.

### Step 4 — PostgreSQL and MySQL parity

- Reuse SQL/ORM public APIs with maintained Rust database drivers; TLS, pooling, dialect-aware bindings, transaction and DDL behavior.
- Add schema import/migration support for both dialects and explicit feature/capability diagnostics.
- **Gate:** same portable application logic performs CRUD, relations and forms-independent transactions on SQLite, Postgres and MySQL; each dialect has a tested pull→diff→migrate path in CI/service tests. Fail clearly for unsupported portable mappings.

### Step 5 — Forms, typed binding and validation

- `req.form<T>()`, safe URL-encoded parsing, bounded multipart uploads, model/input distinctions, typed validation rules and structured errors.
- Glaze helpers/components for values, field errors, CSRF-ready forms and constraints; preserve accessibility and safe escaping.
- **Gate:** successful and invalid inputs re-render correctly; malformed/hostile payloads are bounded and rejected, not panics or unsafe HTML/SQL.

### Step 6 — Sessions, local authentication and policies

- DB-persisted sessions, cookies, expiry/rotation/logout, Argon2id, verified login/logout flows, CSRF and permission policies with request-explicit identity.
- Secure configuration, rate limiting/abuse controls, safe auth logs and explicit bootstrap.
- **Gate:** unauthenticated/unauthorized users cannot retrieve or mutate protected data; login survives server restart; logout and rotation invalidate old session IDs; CSRF/session fixation/redirect regression cases pass.

### Step 7 — Generated Glaze admin and overrides

- Compile SSR CRUD screens from typed models, auto fields/search/filter/pagination, validation, auth/CSRF, audit-safe errors.
- Typed model options, field/page Glaze overrides, custom actions and Phase 3 theme integration.
- **Gate:** mount a secure usable multi-model admin in one app without writing CRUD pages, then replace one field widget and one whole page using ordinary Glazes.

### Step 8 — Integrated dogfooding, docs and cross-target gates

- Build a substantial **database-backed Kiln application** with users, posts, comments, foreign keys, public read routes, edit forms, login, per-record authorization, schema import, committed migrations and custom admin.
- Dogfood both a new SQLite app and a representative pre-existing imported database; exercise Postgres/MySQL profiles.
- Expand conformance/CI and LSP smoke tests, audit docs/API consistency, inspect native self-contained deployment, measure baseline performance and binary size.
- **Gate:** the full definition of done below passes. Mark Phase 4 complete in README **only after** this gate.

## 14. Conformance and test matrix

| Domain | Positive cases | Negative/edge cases |
| --- | --- | --- |
| SQL codecs | Scalars, records, optional NULL, dates, long text, Unicode | Overflow, mismatched column count/type, unsupported binary/decimal mapping, malformed UTF-8 |
| SQL operations | Bind params, affected counts, insert keys, query ordering | Injection attempts, duplicate column aliases, syntax errors, uniqueness/FK violations |
| Transactions | Commit, explicit rollback, nested restriction, error propagation | Failure mid-write, server task cancellation, busy lock, dropped handle |
| ORM | Default/overridden model names, joins/preloads, filters, pagination | Unknown field, invalid comparison, ambiguous relation, N+1 regression, type mismatch |
| DB import | Existing SQLite/Postgres/MySQL with keys/indexes/relations | Names colliding with Kiln keywords, views/composite keys, unsupported types, repeat pull after edits |
| Migrations | Fresh/apply/pending/verify, additive change, data preserved | Checksums changed, conflicting drift, destructive prompt, concurrent migration, failed partially committed DDL |
| Forms | URL encoding, UTF-8, optional/checkbox, field errors, multipart | Oversize uploads, duplicate scalars, invalid encodings, mass assignment, error HTML injection |
| Auth/session | Login, logout, expiry, rotation, role and record policy | CSRF, fixation, forged/expired cookies, open redirect, brute force, unauthenticated admin |
| Admin | CRUD, field/page overrides, filters, safe pagination | Inaccessible record, forged action, IDOR, readonly mutation, XSS via database values |
| Cross-target | C/native, Linux/Windows app builds; all supported DB dialects | Driver missing, unsupported dialect option, no compiler/linker installed for native output |

Use a mix of compiler/unit tests, source-level diagnostic fixtures and `conformance/` applications with expected outputs/HTTP responses. For SQL integration, create disposable databases with deterministic fixtures. Start real Postgres/MySQL test services in CI when their milestone lands; do not simulate parity with SQLite alone. Use real-browser checks for relevant CSRF/XSS/admin behaviour as supplemental CI, while the **public `kiln test --browser` command remains Phase 6**.

Never equate passing tests with a guarantee of no bugs; the suite is an increasingly strong regression contract.

## 15. Definition of done

- [ ] A standalone Kiln binary can open SQLite without another service and perform typed, parameterized SQL, transactional operations and structured error handling.
- [ ] The same supported SQL/ORM surface works against PostgreSQL and MySQL with documented dialect distinctions, TLS configuration and meaningful conformance coverage.
- [ ] Models are normal structs; schema mapping, primary keys, nullability, defaults and relations are understandable and compiler-checked.
- [ ] Typed `orm` queries reject unknown fields, invalid comparisons and unsupported expressions at compile time; runtime values are bound rather than concatenated; relation loads are explicit and bounded.
- [ ] `kiln db pull` imports existing databases to editable model source plus mapping/snapshot, detects drift and never silently overwrites edits or mutates the source DB.
- [ ] `kiln db diff` produces reviewable versioned, dialect-specific SQL; `status`/`verify`/`migrate` preserve integrity and do not silently apply destructive schema changes.
- [ ] Typed form parsing and validation work with SSR Glazes, field errors, safe default escaping, CSRF, bounded input and the same application/model field rules.
- [ ] Secure local auth and durable sessions work with HTTP handlers; identity is request-bound, cookies are safe, logins rate-limited, passwords hashed with Argon2id, and authorizations enforced on every privileged server action.
- [ ] A functional generated admin provides list/search/filter/create/edit/delete for several models, explicit protection, safe field defaults and per-model/field/page customization using Glazes.
- [ ] Real SQLite and imported-schema dogfood apps run on Linux and Windows; C/native checks and Postgres/MySQL CI suites pass; diagnostics and LSP understand model/query/form fields.
- [ ] API/reference documentation distinguishes implemented features from planned syntax, includes migration/auth threat-model guidance and teaches both raw SQL and ORM workflows.
- [ ] No Wares, project manifest, Embers, reactive browser framework or external Node toolchain is necessary to finish the Phase 4 application.

## 16. Explicitly deferred or optional

- **Phase 5:** Embers, WasmGC, reactive form validation/hydration, typed browser-to-server RPC and JS interoperability.
- **Phase 6:** General multi-folder modules, `kiln.toml`/`kiln.lock`, Wares and registry, official `kiln/ui` component Ware, global cache, formatter/hot reload, public integrated `kiln test` runner and broader agent tooling.
- **Phase 7:** One-command deploy, advanced observability/optimization, SSG, stability commitments, broad cross-architecture compiler/runtime specialization.
- **Separate opt-in extension work:** OIDC/OAuth 2 providers, JWT/API token issuance, WebAuthn/passkeys/MFA, distributed rate limiting, streaming cloud uploads, cross-region migrations and advanced enterprise DB features. Keep extension points; **do not claim any of these exists until tested and documented**.
- **Non-goals:** Inventing an ORM-only SQL dialect; implicit database queries when reading a normal field; auto-generating production migrations at startup; storing passwords or sessions insecurely to simplify demos; forcing one canonical user schema; enforcing Wares adoption; inventing a second template language for admin.

## 17. Final architectural review principles

- A simple SQLite-backed site should be almost as easy to start as a file-backed one.
- Existing database owners should be able to adopt Kiln incrementally and without schema surprises.
- Every database query and permission boundary should be explainable, inspectable and testable.
- Models, forms, auth and admin should compose through ordinary Kiln types/functions rather than hidden configuration frameworks.
- Security is a **compiler/runtime/application contract**, not merely a set of HTML controls.
- Established maintained database and crypto libraries are replaceable implementation choices; capability, stable behaviour and good DX are the product.

**In one sentence:** **Phase 4 turns Kiln from a native web renderer into a database-backed application platform: typed data, reviewed schema changes, trustworthy forms, secure identities and a customizable admin—all without the glue.**
