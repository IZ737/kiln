//! The native backend end to end: compiling, linking and running programs.
//! `crates/kiln_cli/tests/conformance.rs` runs the conformance programs
//! with it as well.

use kiln_native::Target;
use std::path::PathBuf;
use std::process::Command;

fn compile(source: &str) -> Result<Vec<u8>, Vec<String>> {
    compile_for(source, Target::host())
}

fn compile_for(source: &str, target: Target) -> Result<Vec<u8>, Vec<String>> {
    let parsed = kiln_syntax::parse(source);
    assert!(parsed.diagnostics.is_empty(), "{:?}", parsed.diagnostics);
    let program = kiln_check::check_program(&parsed.module);
    assert!(program.diagnostics.is_empty(), "{:?}", program.diagnostics);
    let ir = kiln_ir::lower(&parsed.module, &program).unwrap();
    kiln_native::compile(&ir, target).map_err(|ds| ds.into_iter().map(|d| d.message).collect())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("kiln-native-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

const HI: &str = "fn main() {\n    print(\"hi ${6 * 7}\")\n}\n";

#[test]
fn builds_a_static_executable() {
    if kiln_native::unavailable(Target::Linux).is_some() {
        return;
    }
    let object = compile_for(HI, Target::Linux).unwrap();
    let exe = scratch("hi");
    kiln_native::link(&object, Target::Linux, &exe).unwrap();
    if cfg!(target_os = "linux") {
        let out = Command::new(&exe).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hi 42\n");
    }
    // A static executable has no program interpreter (dynamic loader).
    let bytes = std::fs::read(&exe).unwrap();
    let (phoff, phnum) = (
        u64::from_le_bytes(bytes[32..40].try_into().unwrap()) as usize,
        u16::from_le_bytes(bytes[56..58].try_into().unwrap()) as usize,
    );
    let interp = (0..phnum).any(|i| {
        let at = phoff + 56 * i;
        u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap()) == 3
    });
    assert!(!interp, "the executable needs a dynamic loader");
    let _ = std::fs::remove_file(&exe);
}

#[test]
fn builds_a_windows_executable() {
    if kiln_native::unavailable(Target::Windows).is_some() {
        return;
    }
    let object = compile_for(HI, Target::Windows).unwrap();
    let exe = scratch("hi.exe");
    kiln_native::link(&object, Target::Windows, &exe).unwrap();
    let bytes = std::fs::read(&exe).unwrap();
    assert_eq!(&bytes[..2], b"MZ");
    let pe = u32::from_le_bytes(bytes[0x3c..0x40].try_into().unwrap()) as usize;
    assert_eq!(&bytes[pe..pe + 4], b"PE\0\0");
    // The program's code is in the sections kiln added to the runtime.
    let sections = u16::from_le_bytes([bytes[pe + 6], bytes[pe + 7]]) as usize;
    let table = pe + 24 + u16::from_le_bytes([bytes[pe + 20], bytes[pe + 21]]) as usize;
    let names: Vec<&[u8]> = (0..sections)
        .map(|i| &bytes[table + 40 * i..table + 40 * i + 8])
        .collect();
    assert!(names.contains(&&b".ktext\0\0"[..]), "{names:?}");
    // Windows runs it directly; elsewhere `KILN_WINE` names a Wine to run
    // it with.
    let runner = if cfg!(windows) {
        Some(Command::new(&exe))
    } else {
        std::env::var_os("KILN_WINE").map(|wine| {
            let mut c = Command::new(wine);
            c.arg(&exe).env("WINEDEBUG", "-all");
            c
        })
    };
    if let Some(mut runner) = runner {
        let out = runner.output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "hi 42\n");
    }
    let _ = std::fs::remove_file(&exe);
}

#[test]
fn reports_what_it_cannot_build() {
    // Sets cannot be map keys yet, in either backend.
    let errors =
        compile("fn main() {\n    let m: Map<Set<Int>, Int> = Map()\n    print(m.len())\n}\n")
            .unwrap_err();
    assert_eq!(
        errors,
        ["a map key or set item of this type is not supported by the compiler yet"]
    );
}
