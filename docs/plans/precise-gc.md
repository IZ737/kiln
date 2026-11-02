# Phase 7 — A precise garbage collector

**Status: implemented (2026-11-02), in 0.18.0.** This plan replaced the
Boehm collector with Kiln's own collector. Where it and the code differ,
the code (`crates/kiln_rt/src/gc.rs`) and `docs/design.md` are
authoritative.

Roadmap placement: Phase 7 (Kiln 1.0 readiness), alongside the LLVM
`kiln fire` backend, which shares the root contract below.

## 1. Why

Phase 1 linked the Boehm collector because it needed no help from the
compiler: it scans every stack word and every heap word and treats
anything that looks like a pointer as one. That was the right call then,
and it has cost us since:

- **It guesses.** An `Int` or a `Float` that happens to look like an
  address keeps an object alive, and whole task stacks (one megabyte each,
  one per connection) are scanned word by word on every collection.
- **It owns the stacks.** The task scheduler has to tell Boehm about every
  stack switch (`GC_set_stackbottom`, a push-other-roots hook), and Rust
  code in `kiln_rt` has to follow rules written around what Boehm can and
  cannot see.
- **It is a C dependency.** Programs built through the C backend need
  `libgc-dev`; the native backend vendors and builds `bdwgc` for musl and
  mingw; a second collector would have to be added for any new target.
- **It cannot grow.** `docs/design.md` plans escape analysis, request
  arenas and, later, concurrent marking. Each needs the compiler and the
  collector to agree on where references are, which a conservative
  collector cannot offer.

A precise collector knows exactly which words hold references. This plan
builds one in `kiln_rt` (new runtime code goes in Rust) and teaches the
backends (C, Cranelift and LLVM) to tell it where the references are.

## 2. What "precise" means here

Kiln values come in two shapes. Typed values (`Int`, `Float`, `Bool`,
`Duration`, and pointers to strings, lists, structs, ...) are known to the
compiler at every use. Erased values (`kv` slots in lists, maps, records,
enum fields, boxes and closure arguments) hold a typed value whose type
only the code that put it there knew.

The collector is **precise about roots and about every typed layout**,
and **validates each erased slot** rather than trusting it:

- It never scans a machine stack, a register or static data. The only
  roots are the shadow-stack slots generated code writes (section 4), the
  program's globals and the handful of runtime statics that hold Kiln
  values, all registered with the collector.
- Every object carries a layout: *atomic* (no references: string bytes,
  hash indexes), a *pointer bitmap* (structs, records, variants, closure
  environments, cells and the runtime's own structs), or *slots*.
- A word in a *slots* object, or a shadow-stack slot holding an erased
  value, is followed only if it is the address of, or inside, a live object
  of the collector's heap. An `Int` that passes that check retains an
  object, as under Boehm; nothing else can go wrong, because the collector
  never moves objects, so a wrongly retained word is never written back.

Over time more erased slots become typed (section 9); the collector's
interface does not change when they do.

## 3. The collector

A **non-moving, stop-the-world mark-and-sweep** collector in
`crates/kiln_rt/src/gc.rs`, run on the thread that runs Kiln's cooperative
tasks. Concurrent marking (the design document's long-term aim) is a later
step on the same object layout; nothing here prevents it.

- **Heap.** Small objects (up to 2 KiB) live in 64 KiB blocks of one size
  class each, with allocation, mark and layout bits beside the block;
  large objects get their own block. A block table maps any address to its
  block in O(1), which is what makes validating erased slots cheap.
- **Headers.** One word per object: the layout kind and, for a bitmap
  layout, the bitmap itself (objects of up to 56 words), or the size for
  slots and atomic objects.
- **Allocation.** `k_alloc(n)` (slots), `k_alloc_atomic(n)`,
  `k_alloc_ptrs(n)` (every word a reference) and `k_alloc_bits(n, bitmap)`.
  Memory is zeroed. Allocation never collects: it bumps a counter, and
  when the counter passes the threshold it sets `k_gc_pending`.
- **Trigger.** A collection runs at the next safepoint after the heap has
  grown to twice what was live after the previous one (never below 8 MiB
  of growth). `KILN_GC_STRESS=1` collects at every safepoint and fills
  freed memory with a poison byte, so a missing root is a crash in the test
  suite rather than a rare corruption in production.
- **Marking.** From the roots, with an explicit mark stack; interior
  pointers (a `kstr`'s `data` into its buffer, a slice into another's) are
  resolved to their object through the block table.
- **Sweeping.** Eagerly, block by block, rebuilding each block's free list;
  empty blocks return to the block pool, and large objects to the system.
- **Reporting.** `KILN_GC_LOG=1` prints each collection's live bytes, freed
  bytes and time, for tuning.

Boehm's uses in the runtime map onto `k_gc_base` (the object an address
is inside, for `k_web_marked`), explicit root registration (`k_gc_root`)
for the runtime's statics and tasks, and nothing else.

## 4. Roots: the shadow stack

Every generated function that holds a pointer-typed or erased (`kv`)
value keeps a **frame record** on its machine stack:

```c
struct kgc_frame { struct kgc_frame *prev; int64_t n; kv slots[n]; };
```

On entry the function zeroes the slots, stores `k_gc_top` into `prev` and
sets `k_gc_top` to the record; on every return it restores `prev`. Every
pointer or `kv` value that is live across a call (a local, a parameter, or
a temporary such as a call's result used as a later argument) is **stored
into a slot when it is defined**. Reads need not come from the slot: the
collector never moves objects, so the register copy stays valid. The
backends keep the number of slots small by reusing temporaries' slots
once the expression that needed them is done.

`k_gc_top` belongs to the running task; `k_task_switch` saves and
restores it like the other per-task runtime state, and the collector
walks every task's chain. A panic that `longjmp`s to the server's trap
restores the `k_gc_top` the trap saw.

Why this and not native stack maps (Cranelift's user stack maps, LLVM's
statepoints): stack maps need a frame walker that understands every
frame on the stack, including the C and Rust runtime frames between
generated frames (a list method calling a closure, hyper calling a
handler), on both Linux and Windows and for every suspended task, and
they come in a different format from each backend. The shadow stack is a
few stores per function, is the same code in the C, Cranelift and LLVM
backends, and lets runtime code root a value with one call. It is the
root protocol for 1.0; a backend may move to native stack maps later
without the collector noticing, since the collector only asks for roots.

## 5. Safepoints

A collection runs only where every live reference is in a slot: at
**polls**, never inside `k_alloc`. The backends emit a poll (`if
(k_gc_pending) k_gc_collect()`) at the entry of every user function and
closure and at every loop back-edge. Generated `show`, `eq`, `hash` and
`cmp` helpers do not poll, so runtime code that calls them (every map
operation does) needs no rooting around the call.

Runtime code therefore roots its values in exactly one situation:
**across a call into generated user code** (a closure, a handler, a route's
pages function). `k_gc_push(&value)` / `k_gc_pop()` in C and a guard
type in Rust cover the list and map methods that take closures, the web
server's handler calls, the static export and the sort-by key function.
`kiln_rt`'s rule becomes simpler than under Boehm: a Kiln value may sit
anywhere in Rust (stack or heap) *between* polls, and only a value held
across a call into generated code needs rooting.

## 6. Object layouts

| Allocation | Layout |
| --- | --- |
| String buffers, hash indexes, `live` arrays, bytes | atomic |
| `kstr`, `klist`, `kmap`, `kclosure`, `kbox`, `kapp`, routes, stores | bitmap written in the runtime |
| Structs, records, variants, closure environments, cells | bitmap from the backend's field types |
| List items, map keys and values, enum fields built by generic runtime code, boxes | slots (validated) |

A `kv` field whose static type is a pointer counts as a reference in a
bitmap; `Int`, `Float`, `Bool` and `Duration` fields do not. The C
backend lays Kiln structs out as C structs of 8-byte fields, and the
native backend as slot arrays, so a field's index is its bit in both.

## 7. What does not change

- **Embers and WasmGC.** Browser code uses the browser's collector; the
  WasmGC backend, `ember.js` and hydration are untouched.
- **Values.** `kv`, `kstr`, `klist`, every `k_*` function's signature and
  the type descriptions (`kdesc`) stay as they are; the backends gain a
  few calls, not a new value model.
- **Tasks.** The scheduler keeps its stacks and switch; it loses the Boehm
  hooks and gains one saved pointer per task.
- **Build.** `kiln` still carries a prebuilt runtime per target, and
  programs are still single static executables. The C backend no longer
  needs `libgc-dev`, and `vendor/bdwgc` goes away.

## 8. Steps

1. **Collector** in `kiln_rt/src/gc.rs`: heap, allocation, roots,
   marking, sweeping, stress and log modes; `kiln.c` calls it through
   `k_alloc*`; Boehm removed from both build scripts and the C link line.
2. **Native backend**: frame records, slot stores for locals and
   temporaries, polls, layout bitmaps at allocation sites, the globals
   table registered as a root range.
3. **C backend**: the same, with locals and temporaries living in the
   frame record so no pointer is ever only in a C temporary.
4. **Runtime rooting**: `k_gc_push`/`k_gc_pop` around every call into
   user code in `kiln.c` and `kiln_rt`; `k_gc_top` in `k_task_switch`;
   runtime statics registered.
5. **Tests**: every conformance program under `KILN_GC_STRESS=1` on both
   backends in `tests/conformance.rs`; a `conformance/gc` program that
   allocates past several collections, keeps data alive across closures,
   tasks and panics, and prints what it kept; `tests/server.rs` with a
   collection during concurrent requests; `clean-check.sh` and
   `windows-check.sh` unchanged.
6. **Docs**: `design.md`'s memory section, the README's requirements and
   crate table, this plan's status.

## 9. After this plan

- **Typed erased slots.** Lists and maps can carry their item kind from
  the allocation site, and `k_box` a flag, so the validated path shrinks
  to records and enum fields built by generic runtime code.
- **Native stack maps** for the Cranelift and LLVM backends where
  measurement shows the frame stores matter.
- **Incremental or concurrent marking** with a write barrier, once
  programs run tasks on several cores.
- **Request arenas** and escape analysis, as `design.md` plans.
