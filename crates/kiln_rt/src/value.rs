//! Kiln values as the C runtime lays them out, and the C runtime functions
//! that make them. The layouts mirror the typedefs in `kiln.c`; a change
//! there changes these too.
//!
//! Every value is a 64-bit slot (`Kv`): an integer, a float's bits, a bool,
//! or a pointer into the collector's heap. Values made here are kept only
//! in locals (on the stack, which the collector scans) or in other Kiln
//! values, never in memory from Rust's allocator.

use std::ffi::{c_char, c_int, c_void, CStr};

pub type Kv = u64;

/// `kdesc`: a description of a type, which the backends write into the
/// program for runtime code that works on any type.
#[repr(C)]
pub struct KDesc {
    pub kind: i64,
    pub n: i64,
    pub name: *const c_char,
    pub parts: *const *const KDesc,
    pub labels: *const *const c_char,
}

pub const D_STR: i64 = 1;
pub const D_INT: i64 = 2;
pub const D_FLOAT: i64 = 3;
pub const D_BOOL: i64 = 4;
pub const D_DURATION: i64 = 5;
pub const D_OPT: i64 = 6;
pub const D_LIST: i64 = 8;
pub const D_MAP: i64 = 9;
pub const D_SET: i64 = 10;
pub const D_STRUCT: i64 = 11;
pub const D_ENUM: i64 = 12;
pub const D_RECORD: i64 = 13;

impl KDesc {
    /// Part `i`: an item, key or value type, a field's type or a variant.
    pub fn part(&self, i: i64) -> &KDesc {
        unsafe { &**self.parts.add(i as usize) }
    }

    /// The name of field `i`, or None for a variant's unnamed field.
    pub fn label(&self, i: i64) -> Option<&str> {
        let p = unsafe { *self.labels.add(i as usize) };
        if p.is_null() {
            None
        } else {
            Some(unsafe { CStr::from_ptr(p) }.to_str().unwrap_or(""))
        }
    }

    /// Whether the fields have names (a struct, record or variant with
    /// named fields).
    pub fn named(&self) -> bool {
        self.n > 0 && self.label(0).is_some()
    }

    /// A variant's name.
    pub fn name(&self) -> &str {
        if self.name.is_null() {
            return "";
        }
        unsafe { CStr::from_ptr(self.name) }.to_str().unwrap_or("")
    }
}

#[repr(C)]
pub struct KStr {
    pub len: i64,
    pub data: *const u8,
}

impl KStr {
    pub fn bytes(&self) -> &[u8] {
        if self.len == 0 {
            return &[];
        }
        unsafe { std::slice::from_raw_parts(self.data, self.len as usize) }
    }
}

#[repr(C)]
pub struct KList {
    pub len: i64,
    pub cap: i64,
    pub items: *mut Kv,
}

#[repr(C)]
pub struct KMap {
    pub len: i64,
    pub used: i64,
    pub cap: i64,
    pub keys: *mut Kv,
    pub vals: *mut Kv,
    pub live: *mut bool,
    pub index: *mut i64,
    pub slots: i64,
    pub hash: *const c_void,
    pub eq: *const c_void,
}

#[repr(C)]
pub struct KBox {
    pub v: Kv,
}

#[repr(C)]
pub struct KEnum {
    pub tag: i64,
    pub f: [Kv; 0],
}

extern "C" {
    pub fn k_alloc(n: usize) -> *mut c_void;
    pub fn k_str_n(s: *const c_char, n: i64) -> *mut KStr;
    pub fn k_list(n: i64, items: *const Kv) -> *mut KList;
    pub fn k_list_push(l: *mut KList, item: Kv);
    pub fn k_map_set(m: *mut KMap, key: Kv, val: Kv);
    pub fn k_set_add(m: *mut KMap, item: Kv);
    pub fn k_box(v: Kv) -> *mut KBox;
    pub fn k_enum(tag: i64, n: c_int, fields: *const Kv) -> *mut KEnum;
    /// A map or set for keys of the described type, or NULL for a key type
    /// JSON cannot read.
    pub fn k_json_new_map(key: *const KDesc) -> *mut KMap;
}

pub fn ptr<T>(v: Kv) -> *mut T {
    v as usize as *mut T
}

pub fn kv<T>(p: *mut T) -> Kv {
    p as usize as Kv
}

/// A new Kiln string with these bytes.
pub fn string(bytes: &[u8]) -> Kv {
    kv(unsafe { k_str_n(bytes.as_ptr().cast(), bytes.len() as i64) })
}

/// The bytes of the string `v`.
pub fn bytes<'a>(v: Kv) -> &'a [u8] {
    unsafe { (*ptr::<KStr>(v)).bytes() }
}

/// `n` slots in the collector's heap, zeroed.
pub fn slots(n: i64) -> *mut Kv {
    unsafe { k_alloc(std::mem::size_of::<Kv>() * (n as usize + 1)).cast() }
}
