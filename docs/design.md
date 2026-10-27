# Kiln design overview

This is a summary of the language design. Decisions here can still change
until 1.0; settled questions are in the [decision log](decisions.md). The broader product
philosophy, frontend/backend developer experience, performance escape
hatches, interoperability and adoption strategy are in
[Kiln's long-term vision](vision.md). The [README roadmap](../README.md#roadmap)
controls phase sequencing; aspirational ideas in the vision are not
automatically commitments for 1.0.

**Implementation status (v0.11.0):** native Linux/Windows builds, HTTP routing,
JSON, server-rendered Glazes, typed styles/themes, explicit WasmGC Embers,
typed RPC, browser/npm interop, local folder modules, SQL/ORM, sessions,
auth and the generated admin, Wares, `kiln test`, `kiln fmt`, `kiln dev`
and `kiln mcp` are implemented. The `kiln fire`/deployment workflow
remains planned. The goals and examples below
include future design; use the [current references](README.md#language-reference)
for supported syntax and APIs.

## Core principle

Most web development today is gluing together problems that were solved
long ago (routing, templates, the ORM, auth, forms, the admin, asset builds,
deployment) and sometimes reinventing one. Kiln removes that work: every one
of those pieces ships with the language and they already fit together, so a
project that follows the conventions needs no configuration and no glue
code. The conventions are defaults, not walls. Any piece can be configured
in depth, or replaced, where it is used. Every design decision is checked
against this rule.

## Progressive integration

Kiln provides a consistent developer experience whether used for an
individual component, one application layer or an entire full-stack
application.

The compiler uses all available information to provide type checking, code
generation and optimization. Full-stack Kiln applications benefit from
deeper automatic integration, while applications containing external systems
use explicit, typed contracts and adapters to unlock as many of the same
capabilities as possible.

No application feature should unnecessarily require ownership of the entire
technology stack.

## Developer experience, performance and interoperability

**Frontend development matters as much as backend development.** Kiln aims
to unify typed forms and validation, accessible Glaze components, native
CSS, Ember interactivity, server functions and data access so full-stack
developers do not recreate the glue between separate frontend and backend
frameworks. The compiler should share types and generate the routine
communication, while leaving explicit low-level APIs available.

**Optimize hotspots, not whole applications.** Ordinary Kiln stays
garbage-collected. Future compiler optimization, optional arena/region
allocation and more specialized numeric/data-layout operations should allow
performance-sensitive code to stay in Kiln where practical, with compiler
checks preventing memory-lifetime errors. These advanced mechanisms are
later design explorations, not mandatory ownership rules.

**Interoperability goes both ways.** Kiln should call existing libraries,
and existing applications should be able to call Kiln modules. Proposed
paths include C-compatible FFI, generated Python/Node bindings, portable
Wasm components and browser JavaScript/npm integration through typed
bindings. For Phase 4 browser Embers, the finalized direction is **typed
browser/npm bindings by default, with a deliberate checked `js.Value`
dynamic escape hatch**; Kiln developers need not author JavaScript. Mixed
stacks must remain viable, and interop boundaries must handle memory,
errors, security, lifecycle and ownership explicitly. See the
[Phase 4 interop contract](plans/phase-4.md#8-javascript-and-browser-interoperability).

The complete reasoning and future scope live in [vision.md](vision.md).

## Goals

1. **One binary deploys**, with templates and assets embedded.
2. **Batteries included for the web**: HTTP/1.1 and 2, TLS, routing,
   templating, JSON, forms, SQL with an ORM, a generated admin UI, sessions,
   auth and background jobs.
3. **Errors at compile time**: routes, template variables, SQL results and
   JSON shapes are type-checked.
4. **Safe by default**: auto-escaping templates, parameterized SQL, signed
   cookies, CSRF protection on.
5. **Fast edit loop**: sub-second debug builds with live reload.
6. **No async/await split**: every request runs on a lightweight thread.
7. **One language for server and browser**: components render on the server
   and become interactive through WebAssembly only where needed.
8. **Easy for coding agents**: an LLM that has never seen Kiln can write
   correct code from the material shipped with the compiler.

Non-goals for 1.0: systems programming, an interpreter, and reimplementing
the JavaScript ecosystem (browser code calls npm packages through generated
bindings).

## Types

- Static types with local inference. Function signatures are always typed.
- Primitives: `Int` (64-bit), `Float`, `Bool`, `String` (UTF-8), `Bytes`
  (binary data), `Duration` (`250ms`, `2s`). Money is whole cents in an
  `Int` until a decimal type is designed.
- `List<T>`, `Map<K, V>`, `Set<T>`; structs with defaults; enums with payloads
  and exhaustive `match`; structural interfaces as bounds on generic
  functions (`fn label<T: Named>(x: T)`), with no interface values; generics
  by monomorphization.
- No null. `T?` is optional and the missing value is `none`.
- Errors are values: `fn f() -> T ! E`, propagated with `try`, handled with
  `catch`. A bare `!` uses the standard `Error` and accepts any error; `! E`
  only passes on errors of type `E`. See [types.md](reference/types.md). Panics are for bugs only, and the HTTP server recovers them per
  request.
- JSON, form and SQL encoding are derived at compile time, with no runtime
  reflection.

## Programs and folders

A folder is one module. All of its `.kiln` files share one
namespace, so splitting a file in two never needs an import, and the
folder's glazes see the same types and functions. Imports stay per file.

**Phase 4.2:** [multi-folder projects and local modules](plans/phase-4.2.md) moves project structure **before Phase 5 data/ORM/auth**. A subfolder is a separate local module, imported with `import posts` (nested: `import posts.search`), and `pub` exports its API. Same-folder `.kiln` files still share declarations, but imports remain per source file. Compiler resolution, type checking, builds, Glazes/Embers and assets, diagnostics and LSP navigation share folder-module binding; local imports require no registry, Ware installation, lockfile or compulsory manifest.

**Phase 4.2 completion:** A manifest-free multi-folder application builds and runs offline with protected module visibility, deterministic path/cycle/name diagnostics and working module-local server Glazes and client Embers. The optional minimal `kiln.toml` may identify a project root and entrypoint; no project file is necessary for an unambiguous simple or multi-folder program.

### Phase 4.2: minimal project manifest

A declarative optional `kiln.toml` helps select the app entrypoint without procedural configuration or scripts:

~~~toml
[package]
name = "fieldnotes"
kind = "app"
entry = "main.kiln"
~~~

Explicit CLI arguments take precedence over manifest defaults. Manifest-relative entry paths must stay inside the project root; parsing/diagnostics are source-linked. No plaintext secrets, per-folder manifests or automatic dependency installation. `kiln-deploy.toml` remains separate in Phase 7.

See the [project reference](reference/projects.md) for the implemented root, entry, path and visibility rules.

### Phase 6: extending the project manifest for Wares

The [Phase 6 implementation plan](plans/phase-6.md) **reuses** Phase 4.2's folder/import/visibility model and extends the existing `kiln.toml` schema with package version/publishing identity, build/test defaults and versioned local/Git/registry **Wares**. It also adds `kiln.lock`: the resolved external dependency graph, exact releases/commits and integrity/source information for reproducible builds. Ordinary local folder modules require neither `[wares]` nor a lockfile.

Illustrative future Phase 6 extension (these `[wares]` names and fields are proposals, not current working syntax):

~~~toml
[package]
name = "fieldnotes"
version = "0.1.0"
kind = "app"
entry = "main.kiln"

[wares]
"kiln/ui" = "0.1"
"example/markdown" = "1.2"

[build]
target = "linux"

[test]
browser = true
~~~

`kiln add` / `kiln remove` should mutate the manifest and lockfile together, with explicit safe resolution. **`kiln upgrade` is for upgrading the compiler**, not Ware dependencies. Dependency verification and publishing do not execute arbitrary install scripts. Future `[workspace]` and multi-package publishing are optional later scope, not requirements for Phase 4.2 or basic Phase 6 Wares.

### Phase 6: global Ware storage and caching

**Decision:** Wares are stored in **one shared, machine-local cache per
operating-system user** by default, **not** copied to a per-project
`ware_modules/`, `node_modules/` or similar directory. Projects keep
their small declarative `kiln.toml` and exact `kiln.lock` only; resolution
finds an immutable Ware in the global store using the lockfile's source,
version and verified content hash. Multiple projects can share the same
cached package version without having to install it repeatedly. Multiple
versions of a Ware must coexist safely and each project resolves its own
dependency graph.

- **Scope and paths:** Choose platform-standard per-user cache locations
  (Windows: a suitable LocalAppData directory; Linux: the XDG cache
  location, falling back to the user's home cache directory; macOS:
  platform-standard cache directory). Make the root inspectable and
  optionally configurable for CI and constrained/offline machines.
  A truly **cross-user, machine-wide** cache can be explored later, but
  needs stricter permissions, ownership and trust boundaries; don't
  require admin privileges or share writable cache files across users.
- **Store identity:** Use content-addressed, immutable package content,
  verified by hashes and registry/source metadata. Download to temporary
  files and atomically publish only after integrity verification; reject
  tampered, incomplete or mismatched content. Use safe cross-process
  coordination so simultaneous builds cannot corrupt a shared cache.
  Cached source is untrusted until checked; prevent path traversal or
  archive extraction outside its dedicated content directory.
- **Project isolation:** The cache never determines the selected
  dependency version. `kiln.lock` does. Two apps can depend on different
  versions independently, and installing/removing a Ware in one project
  must not alter another's lockfile or resolved graph. Unversioned
  **local path modules** remain live source files and are not copied
  into the immutable global Ware cache.
- **Compiled artifacts:** Optionally cache compiled Ware/object outputs
  separately from immutable source, keyed by content hash **plus** Kiln
  compiler/runtime version, target triple, feature set, optimization and
  relevant build configuration. Never reuse incompatible artifacts.
- **Offline/reproducible builds:** An offline or locked mode such as
  `kiln build --offline` must use locally available, verified locked
  Wares without contacting the network; report a missing hash/version
  clearly rather than silently upgrading or downloading alternatives.
  CI should be able to prefill and reuse the same cache.
- **Maintenance UX (proposed commands):** `kiln cache dir`,
  `kiln cache stats`, `kiln cache verify` and `kiln cache prune`.
  Cache pruning should preview candidates or make its effect explicit,
  and avoid deleting data in active use. Stale package contents may be
  evicted and fetched again later; do not promise to detect every
  project on the machine or assume an unused version can never be
  needed for offline builds.
- **Deployment:** Compiler outputs remain self-contained. Production
  executables must not need access to the developer's cache, Wares
  registry or network to load their compiled Ware code at runtime.

**Phase 6 acceptance:** Two unrelated projects locked to the same Ware
version/hash download it **once** and share verified cached contents;
one project can pin a different version without affecting the other;
offline rebuild works after cache priming; integrity corruption is
detected; concurrent cache access is safe; `kiln cache` offers usable
inspection and pruning; and no per-project dependency directory is
needed to run or build an application.

## Memory and concurrency

- A concurrent, non-moving, tri-color mark-and-sweep GC, as in Go. Phase 1
  links the Boehm collector; phase 7 replaces it with a precise collector.
- Escape analysis, and later request-scoped arenas for garbage that dies when
  a response is sent.
- An M:N scheduler of lightweight tasks; blocking I/O parks the task. Tasks
  spawned in a request are cancelled when it ends. Values shared between tasks
  must be immutable or wrapped in `Mutex`, `Atomic` or a channel.
- Phase 2 runs tasks on one core, cooperatively: a task gives way only
  where it waits. Programs must not rely on that. Changing a shared
  mutable value from several tasks (such as a `var` captured by two
  handlers) works today but is not a guarantee, and the compiler may later
  reject it, so that tasks can move to several cores without changing
  what programs mean.

## Compilation

The compiler is written in Rust. Source is parsed, type-checked and lowered
to a typed mid-level IR, which feeds a backend:

| Backend | Use | Phase |
| --- | --- | --- |
| C (via clang, gcc or `zig cc`) | First backend; free cross-compilation | 1 |
| Cranelift | Self-contained native compilation; fast debug builds | 1.5 |
| WasmGC | Browser code (embers) | 4 |
| LLVM | Optimized release builds | 7 |

The IR (`crates/kiln_ir`) is a typed tree: names are resolved, closures and
field shorthands are lifted into functions with their captures listed (a
captured `var` lives in a heap cell), call arguments are in parameter order
with defaults filled in, and every place a value changes representation,
such as an `Int` stored as a list item, is an explicit coercion. Generic
functions and methods are monomorphized here: each use gets a copy for its
type arguments (`show<Int>`, `show<Post>`), so generic code can print,
compare and hash its values. It stays
a tree of blocks, loops and `if`s rather than a control-flow graph, so the
C backend maps it almost line for line; later backends can flatten it.
`kiln build --emit-ir` writes its readable form.

The phase 1 C backend (`crates/kiln_codegen`) emits one C file with the
runtime (`runtime/kiln.c`) in front, and builds it with `$CC` or `cc`
against the Boehm collector. Values keep their natural C types (`int64_t`,
`double`, `bool`, pointers); list, map and set items, the fields of generic
structs and enums, and closure arguments use a uniform 64-bit slot, so the
runtime's collection code is shared by every item type. A call that fails sets a
thread-local flag that `try` and `catch` check. Parts of the language the
backend does not support yet are reported as E0401.

The native backend (`crates/kiln_native`, phase 1.5) generates machine code
from the same IR with Cranelift. For Linux it links the code into a static
executable with the wild linker, run inside `kiln`. For Windows, `kiln`'s
build links the runtime into an executable with no base relocations, and
`kiln` adds each program's code and data to a copy of it as new sections,
applying the relocations itself and writing the program's entry points into
a table the runtime's `main` reads. Values are stored as in the C backend
and both call the same runtime, which `kiln`'s build compiles once per
target, with the Boehm collector and a C library (musl for Linux, the
`msvcrt` that ships with Windows), for `kiln` to carry. Struct fields
are slots, so a struct is an array of them. Where the C backend writes a
show, equality, hash or ordering function for each type, the native one
writes a description of the type into the executable's data, and runtime
functions read it. Both backends report the same E0401 errors, and the
conformance programs run with both.

### Phase 1.5: self-contained toolchain and native executables

Phase 1.5 comes after the working C backend and before phase 2's HTTP work.
Its goal is **zero installation-time toolchain and runtime dependencies for
end users**, not eliminating libraries from Kiln's implementation:

- Ship a prebuilt `kiln` executable that can compile supported CLI programs
  without requiring a separately installed C compiler, linker, Rust toolchain
  or garbage collector. Building the compiler itself from source may still
  use Rust, Cargo and third-party libraries.
- Bring the Cranelift backend forward from a later phase to generate native code
  directly from Kiln's typed IR. Provide an integrated or bundled linking
  path rather than invoking a system-installed linker. Retain the C backend
  as a supported alternative and correctness reference.
- Bundle and statically link the Kiln runtime, Boehm GC and required system
  support into generated executables. Kiln's standard-library API remains
  available; users simply do not install its implementation separately.
- Target Linux x86-64 first, with an explicit follow-up for native Windows
  x86-64. Windows should ship a standalone `kiln.exe` that generates Windows
  `.exe` programs using a bundled or integrated PE/COFF linking path and
  a bundled Kiln runtime and GC. Neither compilation nor execution should
  require users to install a C compiler, linker, Rust, GC or redistributable
  runtime package. Normal system libraries included with a supported Windows
  installation are allowed. Other platforms may follow separately.
- Run the existing phase 1 conformance suite against the new backend and
  verify the installation guarantee in clean environments; add Windows CI
  coverage as the Windows target is implemented.

**Done when:** On a clean, compatible Linux x86-64 machine, downloading
one `kiln` executable is sufficient to compile supported CLI programs
without installing build tools or runtime libraries, and the resulting
executables run in a compatible minimal environment without separately
installed shared libraries (for example, a `FROM scratch` container).
Compiler-from-source dependencies do not count against this goal.

**Windows follow-up (not a gate on phase 1.5 or phase 2):** On a clean,
compatible Windows x86-64 machine, downloading only `kiln.exe` is sufficient
to compile supported CLI programs to Windows `.exe` files, and the outputs
run without separately installing development tools, Boehm GC or additional
runtime redistributables. Validate with the same conformance programs in
Windows CI. The C backend can remain available separately where its external
toolchain dependencies are acceptable.

**Progress.** The Cranelift backend, the built-in linker and the bundled
runtime work on Linux x86-64 and are the default there: every conformance
program gives the same output as with the C backend, a build runs with no
`PATH`, and CI runs the executable in an empty root directory (no libc, no
`/proc`). `kiln` itself is built against musl into one static executable
(about 18 MB) that runs on any Linux x86-64 system. CI and every release
run all the conformance programs with it inside an empty `FROM scratch`
container (`scripts/clean-check.sh`), and pushing a `v*` tag publishes it
as a GitHub release. That meets the phase 1.5 goal for Linux x86-64.

For Windows, `kiln build --target windows` makes `.exe` files that use only
`kernel32`, `msvcrt`, `user32` and `ws2_32` (sockets), with a manifest that makes file names,
arguments and the console UTF-8. `kiln.exe` is cross-compiled from Linux
(`x86_64-pc-windows-gnu`) and carries both runtimes, as the Linux `kiln`
does, so each builds for both systems. CI runs every conformance program
with `kiln.exe` on a Windows machine with no C compiler on the `PATH`
(`scripts/windows-check.sh`), and releases ship it next to the Linux
`kiln`.

### What phase 1 leaves for later

Phase 1 is done when native CLI programs compile and run correctly; the
programs in `conformance/` are the evidence. These gaps are known and
deliberate:

- The `sql` and `auth` module stubs type-check but report E0401 when built; their implementation belongs to Phase 5, after Phase 4's browser codegen and RPC.
  Phase 2 has started with `web`'s server and routing; see below.
- Built-in methods cannot have default arguments yet, so `pad_start` and
  `pad_end` take their fill text explicitly.
- `upper` and `lower` change only ASCII letters.
- Maps, sets and functions cannot be map keys or sorted by value.
- Methods cannot be used as values (`ps.map(P.get)`); write a closure.
- The collector is Boehm's conservative one until phase 7.

### Phase 2: the HTTP server

Phase 2 is done when a real HTTP API runs as a native binary, and it is
done: [examples/api](../examples/api/main.kiln) is one, and the `web_*`
conformance programs run as servers on Linux and Windows with both
backends. It was built in steps, each on both backends and both targets:

1. **The server and routing (done).** `web.app()`, routes with typed
   parameters (`/posts/{id: Int}`), `text`, `html`, `redirect` and
   `not_found` responses, request headers, query parameters, cookies and
   bodies, and `listen`. The server is HTTP/1.1, written in the shared
   runtime (`kiln.c`) over the system's sockets (Winsock on Windows). At
   first it served one connection at a time; step 3 changed that. A
   handler that fails or panics gets a 500 response and the error is
   logged; the server keeps running. Requests with no matching route get
   404, and a path that matches with another method gets 405 with `allow`.
2. **JSON (done).** `json.encode`, `json.decode<T>`, `web.json` and
   `req.json<T>()`. The compiler writes a description of each type
   involved (the same `kdesc` the native backend uses for showing and
   comparing, now also written by the C backend), and one encoder and
   decoder in the runtime walk it, so both backends behave the same. A
   body that `req.json` cannot read gets 400 with the reason as JSON.
3. **Concurrency (done).** Each connection runs on a lightweight task, so
   a slow request does not hold up the others, and connections are kept
   alive. Tasks are cooperative and share one core: the scheduler in
   `kiln.c` switches stacks where a task waits for a socket or for
   `time.sleep`, and polls the sockets (`poll`, `WSAPoll` on Windows) when
   no task is ready. Sockets never block. The collector scans the stacks
   of waiting tasks. Running tasks on several cores is a later milestone;
   the runtime's task functions (spawn, wait, sleep) and Kiln's future
   task API are meant to stay the same when it comes, which is why Kiln
   does not promise that tasks can share mutable values (see Memory and
   concurrency).
4. **Errors and settings for real deployments (done).** `web.error(status,
   message)` is a standard `Error` that the server answers with its status
   and message; the runtime marks it with hidden slots after the `Error`'s
   field, so it keeps its status through `try` and `catch`. Other errors
   get 500 and keep their message out of the response. Error responses are
   JSON for clients that speak JSON, or come from `app.on_error`.
   `web.app` takes the limits, the timeout, the keep-alive time and
   whether to keep an access log, each with a default that suits
   production.

### Phase 2.5: VS Code developer experience (done)

Phase 2.5 follows the completed HTTP milestone and precedes phase 3's
frontend work. It improves the editing and feedback loop without changing
Kiln's language or moving Glaze rendering ahead of phase 3.

- Build a dedicated VS Code extension with `.kiln` and `.glaze` language
  registration, highlighting and brand-aligned file icons.
- Use `kiln lsp` as the single source of language intelligence: existing
  diagnostics and quick fixes, plus completion, hover types and signatures,
  go to definition and symbol navigation for Kiln programs.
- Provide VS Code commands for checking, building and running Kiln projects,
  with a configurable compiler path and clear error/output reporting.
- Preserve the LSP's usefulness for other editors. Test unsaved files,
  shared-directory programs, incomplete source and Linux/Windows behavior.
- Defer debugging (including DAP), rename and find references, advanced
  Glaze semantics, formatting, hot reload and broader tooling to separate
  future milestones (with the existing phase 6 scope retained).

**Done when:** A developer can install the extension and comfortably
write, navigate, check, build and run Kiln programs from VS Code without
setting up the LSP manually, while other LSP editors benefit from the
same compiler-backed language features.

For the feature boundaries, implementation stages and validation plan, see
[the phase 2.5 plan](plans/phase-2.5.md).

## Web standard library

`web` (server, router, static files), `template` (glazes), `json`, `form`,
`validate`, `sql` (SQLite first, so local development needs no database
server, then Postgres and MySQL; migrations), `orm`, `admin`, `session`, `auth`
(Argon2, CSRF, rate limiting, JWT, OAuth 2/OIDC), `http.client`, `log`,
`metrics`, `trace`, `cache`, `jobs`, `ws`, `sse`, `mail`, `crypto`, `time`,
`fs`, `os`, `env`, `io`, `text`, `test`.

### ORM

See the [Phase 5 implementation specification](plans/phase-5.md) for the
proposed typed query APIs, dialect handling, schema import, migrations,
validation, security and acceptance gates. Examples in that plan are
**target syntax**, not current executable features.

The `orm` module ships with the standard library, so a database-backed site
needs no third-party package.

- **Models are ordinary structs.** Table and column names are derived
  (`Post` maps to `posts`, `author_id` to a foreign key) and can be
  overridden. Relations are typed fields: `author: User` and
  `comments: List<Comment>` load on request, never implicitly in a loop.
- **Typed queries.** `orm.find<Post>(db).where(.published).order_by(.created).limit(20)`
  is checked at compile time: a misspelled field or a comparison of the
  wrong type is a build error. Raw SQL stays available through `sql.query<T>`.
- **Migrations from models.** `kiln db diff` compares models with the database
  and writes a migration; `kiln db migrate` applies it.
- **Existing databases.** `kiln db pull` reads the schema of an existing
  Postgres, MySQL or SQLite database, including keys, foreign keys,
  indexes, defaults and enums, and generates `.kiln` model files with their
  relations. The generated models are normal code to edit; later pulls
  report drift instead of overwriting changes.

### Admin UI

The `admin` module generates a Django-style admin site from ORM models, a
line per model:

```kiln
let site = admin.site(db, sessions, fn(who, action, model) { who.roles.contains("admin") })
admin.pages<Post>(app, "/admin/posts", site)
```

- List, search, sort, create, edit and delete pages for every model, with
  forms derived from field types.
- Protected by `auth`: nothing is allowed until the program's
  `authorize` function says yes, per action and model.
- **Customizable in layers (planned):** per-model options in code (columns,
  search fields, filters, read-only fields, custom actions), field widgets
  replaced by components, and pages replaced by glazes. The current API
  and its scope are in [the data reference](reference/data.md#admin).

## Glazes and embers

`.glaze` declares a server-only component; `.ember` declares a browser-capable
component that renders initial HTML on the server and hydrates selectively.
Only `.ember` permits reactive state and event handlers. Both use the same
markup grammar. See the [implemented source contract](reference/glazes.md#embers).

Glazes (`.glaze`) are templates compiled to typed functions, with
context-aware escaping, typed props, layouts, components and reverse routing.
Phase 3 implements server-side Glaze rendering without a browser runtime.
Phase 4 adds Embers compiled to WebAssembly (WasmGC) that hydrate selected
components; `server fn` and generated typed RPC belong to that phase,
**without relying on SQL, ORM, sessions or auth**.
See the [Phase 4 implementation plan](plans/phase-4.md).
Phase 5 adds database-backed and authenticated uses of those RPC contracts,
sharing validation across SSR and reactive forms.

### Phase 3: native styling, Finishes and themes

See [the full Phase 3 plan](plans/phase-3.md) for the ordered implementation steps,
exact scope and acceptance tests:

- Glazes may co-locate markup, logic, scoped `<style>` CSS and typed
  `ui` expressions, or import ordinary CSS and CSS Modules.
- Native style values use a built-in `Ui` type, with concise properties
  such as `bg`, `fg`, `p`, `gap` and `rounded`. In Glazes use
  `ui={{ bg: color.surface, p: space.lg }}`; in Kiln functions construct
  the same type with `ui({ ... })`. Styles compile to standard CSS.
- **Finishes** are ordinary functions that return `Ui` or records of
  named `Ui` parts, accept typed parameters/enums and compose through
  deterministic `ui.merge`. No special `finish` keyword or DSL.
- **Themes** provide compatible Finish functions and typed CSS design
  tokens. Switching the selected theme and rebuilding can transform
  colours, typography and layout while keeping Glazes/content unchanged.
- The compiler/LSP will understand style values, tokens, Finish functions,
  CSS Modules and CSS imports. Kiln's docs site is the Phase 3 dogfood
  acceptance target.

These are Phase 3 *targets*, not currently implemented syntax. Embers,
reactive style updates, SQL/auth, Wares and SSG remain later milestones.

### Static site generation

The same app can be built as a static site, for blogs, docs and marketing
pages that need no server. Planned for phase 7:

- `kiln fire --static` renders every route to HTML files, with assets, ready
  for any static host or CDN. Routes with parameters list their pages:
  `app.get("/posts/{id: Int}", show_post).static(fn() { all_post_ids() })`.
- Mixed apps prerender the routes marked `.static(...)` at build time and
  serve the rest dynamically from the same binary, optionally re-rendering a
  static page on a schedule (`.static(..., refresh: 10m)`).
- Embers still work on static pages: they hydrate in the browser as usual.
  Server functions need a server, so a fully static export reports any ember
  that calls one as a build error.

### Installable apps (PWAs)

Planned for phase 8+, after 1.0. Marking an app installable
(`app.pwa(name: "Notes", icon: "icon.png")`) makes `kiln fire` generate the
web manifest, icons at every size and a service worker that caches the app
shell and assets, versioned with each release. A standard library `local`
module gives embers a typed store in the browser (IndexedDB underneath) for
local-first data: changes queue while offline, and a `sync` layer pushes
them to the server and pulls other changes when the connection returns, with
a conflict rule chosen per model (last write wins, merge fields, or a custom
function). The same ORM models describe both sides. Push notifications and
background sync come with it.

## Ecosystem and application delivery

Planned Wares are versioned source packages using the optional
[`kiln.toml` project manifest](#phase-6-project-manifests-and-ware-dependencies)
and generated `kiln.lock`; fetching them should not run arbitrary package
install scripts. Official Wares are intended to be especially auditable and
self-contained. Project templates remain a separate mechanism. A future
Ware registry and Kiln's own documentation website are important
dogfooding projects, not just showcase demos.

The proposed `kiln deploy` and `kiln release` workflows should work with
existing infrastructure defined in `kiln-deploy.toml` rather than attempt
to replace Terraform. They should coordinate builds, migrations, process
activation and health checks with an inspectable plan and explicit
confirmation (with a flag for noninteractive CI); database rollback and
destructive changes need separate safeguards. See
[the full vision](vision.md#8-one-command-builds-releases-and-deployment)
for the intended operating model. These commands are not promises of
current implementation.

## Editor support

`kiln lsp` is a language server (LSP) for any editor. The first version, in
phase 1, reports the same diagnostics as `kiln check` as you type and offers
their suggested fixes as quick fixes; [editors.md](guides/editors.md) shows how to
set it up. Phase 2.5 added completion, hover types and signatures, go to
definition and symbol navigation, plus a VS Code extension with
Kiln/Glaze highlighting, icons and Check/Run/Build commands. These language features stay in the Rust
LSP and benefit other LSP clients. Phase 6 expands refactoring (find
references and rename), deep Glaze prop/component navigation and the rest
of the developer workflow. Debugging remains a separate later milestone.

## Integrated testing (Phase 6)

Kiln's planned `kiln test` command will run **unit**, **integration** and
**real-browser frontend/end-to-end** test suites through one native tooling
experience, using Kiln-authored assertions and shared fixtures. The
compiler/toolchain should own test discovery, isolated app startup,
results, screenshots and CI output. Browser automation should reuse
an established browser/protocol implementation rather than inventing
another browser or requiring a separate Node testing project.

Phase 4 adds real-browser CI smoke and conformance tests for Embers,
including hydration, browser events, reactive state and typed RPC.
Phase 5 extends that coverage with authenticated database-backed RPC and
shared client/server validation.
Phase 6 delivers the public `kiln test` runner for unit, integration and
real-browser tests, including Ember behaviour from the outset.
Browser binaries may need downloading/caching and are not bundled into
Kiln server executables. Detailed scope and acceptance criteria:
[testing.md](plans/testing.md).

## Built for coding agents

- Familiar syntax and one way to do each thing, so model priors transfer.
- Targeted errors for habits from other languages (`null`, `;`, `&&`,
  `function`), each with a machine-applicable fix.
- `kiln check --json` and `kiln explain <code>` from phase 0; later an agent
  guide (`kiln docs --llm`, `llms.txt`), a generated `AGENTS.md` in new
  projects, an MCP server (`kiln mcp`), task examples tested in CI, and an
  agent benchmark that gates releases.

## Moving the runtime off C

The network-facing parts of the runtime (HTTP, JSON, later TLS) are moving
from hand-written C to maintained Rust libraries (hyper, serde_json,
rustls), with hyper running on Kiln's own tasks. JSON and the HTTP server
have moved. The plan, its rules and
its order are in [runtime-rust.md](plans/runtime-rust.md).

The runtime's Rust part is `crates/kiln_rt`, a static library that every
program links next to the C runtime, for both backends and both targets.
It exports C functions, so the backends call it like the rest of the
runtime.

## Settled questions

Questions that were open here are settled in the [decision log](decisions.md),
which records every language decision with its reason.
