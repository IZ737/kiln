# Kiln — long-term vision

> **Write the business logic, not the glue.**

Kiln is an experimental, compiled, garbage-collected, web-first programming
language and integrated application platform. The motivating question is:
**what if building a full-stack application felt as cohesive as writing one
program, while producing fast native executables that are simple to deploy?**

Kiln is not intended to invent new versions of every solved problem. Routing,
forms, validation, SQL, authentication, background jobs, frontend components
and deployments already exist. The opportunity is to make them **fit together
by default**, remove repetitive configuration and duplicated type
definitions, and let developers focus on the parts unique to their
applications.

This document describes the **destination and design principles**, not a
claim that every capability exists today. For implementation status and
milestones see the [README roadmap](../README.md#roadmap); for detailed type,
compiler and API decisions see [design.md](design.md). The concrete Phase 2.5
editor milestone has its [own plan](phase-2.5.md). Syntax and commands labeled
*proposed* below are not guaranteed to work yet. Decisions remain changeable
before 1.0.

## 1. Product identity

Kiln aims to combine:

- **High-level productivity:** familiar, readable syntax and a
  batteries-included application framework, closer to the convenience of
  Django than to assembling a library stack by hand.
- **Native performance:** ahead-of-time compilation and an efficient runtime,
  with Go-like performance as an aspiration to validate through benchmarks,
  not an established guarantee.
- **A coherent full stack:** one type system and set of conventions for
  backend, server-rendered UI, interactive browser components and tooling.
- **Simple distribution:** one downloadable compiler for supported targets,
  and one self-contained executable for a deployed server application.
- **Progressive adoption:** use one Kiln function, module, service, frontend
  component or a whole application; never require a wholesale rewrite.

Kiln is **web-first**, but should remain enjoyable for CLI tools and other
small programs. Desktop apps, games and TUIs are possible future extensions,
not reasons to delay the web platform.

The ambition is not "another language that can build a website." It is a
language that eliminates a substantial amount of the integration work that
normally sits *between* individually capable tools.

## 2. Principles that guide every decision

1. **Convention over configuration; configuration over restriction.** The
   ordinary application follows sensible defaults and needs little setup.
   Every major subsystem should remain inspectable, configurable or
   replaceable where a real application requires it.
2. **One application, not two loosely coupled codebases.** A full-stack Kiln
   app should not feel like separate frontend and backend projects forced to
   communicate through hand-written glue.
3. **Progressive integration.** Kiln can own part of a stack without
   demanding the rest. All-Kiln projects gain deeper compiler-assisted
   integration, but mixed-language projects remain first-class.
4. **Low complexity floor, high performance ceiling.** Start with ordinary
   GC-managed code and high-level APIs. Only optimize particular hotspots
   when measurement justifies it; do not impose systems-language concepts on
   routine web development.
5. **No artificial abstractions.** If raw HTML, CSS, HTTP handlers, SQL or
   manual state management is more suitable, make it possible. Defaults
   should save work rather than hide important behaviour.
6. **Integrate before reinventing.** Prefer interoperating with trustworthy
   existing implementations and ecosystems over rebuilding every library,
   cloud provider or developer tool.
7. **Compiler-backed confidence.** Share definitions across boundaries and
   catch mismatches as early as possible. Diagnostics should be useful to
   humans and machine-applicable by coding agents.
8. **Safe and observable by default.** Secure defaults, predictable errors,
   explicit consequential actions, testable builds and visible deployment
   plans matter more than a short demo.
9. **One maintained way to do ordinary tasks.** Avoid multiplying tools,
   configuration languages and competing conventions without a concrete
   developer benefit.

The standard for success is **less glue code and less cognitive overhead**,
not simply more items in the standard library.

## 3. Backend: the batteries are actually included

The intended standard experience includes typed HTTP handlers and routing,
request/response handling, JSON, error propagation, logging, tracing,
metrics, configuration, filesystem and time utilities, and a lightweight
concurrent runtime. A small API should work without assembling a framework.

### Data, forms and authentication

- **SQL and ORM:** SQLite for a zero-service local start, with PostgreSQL and
  MySQL as application needs grow. Ordinary Kiln structs describe models;
  typed query composition catches field and type mistakes, and raw SQL
  remains available.
- **Existing databases:** `kiln db pull` can import an existing schema into
  editable typed models, not force a new database or a greenfield schema.
- **Migrations:** `kiln db diff` generates changes for review; `kiln db
  migrate` applies explicit migrations. Production deployment should not
  invent an unreviewed migration from current model definitions.
- **Forms and validation:** one schema/type definition should inform server
  validation, input constraints, field errors and frontend form generation
  rather than duplicating each rule in several packages.
- **Auth and sessions:** built-in paths for sign-in, session management,
  permissions, CSRF protection, password security and common identity
  integrations, while permitting specialized policies and external providers.
- **Generated admin:** a useful, secured CRUD administration interface
  derived from models, customizable from individual fields through whole
  Glaze pages, not an uneditable generated site.

### Durable background work

The job system is a core example of eliminating glue: a developer should not
need to introduce and configure an entirely separate Celery-like ecosystem
just to perform routine asynchronous or scheduled tasks.

The direction is an integrated `jobs` API with retries, schedules,
observability and durable storage. SQLite can serve small installations,
with a PostgreSQL-backed option for larger systems and a separate
`./app worker` process when scaling. Transactional scheduling and
outbox-style correctness matter: saying a job was queued should mean the
system can recover it after process failure. Exact queue design remains a
future implementation decision.

The same compiled application can expose server, worker and migration entry
points without needing Kiln installed on the destination host.

## 4. Frontend DX is equally important

Frontend developers repeatedly rebuild forms, validation feedback, data
fetching, loading/error states, authentication screens, interactivity,
styling conventions and the client/server transport layer. Kiln should
remove as much *frontend* integration work as it removes from the backend.

### Glazes: server-first components without a separate UI language

Glazes (`.glaze`) are typed, HTML-oriented templates and components. The
planned frontend experience includes layouts, slots, reusable components,
scoped styles, design tokens, context-aware escaping and server rendering
by default. Phase 3 adds native typed `Ui` records with concise
Tailwind-inspired keys (`bg`, `fg`, `p`, etc.), co-located `<style>`,
ordinary CSS/CSS Modules and **Finishes**: ordinary functions returning
reusable styling values or multi-part records. Compatible theme contracts
can radically restyle the same content and Glazes, including layout.
See the [Phase 3 implementation plan](phase-3.md) for the agreed scope,
API sketches and acceptance gates.

- **CSS-first:** full native CSS must always work. Provide helpful design
  tokens, responsive defaults and optionally attractive, accessible
  component primitives, not a mandatory proprietary styling dialect.
- **Accessible defaults:** forms, controls, focus/keyboard interactions and
  validation errors should have sound defaults while remaining overridable.
- **Code-defined routes:** routing should remain expressible explicitly in
  application code; a mandatory file-based routing convention is not the
  goal. Glaze files define views/components rather than automatically
  claiming URLs.
- **Typed forms:** a field, validation rule and server operation should not
  require independent descriptions in the database, HTTP API and browser
  client. Generate the usual binding, submission and error-handling
  machinery, with override points for nonstandard behaviour.
- **Progressive frontend use:** a project should remain free to serve
  ordinary HTML, CSS and JavaScript; adopting a Glaze should not require
  converting the whole frontend.

### Embers: interactivity when and where needed

Interactive Glazes become **Embers**, eventually compiled to browser
WebAssembly using WasmGC. A static or mostly server-rendered page should
not pay for a full client application just because one control is
interactive.

The intended experience includes reactive state, lifecycle/event handling,
typed server functions, generated client/server RPC and ergonomic error
handling. Caching, invalidation, optimistic updates and live updates
(SSE/WebSockets) should have good defaults where the compiler can justify
them, but observable and explicit overrides. Avoid hidden invalidation
behaviour that is impossible to inspect or debug.

The key promise is that **a Kiln server function and the UI calling it share
one type contract**, without hand-authoring route wrappers, fetch boilerplate,
duplicate DTOs or serialization code.

### Static and offline variants

The same component system should support SSR, selective interactivity and,
later, `kiln fire --static` for static-site generation or hybrid
prerendering. Much later, optional PWA/local-first capabilities could reuse
typed models for offline data and synchronization. These are distinct
roadmap milestones, not required for the first useful Glaze application.

## 5. Performance without leaving the language

Most applications should work well with ordinary managed Kiln values.
The developer who discovers one unusually CPU- or allocation-intensive
function should be able to optimize *that function* rather than extract it
into a Rust/C service with a new build process and foreign-language glue.

The planned performance ladder is:

1. **Simple default:** GC-managed objects and readable code, compiled to
   native instructions with no manual ownership bookkeeping.
2. **Compiler optimization:** improved code generation, generic
   specialization, escape analysis and LLVM-backed optimized release builds.
   Profile and benchmark rather than assume a particular speedup.
3. **Targeted low-level tools:** optional SIMD/vectorized operations,
   efficient data layouts and allocation-aware APIs where they materially
   help computation or memory locality.
4. **Opt-in alternative allocation:** scoped arenas/regions or request-scoped
   allocation for suitable temporary data, with the compiler enforcing
   non-escaping references or explicit promotion/copying where necessary.
5. **External escape hatch:** foreign-function interfaces if a specialized
   existing implementation is still the right tool.

These mechanisms solve different problems. Arenas can reduce GC pressure but
do not inherently speed up CPU-bound arithmetic; SIMD does not help a
database-bound endpoint. The optimization path must remain empirical.

**Garbage collection remains the default.** The roadmap currently uses
Boehm, with a planned future precise GC. Opt-in memory control is **not** a
proposal to make ownership types or manual frees mandatory for ordinary
Kiln applications, nor a promise to match every systems-language feature.

Where practical, improved internals should leave a function's public Kiln
signature—and the code calling it—unchanged.

## 6. Interoperability in both directions is an adoption strategy

A new language cannot expect developers to discard existing code or wait
for a duplicate of every popular ecosystem library. Interoperability is
therefore a product feature, not a grudging escape hatch.

### Kiln consumes existing libraries and services

- **Native interop:** planned C-ABI FFI and generated or declared bindings
  for C libraries, with Rust usable through a stable C-compatible bridge.
  Python/Node libraries with native wrappers may be reachable where the
  runtime and ABI permit.
- **Service interop:** HTTP, typed API contracts and potential OpenAPI
  generation/import let Kiln coexist with existing Django, Node, Go and
  other services.
- **Portable components:** explore the WebAssembly Component Model and WIT
  for typed cross-language components where runtime support makes sense.

### Existing applications consume Kiln

Kiln should also be able to export callable native functionality and
generate wrappers or headers so an existing Rust, C, Python or Node
application can adopt **one Kiln component** without rewriting the rest.
A performance-sensitive feature can be introduced incrementally.

Cross-language work must specify ownership, allocation, GC roots,
strings/collections, errors, callbacks, threading and version compatibility.
A C ABI alone does not make arbitrary Python or Rust types seamlessly
interchangeable. Keep these boundaries explicit and testable.

### JavaScript and npm for browser Embers

The Phase 6 frontend plan includes JS interoperability so Kiln programs
can reuse the massive existing browser/npm ecosystem instead of
reimplementing it. The preferred direction is:

- Generate type-safe bindings from TypeScript declarations when mappings are
  clear; allow explicit bindings for unusual APIs.
- Bridge promises, events, callbacks, object handles and JS exceptions
  predictably across Wasm/JS.
- Bundle browser dependencies as part of Kiln's normal build workflow,
  without requiring every Kiln author to become a bundler expert.
- Support framework-independent browser libraries first; React-specific
  components and Node-only modules are not automatically compatible.

**"Access to npm" does not mean "every npm package works everywhere."**
Interop should be honest about browser/runtime constraints and give useful
diagnostics when an API cannot be mapped.

The adoption path is intentionally incremental: **try one Kiln module,
service or component → keep existing dependencies → choose Kiln more broadly
if its integrated developer experience proves better.**

## 7. Wares, templates and a real ecosystem

**Wares** are source packages, managed through a coherent manifest/lockfile
and eventually an official registry. The initial implementation can support
local/Git dependencies and reproducible immutable versions before a
full registry exists.

- The compiler/toolchain handles package compilation; avoid npm-style
  arbitrary `postinstall` scripts and surprising code execution merely
  from fetching a dependency.
- Package provenance, locked versions, dependency auditing and transparent
  capability/effect reporting should help developers understand what they
  are introducing. Compile-time reporting is **not** a claim of OS-level
  sandboxing.
- Official Kiln-maintained Wares should be especially small, auditable and
  preferably self-contained without third-party transitive dependencies.
  Powerful permissions or extension hooks should be transparent and require
  explicit user approval.
- **Project templates are different from Wares:** `kiln new --template`
  could instantiate files/variables from trusted templates without executing
  arbitrary initialization code.

Possible optional official Wares include game development
(`@kiln/game`), desktop apps and perhaps TUIs. CLI programming itself
should remain straightforward through the standard library. Some future
non-web UI targets might reuse Glaze/Ember concepts, but web development is
the core priority, and target-specific UI differences must stay explicit.

A future official Ware registry should itself be a Kiln application; it is
both useful infrastructure and a substantial dogfooding target. A minimal
static registry or Git-backed index may come first.

## 8. One-command builds, releases and deployment

### Distribution and developer setup

The normal supported-platform experience should be to install one compiler
binary, use the same CLI for checking/building/running, and upgrade it
without separately installing Rust, a C compiler, a linker or a GC. The
compiler may depend on Rust and other tools **when built from source**.
The produced executable includes its necessary runtime/GC and does not
require Kiln on the deployment host.

On Linux, a genuinely statically linked server binary should fit in a
minimal `FROM scratch` container, subject to normal application resources
such as trusted CA certificates, storage and time-zone data. Windows has
its own platform linking constraints. The C backend remains useful as an
alternative; Cranelift is the current native backend, with LLVM intended
for later optimized releases.

### Build, deploy and release are separate operations

The proposed roles are:

| Command | Responsibility | Status |
| --- | --- | --- |
| `kiln upgrade` | Update the installed compiler through checksum-verified releases | Implemented; see the [README install instructions](../README.md#install) |
| `kiln fire` | Produce an optimized release artifact | Planned |
| `kiln deploy <target>` | Deliver and activate an artifact on configured infrastructure | Proposed |
| `kiln release <target>` | Run checks/tests, build, prepare, confirm, deploy and verify | Proposed |
| `kiln deploy <target> --dry-run` | Explain intended changes without applying them | Proposed |

The update/install workflow was developed ahead of Phase 2.5. The
future commands and spellings are illustrative until implemented;
the [roadmap](../README.md#roadmap) remains authoritative for scheduling.

### Application deployment, not another Terraform

The preferred model is an explicit `kiln-deploy.toml` describing **existing
infrastructure**, deployment targets and overrides, rather than Kiln owning
the entire cloud resource lifecycle. Targets might include an existing VPS
over SSH, containers or adapter-backed hosts. Existing systemd units,
reverse proxies and infrastructure-as-code tools should remain usable.
Provider integrations may live outside the compiler.

A deployment may coordinate a server process, background workers, assets,
health checks, versioned releases and **database migrations**. It should:

- Generate an inspectable plan and ask for confirmation before modifying a
  deployed environment.
- Require an explicit `--yes` (or equivalent) in non-interactive CI rather
  than treating an absent prompt as consent. Skipping a prompt does **not**
  skip tests, integrity verification or safety checks.
- Treat potentially destructive database changes as a distinct high-risk
  operation, with explicit approval and appropriate backup/compatibility
  policy.
- Apply previously reviewed migrations with locking and appropriate
  ordering for the deployment strategy; never claim that rolling back an
  executable automatically reverses a destructive schema change.
- Be explicit about ownership of existing infrastructure, secrets,
  credentials, downtime expectations, worker draining and rollback.
- Make common deployments one command, with transparent low-level control
  for applications that need it.

A future AI agent may inspect a release plan through structured tooling, but
must not silently deploy or authorize destructive operations.

## 9. First-class human and AI developer experience

The developer experience starts before the code is running: install,
editor support, compiler messages, tests, a fast feedback loop, docs,
debugging and deployment should feel like parts of one toolchain.

- **Editors:** the basic LSP is implemented. Phase 2.5 adds a dedicated
  VS Code extension, icons, syntax highlighting and enhanced compiler-backed
  completion/navigation; intelligence belongs in the shared LSP so other
  editors benefit.
- **Testing and formatting:** a consistent test runner, formatter, hot
  reload and richer editor refactorings are planned for Phase 5. A real
  application should be easy to check locally and in CI.
- **Agent-friendly contracts:** structured JSON diagnostics, stable error
  codes, machine-applicable fixes, generated docs/examples and explicit
  tool schemas minimize guesswork for coding agents.
- **Compiler/tooling MCP:** `kiln mcp` could expose check, build, test,
  introspection, generated documentation and eventually debugging as
  structured tools. MCP is a convenience interface to the same semantics,
  not an alternative compiler.
- **Application MCP:** separately, future Kiln standard-library support
  could let applications implement MCP servers/clients, with Kiln types
  supplying tool input/output schemas. This is not the same as the
  compiler's own MCP server.
- **Debugger (later, not Phase 2.5):** a future debugger should allow
  ordinary human workflows via CLI/editor/DAP and structured inspection for
  AI agents via MCP: frames, variables, typed values, step/breakpoints and
  perhaps snapshots/diffs. It must be designed against the real backends,
  optimized-code limitations and source mappings.

The goal is **better outcomes per developer hour**, not maximal automation
with hidden behavior. AI-generated code still needs conformance coverage,
reviewable changes and sound engineering decisions.

## 10. Dogfooding, credibility and adoption

Kiln began as a personal hobby and learning project. Keeping it fun, with
no artificial public deadlines or obligations before it is ready, is
important. At the same time, sustained real applications are essential if
Kiln is eventually to earn trust beyond its creator.

A sensible progression:

1. Finish the core language/tooling milestones, then build real small
   applications, not only compiler probes.
2. Build the Kiln **documentation website in Kiln** as the Phase 3
   acceptance target.
3. Build increasingly demanding internal services, such as the future
   **Ware registry in Kiln**, to exercise HTTP, persistence, auth, admin,
   package APIs, runtime behavior and deployment.
4. Eventually use Kiln for a personal site or useful production project,
   where its reliability can be measured over time.
5. Publish clear examples, honest performance measurements and
   side-by-side integration-code comparisons with mainstream stacks; invite
   outside adoption only when documentation, security and compatibility
   are credible.

Using Kiln only for a backend, a single embedded module or a frontend
component must remain a valid adoption path. Broad adoption is an
**aspiration**, not a success condition for early milestones or a reason to
rush a 1.0 release.

The project also demonstrates compiler architecture, product design,
testing and AI-directed engineering. Those benefits stand even if uptake
is modest.

## 11. Scope boundaries and decision status

### Established direction

- Web-first, convention-driven, tightly integrated but deeply configurable.
- Garbage-collected and native by default; no mandatory manual memory model.
- Glazes plus future Embers, with native CSS support and progressive adoption.
- Wares as extension packages, with provenance and disciplined installs.
- Strong tooling, agent accessibility and real-project dogfooding.
- Full-stack Kiln should reduce hand-written frontend/backend boundaries.
- Both **Kiln calling external code** and **external code calling Kiln**
  matter for eventual adoption.

### Designed but not yet implemented end to end

Integrated ORM/auth/admin/jobs, Glaze rendering, Embers on WasmGC, generated
RPC, broad npm interop, full Wares/registry, `kiln fire` release workflow,
`kiln deploy`/`kiln release`, optimized LLVM builds, and much of the planned
developer tooling. Existing parser/type-checker support for a construct is
not the same as a working native runtime implementation; consult current
[stdlib status](stdlib.md) and tests.

### Later explorations, not fixed promises

Opt-in arenas and explicit allocation controls, SIMD/data-layout APIs,
general bidirectional FFI/binding generation, Wasm Component Model/WIT,
fully featured MCP application support, AI-assisted debugger snapshots,
PWA offline synchronization, cross-target Glaze/Ember reuse in desktop,
TUI and games, and additional platforms. These require design,
compatibility and maintenance trade-off reviews.

### Deliberate non-goals

- Reinventing all infrastructure provisioning as a Kiln-owned Terraform.
- Recreating the entire npm/Python/Rust ecosystem in Kiln.
- Requiring use of every Kiln layer to benefit from any one layer.
- Forcing unusual applications through fixed templates or styling systems.
- Making low-level ownership complexity mandatory just to write a CRUD app.
- Sacrificing correctness, transparency or long-term maintainability merely
  to shorten demonstrations or hit arbitrary dates.

## 12. How to judge progress

The [phased roadmap](../README.md#roadmap) defines executable outcomes:
CLI programs, a real HTTP API, the Kiln docs site, an authenticated
database-backed app, an ergonomic development workflow, an interactive
full-stack app, and eventually Kiln 1.0 ready for personal production use.

Across those milestones, ask:

- How much application-specific **business logic** was written relative to
  adapters, configuration and repeated declarations?
- Can a developer see, test and override Kiln's generated decisions?
- Can Kiln coexist with an existing system without a rewrite?
- Does an optimized hotspot remain part of the same application and type
  system?
- Do native outputs behave correctly and predictably on clean machines?
- Can a newcomer and a coding agent diagnose problems without guesswork?
- Does real dogfooding reveal useful simplifications rather than new hidden
  complexity?

**Kiln does not have to reinvent what developers can do. It should
dramatically reduce how much work they have to do to accomplish it.**
