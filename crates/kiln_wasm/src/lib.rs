//! The WebAssembly backend: compiles a program in the IR to a WasmGC
//! module for the browser, where embers run.
//!
//! The module is written in the text format by [`emit`], assembled by the
//! `wat` crate and checked by `wasmparser`'s validator, so a mistake in the
//! backend is a compiler error rather than a broken page. Strings are the
//! browser's own: the module imports its string constants from
//! `runtime/kiln.js`, which reads them from the module's `kiln.strings`
//! custom section.

mod emit;

use kiln_diagnostics::{codes, Diagnostic, Span};
use kiln_ir::Program;

pub use emit::Roots;

/// The JavaScript half of the runtime: loads a module and gives it the
/// functions it imports.
pub const RUNTIME_JS: &str = include_str!("../runtime/kiln.js");
/// The bridge that takes over a page's embers (see `runtime/ember.js`).
pub const EMBER_JS: &str = include_str!("../runtime/ember.js");

/// A compiled module.
pub struct Module {
    /// The binary, with its strings in the `kiln.strings` section.
    pub wasm: Vec<u8>,
    /// The text format, for `--emit-wat`.
    pub wat: String,
}

/// Compiles `program` to a module that runs the functions reachable from
/// `roots`. Errors are code the browser cannot run (E0218, E0401).
pub fn compile(program: &Program, roots: &Roots) -> Result<Module, Vec<Diagnostic>> {
    let mut g = emit::Gen::new(program);
    let wat = g.module(roots);
    if !g.diags.is_empty() {
        let mut diags = std::mem::take(&mut g.diags);
        dedup(&mut diags);
        return Err(diags);
    }
    let internal = |what: String| {
        vec![Diagnostic::error(
            codes::NOT_COMPILED_YET,
            format!("internal error in the WebAssembly backend: {what}"),
            Span::default(),
        )
        .with_help("this is a bug in the compiler; please report it with the program")]
    };
    // For debugging the backend: where to write the text it made.
    if let Some(path) = std::env::var_os("KILN_DEBUG_WAT") {
        let _ = std::fs::write(path, &wat);
    }
    let mut wasm = match wat::parse_str(&wat) {
        Ok(w) => w,
        Err(e) => return Err(internal(e.to_string())),
    };
    let mut features = wasmparser::WasmFeatures::default();
    features.insert(wasmparser::WasmFeatures::GC);
    features.insert(wasmparser::WasmFeatures::REFERENCE_TYPES);
    features.insert(wasmparser::WasmFeatures::FUNCTION_REFERENCES);
    if let Err(e) = wasmparser::Validator::new_with_features(features).validate_all(&wasm) {
        let at = function_at(&wasm, e.offset()).unwrap_or_default();
        return Err(internal(format!("the module is not valid: {e}{at}")));
    }
    let strings = serde_json::to_vec(&g.strings).unwrap_or_default();
    custom_section(&mut wasm, "kiln.strings", &strings);
    Ok(Module { wasm, wat })
}

/// The name of the function whose code is at `offset`, for messages.
fn function_at(wasm: &[u8], offset: u64) -> Option<String> {
    use wasmparser::{Name, Parser, Payload};
    let mut imported = 0u32;
    let mut index = None;
    let mut bodies = 0u32;
    let mut names = std::collections::HashMap::new();
    for payload in Parser::new(0).parse_all(wasm) {
        match payload.ok()? {
            Payload::ImportSection(s) => {
                for imp in s.into_imports() {
                    if matches!(imp.ok()?.ty, wasmparser::TypeRef::Func(_)) {
                        imported += 1;
                    }
                }
            }
            Payload::CodeSectionEntry(body) => {
                if body.range().contains(&offset) {
                    index = Some(imported + bodies);
                }
                bodies += 1;
            }
            Payload::CustomSection(c) => {
                if let wasmparser::KnownCustom::Name(reader) = c.as_known() {
                    for name in reader {
                        if let Ok(Name::Function(map)) = name {
                            for n in map.into_iter().flatten() {
                                names.insert(n.index, n.name.to_string());
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let i = index?;
    Some(format!(
        ", in `{}`",
        names.get(&i).cloned().unwrap_or_else(|| i.to_string())
    ))
}

/// One message for each place, as a function reached twice would report
/// it twice.
fn dedup(diags: &mut Vec<Diagnostic>) {
    let mut seen = std::collections::HashSet::new();
    diags.retain(|d| seen.insert((d.span.start, d.span.end, d.message.clone())));
}

/// Appends a custom section to a module.
fn custom_section(wasm: &mut Vec<u8>, name: &str, data: &[u8]) {
    let mut body = Vec::new();
    leb(&mut body, name.len() as u64);
    body.extend_from_slice(name.as_bytes());
    body.extend_from_slice(data);
    wasm.push(0);
    leb(wasm, body.len() as u64);
    wasm.extend_from_slice(&body);
}

fn leb(out: &mut Vec<u8>, mut n: u64) {
    loop {
        let byte = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(byte);
            return;
        }
        out.push(byte | 0x80);
    }
}

/// A Node.js script that runs a program compiled with [`Roots::Main`]: the
/// runtime and the module in one file, for `kiln build --target wasm`.
pub fn node_runner(module: &Module) -> String {
    let b64 = base64(&module.wasm);
    format!(
        "// Built by kiln: run with `node` (22 or later).\nconst KILN_MODULE = \"{b64}\";\n{RUNTIME_JS}\nawait runMain(KILN_MODULE);\n"
    )
}

/// The script a page with embers loads: the runtime and the bridge, which
/// fetches the module from `wasm_url`, sends calls of `server fn`s with
/// `rpc_version`, which the server checks, and loads the npm packages
/// `npm` names (each Kiln name with the URL of its module).
pub fn browser_script(wasm_url: &str, rpc_version: &str, npm: &[(String, String)]) -> String {
    let quote = |s: &str| serde_json::to_string(s).expect("a string is JSON");
    let packages: serde_json::Map<String, serde_json::Value> = npm
        .iter()
        .map(|(name, url)| (name.clone(), url.clone().into()))
        .collect();
    format!(
        "// Built by kiln: the runtime of this site's embers.\nconst KILN_WASM = {};\nconst KILN_RPC = {};\nconst KILN_NPM = {};\n{RUNTIME_JS}\n{EMBER_JS}",
        quote(wasm_url),
        quote(rpc_version),
        serde_json::Value::Object(packages)
    )
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (u32::from(c[0]) << 16)
            | (u32::from(*c.get(1).unwrap_or(&0)) << 8)
            | u32::from(*c.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= c.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}
