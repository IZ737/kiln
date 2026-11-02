//! Kiln's garbage collector: a non-moving, stop-the-world mark-and-sweep
//! collector for the values of one program. The plan and the rules are in
//! `docs/plans/precise-gc.md`; in short:
//!
//! - Every object has a one-word header giving its layout: *atomic* (no
//!   references), *pointers* (every word a reference or 0), a *bitmap*
//!   (which words are references) or *slots* (words that may hold a
//!   reference or a plain number: each is followed only if it is the
//!   address of, or inside, a live object of this heap).
//! - Roots are the shadow-stack frames generated code links from
//!   `k_gc_top`, the per-task state the C runtime keeps, and the ranges
//!   registered with [`k_gc_add_roots`]. The collector never scans a
//!   machine stack.
//! - Allocation never collects. It counts bytes, and past the threshold
//!   sets [`k_gc_pending`]; generated code polls that at safepoints and
//!   calls [`k_gc_collect`], where every live reference is in a root.
//!
//! The heap is made of 64 KiB blocks. A small block holds objects of one
//! size class; a large object has a block of its own, spanning as many
//! 64 KiB units as it needs. A hash table from a unit's address to its
//! block makes finding the object an address is inside a few loads, which
//! is what validating a slot costs.
//!
//! The runtime runs Kiln code and its tasks on one thread, and so does the
//! collector; nothing here is thread-safe.

use std::alloc::{alloc_zeroed, dealloc, Layout};
use std::cell::UnsafeCell;
use std::ffi::c_void;

/// A Kiln value slot, as `kv` in the C runtime.
pub type Kv = u64;

/// Set when enough has been allocated for a collection to be worth it;
/// generated code checks it at safepoints and calls [`k_gc_collect`].
#[no_mangle]
pub static mut k_gc_pending: u8 = 0;

/// The innermost shadow-stack frame of the running task; see
/// [`Frame`]. Generated code links and unlinks frames here, and the task
/// switch saves and restores it.
#[no_mangle]
pub static mut k_gc_top: *mut Frame = std::ptr::null_mut();

/// A shadow-stack frame: what generated code keeps on its machine stack
/// for the values it holds across calls. `slots` has `n` words.
#[repr(C)]
pub struct Frame {
    pub prev: *mut Frame,
    pub n: i64,
    pub slots: [Kv; 0],
}

/// The layouts, as the low byte of an object's header. 0 is a free object.
pub const ATOMIC: u64 = 1;
pub const SLOTS: u64 = 2;
pub const PTRS: u64 = 3;
pub const BITS: u64 = 4;

const UNIT: usize = 64 * 1024;
const HEADER: usize = 8;
/// Object sizes with the header, one block of each class at a time.
const CLASSES: [usize; 24] = [
    16, 32, 48, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384, 448, 512, 640, 768, 1024, 1280,
    1536, 2048, 3072, 4096,
];
const LARGEST_SMALL: usize = 4096;
/// Collect once this much has been allocated since the last collection,
/// or as much as was live then, whichever is more.
const MIN_GROWTH: usize = 8 * 1024 * 1024;
const POISON: u8 = 0xDD;

struct Block {
    base: usize,
    /// The size of the allocation: `UNIT` for a small block.
    size: usize,
    /// The size class of the block's objects; `None` for a large object.
    class: Option<usize>,
    /// A mark bit per object.
    marks: Vec<u64>,
}

impl Block {
    /// The size of each object with its header: the whole block for a
    /// large object.
    fn object_size(&self) -> usize {
        match self.class {
            Some(c) => CLASSES[c],
            None => self.size,
        }
    }
}

struct Heap {
    blocks: Vec<Block>,
    /// Open addressing from `addr / UNIT` to an index in `blocks`, or
    /// `usize::MAX` for an empty entry.
    table: Vec<(usize, usize)>,
    table_used: usize,
    /// The free list of each class: the address of the first free object
    /// (its header), or 0.
    free: [usize; CLASSES.len()],
    /// Bytes allocated since the last collection, and live after it.
    allocated: usize,
    live: usize,
    threshold: usize,
    /// Pending marks: object headers whose contents are still to be scanned.
    stack: Vec<usize>,
    roots: Vec<(usize, usize)>,
    /// `KILN_GC_STRESS`: collect at every `n`th safepoint, and poison what
    /// is freed; 0 when off.
    stress: u32,
    polls: u32,
    log: bool,
    collections: u64,
}

struct Global(UnsafeCell<Option<Heap>>);
unsafe impl Sync for Global {}
static HEAP: Global = Global(UnsafeCell::new(None));

fn heap() -> &'static mut Heap {
    let slot = unsafe { &mut *HEAP.0.get() };
    slot.get_or_insert_with(Heap::new)
}

fn env_number(name: &str) -> u32 {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() && v != "0" => v.trim().parse().unwrap_or(1),
        _ => 0,
    }
}

impl Heap {
    fn new() -> Heap {
        let stress = env_number("KILN_GC_STRESS");
        Heap {
            blocks: Vec::new(),
            table: vec![(0, usize::MAX); 1024],
            table_used: 0,
            free: [0; CLASSES.len()],
            allocated: 0,
            live: 0,
            threshold: MIN_GROWTH,
            stack: Vec::new(),
            roots: Vec::new(),
            stress,
            polls: 0,
            log: env_number("KILN_GC_LOG") != 0,
            collections: 0,
        }
    }

    // ----- the block table -----

    fn table_insert(&mut self, unit: usize, block: usize) {
        if (self.table_used + 1) * 2 > self.table.len() {
            let bigger = vec![(0, usize::MAX); self.table.len() * 2];
            let old = std::mem::replace(&mut self.table, bigger);
            self.table_used = 0;
            for (u, b) in old {
                if b != usize::MAX {
                    self.table_insert(u, b);
                }
            }
        }
        let mask = self.table.len() - 1;
        let mut i = hash(unit) & mask;
        while self.table[i].1 != usize::MAX {
            i = (i + 1) & mask;
        }
        self.table[i] = (unit, block);
        self.table_used += 1;
    }

    fn table_get(&self, unit: usize) -> Option<usize> {
        let mask = self.table.len() - 1;
        let mut i = hash(unit) & mask;
        loop {
            let (u, b) = self.table[i];
            if b == usize::MAX {
                return None;
            }
            if u == unit {
                return Some(b);
            }
            i = (i + 1) & mask;
        }
    }

    // ----- allocation -----

    fn new_block(&mut self, size: usize, class: Option<usize>) -> usize {
        let layout = Layout::from_size_align(size, UNIT).unwrap();
        let base = unsafe { alloc_zeroed(layout) } as usize;
        if base == 0 {
            out_of_memory();
        }
        let nobjs = match class {
            Some(c) => UNIT / CLASSES[c],
            None => 1,
        };
        let index = self.blocks.len();
        self.blocks.push(Block {
            base,
            size,
            class,
            marks: vec![0; nobjs.div_ceil(64)],
        });
        for u in (base..base + size).step_by(UNIT) {
            self.table_insert(u / UNIT, index);
        }
        index
    }

    fn alloc(&mut self, bytes: usize, header: u64) -> *mut c_void {
        let total = bytes.max(1) + HEADER;
        if total > LARGEST_SMALL {
            return self.alloc_large(total, header);
        }
        let class = CLASSES.iter().position(|&c| c >= total).unwrap();
        let size = CLASSES[class];
        let mut p = self.free[class];
        if p == 0 {
            let b = self.new_block(UNIT, Some(class));
            let base = self.blocks[b].base;
            // Thread the new block's objects onto the free list, last first.
            let n = UNIT / size;
            let mut next = 0usize;
            for i in (0..n).rev() {
                let obj = base + i * size;
                unsafe { *((obj + HEADER) as *mut usize) = next };
                next = obj;
            }
            p = next;
        }
        self.free[class] = unsafe { *((p + HEADER) as *const usize) };
        self.allocated += size;
        if self.allocated >= self.threshold {
            unsafe { k_gc_pending = 1 };
        }
        unsafe {
            std::ptr::write_bytes((p + HEADER) as *mut u8, 0, size - HEADER);
            *(p as *mut u64) = header;
        }
        (p + HEADER) as *mut c_void
    }

    fn alloc_large(&mut self, total: usize, header: u64) -> *mut c_void {
        let size = total.div_ceil(UNIT) * UNIT;
        let b = self.new_block(size, None);
        let base = self.blocks[b].base;
        self.allocated += size;
        if self.allocated >= self.threshold {
            unsafe { k_gc_pending = 1 };
        }
        unsafe { *(base as *mut u64) = header };
        (base + HEADER) as *mut c_void
    }

    // ----- finding objects -----

    /// The object `addr` is inside, if it is a live object of this heap:
    /// its header address, its block and its index in the block.
    fn object_of(&self, addr: usize) -> Option<(usize, usize, usize)> {
        if addr < UNIT {
            return None;
        }
        let b = self.table_get(addr / UNIT)?;
        let block = &self.blocks[b];
        let offset = addr - block.base;
        let (obj, index) = match block.class {
            Some(c) => {
                let size = CLASSES[c];
                let i = offset / size;
                if (i + 1) * size > UNIT {
                    return None; // the tail no object fits in
                }
                (block.base + i * size, i)
            }
            None => (block.base, 0),
        };
        if unsafe { *(obj as *const u64) } == 0 {
            return None; // free
        }
        Some((obj, b, index))
    }

    // ----- marking -----

    fn mark_word(&mut self, w: Kv) {
        let Some((obj, b, i)) = self.object_of(w as usize) else {
            return;
        };
        let marks = &mut self.blocks[b].marks;
        let bit = 1u64 << (i % 64);
        if marks[i / 64] & bit != 0 {
            return;
        }
        marks[i / 64] |= bit;
        self.stack.push(obj);
    }

    fn mark_words(&mut self, words: *const Kv, n: usize) {
        for i in 0..n {
            let w = unsafe { *words.add(i) };
            if w != 0 {
                self.mark_word(w);
            }
        }
    }

    fn mark_chain(&mut self, mut frame: *const Frame) {
        while !frame.is_null() {
            let f = unsafe { &*frame };
            self.mark_words(f.slots.as_ptr(), f.n as usize);
            frame = f.prev;
        }
    }

    /// The payload size in words of the object whose header is at `obj`.
    fn words_of(&self, obj: usize) -> usize {
        let b = self.table_get(obj / UNIT).unwrap();
        (self.blocks[b].object_size() - HEADER) / 8
    }

    fn drain(&mut self) {
        while let Some(obj) = self.stack.pop() {
            let header = unsafe { *(obj as *const u64) };
            let payload = (obj + HEADER) as *const Kv;
            match header & 0xff {
                ATOMIC => {}
                BITS => {
                    let mut bits = header >> 8;
                    let mut i = 0;
                    while bits != 0 {
                        if bits & 1 != 0 {
                            let w = unsafe { *payload.add(i) };
                            if w != 0 {
                                self.mark_word(w);
                            }
                        }
                        bits >>= 1;
                        i += 1;
                    }
                }
                _ => {
                    let n = self.words_of(obj);
                    self.mark_words(payload, n);
                }
            }
        }
    }

    // ----- collecting -----

    fn collect(&mut self) {
        let start = std::time::Instant::now();
        let before = self.live + self.allocated;
        for block in &mut self.blocks {
            block.marks.iter_mut().for_each(|m| *m = 0);
        }
        unsafe { k_gc_visit_roots(visit_words, visit_chain) };
        for &(p, n) in &self.roots.clone() {
            self.mark_words(p as *const Kv, n);
        }
        self.drain();
        let live = self.sweep();
        self.live = live;
        self.allocated = 0;
        self.threshold = live.max(MIN_GROWTH);
        self.collections += 1;
        unsafe { k_gc_pending = (self.stress == 1) as u8 };
        if self.log {
            eprintln!(
                "gc: collection {}: {} live of {} bytes, {} blocks, {:?}",
                self.collections,
                live,
                before,
                self.blocks.len(),
                start.elapsed()
            );
        }
    }

    /// Frees what is not marked and returns the bytes still live.
    fn sweep(&mut self) -> usize {
        let poison = self.stress != 0;
        let mut live = 0;
        self.free = [0; CLASSES.len()];
        let mut freed_blocks = Vec::new();
        for (b, block) in self.blocks.iter().enumerate() {
            match block.class {
                None => {
                    if block.marks[0] & 1 != 0 {
                        live += block.size;
                    } else {
                        freed_blocks.push(b);
                    }
                }
                Some(class) => {
                    let size = CLASSES[class];
                    let n = UNIT / size;
                    let mut any = false;
                    // Thread free objects in address order, last first.
                    let mut head = self.free[class];
                    let mut tail: Option<usize> = None;
                    let mut block_list = 0usize;
                    for i in (0..n).rev() {
                        let obj = block.base + i * size;
                        if block.marks[i / 64] & (1 << (i % 64)) != 0 {
                            any = true;
                            live += size;
                            continue;
                        }
                        unsafe {
                            if *(obj as *const u64) != 0 && poison {
                                std::ptr::write_bytes(
                                    (obj + HEADER) as *mut u8,
                                    POISON,
                                    size - HEADER,
                                );
                            }
                            *(obj as *mut u64) = 0;
                            *((obj + HEADER) as *mut usize) = block_list;
                        }
                        block_list = obj;
                        if tail.is_none() {
                            tail = Some(obj);
                        }
                    }
                    if !any {
                        freed_blocks.push(b);
                        continue;
                    }
                    if let Some(t) = tail {
                        unsafe { *((t + HEADER) as *mut usize) = head };
                        head = block_list;
                    }
                    self.free[class] = head;
                }
            }
        }
        // Free the empty blocks and rebuild the table for the rest.
        if !freed_blocks.is_empty() {
            let mut freed = std::mem::take(&mut freed_blocks).into_iter().peekable();
            let old = std::mem::take(&mut self.blocks);
            for (b, block) in old.into_iter().enumerate() {
                if freed.peek() == Some(&b) {
                    freed.next();
                    unsafe {
                        dealloc(
                            block.base as *mut u8,
                            Layout::from_size_align(block.size, UNIT).unwrap(),
                        )
                    };
                } else {
                    self.blocks.push(block);
                }
            }
            self.table.iter_mut().for_each(|e| *e = (0, usize::MAX));
            self.table_used = 0;
            for b in 0..self.blocks.len() {
                let (base, size) = (self.blocks[b].base, self.blocks[b].size);
                for u in (base..base + size).step_by(UNIT) {
                    self.table_insert(u / UNIT, b);
                }
            }
        }
        live
    }
}

fn hash(unit: usize) -> usize {
    let mut x = unit as u64;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x as usize
}

fn out_of_memory() -> ! {
    eprintln!("panic: out of memory");
    std::process::exit(101);
}

extern "C" fn visit_words(words: *const Kv, n: i64) {
    heap().mark_words(words, n as usize);
}

extern "C" fn visit_chain(top: *const c_void) {
    heap().mark_chain(top as *const Frame);
}

extern "C" {
    /// In the C runtime: calls `words` for each range of root words of the
    /// program and its tasks, and `chain` for each task's shadow stack.
    fn k_gc_visit_roots(words: extern "C" fn(*const Kv, i64), chain: extern "C" fn(*const c_void));
}

// ----- the C interface -----

/// `n` zeroed bytes whose words may each hold a reference or a number.
#[no_mangle]
pub extern "C" fn k_rt_alloc(n: usize) -> *mut c_void {
    heap().alloc(n, SLOTS)
}

/// `n` zeroed bytes holding no references.
#[no_mangle]
pub extern "C" fn k_alloc_atomic(n: usize) -> *mut c_void {
    heap().alloc(n, ATOMIC)
}

/// `n` zeroed bytes whose every word is a reference or 0.
#[no_mangle]
pub extern "C" fn k_alloc_ptrs(n: usize) -> *mut c_void {
    heap().alloc(n, PTRS)
}

/// `n` zeroed bytes where bit `i` of `bits` says whether word `i` is a
/// reference. Objects of more than 56 words are treated as slots.
#[no_mangle]
pub extern "C" fn k_alloc_bits(n: usize, bits: u64) -> *mut c_void {
    let words = n.div_ceil(8);
    let header = if words > 56 || bits >> 56 != 0 {
        SLOTS
    } else if bits == 0 {
        ATOMIC
    } else {
        BITS | (bits << 8)
    };
    heap().alloc(n, header)
}

/// Registers `n` words at `p` as roots for the rest of the program.
#[no_mangle]
pub extern "C" fn k_gc_add_roots(p: *const Kv, n: i64) {
    heap().roots.push((p as usize, n as usize));
}

/// Runs a collection. Only generated code and the runtime call it, at
/// safepoints, where every live reference is in a root.
#[no_mangle]
pub extern "C" fn k_gc_collect() {
    let h = heap();
    if h.stress > 1 {
        h.polls += 1;
        if !h.polls.is_multiple_of(h.stress) && h.allocated < h.threshold {
            return;
        }
    }
    h.collect();
}

/// Starts the collector: reads its settings and arms stress mode.
#[no_mangle]
pub extern "C" fn k_gc_init() {
    let h = heap();
    if h.stress != 0 {
        unsafe { k_gc_pending = 1 };
    }
}

/// The start of the object `p` points into, or NULL.
#[no_mangle]
pub extern "C" fn k_gc_base(p: *const c_void) -> *mut c_void {
    match heap().object_of(p as usize) {
        Some((obj, _, _)) => (obj + HEADER) as *mut c_void,
        None => std::ptr::null_mut(),
    }
}

/// The size in bytes of the object at `p`, which [`k_gc_base`] gave, at
/// least what was asked for.
#[no_mangle]
pub extern "C" fn k_gc_size(p: *const c_void) -> usize {
    let h = heap();
    match h.object_of(p as usize) {
        Some((obj, _, _)) => h.words_of(obj) * 8,
        None => 0,
    }
}
