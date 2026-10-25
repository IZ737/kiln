//! Makes Windows executables without a linker. The Windows runtime is an
//! executable linked when `kiln` is built (see `build.rs`), with the Kiln
//! runtime, the garbage collector and the C library already in it. To make
//! a program, `kiln` adds the object Cranelift made to a copy of it:
//!
//! 1. The object's sections become three new sections at the end of the
//!    image: code, read-only data and data.
//! 2. Its relocations are applied, against its own symbols and the
//!    runtime's, whose addresses `build.rs` listed.
//! 3. The runtime's `kiln_entry` table gets the addresses of the program's
//!    `kiln_init` and `kiln_main`, which the runtime's `main` calls.
//!
//! The runtime has no base relocations, so Windows always loads it at its
//! preferred address and the addresses written here stay right. The garbage
//! collector finds the program's globals itself: it scans every writable
//! section of the image.

use crate::runtime;
use object::{Object, ObjectSection, ObjectSymbol, RelocationFlags, RelocationTarget, SectionKind};
use std::collections::HashMap;
use std::path::Path;

/// Adds the object that [`crate::compile`] made to the runtime and writes
/// the executable to `out`.
pub(crate) fn link(object: &[u8], out: &Path) -> Result<(), String> {
    let image = add_program(runtime::WINDOWS_EXE, runtime::WINDOWS_SYMBOLS, object)
        .map_err(|e| format!("cannot make the Windows executable: {e}"))?;
    std::fs::write(out, image).map_err(|e| format!("cannot write {}: {e}", out.display()))
}

// Relocation types for x86-64 (`IMAGE_REL_AMD64_*`).
const ADDR64: u16 = 1;
const ADDR32NB: u16 = 3;
const REL32: u16 = 4;
const REL32_5: u16 = 9;

// Section characteristics (`IMAGE_SCN_*`).
const CODE: u32 = 0x0000_0020;
const INITIALIZED_DATA: u32 = 0x0000_0040;
const EXECUTE: u32 = 0x2000_0000;
const READ: u32 = 0x4000_0000;
const WRITE: u32 = 0x8000_0000;

/// A new section of the image: the object sections placed in it, one after
/// another.
struct Group {
    name: &'static [u8; 8],
    characteristics: u32,
    bytes: Vec<u8>,
    rva: u32,
}

fn add_program(exe: &[u8], symbols: &str, object: &[u8]) -> Result<Vec<u8>, String> {
    let mut img = exe.to_vec();
    let coff = rd32(&img, 0x3c) as usize + 4;
    let sections = rd16(&img, coff + 2) as usize;
    let opt = coff + 20;
    let table = opt + rd16(&img, coff + 16) as usize;
    let base = rd64(&img, opt + 24);
    let section_align = rd32(&img, opt + 32);
    let file_align = rd32(&img, opt + 36);
    let headers_end = (0..sections)
        .map(|i| rd32(&img, table + 40 * i + 20))
        .filter(|&raw| raw != 0)
        .min()
        .unwrap_or(rd32(&img, opt + 60)) as usize;

    let runtime: HashMap<&str, u32> = symbols
        .lines()
        .filter_map(|line| {
            let (name, rva) = line.split_once(' ')?;
            Some((name, u32::from_str_radix(rva, 16).ok()?))
        })
        .collect();

    let obj = object::File::parse(object).map_err(|e| e.to_string())?;
    let mut groups = [
        Group {
            name: b".ktext\0\0",
            characteristics: CODE | EXECUTE | READ,
            bytes: Vec::new(),
            rva: 0,
        },
        Group {
            name: b".krdata\0",
            characteristics: INITIALIZED_DATA | READ,
            bytes: Vec::new(),
            rva: 0,
        },
        Group {
            name: b".kdata\0\0",
            characteristics: INITIALIZED_DATA | READ | WRITE,
            bytes: Vec::new(),
            rva: 0,
        },
    ];
    // Where each object section went: its group and offset in it.
    let mut placed = HashMap::new();
    for section in obj.sections() {
        let group = match section.kind() {
            SectionKind::Text => 0,
            SectionKind::ReadOnlyData
            | SectionKind::ReadOnlyDataWithRel
            | SectionKind::ReadOnlyString => 1,
            SectionKind::Data | SectionKind::UninitializedData => 2,
            _ => continue,
        };
        let bytes = &mut groups[group].bytes;
        let offset = bytes
            .len()
            .next_multiple_of(section.align().max(1) as usize);
        bytes.resize(offset, 0);
        if section.kind() == SectionKind::UninitializedData {
            bytes.resize(offset + section.size() as usize, 0);
        } else {
            bytes.extend_from_slice(section.data().map_err(|e| e.to_string())?);
        }
        placed.insert(section.index(), (group, offset));
    }
    let mut next_rva = rd32(&img, opt + 56).next_multiple_of(section_align);
    for group in groups.iter_mut().filter(|g| !g.bytes.is_empty()) {
        group.rva = next_rva;
        next_rva = (next_rva + group.bytes.len() as u32).next_multiple_of(section_align);
    }
    let rvas = groups.each_ref().map(|g| g.rva as u64);

    let address = |symbol: &object::Symbol| -> Result<u64, String> {
        match symbol.section_index() {
            Some(index) => {
                let &(group, offset) = placed
                    .get(&index)
                    .ok_or_else(|| format!("`{}` is in a section it cannot place", name(symbol)))?;
                Ok(base + rvas[group] + offset as u64 + symbol.address())
            }
            None => runtime
                .get(name(symbol))
                .map(|&rva| base + rva as u64)
                .ok_or_else(|| format!("the runtime has no `{}`", name(symbol))),
        }
    };
    let mut patches = Vec::new();
    for section in obj.sections() {
        let Some(&(group, start)) = placed.get(&section.index()) else {
            continue;
        };
        for (offset, reloc) in section.relocations() {
            let RelocationTarget::Symbol(index) = reloc.target() else {
                return Err("a relocation has no symbol".into());
            };
            let symbol = obj.symbol_by_index(index).map_err(|e| e.to_string())?;
            let RelocationFlags::Coff { typ } = reloc.flags() else {
                return Err("the object is not COFF".into());
            };
            let at = start + offset as usize;
            let place = base + rvas[group] + at as u64;
            patches.push((group, at, typ.0, address(&symbol)?, place));
        }
    }
    for (group, at, typ, target, place) in patches {
        let bytes = &mut groups[group].bytes;
        match typ {
            ADDR64 => {
                let addend = rd64(bytes, at);
                bytes[at..at + 8].copy_from_slice(&target.wrapping_add(addend).to_le_bytes());
            }
            ADDR32NB => {
                let value = target - base + rd32(bytes, at) as u64;
                let value = u32::try_from(value).map_err(|_| "an address is out of range")?;
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            REL32..=REL32_5 => {
                // The offset is from the end of the 4 bytes, and of the
                // `typ - REL32` bytes of the instruction that follow them.
                let after = place + 4 + (typ - REL32) as u64;
                let addend = rd32(bytes, at) as i32 as i64;
                let value = (target as i64).wrapping_sub(after as i64) + addend;
                let value = i32::try_from(value).map_err(|_| "a jump is out of range")?;
                bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
            }
            _ => return Err(format!("unsupported relocation type {typ}")),
        }
    }

    // The entry table, in the runtime's `.data`.
    let entry = |name: &str| -> Result<u64, String> {
        let symbol = obj
            .symbol_by_name(name)
            .ok_or_else(|| format!("the program has no `{name}`"))?;
        address(&symbol)
    };
    let (init, main) = (entry("kiln_init")?, entry("kiln_main")?);
    let table_rva = *runtime
        .get("kiln_entry")
        .ok_or("the runtime has no `kiln_entry`")?;
    let table_at = (0..sections)
        .map(|i| table + 40 * i)
        .find_map(|h| {
            let (va, raw_size, raw) = (rd32(&img, h + 12), rd32(&img, h + 16), rd32(&img, h + 20));
            (table_rva >= va && table_rva + 16 <= va + raw_size)
                .then(|| (raw + table_rva - va) as usize)
        })
        .ok_or("the runtime's `kiln_entry` has no bytes in the file")?;
    img[table_at..table_at + 8].copy_from_slice(&init.to_le_bytes());
    img[table_at + 8..table_at + 16].copy_from_slice(&main.to_le_bytes());

    // The new section headers and their contents.
    let added: Vec<&Group> = groups.iter().filter(|g| !g.bytes.is_empty()).collect();
    if table + 40 * (sections + added.len()) > headers_end {
        return Err("the runtime has no room for more section headers".into());
    }
    for (i, group) in added.iter().enumerate() {
        img.resize(img.len().next_multiple_of(file_align as usize), 0);
        let raw = img.len() as u32;
        let raw_size = (group.bytes.len() as u32).next_multiple_of(file_align);
        img.extend_from_slice(&group.bytes);
        img.resize(raw as usize + raw_size as usize, 0);
        let h = table + 40 * (sections + i);
        let header = &mut img[h..h + 40];
        header.fill(0);
        header[..8].copy_from_slice(group.name);
        header[8..12].copy_from_slice(&(group.bytes.len() as u32).to_le_bytes());
        header[12..16].copy_from_slice(&group.rva.to_le_bytes());
        header[16..20].copy_from_slice(&raw_size.to_le_bytes());
        header[20..24].copy_from_slice(&raw.to_le_bytes());
        header[36..40].copy_from_slice(&group.characteristics.to_le_bytes());
        // SizeOfCode or SizeOfInitializedData.
        let field = if group.characteristics & CODE != 0 {
            opt + 4
        } else {
            opt + 8
        };
        let size = rd32(&img, field) + raw_size;
        img[field..field + 4].copy_from_slice(&size.to_le_bytes());
    }
    let count = (sections + added.len()) as u16;
    img[coff + 2..coff + 4].copy_from_slice(&count.to_le_bytes());
    img[opt + 56..opt + 60].copy_from_slice(&next_rva.to_le_bytes());
    // No checksum: Windows checks it only for drivers and system DLLs.
    img[opt + 64..opt + 68].fill(0);
    Ok(img)
}

fn name<'d>(symbol: &object::Symbol<'d, '_>) -> &'d str {
    symbol.name().unwrap_or("?")
}

fn rd16(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn rd32(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}

fn rd64(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}
