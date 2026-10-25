# Plan: less C in the runtime

**Status: proposal, not started.** Nothing here is implemented. It needs a
decision (see [The decision](#the-decision-hyper-and-kilns-tasks)) before any code moves.

## The problem

Every Kiln program carries a runtime written in C: `kiln.c` (about 3,400
lines, shared by both backends) and `native.c`. Most of it is ordinary
value code (strings, lists, maps, numbers, time). But two parts read bytes
straight from the network:

- **HTTP** (`web`, about 900 lines): the request line and header parser,
  body framing, keep-alive, percent-decoding, query strings and cookies.
- **JSON** (`json`, about 600 lines): the decoder that `req.json<T>()`
  runs on request bodies.

Hand-written parsers of untrusted input, in C, are where small projects get
security bugs: buffer overruns, request smuggling through conflicting
`content-length` and `transfer-encoding`, integer overflow in sizes, deep
nesting. Later milestones add more of the same: TLS, an HTTP client, form
bodies, multipart uploads, cookies with signing.

Kiln's vision already says *integrate before reinventing* (principle 6).
This plan applies it to the runtime: hand the network-facing parts to
widely used, fuzzed, maintained libraries, behind small interfaces so each
one can be swapped later.

## The proposal in one paragraph

Add a Rust crate, `crates/kiln_rt`, compiled as a static library and linked
into every Kiln program next to what is there today. It exports `k_*`
functions with the C ABI, so generated code (Cranelift or C) calls it
exactly as it calls `kiln.c`, and `gen.rs` does not change. It builds
HTTP on [`hyper`](https://hyper.rs) (HTTP/1.1 server, later client), JSON on
[`serde_json`](https://github.com/serde-rs/json), and later TLS on
[`rustls`](https://github.com/rustls/rustls). Hyper runs **on Kiln's own
tasks**, not on tokio. The Boehm collector, the task switch and the core
value code stay in C for now. Users still download one `kiln` and need no C
compiler, Rust or anything else.

## The decision: hyper and Kiln's tasks

Hyper is the obvious HTTP library, but most Rust code runs it on tokio,
and Kiln already has a scheduler: cooperative, single-core, stackful tasks
in `kiln.c`, where a handler is ordinary straight-line code that may wait
(for a socket, `time.sleep`, later a database) and switch stacks in the
middle. Four ways to fit them together:

### A. Hyper on Kiln's tasks (recommended)

Hyper 1.x does not depend on tokio. It asks for an I/O type that implements
its `Read` and `Write` traits, and optionally a `Timer`. HTTP/1 connections
need no executor at all.

Each connection still gets a Kiln task, as now. On it, the runtime drives
`hyper::server::conn::http1::Builder::serve_connection` with a tiny
`block_on`:

1. Poll the connection future.
2. If it is pending, the socket wrapper has recorded what it was waiting
   for (readable or writable) and the timer wrapper any deadline. Call
   `k_task_wait(fd, write, deadline)`, which parks this task and runs the
   others, exactly as the C server does today.
3. When the task wakes, poll again.

The waker does nothing: the only things that can make a connection ready
are the ones `k_task_wait` already watches. Hyper's service function
collects the body (within `max_body`), builds the `Request` slot array in
GC memory, calls the Kiln handler **synchronously** through a small C
shim, and copies the `Response` out. The handler runs on the same task
stack, so it can wait and switch tasks in the middle of the poll: the
future is pinned on that stack and nothing else touches it meanwhile.

Why this one: one scheduler, the stackful-task model and its future
multicore path stay as decided for phase 2, handlers stay plain code, and
hyper is in charge of everything that touches raw bytes (parsing, chunked
encoding, framing, keep-alive, `100-continue`, header limits and read
timeouts). It is the least code to write that still removes the parser.

Costs: an adapter of perhaps 300 lines of Rust (socket wrapper, timer,
`block_on`, service shim), and the rules in [GC and panics](#gc-and-panics).
`k_task_wait` may need to wait for read *or* write at once if hyper ever
asks for both; HTTP/1 serving should not.

### B. Parsing libraries only

Use `httparse` (the parser inside hyper) for request heads and
`serde_json` for JSON, and rewrite the connection loop (body framing,
chunked decoding, keep-alive, limits) by hand in Rust.

Smaller and simpler to wire up, and memory-safe. But the framing logic is
where request smuggling lives, and it stays ours to get right. B is a
reasonable fallback if A's adapter turns out awkward.

### C. Tokio as the event loop

Run a tokio current-thread runtime as the reactor and make each Kiln task a
stackful coroutine that tokio resumes (for example with `corosensei`).

This would give the whole tokio ecosystem (hyper-util, tower, database
drivers) without adapters, and tokio's multi-threaded scheduler is one
possible multicore path. But it replaces the scheduler phase 2 just built,
adds a second model of concurrency to reason about, adds most of a
megabyte to every program, and needs coroutine switching inside tokio
tasks to cooperate with Boehm's stack scanning. Too much change for the
problem; worth revisiting when multicore is planned.

### D. Stay in C with C libraries

Use [`llhttp`](https://github.com/nodejs/llhttp) (Node's parser) and
[`yyjson`](https://github.com/ibireme/yyjson), and later mbedTLS.

The fewest build changes, and these are battle-tested. But the connection
handling around the parser is still hand-written C, it does nothing about C
being the language new runtime code is written in, and the compiler is
already a Rust project, so Rust libraries fit the repo better.

## What stays in C

- **The Boehm collector.** It is C, vendored, and long-lived; a precise GC
  is a phase 7 item and would replace it then, not now.
- **The task switch** (the assembly on x86-64 ELF, fibers on Windows) and
  the scheduler around it. It is small, does not parse input, and Boehm's
  stack hooks are C. It could move to Rust later with `naked` functions;
  there is no hurry.
- **Core value code**: strings, lists, maps, numbers, `show`/`eq`/`hash`,
  `k_desc_*`. It handles Kiln values, not network bytes, and moving it is
  a lot of churn for little safety gain.

The rule from then on: **new runtime code goes in Rust**, and C code moves
when it starts handling untrusted input (for example RFC 3339 parsing if it
gets exposed to request data) or when it needs touching anyway.

## Interfaces that keep each library replaceable

- **Generated code ↔ runtime**: unchanged `k_*` C functions, listed in
  `kiln_native/src/runtime.rs` and in a header for the C backend. Which
  language implements a function is invisible to both backends.
- **Inside `kiln_rt`**: one module per concern, each with one file that
  touches the library.
  - `web/http1.rs` is the only file that names hyper. The rest of `web`
    (routing, building `Request`, error responses, the access log) works on
    `http::Request` and `http::Response`, the shared types crate that hyper,
    axum and others all use.
  - `json/serde.rs` is the only file that names serde_json. The encoder and
    decoder walk a `kdesc` (as today) and talk to a small "write a JSON
    value / read the next token" interface. Decoding uses
    `DeserializeSeed` driven by the `kdesc`, so there is no intermediate
    `serde_json::Value`.
  - Later, `tls/` behind a `TlsAcceptor`-shaped function, and `http`
    client behind `http.client`'s API.

Swapping a library means rewriting one file, not the runtime.

## GC and panics

Rust code inside a Boehm-collected program needs three rules:

1. **Boehm does not scan Rust's heap.** It scans task stacks, registers and
   static data conservatively, but a `Vec` or `Box` allocated by Rust lives
   in `malloc` memory it never looks at. A Kiln pointer that is only held
   there can be collected. So `kiln_rt` never stores a Kiln value in Rust
   heap memory across an allocation: it builds lists and strings directly
   in GC memory through runtime calls (as the JSON decoder must), and
   copies a handler's response out into Rust bytes before going on. A
   debug build can enforce this with a wrapper type that is only valid on
   the stack.
2. **No `longjmp` across Rust frames.** A Kiln panic in a handler jumps to
   `k_panic_jmp` with `__builtin_longjmp`, which skips any frames in
   between. The panic trap is therefore set in the C shim that calls the
   handler, below every Rust frame, so the jump never crosses Rust code.
   The per-task state already saved in `k_task_switch` covers the rest.
3. **Rust panics abort.** `kiln_rt` builds with `panic = "abort"`; a Rust
   panic is a runtime bug, prints a message and exits, and never unwinds
   into C or Cranelift frames.

## Builds, targets and size

**Users** see no change: `kiln` still carries a prebuilt runtime per target,
and programs are still single static executables. The phase 1.5 guarantee
(no C compiler, linker, Rust or GC to install) holds, and `clean-check.sh`
and `windows-check.sh` keep proving it.

**Building `kiln` from source** gains a step. `kiln_native/build.rs`
already compiles the C runtime per target; it would also run
`cargo build -p kiln_rt --release --target <t>` with a separate target
directory (so the nested build does not wait on the outer build's lock):

- Linux: `x86_64-unknown-linux-musl`. The staticlib links with the existing
  objects through wild. Rust's std for that target ships its own musl
  `libc.a` and `libunwind.a`; we should link against Rust's copy rather
  than the system one so versions match, which may also make `musl-tools`
  needed only for compiling the C files.
- Windows: `x86_64-pc-windows-gnu`. The staticlib joins `native.o` and
  `gc.o` in the mingw link of `runtime.exe`. Rust's std adds imports of
  `ntdll`, `bcrypt` and `userenv`, all system DLLs, which phase 1.5 allows.
- The C backend: the host target, linked next to `-lgc`.

`rust-toolchain.toml` lists the two targets, so rustup installs them
automatically. CI already builds release binaries for both.

**Size.** A Rust staticlib contains all of std and is tens of megabytes
before linking, which would bloat `kiln`, since it embeds the runtime for
both targets. So build.rs links it once into a single relocatable object
with only the `k_*` symbols exported and unused sections dropped (`ld -r`
plus `--gc-sections`, or the same through wild), and embeds that. Windows
already embeds a linked executable, so only used code is there.

Rough estimates for what each program grows by, to be measured in step 0:
std's used parts and hyper's HTTP/1 server, a few hundred KB; serde_json,
around 100 KB; rustls with the `ring` provider, 1 to 2 MB when TLS
arrives. `kiln` grows by about twice the per-program cost.

For TLS the `ring` provider is preferred over `aws-lc-rs`, which needs
CMake and is harder to cross-compile for musl and mingw. HTTPS clients use
bundled roots (`webpki-roots`) so programs need no system certificate
store.

## Effect on the backends

- **Cranelift**: none beyond `runtime.rs` listing any new or changed
  signatures. Calls stay C ABI.
- **C backend**: still prepends the C part of the runtime, and now also
  links `libkiln_rt.a` for the host. It remains the reference backend, and
  conformance still runs every program through both.
- **Behaviour**: error responses, limits and the access log must not
  change. The `web_*` conformance programs and `tests/server.rs` are the
  check. Hyper's responses differ in small ways (header case, `date`
  header); the conformance harness may need to ignore those.

## Supply chain

Depending on libraries is only safer if they are kept up to date:

- `cargo-deny` in CI for RustSec advisories and licences (all candidates
  are MIT or Apache-2.0).
- Dependabot for Cargo, so security releases arrive as PRs.
- Features kept minimal (`hyper` with `http1` and `server` only, no
  `tokio`; `serde_json` without `arbitrary_precision`).
- Exact versions in `Cargo.lock`, which is already checked in.

## Migration order

Each step is one PR, on both backends and both targets, with the C code it
replaces deleted in the same PR.

0. **Groundwork.** `crates/kiln_rt`, the nested build, prelinking, the
   header for the C backend, `cargo-deny` and Dependabot. Move one trivial
   function (`k_web_reason`) to prove the path through clean-check and
   wine. Measure sizes.
1. **JSON on serde_json.** No I/O and no scheduler involvement, so it
   proves the GC rules on their own. Add nesting-depth and size tests.
2. **HTTP on hyper.** The adapter from option A, request data (percent
   decoding, query strings and cookies on the `percent-encoding`,
   `form_urlencoded` and `cookie` crates), and the limits and timeouts
   mapped to hyper's settings. Delete the C parser and connection loop.
3. **Later, with the features that need them**: `req.form` (phase 3 or 4)
   on `form_urlencoded`, multipart on `multer`, the HTTP client and TLS on
   hyper and rustls, signed cookies for sessions (phase 4).

Steps 0 to 2 fit before phase 3, which adds form handling and would
otherwise add more C parsing.

## Risks

- **Build complexity.** A nested cargo build and two extra rustup targets.
  If that proves fragile, a fallback is a script that builds `kiln_rt`
  first and points build.rs at the output.
- **The adapter is ours.** Option A's `block_on` and socket wrapper are new
  code, but small, and they handle readiness, not bytes.
- **Hyper's defaults** (for example its idle connection behaviour) may not
  match the current server exactly; the conformance tests find that.
- **Binary size** grows; prelinking keeps it in check, and step 0 measures
  it before anything else depends on it.
