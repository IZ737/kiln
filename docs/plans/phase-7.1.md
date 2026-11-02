# Phase 7.1 — LLVM release builds

**Status:** Done in 0.17.0. Phase 7 (Kiln 1.0 readiness) is delivered in
reviewable slices; this is the first. It makes `kiln fire` the optimized
release build the roadmap promises, and leaves the precise garbage
collector, hardening and the 1.0 stability policy to later slices.

## Goal

`kiln fire` builds a faster executable than `kiln build`, by sending the
program through LLVM's optimizer, while `kiln` stays one self-contained
binary and both builds behave exactly the same. Today `kiln fire` builds
what `kiln build` builds: Cranelift's machine code, linked by the linker
inside `kiln` with the runtime it carries. Cranelift is quick and needs
no outside tools, which is what a development build wants, but it does no
inlining and little else that a release build wants.

## Scope

1. **An LLVM backend** (`crates/kiln_native/src/llvm.rs`): a third code
   generator from the same typed IR, next to the C backend
   (`kiln_codegen`) and the Cranelift one (`gen.rs` beside it, whose
   runtime signature table and linker it shares). It writes LLVM IR as
   text (`.ll`), the way the C backend writes C, with the same value
   representation, the same runtime calls and the same type descriptions
   as the Cranelift backend, so one runtime serves all three.
2. **Compiling through clang:** `kiln` does not link LLVM in. It runs a
   `clang` it finds (see Decisions) as `clang -O2 -c` on the `.ll`, for
   the target's triple, and gets an object file back. That object goes
   through the same link as a Cranelift object: the built-in wild linker
   with the musl runtime for Linux, and `pe.rs` with the prelinked
   runtime executable for Windows. So `kiln fire --target windows` works
   from Linux as `kiln build` does, and the result needs no libraries.
3. **`kiln fire`:** uses the LLVM backend when it finds clang, otherwise
   builds with Cranelift and says so in one line, with how to get clang.
   `--backend llvm` on `kiln build`, `kiln run` and `kiln fire` asks for
   it (and fails when there is no clang); `--backend native` on `kiln
   fire` keeps Cranelift. `--emit-llvm out.ll` writes the IR, as
   `--emit-c` and `--emit-ir` do.
4. **The same behaviour:** every conformance program also runs through
   the LLVM backend when clang is installed, as do the I/O, database and
   browser tests that run each backend. E0401 is reported for exactly
   what the Cranelift backend reports it for.
5. **Documentation:** the `kiln fire` reference, the tooling guide, the
   README's command table and `docs/design.md`'s backend table.

## Decisions

- **LLVM is not linked into `kiln`.** Linking LLVM in (through
  `inkwell`/`llvm-sys`) would add about a hundred megabytes to the one
  binary the installers ship, and `kiln` is built statically for musl and
  cross-built for Windows in CI, which LLVM's libraries make much harder.
  Writing `.ll` text and running clang keeps `kiln` small and its build
  simple, and clang is a one-line install everywhere a release is built
  (`apt install clang`, `brew install llvm`, the LLVM Windows installer).
  Phase 1.5's rule stands: `kiln build` and `kiln run` need nothing but
  `kiln`; the optimizer is the one optional tool, and it is only for
  releases.
- **Finding clang:** `KILN_CLANG` when it is set (the path of a clang),
  otherwise the first of `clang`, `clang-21` down to `clang-15` on `PATH`.
  Clang 15 or later is needed: the IR uses opaque pointers. An older or
  broken clang is reported with its version and what is needed.
  Downloading a toolchain into the per-user cache, as Wares are cached, is
  a possible later addition; the plan does not depend on it.
- **Optimization level:** `-O2`. Nothing else is tuned for the host
  (`-march` stays the baseline x86-64), so a release built on one machine
  runs on any other. Integer overflow, division and the other checks the
  language makes are kept: LLVM's `sadd.with.overflow` and friends
  replace Cranelift's, and the runtime's division keeps panicking.
- **One representation, three backends.** Values, slots, closures, type
  descriptions (`k_desc_*`) and the error flag protocol (`k_failed`,
  `k_lib_error`) are those of `runtime/native.c` and `kiln.c`. The LLVM
  backend mirrors `kiln_native/src/gen.rs` function for function, so a
  feature the compiler learns goes into three generators (`AGENTS.md`
  says so), and the conformance tests check all three agree. Every local
  is an `alloca` that LLVM's `mem2reg` turns into registers, which keeps
  the generator simple and leaves the optimizer its usual input.
- **Cheap runtime arithmetic is written out.** The runtime is prebuilt,
  so LLVM cannot inline its functions. `Int` division, remainder,
  negation, `abs`, `min` and `max` are therefore emitted as IR with the
  same checks (`k_div`'s zero and overflow panics, `x % -1 == 0`), which
  is what makes a loop over `i % 7` three times faster than Cranelift's.
  Anything larger stays a call.
- **The garbage collector stayed Boehm** in this slice. It scanned the
  stack and registers conservatively, so optimized code needed no stack
  maps. The precise collector that followed
  ([plan](precise-gc.md)) uses a shadow stack, and the LLVM backend
  emits the same frame records, slot stores and polls as the Cranelift
  one.
- **Not a new ABI:** every function keeps the C calling convention and
  the same symbol names (`kiln_init`, `kiln_main`, `kf_*`, `kh_*`), so
  the runtime calls closures and helpers as it does now.

## Evidence

- `conformance/` runs with `--backend llvm` in `tests/conformance.rs`
  when clang is found (CI's Ubuntu has clang 18), next to `c`, `native`
  and `wasm`, and as `windows-llvm` under Wine next to `windows`; all 54
  programs pass, the servers included. `tests/io.rs` and
  `tests/databases.rs` run their programs through it too.
- `tests/fire.rs` checks that `kiln fire` builds through LLVM when clang
  is there, falls back with the note when `KILN_CLANG` points nowhere,
  and fails with `--backend llvm` then.
- `kiln build --backend llvm --target windows` makes an `.exe` from an
  LLVM object through `pe.rs` (LLVM's `.pdata`/`.xdata` are placed with
  the read-only data), and CI's Windows job builds and runs a release
  with the clang on its runner.
- Measured on four small CPU-bound programs (2026-11-02, one machine,
  `kiln build` with Cranelift against `kiln fire` with clang 18): a loop
  of `Int` arithmetic 3.0 times faster, struct allocation and field
  arithmetic 1.6 times, recursive `fib` 1.4 times, and a string-building
  program 1.0 times (it spends its time in the runtime, which is prebuilt
  either way). The C backend, which compiles `kiln.c` in the same
  translation unit and so inlines the runtime, is faster than LLVM on
  that last one, which is the case for the LTO step below.

## Explicit boundaries

- **Later Phase 7 slices:** the precise garbage collector, security
  hardening, performance work in the runtime, the 1.0 stability and
  compatibility policy, and a `kiln fire` that strips or embeds assets.
- **Phase 8:** `kiln deploy`/`kiln release`, remote hosts and anything
  that uploads a build.
- No `-march=native`, PGO or LTO across the runtime in this slice: the
  runtime is prebuilt inside `kiln`, so cross-module LTO would need the
  runtime as bitcode, which is a possible later step and the one that
  would speed up runtime-bound programs (string handling, JSON, the
  database clients).
